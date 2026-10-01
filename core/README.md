# Pacific core

The Rust core, extracted from the `pacific` monorepo so that every Pacific
deployment builds the **same** crates instead of each pinning its own revision of
the monorepo.

| crate | what it is |
|---|---|
| `pacific-wire` | the wire protocol — canonical CBOR/JSON envelopes |
| `pacific-core` | the engine: MLS groups, ed25519 identity, deltas, SQLite store |
| `pacific-media` | media description, budgeting, validation, compression (no deps, builds for wasm) |
| `pacific-geometry` | the deterministic solver engines (no LLM, no I/O, no deps) |
| `pacific-ffi` | the one language boundary — UniFFI staticlib consumed by Swift |
| `uniffi-bindgen` | the bindings generator binary |
| `media-wasm` / `viz-wasm` | browser shells over `pacific-media` / `pacific-geometry` |

`coordination/*.icd.json` are the machine-read interface contracts the conformance
tests in `pacific-core` assert against. They are part of the core's interface, so
they travel with it.

## Why this repo exists

The crates used to live in `pacific.git`, and each consumer reached into that
monorepo at a revision of its own choosing. Measured at extraction time:

| consumer | crate | pinned monorepo rev | state |
|---|---|---|---|
| `arc/planes/membership` (arc-node) | `pacific-core` | `47a9a82` | 101 files / ~39k lines behind the app |
| `arc/planes/relay` (semaphore) | `pacific-wire` | `9e64f79` | further back still |
| `arc/planes/waker` | `pacific-wire` | `9e64f79` | " |
| iOS app | `pacific-ffi` | working tree | current |

So three different cores were in flight at once, across a boundary that carries a
wire protocol — including a `SpaceID -> IdentityKey` rename that is wire-visible.

A second, quieter instance of the same problem: `pacific-core` pins
`lodedb-core` at tag `v2.0.2`, but the monorepo carried a local `[patch]` onto a
sibling checkout that actually resolved **v2.0.3**. Laptop builds and deploy
images were compiling different versions of it. That patch is deliberately absent
here — see "Local development" below.

## Release baseline

This repo sits on the code from the **current release: TestFlight 0.1.0 (109)**,
uploaded 27 Aug 2026 and `VALID` — monorepo commit `b393492`, tagged here `v0.1.0`.

There are no tags in the monorepo; the release is identified by build number, and
build numbers are the commit count on `feat/marketplace` (the release line — the
iOS `TESTFLIGHT.md` is explicit that `main` is not it). Build 109 is therefore
commit 109 on that line. The two commits that follow it there touch only Swift and
docs, so the core at the release and the core at the branch tip are byte-identical;
`filter-repo` pruned both as empty. Nothing above the release is carried here
except the manifest changes described below — no crate source differs from `b393492`.

## Provenance

Extracted from `pacific.git` with `git filter-repo`, preserving full history for
the extracted paths (45 commits, all five branches). Rewritten revisions map as:

| monorepo rev | this repo |
|---|---|
| `47a9a82d19338436402284edb138f26ca6bf7a64` | `8cce845a58c8be1ebc9a78427a3373a1b01b62c6` |
| `9e64f790afdfd163785c1416174aa66bbae10db7` | `ea07e7908d3a85faae23f9b48aa18e4a632d0305` |

The full old→new table is in `.git/filter-repo/commit-map`.

## Building

```sh
cargo check --workspace
cargo test --workspace
```

The workspace resolves standalone — no sibling checkouts required. `lodedb-core`
comes from its pinned tag; the relay used by `pacific-core`'s integration tests
comes from its pinned `arc` revision.

`Cargo.lock` still reflects the monorepo's resolution and needs one
`cargo generate-lockfile` on a networked machine to pick up the changed sources.

### Local development

To iterate against sibling checkouts instead of the pins:

```sh
cp .cargo/config.toml.example .cargo/config.toml   # untracked; edit the paths
```

`[patch]` lives there rather than in `Cargo.toml` on purpose. A patch in the
manifest is committed, so it reaches every consumer and every build image, and the
laptop build silently stops matching the deployed one — which is exactly how the
`lodedb-core` pin drifted.

## The arc cycle

`arc` depends on `pacific-core`, and `pacific-core`'s **tests** depend on arc's
`relay` (19 of 25 integration test files spin up a real relay). That cycle is safe
only because it is dev-only — dev-dependencies never enter a production build, and
`src/transport.rs`'s use of relay is `#[cfg(test)]` — and because both ends are
rev-pinned, so neither can chase the other. Bump the pin deliberately.
