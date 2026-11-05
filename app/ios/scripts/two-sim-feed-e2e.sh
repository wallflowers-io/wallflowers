#!/usr/bin/env bash
#  two-sim-feed-e2e.sh — the Active Scroll Feed exercised from TWO OPPOSITIONAL
#  PERSPECTIVES over the PRODUCTION relay (FEED-TRANSMISSION §14 follow-up).
#
#    SIM A = Ha-eun  (attendee / wanter)  — loads the student slice of the fixture
#    SIM B = Minho   (organiser / offerer) — loads the band slice: his group, the
#            practice room, the band events he organises, the Boss DS-1 listing
#
#  Both devices pair over wss://arc.wallflowers.io/v1/relay (override with RELAY_URL=... for a
#  local run), fold their own slices as REAL objects (FixtureLoader — same core
#  calls the UI makes), then the oppositional flow runs live:
#    · B's OFFERS listing travels to A (and A's postures to B) — hop rows, real
#    · B invites A to a band event over the pairwise Contact channel
#    · A RSVPs (contact.inviteReply — the S3 "always in RSVP" loop)
#  Exit criteria: cross-travelled listings visible in BOTH directions, and both
#  feeds screenshotted for reading (the §8 bar: screenshots read by the author).
#
#  Follows scripts/two-sim-ticketing-e2e.sh mechanics exactly: SIMCTL_CHILD_ env
#  for identity+relay, the dev-commands.txt drain (simctl openurl is dead on
#  iOS 26), convergence polling over sleeps.

set -euo pipefail
cd "$(dirname "$0")/.."

MODEL_A="${MODEL_A:-iPhone 17 Pro}"
MODEL_B="${MODEL_B:-iPhone 17}"
RELAY_URL="${RELAY_URL:-wss://arc.wallflowers.io/v1/relay}"
APP=ui/build/Build/Products/Debug-iphonesimulator/WallFlowers.app
FIXTURE=ui/Fixtures/feed-dataset-highschool.json
SHOTS=ui/build/shots
LINK_SETTLE="${LINK_SETTLE:-1.5}"
STEP=boot

step() { STEP="$1"; echo "== $1"; }
fail() { echo "✗ FAILED at [$STEP]"; exit 1; }

sim_for() { # name model -> udid
  local udid
  udid=$(xcrun simctl list devices | grep -F "$1 (" | head -1 | grep -oE '[0-9A-F-]{36}' || true)
  [ -n "$udid" ] || udid=$(xcrun simctl create "$1" "$2")
  echo "$udid"
}

dev_dir()  { echo "$(xcrun simctl get_app_container "$1" network.pacific data)/Library/Application Support/pacific"; }
dev_file() { cat "$(dev_dir "$1")/$2" 2>/dev/null || true; }
open_url() { mkdir -p "$(dev_dir "$1")"; echo "$2" >> "$(dev_dir "$1")/dev-commands.txt"; sleep "$LINK_SETTLE"; }

until_ok() { # seconds label cmd...
  local budget="$1" label="$2"; shift 2
  local t0=$(date +%s)
  until "$@"; do
    [ $(( $(date +%s) - t0 )) -lt "$budget" ] || { echo "timeout: $label"; fail; }
    sleep 1
  done
  echo "   ok: $label"
}

step "build"
if [ "${SKIP_BUILD:-0}" != "1" ] || [ ! -d "$APP" ]; then
  xcodebuild -project ui/WallFlowers.xcodeproj -scheme WallFlowers -sdk iphonesimulator \
    -destination "platform=iOS Simulator,name=$MODEL_A" -derivedDataPath ui/build build -quiet
fi
[ -d "$APP" ] || fail

step "simulators"
SIM_A="${SIM_A:-$(sim_for "E2E-Feed-A" "$MODEL_A")}"
SIM_B="${SIM_B:-$(sim_for "E2E-Feed-B" "$MODEL_B")}"
for S in "$SIM_A" "$SIM_B"; do
  xcrun simctl boot "$S" 2>/dev/null || true
  xcrun simctl bootstatus "$S" -b >/dev/null
  xcrun simctl terminate "$S" network.pacific 2>/dev/null || true
  for i in 1 2 3; do
    xcrun simctl uninstall "$S" network.pacific 2>/dev/null || true
    xcrun simctl get_app_container "$S" network.pacific >/dev/null 2>&1 || break
    sleep 1
  done
  ! xcrun simctl get_app_container "$S" network.pacific >/dev/null 2>&1 || fail
  xcrun simctl keychain "$S" reset 2>/dev/null || true
  xcrun simctl install "$S" "$APP"
  xcrun simctl privacy "$S" grant all network.pacific 2>/dev/null || true
done

step "slice-fixture"
python3 - "$FIXTURE" /tmp/feed-A.json /tmp/feed-B.json <<'EOF'
import json, sys
src, out_a, out_b = sys.argv[1:4]
data = json.load(open(src))
objs = data["objects"]

def organiser(o):
    return next((e["target"] for e in o.get("edges", []) if e["rel"] == "organised_by"), None)

