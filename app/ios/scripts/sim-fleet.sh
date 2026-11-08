#!/bin/bash
# sim-fleet — the three-sim UI-iteration loop (11 Aug; retargeted 15 Aug).
#
# Identities delivered by DRIVING THE REAL
# FIRST-BOOT SCREENS through XCUITest (FleetIdentityUITests) — the same path a
# user walks: door → name+mint → face → bio → recovery key → fingerprint.
#
# It used to ride the launch environment instead (PACIFIC_IDENTITY_NAME plus
# PACIFIC_IDENTITY_COLOR/NOTE, added 11 Aug), because first boot was ONE field and
# there was no UI path to a complete profile — nothing to drive. The sequence
# closed that gap and those two extra variables are gone;
# PACIFIC_IDENTITY_NAME survives only as a CI skip for suites that are not about
# onboarding. The point is not tidiness: a bug in the app's own profile write now
# FAILS the fleet instead of being routed around by it.
#
#   scripts/sim-fleet.sh fresh     wipe app data on all three, build, install, identify, verify
#   scripts/sim-fleet.sh deploy    build → reinstall → identify → verify
#   scripts/sim-fleet.sh identify  re-walk first boot on all three (no build)
#   scripts/sim-fleet.sh verify    prove every sim runs THIS commit's build
#   scripts/sim-fleet.sh shot DIR  screenshot all three
#
# WHY THIS EXISTS (11 Aug, the "pro max is ancient" bug): two traps.
#   1. THE PATH. ~90 `Pacific-*` DerivedData dirs have accumulated from months of
#      worktrees; `ls -dt` guesses among them and can hand you a stale bundle.
#      The only correct source is xcodebuild's own BUILT_PRODUCTS_DIR for THIS
#      project — asked for, never inferred.
#   2. THE DATA. `simctl install` replaces the BUNDLE and leaves the DATA
#      container untouched, so an old fold/identity/UserDefaults state survives
#      every redeploy and reads as an ancient build. `uninstall` takes the data
#      container with it — that, not `erase`, is the fresh install.
#   NEVER `simctl erase`: it also destroys the device's on-demand OS assets, and
#   NLContextualEmbedding is one of them — WallFlowersApp.init fatalErrors when the
#   union embedder cannot be built, so an erased sim crash-loops until Apple
#   re-downloads the asset. (Learned the hard way, 11 Aug.)
# And the proof: every deploy ends by reading CFBundleVersion back OFF each sim
# and comparing it to the git commit count. A silent partial deploy is now loud.

set -euo pipefail
cd "$(dirname "$0")/.."
REPO="$PWD"
BUNDLE=network.pacific
UI="$REPO/ui"

# udid | device | name | color | note (percent-encoded) | city
#
# COLOUR is tapped as a SWATCH on the FACE step, so it has to be one of the seven
# in CoreService.avatarSwatches — a colour that is not is left at the key-derived
# default rather than silently applied by a back door. That constraint is the
# feature: the fleet can only give itself what a user can give themselves.
FLEET=(
  "4244D0F1-C55A-4ADD-AE15-06ACD9C1627D|iPhone 17 Pro|Dare Balogun|E7A83C|NTS%20resident%20DJ.%20Potential%20CMO.|London"
  "7CCD31D2-6A91-4DEB-8A88-752B9996B93D|iPhone 17 Pro Max|Yanis Sabir|33C08C|Founder%2C%20DIGITAL%20AF.%20Bridging%20art%20%26%20tech%20through%20cultural%20programming.|Seoul"
  "E36E02F6-A025-4664-B47C-FE1FD2C308EA|iPhone 17|Archie Waterworth|7A6FF0|Cultural%20coordinator.%20Runs%20events%20in%20London.|London"
)

# Where build-for-testing leaves the app + the .xctestrun the identify step runs.
DD="$UI/build/dd"

expected_build() { git -C "$REPO" rev-list --count HEAD; }

# THE canonical product path. build-for-testing puts BOTH halves the fleet needs
# — the app and the UI-test runner — under one derived-data root we own, so this
# is a lookup rather than a guess among ~90 stale `Pacific-*` dirs.
app_path() {
  local p; p=$(find "$DD/Build/Products" -name 'WallFlowers.app' -not -path '*UITests*' 2>/dev/null | head -1)
  [ -n "$p" ] || { echo "no built WallFlowers.app (run a build first)" >&2; return 1; }
  echo "$p"
}

