#!/usr/bin/env bash
set -euo pipefail

# Builds the arc-resolver FFI as a static XCFramework — one slice per Apple target — for the
# ArcResolverKit Swift package. Built with --no-default-features so `ort` is out (it has no
# iOS/simulator prebuilt): the platform injects its embedder via the C callback
# (CallbackEmbedder). Mirrors lodedb's scripts/package_xcframework.sh.
#
#   LODE ... arc-resolver crate at ../../..  (LocalStore/graph/arc-resolver)
#   header  ../../../include/{arc_resolver.h,module.modulemap}
#   output  ../Artifacts/ArcResolverFFI.xcframework

ROOT="$(cd "$(dirname "$0")/.." && pwd)"                 # .../swift/ArcResolverKit
CRATE="$(cd "${ROOT}/../.." && pwd)"                     # .../arc-resolver
ARTIFACTS="${ROOT}/Artifacts"
HEADER_DIR="${CRATE}/include"
OUTPUT="${ARTIFACTS}/ArcResolverFFI.xcframework"

export PATH="${HOME}/.cargo/bin:${PATH}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-18.0}"

# Device, simulator, and host-mac (for `swift test` / previews). arm64-only, matching lodedb.
TARGETS="${ARC_XCFRAMEWORK_TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin}"

rm -rf "${OUTPUT}"
mkdir -p "${ARTIFACTS}"

# Namespace the header + modulemap under a module-named subdir. Without this, this static
# xcframework and lodedb's both copy a top-level `module.modulemap` into the app's shared
# `include/`, and the build fails with "Multiple commands produce include/module.modulemap".
STAGED_HEADERS="${ARTIFACTS}/.staged-headers"
rm -rf "${STAGED_HEADERS}"
mkdir -p "${STAGED_HEADERS}/ArcResolverFFI"
cp "${HEADER_DIR}/arc_resolver.h" "${STAGED_HEADERS}/ArcResolverFFI/"
cp "${HEADER_DIR}/module.modulemap" "${STAGED_HEADERS}/ArcResolverFFI/"

INSTALLED_TARGETS="$(rustup target list --installed)"
xcframework_args=()

for target in ${TARGETS}; do
  if ! printf '%s\n' "${INSTALLED_TARGETS}" | grep -qx "${target}"; then
    echo "Skipping Rust target ${target}; install it with: rustup target add ${target}" >&2
    continue
  fi

  echo ">> building libarc_resolver.a for ${target}"
  cargo build \
    --manifest-path "${CRATE}/Cargo.toml" \
    -p arc-resolver \
    --no-default-features \
    --release \
    --target "${target}"

  library="${CRATE}/target/${target}/release/libarc_resolver.a"
  if [[ ! -f "${library}" ]]; then
    echo "Expected native library was not produced: ${library}" >&2
    exit 1
  fi
  xcframework_args+=(-library "${library}" -headers "${STAGED_HEADERS}")
done

if [[ "${#xcframework_args[@]}" -eq 0 ]]; then
  echo "No native libraries were built; cannot create XCFramework" >&2
  exit 1
fi

xcodebuild -create-xcframework "${xcframework_args[@]}" -output "${OUTPUT}"
echo "Created ${OUTPUT}"
