# Pacific / WallFlowers — one entry point over the product and the two business repositories.
#
# WHAT THIS IS, AND WHAT IT DELIBERATELY IS NOT. Every target here DELEGATES.
# `deploy-arc` runs arc/hosting/deploy-arc.sh; `deploy-auth` runs site's
# justfile; `wasm` runs app/web/build-wasm.sh. None of them reimplements a build,
# because a second way to build is a second thing to be wrong — the same argument
# the doctrine makes about the ICD, applied to the pipeline. This file is an INDEX:
# it knows what exists, what order it goes in, and what "green" means. When CI
# arrives it should call these targets, not reproduce them.
#
# ── WHY THERE IS NO CI YET, AND IT IS NOT THE PIPELINE'S FAULT ───────────────
#
# Checked 18 Sep 2026 with `git ls-remote` over SSH, which is what `git push`
# uses. HTTPS gives a misleading answer here: it reports an auth failure for a
# repository that does not exist, so it cannot distinguish "no access" from "no
# repo". SSH can.
#
#   app        no remote                          (web + ios joined 18 Sep; ios's old
#                                                  origin was a local filesystem path)
#   arc        git@codeberg.org:phezman/arc.git   exists, remote behind local
#   core       git@codeberg.org:phezman/pacific-core.git   DOES NOT EXIST
#   site       git@codeberg.org:phezman/kenjin.git         exists, to be renamed site
#   branding   no remote                          (extracted from the workspace 18 Sep)
#
# `make status` reports the live state; this table is the state on the day the
# workspace was split, kept because it explains why there was no pipeline.
#
# So: most of the five had nowhere to push. **There is no pipeline because there is nothing pushed
# to build.** That is the blocker `arc@ac85d89` named — planes/relay pins
# pacific-wire to a codeberg rev predating the vocabulary work, and there is
# nothing to bump to because the vocabulary work has never left this laptop.
#
# Order, therefore: `make status` → push paths → CI. Not the other way round.
#
# ── WHAT CODEBERG ACTUALLY OFFERS, when the push path exists ─────────────────
#
# Two options, and the heavy build fits NEITHER without a self-hosted runner:
#
#   Woodpecker (hosted)     ci.codeberg.org. Manual onboarding by volunteers,
#                           linux/amd64 only, "resource usage must be reasonable
#                           for the intended use-case", explicitly as-is.
#   Forgejo Actions         .forgejo/workflows/ (falls back to .github/workflows/).
#                           Disabled per repo by default: Settings > Units >
#                           Overview. Codeberg-hosted runners are open alpha and
#                           limited; self-hosted is the supported path. Runners
#                           connect OUTBOUND, so no public IP is needed. Labels
#                           are <name>:<type>://<image>, type one of docker | lxc
#                           | host.
#
# THE HEAVY BUILD IS NOT A REASONABLE USE OF A NON-PROFIT'S SHARED RUNNERS.
# arc/hosting/deploy-arc.sh already records why: mls-rs plus bundled SQLite "would
# take hours and OOM" on kenjin-01 (1 vCPU, 961Mi, no swap). That rules out both
# the shared Woodpecker instance AND kenjin-01 as a runner. The build happens on
# a laptop today; if it should stop happening there, the answer is a build host
# registered as a self-hosted Forgejo runner, not a workflow file.
#
# What IS worth putting on hosted CI the day the code is pushed: the checks.
# `make check` is cheap, catches the cross-repo drift this tree keeps producing,
# and needs no cross-compiler.
#
# Usage: `make` for the list.

SHELL      := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c
# This file lives at the root of product/, the repository that holds app, arc and
# core. The workspace is product's parent: the site and the marks are under
# business/ (website, branding). Neither `wallflowers/` nor `business/` is a repository.
PRODUCT    := $(shell cd $(dir $(lastword $(MAKEFILE_LIST))) && pwd)
CORE       := $(PRODUCT)/core
WS         := $(abspath $(PRODUCT)/..)
REPOS      := product business/website business/branding
export PATH := $(HOME)/.cargo/bin:$(PATH)
PY         := $(WS)/.venv/bin/python
PINNED     := $(PRODUCT)/tools/pinned.sh
ARC_HOST   := https://arc.wallflowers.io

.DEFAULT_GOAL := help
.PHONY: help status check check-icd check-core check-arc check-auth check-harness check-door e2e e2e-browser \
        wasm serve stop deploy-arc deploy-auth deploy-site live site-setup

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

check-auth:  ## the Arc account store (pytest)
	@printf '  auth    '
	@$(PINNED) auth sh -c 'cd $(WS)/business/website/auth && $(PY) -m pytest -q'

