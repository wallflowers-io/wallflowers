# WallFlowers — one entry point: the checks and the four deploys.
#
# Every target DELEGATES to the script that does the work; none reimplements a build,
# because a second way to build is a second thing to be wrong. CI calls these targets
# (.github/workflows/): ci.yml the checks on every pull request, deploy.yml the deploys.
#
# Usage: `make` for the list.

SHELL      := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c
# This file lives at the root of the repository that holds app, arc and core. Its parent
# is the workspace, where a website checkout may sit under business/ for the checks that
# compare against it; nothing here needs one.
PRODUCT    := $(shell cd $(dir $(lastword $(MAKEFILE_LIST))) && pwd)
CORE       := $(PRODUCT)/core
WS         := $(abspath $(PRODUCT)/..)
REPOS      := $(notdir $(PRODUCT))
export PATH := $(HOME)/.cargo/bin:$(PATH)
# The workspace's .venv where there is one; otherwise python3 (CI).
PY         ?= $(if $(wildcard $(WS)/.venv/bin/python),$(WS)/.venv/bin/python,python3)
PINNED     := $(PRODUCT)/tools/pinned.sh
# The Rust workspace's members under one directory, as -p flags: read from Cargo.toml's
# members, so a crate added there is tested here. $(call PKGS,core)
PKGS = $(shell cd $(PRODUCT) && PATH='$(PATH)' cargo metadata --no-deps --format-version 1 | $(PY) -c 'import json,sys; r=sys.argv[1]; print(" ".join("-p "+p["name"] for p in json.load(sys.stdin)["packages"] if p["manifest_path"].startswith(r)))' $(PRODUCT)/$(1)/)
ARC_HOST   := https://arc.wallflowers.io

.DEFAULT_GOAL := help
.PHONY: help status check check-icd check-core check-arc check-auth check-harness check-door e2e e2e-browser ci ci-hooks \
        wasm serve stop deploy-app deploy-door deploy-arc deploy-docs live site-setup

help:  ## this list
	@awk 'BEGIN{FS=":.*## "} /^[a-z][a-z0-9-]*:.*## /{printf "  \033[1m%-14s\033[0m %s\n",$$1,$$2}' $(MAKEFILE_LIST)

# ── provenance ──────────────────────────────────────────────────────────────
# The question this repository could not answer on 18 Sep: what is deployed?
# It took dating binaries by mtime and correlating against git log, because
# nothing recorded it. Three columns, because three things can disagree: what is
# in the tree, what is in the remote, and what is on the box.

status:  ## what is committed, what is pushed, what is live
	@printf '\n  %-18s %-9s %-9s %-7s %s\n' REPO LOCAL REMOTE DIRTY ORIGIN
	@cd $(WS) && for d in $(REPOS); do \
		[ -d "$$d/.git" ] || { printf '  %-18s %s\n' "$$d" "(no repository)"; continue; }; \
		loc=$$(cd $$d && git rev-parse --short HEAD); \
		dirty=$$(cd $$d && git status --porcelain | wc -l | tr -d ' '); \
		org=$$(cd $$d && git remote -v 2>/dev/null | head -1 | awk '{print $$2}'); \
		if [ -n "$$org" ]; then \
			rem=$$(timeout 12 git ls-remote --exit-code "$$org" HEAD 2>/dev/null | head -1 | cut -c1-7); \
			[ -n "$$rem" ] || rem='NO REPO'; \
		else org='(none)'; rem='—'; fi; \
		printf '  %-18s %-9s %-9s %-7s %s\n' "$$d" "$$loc" "$$rem" "$$dirty" "$$org"; \
	done
		@printf '\n  LIVE\n'
	@printf '    arc %-12s %s\n' "/v1/version" "$$(curl -s --max-time 10 $(ARC_HOST)/v1/version | head -c 140 || echo unreachable)"
	@printf '    arc %-12s %s\n' "/v1/health"  "$$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 $(ARC_HOST)/v1/health || echo '—')"
	@printf '    arc %-12s %s\n' "/auth/health" "$$(curl -s --max-time 10 $(ARC_HOST)/auth/health | head -c 100 || echo '—')"
	@echo

# ── the gate ────────────────────────────────────────────────────────────────
# Everything that must be green before anything ships. Ordered cheapest-first so
# a typo fails in seconds rather than after a cross-compile.

