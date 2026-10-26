# WallFlowers: how a change reaches production

Read before you write. Every change follows this path; there is no other.

## The repositories

| Path | Is | Remote |
|---|---|---|
| `~/wallflowers/wallflowers_public` | **this repository**: app, arc, core — all code | github.com/wallflowers-io/wallflowers (public, AGPL) |
| `~/wallflowers/product` | the records: `core/docs`, the doctrine (`core/CLAUDE.md`), roles | Codeberg (private) |
| `~/wallflowers/business/website` | www.wallflowers.io | Codeberg (private); deploys on its own path |

- **Never add GitHub as a remote of `product`, nor push its history anywhere public.** Its 3,776 commits hold material that may not be published. Code from a `product` branch is ported: its diff replayed onto `origin/main` here, as a PR.
- Commits here are authored `WallFlowers <dev@wallflowers.io>` (this repository's git config). No personal address enters public history.

## One change, one branch, one PR

1. **Branch from `origin/main`, in its own worktree.** The main checkout stays on `main`, clean.
   `git -C ~/wallflowers/wallflowers_public worktree add ~/wallflowers/wt/<name> -b <type>/<name> origin/main`
   `<type>`: `fix`, `feat`, `test`, `docs`, `ci`, `chore`, `port` (from `product`).
2. **Focused.** One purpose per PR. A title that needs "and" is two PRs. Small: under ~400 lines changed, generated and vendored files aside. Merged or closed the same day.
3. **Tested.** A behaviour change carries the test that fails without it; show it red first. Never weaken a test to make it pass. A known red is named in the PR body with its NC.
4. **Checked locally** with the suites the change touches: `make check-icd`, `check-auth`, `check-core`, `check-arc`, `check-door` (`make` lists them).
5. **Commit in labelled chunks**, each with only its own paths: `git commit -F <msg> -- <paths>`.
6. **Open the PR:** `gh pr create --base main`. CI (`.github/workflows/ci.yml`) runs every suite on it.

## Merge

- **Required to pass:** `icd`, `auth`, `core`, `arc`. `door` runs but is not yet required (NC-93, NC-133).
- **On Ralph's word,** once they pass: `gh pr merge <n> --squash --delete-branch`.
- `main` takes changes only through PRs: no direct push, no force push.

## Deploy

Four routes, each a Makefile target, run by `.github/workflows/deploy.yml` from `main`:

| Route | Ships | Restart |
|---|---|---|
| `arc` | the five planes → wallflowers-01; the account store when `arc/auth` changed | the planes; auth only when shipped (cuts sign-ins for seconds) |
| `door` | the Door binary and its configuration → door-01 | yes: **every session ends** |
| `app` | the webapp and its `/assets` → door-01 | none |
| `docs` | `app/door/docs/dist` → docs.wallflowers.io | none |

- **From here, on Ralph's word:** `make ship ROUTES="<routes>"` dispatches `deploy.yml` on `main`, approves its `production` gate and follows the run to its end. Then `make live`.
- **Order:** `arc` before `door` before `app` when several ship (the workflow keeps it). A Door change to `/v2/signin.js` needs egregores-echoes.com's SRI updated in the same window (`core/docs/launch/deploy.md`, U3).
- **Never deploy from a working tree.** `make deploy-<route>` on a laptop is for a broken pipeline only, from a clean checkout of `main`, and is said.
- `core/docs/launch/deploy.md` (in `product`) is the authority on every unit and its path.

## Leave no mess

When your PR merges or closes, in the same turn:
- `git -C ~/wallflowers/wallflowers_public worktree remove ~/wallflowers/wt/<name>`; `git -C ~/wallflowers/wallflowers_public branch -D <type>/<name>`;
- a port: its `product` branch and worktree deleted too;
- nothing left at `~/wallflowers/` or in this repository that is not committed; scratch files in your session's scratchpad;
- every server, browser and background job you started, stopped.
