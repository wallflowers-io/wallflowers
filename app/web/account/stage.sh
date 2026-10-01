#!/usr/bin/env bash
#
# Put the account module and the core it calls beside every surface that needs
# them.
#
#   ./stage.sh          stage into every consumer that exists
#
# WHY STAGE RATHER THAN IMPORT. The consumers are on DIFFERENT ORIGINS on
# purpose — the keyholder is its own origin precisely so a member's page cannot
# reach into it, and kenjin.cc is not the webapp. An origin that could fetch the
# account module from another origin would be an origin that could be made to
# fetch a different one. So each carries its own copy, and this script is what
# makes "its own copy" mean "the same copy".
#
# WHAT IS A BUILD ARTEFACT HERE. Everything this writes. The staged `core/` and
# `pacific-account.js` under each consumer are gitignored where they land; the
# sources are `core-wasm` (built by ../build-wasm.sh) and
# `account/pacific-account.js`. Never edit a staged copy — the next build
# overwrites it, silently, and the edit is gone.
#
# Run ../build-wasm.sh first; this refuses rather than staging a stale core.
set -euo pipefail
cd "$(dirname "$0")"

SRC_CORE="../docs/core"
SRC_JS="pacific-account.js"

[ -f "$SRC_CORE/core_wasm_bg.wasm" ] || {
  echo "no core to stage — run app/web/build-wasm.sh first" >&2
  exit 1
}

# The header above says this "refuses rather than staging a stale core", and for
# a while the check above was the whole of it — which only catches a core that is
# ABSENT. A core built a week ago is present, stages silently onto every surface,
# and is exactly the failure the sentence promised to prevent. So compare it
# against the sources it was built from.
CORE_SRC="${CORE_SRC:-../../../core}"
if [ -d "$CORE_SRC" ]; then
  NEWER="$(find "$CORE_SRC" -name target -prune -o -name vendor -prune -o \
             \( -name '*.rs' -o -name '*.toml' \) \
             -newer "$SRC_CORE/core_wasm_bg.wasm" -print 2>/dev/null | head -3)"
  if [ -n "$NEWER" ]; then
    echo "  the core is older than the workspace it was built from:" >&2
    printf '    %s\n' $NEWER >&2
    echo "  run app/web/build-wasm.sh — it calls this script when it is done." >&2
    exit 1
  fi
fi

# Every surface that holds an account. A missing directory is skipped, not an
# error: the site lives in its own repo and is not always checked out beside
# this one.
TARGETS=(
  "../docs"                                   # the prototype pages
  "../keyholder"                              # Pacific's own origin
  "../webapp"                                 # the webapp — face, door, interior
)

# THE WEBSITE IS NOT A TARGET (24 Sep 2026). `business/website/site/assets/pacific`
# and `.../auth/dev/try` used to be on this list, which meant this script — run to
# answer a question in THIS repository — wrote into a DIFFERENT repository with its
# own deploy. It did, on the day this line was written: a core whose own build had
# just printed `REGRESSION: MLS surface missing`, and an ICD that was mid-restructure
# and carried none of the messages that site's face editor reads. The copies land
# before the verification below runs, so a failure here leaves every target holding
# them; the website would have deployed both without anyone asking for either.
#
# It vendors them now, on its own clock and against a lock, and it checks what IT
# needs rather than trusting this script's list: business/website/scripts/vendor.py,
# `just vendor`. Do not add it back. If the site needs a newer core, the site asks.

SRC_ABS="$(cd "$SRC_CORE" && pwd -P)"

# THE SHARED WEB SOURCE, staged by the same rule and for the same reason as the
# account module. `shared/` holds the one fold and the one generated op table
# that both web clients use. A consumer served as its own document root cannot
# reach ../shared, so it carries a copy — and this script is what makes "its own
# copy" mean "the same copy". Staged copies are gitignored build artefacts:
# never edit one, the next stage overwrites it silently.
#
# Staged only where the target's own pages REFERENCE it, so a consumer that does
# not load it never acquires a file it will not use — and one that starts loading
# it picks it up on the next stage without this list being edited.
# Relative to account/, where line 23 has already moved us: re-deriving it from
# BASH_SOURCE here doubles the path whenever the script is run from outside.
SRC_SHARED="$(cd ../shared && pwd -P)"
SHARED_FILES=(wallflowers-fold.js wallflowers-ops.js wallflowers-client.js wallflowers-client.d.ts)
# SERVED BESIDE pacific.js, WHETHER OR NOT A PAGE HERE LOADS THEM. The client's
# consumers are OTHER SITES (egregore, thedoor, cyp3), which load pacific.js from
# ../docs's origin and these three with it. No page in ../docs references them,
# so the reference rule below cannot see those consumers and would never stage
# them there. So ../docs always carries them.
SERVED_BESIDE_PACIFIC=(wallflowers-client.js wallflowers-client.d.ts wallflowers-fold.js wallflowers-ops.js)
SHARED_EXCLUDE=(); for s in "${SHARED_FILES[@]}"; do SHARED_EXCLUDE+=(--exclude="$s"); done