# Every suite runs under tools/pinned.sh, which digests the source tree before and
# after and VOIDS the result if a file moved — a run that describes a tree which no
# longer exists is not evidence. It also passes the suite's exit status through,
# which the old awk pipeline swallowed: `check` printed "all suites green"
# unconditionally, including over a red suite.

check: check-icd check-auth check-harness check-core check-arc check-door  ## every suite, cheapest first
	@echo "  all suites green, on a tree that did not move"

# O-68 (mdr/icd-pin.md): the ICD at its pin, first. Not under pinned.sh, which keeps one line
# of 72 characters: a refusal here names two hashes.
check-icd:  ## the ICD's bytes at their pin, icd.rs conformance, the web's op bindings (O-68)
	@$(CORE)/coordination/icd-pin.sh check

check-auth:  ## the Arc account store (pytest; a check needing the website says when it skips)
	@printf '  auth    '
	@$(PINNED) auth sh -c 'cd $(PRODUCT)/arc/auth && $(PY) -m pytest -q -rs'

check-harness:  ## the Python harness, and pinned.sh's own summary (NC-122)
	@$(PINNED) --self-test > /dev/null || { $(PINNED) --self-test; exit 1; }
	@printf '  harness '
	@$(PINNED) harness sh -c 'cd $(CORE) && $(PY) -m harness all'

# Each test in its own process (cargo-nextest), all at once. A Harness holds a process-wide lock
# for its life (PACIFIC_STATE_DIR is process-global) and cargo test runs binaries one by one, so
# under cargo test the suite ran on one CPU. nextest runs no doctests: cargo test --doc does.
check-core:  ## pacific-core + core-wasm + pacific-ffi, and core-wasm as the browser builds it; the fold cache on, every hit verified (O-69, FC-2)
	@command -v cargo-nextest >/dev/null || { echo '  core    needs cargo-nextest: brew install cargo-nextest'; exit 1; }
	@printf '  core    '
	@$(PINNED) core sh -c 'cd $(CORE) && { cargo check -q -p core-wasm --target wasm32-unknown-unknown; w=$$?; export PACIFIC_FOLD_CACHE_MODEL=core-suite PACIFIC_FOLD_CACHE_VERIFY=1; cargo nextest run $(call PKGS,core) --no-fail-fast --retries 0; t=$$?; cargo test $(call PKGS,core) --doc; d=$$?; exit $$(( w | t | d )); }'

check-arc:  ## the Arc planes
	@printf '  arc     '
	@$(PINNED) arc sh -c 'cd $(PRODUCT)/arc && cargo test $(call PKGS,arc) --no-fail-fast'
	@if [ -d $(WS)/business/website/site ]; then cd $(PRODUCT)/arc && cargo test -q -p face-render -- --ignored; \
	else echo '  arc     face-render vendored_copies_match_the_website NOT RUN: no business/website beside product'; fi

check-door:  ## the Door (app/door), and its window's and the webapp's node tests
	@cd $(PRODUCT) && node --test app/door/hosting/bake.test.mjs
	@if [ "$$(uname)" = Darwin ]; then cd $(PRODUCT) && node --test app/door/hosting/one-signin/site-setup.test.mjs; \
	else echo '  door    site-setup.test.mjs (the operator tool draws with macOS sips) NOT RUN: not macOS'; fi
	@printf '  door    '
	@$(PINNED) door sh -c 'cd $(PRODUCT)/arc && cargo build -q -p relay && cd $(PRODUCT)/app/door && { DOOR_TEST_PYTHON=$(PY) cargo test --no-fail-fast; s=$$?; cd $(PRODUCT) && node --test app/web/door/return-path.test.mjs app/web/door/pow-worker.test.mjs app/web/door/signin.test.mjs app/web/door/element.test.mjs app/web/door/perf.test.mjs app/web/webapp/door-origin.test.mjs app/web/webapp/register.test.mjs app/web/webapp/session.test.mjs app/web/webapp/card.test.mjs app/web/webapp/create.test.mjs app/web/webapp/events.test.mjs app/web/webapp/events-icd.test.mjs app/web/webapp/events-edit.test.mjs app/web/webapp/events-page.test.mjs app/web/webapp/palette.test.mjs app/web/webapp/rooms.test.mjs app/web/webapp/members.test.mjs app/door/hosting/one-signin/one-signin-today.test.mjs app/door/hosting/one-signin/one-signin-2.1.0.test.mjs app/door/hosting/one-signin/rooms-unmarked.test.mjs app/door/hosting/one-signin/hr-arc-history.test.mjs app/door/hosting/one-signin/founder-r3.test.mjs app/web/shared/wallflowers-ops.test.mjs app/web/webapp/icd.test.mjs app/door/docs/docs.test.mjs app/door/docs/icd-changelog.test.mjs app/web/resources/resources.test.mjs app/web/resources/pdf.test.mjs app/web/webapp/resources-seam.test.mjs app/web/webapp/trade.test.mjs; n=$$?; exit $$(( s | n )); }'

