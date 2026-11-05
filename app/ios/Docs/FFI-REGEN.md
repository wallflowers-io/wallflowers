# The FFI boundary: what regenerates, and why you no longer run it

**Short version: you don't.** `just build`, `just test` and
`just run` regenerate everything they need. This page exists so that when
something looks impossible, you can find out why in one read.

## What "the bindings" are

One language boundary, between the Rust core and the Swift UI:

```
  SwiftUI app  ──imports──▶  PacificCore        (hand-written wrapper)
                                  ▲
                             UniFFI             apple/Sources/UniFFI/pacific_ffi.swift
                                  ▲                 ~13,000 generated lines, COMMITTED
                             PacificCoreRS      apple/Artifacts/…xcframework (device + sim)
                                  ▲
                             pacific-ffi        #[uniffi::export] facade
                                  ▲
                             pacific-core       the engine
```

`apple/build-ios.sh` produces the middle two:

1. compile `pacific-ffi` release for `aarch64-apple-ios-sim` **and** `aarch64-apple-ios`,
   each with its own `SDKROOT` (rusqlite's bundled SQLite C is cross-compiled per slice,
   so one global `SDKROOT` would mis-compile one of them);
2. run `uniffi-bindgen` **against the compiled `.a`** — `uniffi::setup_scaffolding!()`
   embeds metadata in the object file, so the bindings describe what *actually built*,
   not what the source appears to say;
3. lay out headers under a nested `pacific_ffiFFI/` (two static-lib xcframeworks both
   flatten headers into `Products/include/`, and a bare `module.modulemap` at the root
   collides), and copy the generated Swift into `Sources/UniFFI/`;
4. `xcodebuild -create-xcframework` over both slices.

## Why this used to be a trap

`pacific_ffi.swift` is **committed**. Xcode has no idea it is derived and no rule to
rebuild it, so a Rust change without a regen compiles cleanly against yesterday's
bindings. The symptom is `cannot find type X in scope` for a type you are looking at
in the Rust source — which reads like a Swift problem and is not one.

The same shape bit twice more:

| forgotten | symptom |
|---|---|
| `ffi` | `cannot find type X in scope`, for a type that plainly exists |
| `generate` | `Build input file cannot be found`, or a new test that silently never runs — xcodegen GLOBS sources, so a new file is invisible until the project is rewritten |
| `cargo` not on PATH | `exit code 127`. `~/.cargo/bin` is not on a non-login shell's PATH; the script now resolves cargo itself and honours `$CARGO` |

## Why it is safe to always run

Both steps are **fingerprinted by the tool underneath**, so with nothing changed they
are genuine no-ops:

```
generate   0.08s     xcodegen
ffi        1.5s      cargo decides whether anything needs compiling
```

That is the whole reason there is no stamp file, no hash of the source tree, and no
staleness heuristic: cargo already solves this problem correctly, and a second
mechanism guessing at the same question is a mechanism that can be wrong.

Run `just ffi` directly only to force a rebuild.

## The one habit worth keeping

A regen is a **reviewable diff**. `git diff apple/Sources/UniFFI` is an honest record
of exactly what the FFI boundary gained or lost, generated from the compiled artifact —
a better changelog than anything hand-maintained. If that diff surprises you, the
boundary changed in a way you did not intend.