check-harness:  ## the Python harness, and pinned.sh's own summary (NC-122)
	@$(PINNED) --self-test > /dev/null || { $(PINNED) --self-test; exit 1; }
	@printf '  harness '
	@$(PINNED) harness sh -c 'cd $(CORE) && $(PY) -m harness all'

check-core:  ## pacific-core + core-wasm + pacific-ffi, and core-wasm as the browser builds it; the fold cache on, every hit verified (O-69, FC-2)
	@printf '  core    '
	@$(PINNED) core sh -c 'cd $(CORE) && { cargo check -q -p core-wasm --target wasm32-unknown-unknown; w=$$?; PACIFIC_FOLD_CACHE_MODEL=core-suite PACIFIC_FOLD_CACHE_VERIFY=1 cargo test --workspace --no-fail-fast; t=$$?; exit $$(( w | t )); }'

check-arc:  ## the Arc planes
	@printf '  arc     '
	@$(PINNED) arc sh -c 'cd $(PRODUCT)/arc && cargo test --workspace --no-fail-fast && cargo test -p face-render -- --ignored'

check-door:  ## the Door (app/door, its own workspace), and its window's and the webapp's node tests
	@printf '  door    '
	@$(PINNED) door sh -c 'cd $(PRODUCT)/arc && cargo build -q -p relay && cd $(PRODUCT)/app/door && { cargo test --no-fail-fast; s=$$?; cd $(PRODUCT) && node --test app/web/door/return-path.test.mjs app/web/door/pow-worker.test.mjs app/web/door/signin.test.mjs app/web/door/element.test.mjs app/web/door/perf.test.mjs app/web/webapp/door-origin.test.mjs app/web/webapp/register.test.mjs app/web/webapp/session.test.mjs app/web/webapp/card.test.mjs app/web/webapp/create.test.mjs app/web/webapp/events.test.mjs app/web/webapp/events-icd.test.mjs app/web/webapp/events-edit.test.mjs app/web/webapp/events-page.test.mjs app/web/webapp/palette.test.mjs app/web/webapp/rooms.test.mjs app/web/webapp/members.test.mjs app/door/hosting/one-signin/one-signin-today.test.mjs app/door/hosting/one-signin/one-signin-2.1.0.test.mjs app/door/hosting/one-signin/site-setup.test.mjs app/door/hosting/one-signin/rooms-unmarked.test.mjs app/door/hosting/one-signin/hr-arc-history.test.mjs app/door/hosting/one-signin/founder-r3.test.mjs app/web/shared/wallflowers-ops.test.mjs app/door/hosting/bake.test.mjs app/web/webapp/icd.test.mjs app/door/docs/docs.test.mjs app/door/docs/icd-changelog.test.mjs app/web/resources/resources.test.mjs app/web/resources/pdf.test.mjs app/web/webapp/resources-seam.test.mjs app/web/webapp/trade.test.mjs; n=$$?; exit $$(( s | n )); }'

e2e:  ## the WallFlowers path end to end on loopback; outside check for now
	@printf '  e2e     '
	@$(PINNED) e2e sh -c 'cd $(PRODUCT) && $(PY) app/e2e/wallflowers_path.py'

e2e-browser:  ## the path in a visible browser: the real window, a virtual passkey with PRF
	@printf '  e2e-browser '
	@$(PINNED) e2e-browser sh -c 'cd $(PRODUCT) && E2E_KEEP=1 $(PY) app/e2e/browser_path.py'

# ── build and serve ─────────────────────────────────────────────────────────

wasm:  ## build pacific-core for the browser and stage it everywhere
	@cd $(PRODUCT)/app/web && ./build-wasm.sh

serve:  ## docs/ on localhost:8130, core rebuilt if stale
	@$(CORE)/serve-docs.sh

stop:  ## stop what `serve` started
	@$(CORE)/serve-docs.sh stop

# ── deploys ─────────────────────────────────────────────────────────────────
# Each one delegates. They are listed together so the three are visible as one
# system, which they are: the client, the account store and the planes all have
# to agree about the audience and the label set.

deploy-arc:  ## the five planes -> kenjin-01 (stamped, guarded)
	@$(PRODUCT)/arc/hosting/deploy-arc.sh

deploy-auth:  ## the account store -> kenjin-01
	@cd $(WS)/business/website && just deploy-auth

deploy-site:  ## the pages -> kenjin-01
	@cd $(WS)/business/website && just deploy

live:  ## what the deployed Arc reports about itself
	@$(PRODUCT)/arc/hosting/deploy-arc.sh verify

site-setup:  ## a Site's one sign-in, by the owner's passkey: SITE=egregore MARK=<picture>
	@node $(PRODUCT)/app/door/hosting/one-signin/site-setup.mjs "$(SITE)" "$(MARK)"