e2e:  ## the WallFlowers path end to end on loopback; outside check for now
	@printf '  e2e     '
	@$(PINNED) e2e sh -c 'cd $(PRODUCT) && $(PY) app/e2e/wallflowers_path.py'

e2e-browser:  ## the path in a visible browser: the real window, a virtual passkey with PRF
	@printf '  e2e-browser '
	@$(PINNED) e2e-browser sh -c 'cd $(PRODUCT) && E2E_KEEP=1 $(PY) app/e2e/browser_path.py'

# ── the CI runner ───────────────────────────────────────────────────────────
# GitHub's CI jobs (label wf-ci) run here, in a container per job on Colima's wf-ci VM, for as
# long as any are queued; then the VM stops. The harness is dev/ci.sh (CLAUDE.md § Build).

ci:  ## serve GitHub's queued CI jobs on this Mac, then tear down; one run per machine
	@$(HOME)/wallflowers/dev/ci.sh

ci-hooks:  ## a push starts `make ci` in the background (tools/hooks/pre-push)
	@git -C $(PRODUCT) config core.hooksPath tools/hooks && echo '  pushes now start make ci'

# ── build and serve ─────────────────────────────────────────────────────────

wasm:  ## build pacific-core for the browser and stage it everywhere
	@cd $(PRODUCT)/app/web && ./build-wasm.sh

serve:  ## docs/ on localhost:8130, core rebuilt if stale
	@$(CORE)/serve-docs.sh

stop:  ## stop what `serve` started
	@$(CORE)/serve-docs.sh stop

# ── deploys ─────────────────────────────────────────────────────────────────
# Four routes, each a commit shipped by its own script. door-01 holds the Door; the Arc's
# box (wallflowers-01) the planes, the account store and the docs. DEPLOY_KEY is the ssh
# key both take.

DEPLOY_KEY ?= $(HOME)/.ssh/machine0_key
DOOR        = DOOR_BOX=ubuntu@188.166.244.69 DOOR_HOST=app.wallflowers.io DOOR_KEY=$(DEPLOY_KEY) $(PRODUCT)/app/door/hosting/deploy-door.sh
ARC_ENV     = ARC_BOX=ubuntu@167.99.83.136 ARC_KEY=$(DEPLOY_KEY)

deploy-app:  ## the webapp -> app.wallflowers.io; the Door keeps running, no session ends
	@$(DOOR) app

deploy-door:  ## the Door -> door-01; the restart ends every session
	@$(DOOR) door

deploy-arc:  ## the five planes -> wallflowers-01, and the account store when arc/auth changed
	@$(ARC_ENV) $(PRODUCT)/arc/hosting/deploy-arc.sh
	@if $(ARC_ENV) $(PRODUCT)/arc/hosting/deploy-auth.sh changed; then $(ARC_ENV) $(PRODUCT)/arc/hosting/deploy-auth.sh; \
	else echo '  auth    unchanged: not restarted'; fi

deploy-docs:  ## the Community API docs -> docs.wallflowers.io
	@$(ARC_ENV) $(PRODUCT)/app/door/docs/hosting/deploy-docs.sh

live:  ## what each deploy reports about itself
	@$(DOOR) verify
	@$(ARC_ENV) $(PRODUCT)/arc/hosting/deploy-arc.sh verify
	@$(ARC_ENV) $(PRODUCT)/arc/hosting/deploy-auth.sh verify
	@$(ARC_ENV) $(PRODUCT)/app/door/docs/hosting/deploy-docs.sh verify

site-setup:  ## a Site's one sign-in, by the owner's passkey: SITE=egregore MARK=<picture>
	@node $(PRODUCT)/app/door/hosting/one-signin/site-setup.mjs "$(SITE)" "$(MARK)"
