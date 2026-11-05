#!/usr/bin/env bash
#
# Pacific iOS versioning helper.
# Single source of truth = ui/project.yml (App/Info.plist reads the vars).
#
#   scripts/version.sh show                      # print current marketing + build
#   scripts/version.sh set-build                 # sync build number to git commit count, regenerate
#   scripts/version.sh bump [major|minor|patch]  # bump marketing (default patch) + build, regenerate
#
# CFBundleShortVersionString = MARKETING_VERSION   (public semver, e.g. 0.2.0)
# CFBundleVersion            = CURRENT_PROJECT_VERSION (monotonic build = git commit count)
#
# See Docs/VERSIONING.md for the release checklist.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"      # pacific/
UI="$REPO/ui"
YML="$UI/project.yml"
XCODEGEN="${XCODEGEN:-$(command -v xcodegen || echo "$HOME/.local/bin/xcodegen")}"

git_count()     { git -C "$REPO" rev-list --count HEAD; }
get_marketing() { grep -E '^[[:space:]]*MARKETING_VERSION:' "$YML" | head -1 | sed -E 's/.*"([^"]*)".*/\1/'; }
get_build()     { grep -E '^[[:space:]]*CURRENT_PROJECT_VERSION:' "$YML" | head -1 | sed -E 's/.*"([^"]*)".*/\1/'; }

# portable in-place edit (BSD/macOS sed), preserving any trailing comment
set_key() { sed -i '' -E "s|^([[:space:]]*$1:[[:space:]]*\")[^\"]*(\".*)|\1$2\2|" "$YML"; }
regen()   { ( cd "$UI" && "$XCODEGEN" generate >/dev/null ) && echo "→ regenerated WallFlowers.xcodeproj"; }

case "${1:-show}" in
  show)
    echo "marketing (CFBundleShortVersionString): $(get_marketing)"
    echo "build     (CFBundleVersion):            $(get_build)   [git commit count now: $(git_count)]"
    ;;
  set-build)
    set_key CURRENT_PROJECT_VERSION "$(git_count)"; regen
    echo "build number → $(get_build)"
    ;;
  bump)
    part="${2:-patch}"
    cur="$(get_marketing)"; IFS='.' read -r MA MI PA <<<"$cur"
    case "$part" in
      major) MA=$((MA+1)); MI=0; PA=0 ;;
      minor) MI=$((MI+1)); PA=0 ;;
      patch) PA=$((PA+1)) ;;
      *) echo "usage: version.sh bump [major|minor|patch]" >&2; exit 1 ;;
    esac
    new="$MA.$MI.$PA"
    set_key MARKETING_VERSION "$new"
    set_key CURRENT_PROJECT_VERSION "$(git_count)"; regen
    echo "marketing $cur → $new   (build $(git_count))"
    echo "next:  git commit -am \"chore(release): v$new\"  &&  git tag -a \"v$new\" -m \"Pacific $new\""
    ;;
  *)
    echo "usage: version.sh [show | set-build | bump major|minor|patch]" >&2; exit 1 ;;
esac
