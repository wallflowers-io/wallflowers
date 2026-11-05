#!/usr/bin/env bash
# Fetch upstream LodeDB at a pinned tag into target/lodedb and build its
# LodeDBCoreFFI.xcframework. Upstream publishes no xcframework, so SwiftPM cannot
# resolve LodeDBCore by URL. No-op when the pinned tag is already built.
set -euo pipefail

TAG="v2.1.0"   # same upstream as core/pacific-core's lodedb-core and arc-resolver's lodedb-graph
URL="https://github.com/Egoist-Machines/LodeDB.git"

HERE="$(cd "$(dirname "$0")" && pwd)"
DIR="$(cd "$HERE/.." && pwd)/target/lodedb"
XCF="$DIR/swift/LodeDBCore/Artifacts/LodeDBCoreFFI.xcframework"

if [ -d "$XCF" ] && [ "$(git -C "$DIR" describe --tags --exact-match 2>/dev/null)" = "$TAG" ]; then
  exit 0
fi

# cargo/rustup live in ~/.cargo/bin, which is not on PATH for a non-login shell.
export PATH="$HOME/.cargo/bin:$PATH"

rm -rf "$DIR"
git clone -q --depth 1 --branch "$TAG" "$URL" "$DIR"
# Device + simulator for the app; the host slice for the script's own `swift build`.
LODEDB_XCFRAMEWORK_TARGETS="aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin" \
LODEDB_XCFRAMEWORK_REQUIRE_ALL=1 \
  "$DIR/swift/LodeDBCore/scripts/package_xcframework.sh"