b_ids = {"listing:boss-ds1", "group:band-bu", "place:hapjeong-practice"}
b_ids |= {o["id"] for o in objs if o["kind"] == "event" and organiser(o) == "group:band-bu"}

side_b = [o for o in objs if o["id"] in b_ids]
side_a = [o for o in objs if o["id"] not in b_ids]
json.dump({"objects": side_a}, open(out_a, "w"), ensure_ascii=False)
json.dump({"objects": side_b}, open(out_b, "w"), ensure_ascii=False)

hero = next(o["id"] for o in objs if o["kind"] == "event" and organiser(o) == "group:band-bu")
open("/tmp/feed-hero-event.txt", "w").write(hero)
print(f"slice A={len(side_a)} B={len(side_b)} hero={hero}")
EOF
HERO_FIXTURE_ID=$(cat /tmp/feed-hero-event.txt)

step "launch"
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Ha-eun Kang" \
  xcrun simctl launch "$SIM_A" network.pacific >/dev/null
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Minho Lee" \
  xcrun simctl launch "$SIM_B" network.pacific >/dev/null
t0=$(date +%s)
while [ -z "$(dev_file "$SIM_A" dev-contact.txt)" ] || [ -z "$(dev_file "$SIM_B" dev-contact.txt)" ]; do
  [ $(( $(date +%s) - t0 )) -lt 60 ] || { echo "timeout: identities"; fail; }
  sleep 1
done
echo "   ok: identities up"
A_SPACE=$(dev_file "$SIM_A" dev-contact.txt | sed -n 1p)
A_BUNDLE=$(dev_file "$SIM_A" dev-contact.txt | sed -n 2p)
B_SPACE=$(dev_file "$SIM_B" dev-contact.txt | sed -n 1p)
[ -n "$A_SPACE" ] && [ -n "$B_SPACE" ] || fail