staged=0
for t in "${TARGETS[@]}"; do
  parent="$(dirname "$t")"
  if [ ! -d "$parent" ]; then
    echo "  skip   $t (no $parent)"
    continue
  fi
  mkdir -p "$t"
  # ../docs IS the source — build-wasm.sh writes the canonical core there — so
  # staging it would mean deleting the thing being copied. It still needs the js.
  if [ "$(cd "$t" && pwd -P)/core" = "$SRC_ABS" ]; then
    cp "$SRC_JS" "$t/pacific-account.js"
    printf "  stage  %s (js only — it holds the source core)\n" "$t"
  else
    rm -rf "$t/core"
    mkdir -p "$t/core"
    # THE MODULE ONLY, not the wasm-bindgen glue beside it. Every consumer
    # instantiates core_wasm_bg.wasm by hand and supplies all three imports by
    # NAME (see pacific-account.js HOSTFNS and keyholder.js), so core_wasm.js,
    # the .d.ts files and snippets/ are never fetched by anything. Staging them
    # shipped dead ES modules into the site, where `just check` then tried to
    # lint them as source and failed the build.
    cp "$SRC_CORE/core_wasm_bg.wasm" "$t/core/core_wasm_bg.wasm"
    cp "$SRC_JS" "$t/pacific-account.js"
    printf "  stage  %s\n" "$t"
  fi
  for f in "${SHARED_FILES[@]}"; do
    # The staged shared copies are left out of the search: each names itself in
    # its own header, so once one landed it would count as its own reference and
    # never leave, which is how a page came to keep a file it does not load.
    beside=0
    if [ "$t" = "../docs" ]; then
      for b in "${SERVED_BESIDE_PACIFIC[@]}"; do [ "$b" = "$f" ] && beside=1; done
    fi
    if [ "$beside" = 1 ] || grep -qrl --include='*.html' --include='*.js' "${SHARED_EXCLUDE[@]}" "$f" "$t" 2>/dev/null; then
      cp "$SRC_SHARED/$f" "$t/$f" && printf "  shared %s/%s\n" "$t" "$f"
    else
      rm -f "$t/$f"
    fi
  done
  staged=$((staged + 1))
done

# THE STANDARD SIM ANSWER, beside pacific.js: the core's own fold of the committed
# archive, with the core just staged, so it always matches the current fold and
# nobody writes fold output by hand (shared/gen-sim.mjs).
if [ -d ../docs ]; then
  node ../shared/gen-sim.mjs ../docs || { echo "  sim FAILED — the core would not fold the sim archive" >&2; exit 1; }
fi

echo "  $staged staged · core $(stat -f%z "$SRC_CORE/core_wasm_bg.wasm") bytes + $(stat -f%z "$SRC_JS") bytes of js"

# The one assertion worth making here: the module and the core agree about what
# the core exports. A staged pair that disagrees fails at the moment a person
# presses a button, which is the worst place to find out.
node -e '
  const fs = require("fs");
  const js = fs.readFileSync("pacific-account.js", "utf8");
  const ex = WebAssembly.Module.exports(
    new WebAssembly.Module(fs.readFileSync("../docs/core/core_wasm_bg.wasm"))
  ).map(e => e.name);
  // MATCH THE CALL, NOT THE DOT. `\be\.(name)` also matched `e.message` on a
  // caught error and reported "message" as a missing export — a false alarm that
  // fails the stage for a correct module. The module reaches the core exactly one
  // way, `callWasm(e, e.<export>, bytes)`, so that is what is matched.
  const used = [...js.matchAll(/callWasm\(\s*[A-Za-z_$][\w$]*\s*,\s*[A-Za-z_$][\w$]*\.([a-z_][a-z0-9_]*)/g)]
                 .map(m => m[1]);
  // Plus the ABI `callWasm` itself uses. Deliberately listed: they are the
  // contract between this module and every core build, and a core that lost one
  // would break every call rather than one of them.
  used.push("alloc", "memory", "erred", "out_ptr", "out_len");
  if (!/callWasm\(/.test(js)) {
    console.error("  REGRESSION: pacific-account.js makes no callWasm calls — the extractor above");
    console.error("  is matching nothing, so this check is passing vacuously.");
    process.exit(1);
  }
  const missing = [...new Set(used)].filter(n => !ex.includes(n));
  if (missing.length) {
    console.error("  REGRESSION: the module calls exports the core does not have —",
                  missing.join(" "));
    process.exit(1);
  }
  console.log("  module calls", new Set(used).size, "core exports · all present");
'
