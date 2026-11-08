#!/usr/bin/env bash
# Build PacificCoreRS.xcframework + UniFFI Swift bindings from pacific-ffi.
#
# This machine is an Apple Silicon Mac, simulator-only, so we build the ONE slice
# the booted simulator needs: aarch64-apple-ios-sim. On an Intel host you'd use
# x86_64-apple-ios instead; for a device build add aarch64-apple-ios and lipo the
# sim slices together — see ../Docs/IOS-SETUP.md §5. Nothing else changes.
#
# YOU DO NOT NEED TO RUN THIS. `just build|test|run` depend on it, and with
# nothing changed it costs ~1.5s: the two cargo builds below are fingerprinted, so a
# no-op run is a no-op. Run it directly only to force a rebuild.
#
# What it produces, and why forgetting it used to hurt:
#   apple/Sources/UniFFI/pacific_ffi.swift   the generated Swift bindings — COMMITTED,
#                                            so Xcode treats them as ordinary source and
#                                            will compile against a stale copy without
#                                            complaint ("cannot find type X in scope").
#   apple/Artifacts/PacificCoreRS.xcframework  device + simulator slices.
#
# Note that step 2 generates from the COMPILED .a, not from the Rust source: uniffi
# embeds its metadata in the object file. So the bindings describe what actually built.
set -euo pipefail

# Resolve cargo ourselves. It lives in ~/.cargo/bin, which is NOT on PATH for a
# non-login shell — so `just`, hooks and CI all ran this and got exit 127 on a bare
# `cargo`. The justfile already carries the same fallback; this keeps the script
# runnable on its own. $CARGO wins if the caller set it (rustup shims, CI).
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
command -v "$CARGO" >/dev/null 2>&1 || CARGO="$(command -v cargo || true)"
if [ -z "$CARGO" ]; then
  echo "error: cargo not found. Install Rust, or set CARGO=/path/to/cargo." >&2
  exit 1
fi
# Nested tools (uniffi-bindgen resolves rustc) need the toolchain on PATH too.
export PATH="$(dirname "$CARGO"):$PATH"

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

# ── THE CORE IS THE REPO'S core/ ─────────────────────────────────────────────
# A member of the root workspace (#14), whose target/ is one directory up.
# Overridable for a worktree:  PACIFIC_CORE=/path/to/core apple/build-ios.sh
# The Swift imports `pacific_ffi`, generated from whatever .a this built, so a
# different core changes the bindings' contents and not one import.
CORE="${PACIFIC_CORE:-$(cd "$ROOT/../../core" 2>/dev/null && pwd || true)}"
if [ -z "$CORE" ] || [ ! -f "$CORE/pacific-ffi/Cargo.toml" ]; then
  echo "error: no core at '${CORE:-<unset>}' (no pacific-ffi/Cargo.toml)." >&2
  echo "       set PACIFIC_CORE=/path/to/core" >&2
  exit 1
fi
if [ ! -f "$CORE/pacific-core/src/wallet.rs" ]; then
  # A canary, not a version check: wallet.rs is absent from the stale pacific-* forks.
  echo "error: '$CORE' looks like a stale pacific-* fork, not the canonical core." >&2
  echo "       (no pacific-core/src/wallet.rs)" >&2
  exit 1
fi

CRATE="pacific_ffi"
LIB="lib${CRATE}.a"
SIM_TARGET="aarch64-apple-ios-sim"
DEV_TARGET="aarch64-apple-ios"
# Intermediates stay in the iOS tree; the cargo target dir is the root workspace's, so
# a slice built here is the same one `cargo build` at the root would produce.
OUT="$ROOT/target/ios"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cd "$CORE/.." && pwd)/target}"

export IPHONEOS_DEPLOYMENT_TARGET=18.0
cd "$CORE"

# rusqlite's bundled SQLite C is cross-compiled per slice, so SDKROOT must MATCH the
# slice: iphonesimulator for the sim, iphoneos for a physical device. A single global
# SDKROOT would mis-compile one of them.
echo "→ [1/4] compiling $CRATE from $CORE for ${SIM_TARGET} + ${DEV_TARGET}…"
SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)" "$CARGO" build -p pacific-ffi --release --target "$SIM_TARGET"
SDKROOT="$(xcrun --sdk iphoneos --show-sdk-path)"        "$CARGO" build -p pacific-ffi --release --target "$DEV_TARGET"
SIM_LIB="$TARGET_DIR/$SIM_TARGET/release/$LIB"
DEV_LIB="$TARGET_DIR/$DEV_TARGET/release/$LIB"
LIBPATH="$SIM_LIB"   # UniFFI bindings are identical for both slices; generate from the sim one

echo "→ [2/4] generating UniFFI Swift bindings from the compiled .a…"
GEN="$OUT/gen"; rm -rf "$GEN"; mkdir -p "$GEN"
"$CARGO" run -p uniffi-bindgen -- generate \
  --library "$LIBPATH" --language swift --out-dir "$GEN"

echo "→ [3/4] laying out headers (nested per-module so multiple FFI xcframeworks don't collide)…"
# Nest under a per-module subdir: two static-lib xcframeworks (PacificCoreRS + LodeDBCoreFFI)
# both flatten their Headers into Products/include/, so a bare module.modulemap at the root
# collides ("Multiple commands produce include/module.modulemap"). Nesting keeps each map at
# include/pacific_ffiFFI/module.modulemap; SwiftPM still resolves `import pacific_ffiFFI`.
HDIR="pacific_ffiFFI"
HEADERS="$OUT/headers"; rm -rf "$HEADERS"; mkdir -p "$HEADERS/$HDIR"
cp "$GEN"/*.h "$HEADERS/$HDIR/"
cat "$GEN"/*FFI.modulemap > "$HEADERS/$HDIR/module.modulemap"
# the generated Swift becomes the UniFFI package target's source
mkdir -p "$HERE/Sources/UniFFI"
rm -f "$HERE/Sources/UniFFI"/*.swift
cp "$GEN"/*.swift "$HERE/Sources/UniFFI/"

echo "→ [4/4] assembling PacificCoreRS.xcframework (device + simulator slices)…"
rm -rf "$HERE/Artifacts/PacificCoreRS.xcframework"
mkdir -p "$HERE/Artifacts"
xcodebuild -create-xcframework \
  -library "$SIM_LIB" -headers "$HEADERS" \
  -library "$DEV_LIB" -headers "$HEADERS" \
  -output "$HERE/Artifacts/PacificCoreRS.xcframework"

echo "✓ $HERE/Artifacts/PacificCoreRS.xcframework"
echo "✓ $HERE/Sources/UniFFI/ ($(ls "$HERE/Sources/UniFFI" | tr '\n' ' '))"