enc() { python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$1"; }

step "pair"
open_url "$SIM_B" "pacific://pair-scan?bundle=$(enc "$A_BUNDLE")"; sleep 6
open_url "$SIM_A" "pacific://sync"; sleep 5
open_url "$SIM_A" "pacific://pair-accept?peer=$B_SPACE"
open_url "$SIM_B" "pacific://pair-accept?peer=$A_SPACE"
open_url "$SIM_B" "pacific://sync"
open_url "$SIM_A" "pacific://dump-profiles"
t0=$(date +%s)
while [ -z "$(dev_file "$SIM_A" dev-profiles.txt)" ]; do
  [ $(( $(date +%s) - t0 )) -lt 60 ] || { echo "timeout: pairing"; fail; }
  open_url "$SIM_A" "pacific://sync"; open_url "$SIM_A" "pacific://dump-profiles"
done
echo "   paired: A sees $(dev_file "$SIM_A" dev-profiles.txt | head -1)"

step "load-fixtures"
mkdir -p "$(dev_dir "$SIM_A")/fixtures" "$(dev_dir "$SIM_B")/fixtures"
cp /tmp/feed-A.json "$(dev_dir "$SIM_A")/fixtures/feed-A.json"
cp /tmp/feed-B.json "$(dev_dir "$SIM_B")/fixtures/feed-B.json"
open_url "$SIM_A" "pacific://load-fixture?name=feed-A"
open_url "$SIM_B" "pacific://load-fixture?name=feed-B"
t0=$(date +%s)
while [ -z "$(dev_file "$SIM_A" dev-fixture.txt)" ] || [ -z "$(dev_file "$SIM_B" dev-fixture.txt)" ]; do
  [ $(( $(date +%s) - t0 )) -lt 90 ] || { echo "timeout: fixture load"; fail; }
  sleep 2
done
echo "   A: $(dev_file "$SIM_A" dev-fixture.txt | head -1)"
echo "   B: $(dev_file "$SIM_B" dev-fixture.txt | head -1)"

step "listings-travel"   # the market gossip crossing the production relay, both ways.
# listings() NEVER contains own things (those are core.things), so ANY row here
# arrived from the peer; a direct-from-origin row reads hops=0.
t0=$(date +%s)
while :; do
  open_url "$SIM_A" "pacific://sync"; open_url "$SIM_B" "pacific://sync"
  open_url "$SIM_A" "pacific://dump-listings"; open_url "$SIM_B" "pacific://dump-listings"
  A_ROWS=$(dev_file "$SIM_A" dev-listings.txt | grep -c "|" || true)
  B_ROWS=$(dev_file "$SIM_B" dev-listings.txt | grep -c "|" || true)
  [ "$A_ROWS" -gt 0 ] && [ "$B_ROWS" -gt 0 ] && break
  [ $(( $(date +%s) - t0 )) -lt 180 ] || { echo "timeout: listings (A=$A_ROWS B=$B_ROWS)"; fail; }
  sleep 3
done
echo "   travelled listings: A sees $A_ROWS, B sees $B_ROWS"
dev_file "$SIM_A" dev-listings.txt | head -3 | sed 's/^/   A← /'
dev_file "$SIM_B" dev-listings.txt | head -3 | sed 's/^/   B← /'

step "attach-media"      # §14 loose end: media set on ALREADY-GOSSIPED objects must
# re-travel — thing.setPhoto moves the (peer,listing) digest (node.rs listing_digest
# covers the photo b64), so the next publish_market re-announces the DS-1 WITH its
# face. Exit criterion: A's travelled DS-1 row shows photo>0.
for S in "$SIM_A" "$SIM_B"; do
  mkdir -p "$(dev_dir "$S")/fixtures/media"
  cp ui/Fixtures/media/*.jpg ui/Fixtures/media/manifest.json "$(dev_dir "$S")/fixtures/media/"
done
open_url "$SIM_A" "pacific://attach-media"
open_url "$SIM_B" "pacific://attach-media"
t0=$(date +%s)
while [ -z "$(dev_file "$SIM_A" dev-media.txt)" ] || [ -z "$(dev_file "$SIM_B" dev-media.txt)" ]; do
  [ $(( $(date +%s) - t0 )) -lt 90 ] || { echo "timeout: attach-media"; fail; }
  sleep 2
done
echo "   A: $(dev_file "$SIM_A" dev-media.txt)  B: $(dev_file "$SIM_B" dev-media.txt)"

step "media-regossip"    # the digest-refresh proof, both directions
t0=$(date +%s)
while :; do
  open_url "$SIM_A" "pacific://sync"; open_url "$SIM_B" "pacific://sync"; sleep 3
  open_url "$SIM_A" "pacific://dump-listings"; open_url "$SIM_B" "pacific://dump-listings"
  A_DS1=$(dev_file "$SIM_A" dev-listings.txt | grep -F "Boss DS-1" | grep -oE 'photo=[0-9]+' | cut -d= -f2 || true)
  B_MEDIA=$(dev_file "$SIM_B" dev-listings.txt | grep -cE 'photo=[1-9]' || true)
  [ "${A_DS1:-0}" -gt 0 ] && break
  [ $(( $(date +%s) - t0 )) -lt 240 ] || { echo "timeout: DS-1 photo never re-gossiped to A (A_DS1=${A_DS1:-absent} B_rows_with_photo=$B_MEDIA)"; fail; }
  sleep 3
done
echo "   DS-1 photo on A: ${A_DS1} b64 chars; B holds $B_MEDIA travelled rows with photos"

step "invite-and-rsvp"   # B invites A to the band event; A commits GOING.
# Both legs ASSERTED via the dev-invite/dev-rsvp result drops (§8: a leg the
# harness cannot read is a guess — run 3's invites failed ok=0 for two runs
# while the step printed success).
HERO_REAL=$(dev_file "$SIM_B" dev-fixture.txt | grep "^$HERO_FIXTURE_ID=" | cut -d= -f2)
[ -n "$HERO_REAL" ] || { echo "hero event $HERO_FIXTURE_ID not in B's fixture map"; fail; }
open_url "$SIM_B" "pacific://invite?event=$HERO_REAL&peer=$A_SPACE"
t0=$(date +%s)
until [ "$(dev_file "$SIM_B" dev-invite.txt)" = "invite $HERO_REAL ok=1" ]; do
  [ $(( $(date +%s) - t0 )) -lt 30 ] || { echo "timeout: B's invite failed (got: $(dev_file "$SIM_B" dev-invite.txt))"; fail; }
  sleep 2
done
echo "   ok: B's invite authored"
open_url "$SIM_B" "pacific://sync"
open_url "$SIM_A" "pacific://sync"; sleep 4
# A can only answer once the invite has FOLDED on A (setGoing needs the channel).
# dump-events first: it chains refreshEvents→refreshInvites, which a headless
# drive otherwise never runs (the 3s tick's refresh() skips events/invites) —
# without it inviteChannels stays empty and setGoing returns false forever.
t0=$(date +%s)
while :; do
  open_url "$SIM_A" "pacific://dump-events"
  open_url "$SIM_A" "pacific://rsvp?event=$HERO_REAL"; sleep 2
  [ "$(dev_file "$SIM_A" dev-rsvp.txt)" = "rsvp $HERO_REAL ok=1" ] && break
  [ $(( $(date +%s) - t0 )) -lt 90 ] || { echo "timeout: A's RSVP never committed"; fail; }
  open_url "$SIM_A" "pacific://sync"; sleep 3
done
open_url "$SIM_A" "pacific://sync"; open_url "$SIM_B" "pacific://sync"
echo "   invited $HERO_REAL → A (ok=1), A committed GOING (ok=1)"

step "screenshots"
mkdir -p "$SHOTS"
open_url "$SIM_A" "pacific://show-life"; sleep 2
open_url "$SIM_B" "pacific://show-life"; sleep 2
xcrun simctl io "$SIM_A" screenshot "$SHOTS/feed-A.png"
xcrun simctl io "$SIM_B" screenshot "$SHOTS/feed-B.png"

echo "== DONE — read the shots: $SHOTS/feed-A.png · $SHOTS/feed-B.png"
