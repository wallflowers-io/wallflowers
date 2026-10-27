#!/usr/bin/env bash
# serve.sh — the CI runner, for as long as GitHub has jobs for it (make ci).
#
# One run per machine, its PID in $STATE/serve.pid. It starts Colima's wf-ci VM, builds the image
# (tools/ci/Dockerfile), and gives each queued job labelled wf-ci its own container holding a
# one-job (JIT) runner. After IDLE seconds with nothing queued it exits, and on every exit, red,
# Ctrl-C or kill, it deletes the container and the runner and stops the VM (its disk, holding the
# image and the build caches, is kept). A kill -9 cannot be trapped: the next run sweeps what
# that one left.
#
#   tools/ci/serve.sh        CI_IDLE=<seconds> (120)

set -euo pipefail

product="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PATH="/opt/homebrew/bin:$PATH"
REPO=wallflowers-io/wallflowers
LABEL=wf-ci
IDLE=${CI_IDLE:-120}
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/wf-ci"
pidf="$STATE/serve.pid" cidf="$STATE/job.cid" ridf="$STATE/job.runner"

say() { printf '%s  %s\n' "$(date '+%H:%M:%S')" "$*"; }
dk() { docker --context "colima-$LABEL" "$@"; }
unrun() { gh api -X DELETE "repos/$REPO/actions/runners/$1" >/dev/null 2>&1 || true; }

mkdir -p "$STATE"
if [ -f "$pidf" ]; then
  old="$(cat "$pidf")"
  if ps -p "$old" -o command= 2>/dev/null | grep -q 'serve\.sh'; then say "serving already (pid $old)"; exit 0; fi
  say "sweeping a run that did not end (pid $old)"
  gh api --paginate "repos/$REPO/actions/runners" --jq ".runners[] | select(.name | startswith(\"$LABEL-\")) | .id" \
    | while read -r id; do unrun "$id"; done
  rm -f "$pidf" "$cidf" "$ridf"
fi
( set -o noclobber; echo $$ > "$pidf" ) 2>/dev/null || { say "another run started first"; exit 0; }
# The Mac's idle sleep freezes the VM and GitHub abandons the job; a closed lid still does.
caffeinate -i -w $$ &

teardown() {
  trap - EXIT INT TERM HUP
  set +e
  [ -s "$cidf" ] && dk rm -f "$(cat "$cidf")" >/dev/null 2>&1
  [ -s "$ridf" ] && unrun "$(cat "$ridf")"
  colima stop "$LABEL" >/dev/null 2>&1 || true
  rm -f "$cidf" "$ridf" "$pidf"
  say "torn down"
}
trap teardown EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# Jobs for this label not yet taken. A deploy's job waiting on its reviewers is not counted.
queued() {
  { gh api "repos/$REPO/actions/runs?status=queued&per_page=50" --jq '.workflow_runs[].id' || true
    gh api "repos/$REPO/actions/runs?status=in_progress&per_page=50" --jq '.workflow_runs[].id' || true; } 2>/dev/null \
  | while read -r run; do
      gh api "repos/$REPO/actions/runs/$run/jobs?per_page=100" \
        --jq "[.jobs[] | select((.status == \"queued\" or .status == \"pending\") and (.labels | index(\"$LABEL\")))] | length" \
        2>/dev/null || true
    done | awk '{n += $1} END {print n + 0}'
}

say "starting the VM"
colima start "$LABEL" --vm-type vz --cpu 12 --memory 24 --disk 100 --mount none --activate=false >"$STATE/colima.log" 2>&1 \
  || { say "the VM did not start: $STATE/colima.log"; exit 1; }
ids="$(dk ps -aq --filter "label=$LABEL")"; [ -z "$ids" ] || dk rm -f $ids >/dev/null
say "building the image"
dk buildx build -q -t "$LABEL" --build-context auth="$product/arc/auth" --build-context github="$product/.github" \
  "$product/tools/ci" >/dev/null

idle_since=$(date +%s) bad=0
while :; do
  if [ "$(queued)" -gt 0 ]; then
    { read -r cfg; read -r rid; } < <(gh api -X POST "repos/$REPO/actions/runners/generate-jitconfig" \
      -f name="$LABEL-$(date +%s)" -F runner_group_id=1 -f 'labels[]=self-hosted' -f "labels[]=$LABEL" \
      -f work_folder=_work --jq '.encoded_jit_config, .runner.id')
    echo "$rid" > "$ridf"; rm -f "$cidf"
    say "runner $rid"
    # In the background, so a signal reaches the trap now and not when the job ends.
    dk run --rm --label "$LABEL" --cidfile "$cidf" \
      -v "$LABEL-work:/home/runner/actions-runner/_work" -v "$LABEL-cargo:/home/runner/.cargo/registry" \
      "$LABEL" --jitconfig "$cfg" &
    if wait $!; then bad=0; else bad=$((bad + 1)); fi
    unrun "$rid"; rm -f "$ridf" "$cidf"
    # A job's red is the job's; a runner that will not run is ours.
    [ "$bad" -lt 3 ] || { say "the runner failed three times running"; exit 1; }
    idle_since=$(date +%s)
  elif [ $(( $(date +%s) - idle_since )) -ge "$IDLE" ]; then
    say "idle ${IDLE}s"; exit 0
  else
    sleep 10
  fi
done
