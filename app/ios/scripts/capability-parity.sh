#!/usr/bin/env bash
# capability-parity.sh — the "no orphaned capability" gate.
#
# A capability the user cannot reach is a silent fallback in the PRODUCT: the
# backend appears to work while the front quietly drops it. This is the UI twin of
# the no-silent-fallbacks rule, enforced mechanically instead of by discipline.
#
# It fails the build when:
#   1. NO ORPHAN — a capability declared `userFacing` has no caller in a real View
#      (any .swift under ui/App EXCEPT DevLinks.swift and CoreService.swift).
#      DevLinks (the pacific:// dev seam) and CoreService (the definition site) do
#      NOT count as user reach.
#   2. NO SILENT ADDITION — a `projectXxx` DeltaDraft op exists in the generated
#      FFI bindings but is not classified in the manifest below. A new backend op
#      nobody decided a UI fate for breaks CI, so the gap cannot silently reopen.
#
# The manifest is the source of truth for "what a user must be able to do".
# Categories: userFacing (needs a View caller) · systemOnly (an agent/CLA op with
# no direct user control by design) · devOnly (a host-harness seam, DevLink-only).
set -uo pipefail
cd "$(dirname "$0")/.."

UI_DIR="ui/App"
EXCLUDE='CoreService\.swift|Connect/DevLinks\.swift'
FFI="apple/Sources/UniFFI/pacific_ffi.swift"

# "symbol:category" — bash 3.2 compatible (macOS ships no associative arrays).
MANIFEST=(
  # identity / pairing (first-class in NewChatSheet / Me / ChatView)
  "contactBundle:userFacing"
  "pairScan:userFacing"
  "pairAccept:userFacing"
  # messaging (Chats)
  "createGroupConversation:userFacing"
  "sendToGroup:userFacing"
  "sendToDM:userFacing"
  "react:userFacing"
  "reply:userFacing"
  "syncNow:userFacing"
  # objects (Work / ChatRoom)
  "newProject:userFacing"
  "objectThread:userFacing"
  "objectMembers:userFacing"
  "groupAddMember:userFacing"
  # project delta ops (the 12 FFI DeltaDraft constructors)
  "projectAddTimelineItem:userFacing"
  "projectSetItemProgress:userFacing"
  "projectConfigure:userFacing"
  "projectSetItemField:userFacing"
  "projectSetItemSchedule:userFacing"
  "projectSetAssignee:userFacing"
  "projectAddDependency:userFacing"
  "projectRemoveDependency:userFacing"
  "projectSubscribe:userFacing"
  "projectUnsubscribe:userFacing"
  "projectSuppressItem:userFacing"
  "projectTouchSubscription:systemOnly"   # CLA/agent op — no user control by design
  # objectives (Tab 1) — the Objectives editor wires these
  "projectSetObjective:userFacing"
  "projectSetRole:userFacing"
  "projectAddStakeholder:userFacing"
  "projectRemoveStakeholder:userFacing"
  "projectAddLocation:userFacing"
  "projectRemoveLocation:userFacing"
  "projectSetKpi:userFacing"
  "projectRemoveKpi:userFacing"
  # dev / harness seams (intentionally DevLink-only, NOT user capabilities)
  "seedDmDemo:devOnly"
  "seedProjectDemo:devOnly"
  "reexportContact:devOnly"
  "exportProjectId:devOnly"
  "exportRoomId:devOnly"
  "dumpBoard:devOnly"
  "dumpThread:devOnly"
)

category_of() {  # $1 = symbol → prints its category, or empty if unlisted
  local e
  for e in "${MANIFEST[@]}"; do
    [ "${e%%:*}" = "$1" ] && { echo "${e#*:}"; return; }
  done
}

fail=0

echo "— 1. no orphan: every userFacing capability has a View caller —"
for e in "${MANIFEST[@]}"; do
  [ "${e#*:}" = userFacing ] || continue
  sym="${e%%:*}"
  hits=$(grep -rlE "\b${sym}[[:space:]]*\(" "$UI_DIR" --include='*.swift' 2>/dev/null | grep -vE "$EXCLUDE" || true)
  if [ -z "$hits" ]; then
    echo "  ✗ ORPHAN  $sym — built, but no View calls it (DevLinks/CoreService don't count)"
    fail=1
  fi
done
[ "$fail" = 0 ] && echo "  ✓ all userFacing capabilities reachable"

echo "— 2. no silent addition: every FFI project op is classified —"
undeclared=0
while read -r sym; do
  if [ -z "$(category_of "$sym")" ]; then
    echo "  ✗ UNDECLARED  $sym — exists in the FFI but not in the parity manifest"
    fail=1; undeclared=1
  fi
done < <(grep -oE 'public func project[A-Za-z]+\(' "$FFI" | sed -E 's/public func ([A-Za-z]+)\(/\1/' | sort -u)
[ "$undeclared" = 0 ] && echo "  ✓ every project op declared"

echo
if [ "$fail" = 0 ]; then
  echo "✓ capability parity: every built capability has a user-reachable control."
else
  echo "✗ capability parity FAILED — an op is built but unreachable (see above)."
fi
exit $fail