xctestrun_path() {
  local p; p=$(ls -t "$DD/Build/Products"/*.xctestrun 2>/dev/null | head -1)
  [ -n "$p" ] || { echo "no .xctestrun (run a build first)" >&2; return 1; }
  echo "$p"
}

# BUILD-FOR-TESTING, not plain build: the identify step drives the real first-boot
# screens through XCUITest, so the runner has to be built too.
build() {
  echo "building $(git -C "$REPO" log --oneline -1)"
  (cd "$UI" && xcodebuild build-for-testing -project WallFlowers.xcodeproj -scheme WallFlowers \
      -destination 'generic/platform=iOS Simulator' -derivedDataPath build/dd ARCHS=arm64 2>&1 \
    | grep -E "BUILD (SUCCEEDED|FAILED)|error:" | tail -3)
}

boot_all() {
  for row in "${FLEET[@]}"; do IFS='|' read -r udid _ _ _ _ <<<"$row"
    xcrun simctl bootstatus "$udid" -b >/dev/null 2>&1 || xcrun simctl boot "$udid" 2>/dev/null || true
  done
  open -a Simulator
}

# Install + identify one device. Both paths uninstall first: that removes the
# app's DATA container as well as the bundle, which is the actual fresh install.
# Install one device. UNINSTALL FIRST: that removes the app's DATA container as
# well as the bundle, which is the actual fresh install — and it is what leaves
# the device with no identity, so the boot sequence opens at its door.
install_one() {
  local row="$1"
  IFS='|' read -r udid dev name color note city <<<"$row"
  local app; app=$(app_path)

  xcrun simctl terminate "$udid" "$BUNDLE" 2>/dev/null || true
  xcrun simctl uninstall "$udid" "$BUNDLE" 2>/dev/null || true
  # WAIT FOR THE DATA CONTAINER TO ACTUALLY GO (15 Aug). `uninstall` returns before
  # the container is removed, and an `install` that lands inside that window leaves
  # the OLD data in place — app reinstalled, identity still there, boot sequence
  # skipped, and the fleet silently keeps yesterday's identities. Ask the question
  # directly instead of sleeping on a guess.
  local waited=0
  while xcrun simctl get_app_container "$udid" "$BUNDLE" data >/dev/null 2>&1; do
    sleep 0.5; waited=$((waited+1))
    [ "$waited" -lt 40 ] || { echo "STUCK   $dev — data container still present after 20s"; return 1; }
  done
  xcrun simctl install "$udid" "$app"
  echo "installed $dev"
}

# Walk the REAL first-boot sequence on one device, as the persona. This is the
# whole point of the retarget: name, colour, note and city land through the app's
# own screens and its own card-edit path, so the fleet exercises what a user does.
identify_one() {
  local row="$1"
  IFS='|' read -r udid dev name color note city <<<"$row"
  local xctestrun; xctestrun=$(xctestrun_path)
  local log; log="$(mktemp -t "fleet-$(echo "$name" | tr ' ' '-')")"

  # `|| rc=$?` is load-bearing under `set -e`: without it a failing runner aborts
  # the WHOLE script at the subshell, before the line that would have said which
  # device failed and where its log is. A fleet that dies silently is worse than
  # one that fails loudly on one device and carries on.
  local rc=0
  local stamp; stamp=$(mktemp); : > "$stamp"; sleep 1
  ( cd "$UI" && \
    TEST_RUNNER_FLEET_NAME="$name" \
    TEST_RUNNER_FLEET_COLOR="$color" \
    TEST_RUNNER_FLEET_NOTE="$(python3 -c 'import sys,urllib.parse;print(urllib.parse.unquote(sys.argv[1]))' "$note")" \
    TEST_RUNNER_FLEET_CITY="$city" \
    xcodebuild test-without-building -xctestrun "$xctestrun" \
      -destination "id=$udid" \
      -only-testing:WallFlowersUITests/FleetIdentityUITests > "$log" 2>&1 ) || rc=$?

  # Liveness = "did it crash", asked directly. `simctl spawn launchctl list` is
  # unreliable in the seconds after a launch (false "dead", 11 Aug twice), so
  # instead watch the crash reporter: a NEW Pacific .ips after our launch is the
  # only unambiguous failure signal.
  local newest; newest=$(ls -t "$HOME/Library/Logs/DiagnosticReports"/Pacific*.ips 2>/dev/null | head -1)
  if [ -n "$newest" ] && [ "$newest" -nt "$stamp" ]; then
    echo "CRASHED $dev  →  $name  ($(basename "$newest"))"
  elif [ "$rc" -ne 0 ]; then
    echo "FAILED  $dev  →  $name  (log: $log)"
  else
    echo "live    $dev  →  $name  ($color · $city)"
  fi
}

verify() {
  local want; want=$(expected_build); local ok=0 bad=0
  for row in "${FLEET[@]}"; do IFS='|' read -r udid dev name _ _ _ <<<"$row"
    local c v
    c=$(xcrun simctl get_app_container "$udid" "$BUNDLE" app 2>/dev/null || true)
    if [ -z "$c" ]; then echo "MISSING $dev — not installed"; bad=$((bad+1)); continue; fi
    v=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$c/Info.plist" 2>/dev/null || echo "?")
    if [ "$v" = "$want" ]; then echo "OK      $dev  build $v  ($name)"; ok=$((ok+1))
    else echo "STALE   $dev  build $v  (expected $want)"; bad=$((bad+1)); fi
  done
  echo "--- $ok current, $bad stale (expected build $want) ---"
  [ "$bad" -eq 0 ]
}

shot() {
  local dir="${1:-/tmp}"; mkdir -p "$dir"
  for row in "${FLEET[@]}"; do IFS='|' read -r udid _ name _ _ _ <<<"$row"
    local f="$dir/$(echo "$name" | tr ' ' '-').png"
    xcrun simctl io "$udid" screenshot "$f" >/dev/null 2>&1 && echo "shot    $f"
  done
}

identify_all() { for row in "${FLEET[@]}"; do identify_one "$row"; done; }

case "${1:-}" in
  fresh|deploy)
    boot_all; build
    for row in "${FLEET[@]}"; do install_one "$row"; done
    identify_all
    verify ;;
  identify) boot_all; identify_all ;;
  build)  build ;;
  verify) verify ;;
  shot)   shot "${2:-/tmp}" ;;
  *) echo "usage: sim-fleet.sh fresh|deploy|identify|build|verify|shot [dir]"; exit 1 ;;
esac
