#!/bin/sh
# job-started.sh — before a job's first step (ACTIONS_RUNNER_HOOK_JOB_STARTED); root's, read-only.
#
# GitHub, not the harness, picks which queued job a runner takes, so a job this container was not
# made for fails here, before any of its code runs: a fork, a pull request, an actor outside
# CI_TRUSTED, or, in a deploy container (its own caches and the deploy key), any ref but main.
# The harness sets CI_KIND and CI_TRUSTED; the runner sets the rest from the job it took.
set -eu
fail() { echo "job-started: not this container's job: $*" >&2; exit 1; }
[ "${GITHUB_REPOSITORY:-}" = wallflowers-io/wallflowers ] || fail "repository ${GITHUB_REPOSITORY:-}"
case "${GITHUB_EVENT_NAME:-}" in push|workflow_dispatch) ;; *) fail "event ${GITHUB_EVENT_NAME:-}" ;; esac
actor="${GITHUB_TRIGGERING_ACTOR:-${GITHUB_ACTOR:-}}"
[ -n "$actor" ] || fail "no actor"
case " ${CI_TRUSTED:-} " in *" $actor "*) ;; *) fail "actor $actor" ;; esac
head="$(jq -r '.repository.full_name // empty' "${GITHUB_EVENT_PATH:-/dev/null}" 2>/dev/null || true)"
[ "$head" = "$GITHUB_REPOSITORY" ] || fail "head repository $head"
if [ "${CI_KIND:-}" = deploy ]; then
  [ "${GITHUB_REF:-}" = refs/heads/main ] || fail "a deploy from ${GITHUB_REF:-}"
fi
