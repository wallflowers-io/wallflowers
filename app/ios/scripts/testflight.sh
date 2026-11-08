#!/usr/bin/env bash
#
# Pacific TestFlight upload.
#
#   scripts/testflight.sh            # sync build number, archive Release for device, upload
#
# Auth = App Store Connect API key: ~/.appstoreconnect/private_keys/AuthKey_<APPLE_KEY_ID>.p8
# with APPLE_KEY_ID and APPLE_ISSUER_ID read from env/.env. Signing is automatic
# cloud signing on the paid team — -allowProvisioningUpdates lets xcodebuild mint/refresh the
# store profiles and the cloud-managed Apple Distribution certificate headlessly.
#
# The upload happens inside -exportArchive (ExportOptions.plist destination=upload). A build
# then takes ~5–15 min of Apple-side processing before it appears under TestFlight, where it
# waits for the per-build export-compliance answer (deliberately unset in project.yml — see
# the EXPORT COMPLIANCE note there).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"      # pacific/
UI="$REPO/ui"
ARCHIVE="$UI/build-archive/WallFlowers.xcarchive"

# env/.env did not survive the move into the monorepo, so it is read when it is
# there and the variables may come from the environment instead; the checks
# below still refuse if they are missing either way.
if [[ -f "$REPO/env/.env" ]]; then set -a; source "$REPO/env/.env"; set +a; fi
KEY_ID="${APPLE_KEY_ID:?APPLE_KEY_ID not set (env/.env)}"
KEY_FILE="$HOME/.appstoreconnect/private_keys/AuthKey_${KEY_ID}.p8"
[[ -f "$KEY_FILE" ]]              || { echo "missing ASC API key: $KEY_FILE" >&2; exit 1; }
[[ -n "${APPLE_ISSUER_ID:-}" ]]   || { echo "APPLE_ISSUER_ID not set (env/.env)" >&2; exit 1; }

AUTH=(-allowProvisioningUpdates
      -authenticationKeyPath "$KEY_FILE"
      -authenticationKeyID "$KEY_ID"
      -authenticationKeyIssuerID "$APPLE_ISSUER_ID")

"$REPO/scripts/version.sh" set-build          # build number = git commit count + regenerate

rm -rf "$ARCHIVE"
xcodebuild -project "$UI/WallFlowers.xcodeproj" -scheme WallFlowers \
  -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE" \
  archive "${AUTH[@]}"

xcodebuild -exportArchive \
  -archivePath "$ARCHIVE" \
  -exportOptionsPlist "$UI/ExportOptions.plist" \
  -exportPath "$UI/build-archive/export" \
  "${AUTH[@]}"

echo "→ uploaded $("$REPO/scripts/version.sh" show | head -1 | awk '{print $NF}') ($(git -C "$REPO" rev-list --count HEAD)) — appears in App Store Connect → TestFlight after processing"
