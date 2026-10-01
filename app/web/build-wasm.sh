#!/usr/bin/env bash
#
# Build pacific-core for the browser and drop it beside the pages that load it.
#
#   ./build-wasm.sh            the client. One module, MLS included.
#
# The dependency points this way on purpose: the core does not know the web client
# exists, so the consumer fetches. core-wasm is a normal workspace member.
#
# ── ONE MODULE, AND WHY THERE IS NO SECOND ───────────────────────────────────
#
# There was a `--no-mls` flag here promising "the arithmetic-only module, 250 KB,
# no imports". It was a misnomer twice over. MLS is going everywhere — every
# surface that holds an account also holds groups — so there is no surface the
# small module would serve. And it had stopped being true regardless: the flag ran
# the IDENTICAL cargo command as the full build, then skipped the bindgen step, so
# it emitted a 3.3 MB module carrying wasm-bindgen placeholder imports that could
# never link. Deleted rather than repaired: a second artefact nobody wants is not
# worth a feature gate, and a build flag that lies is worse than no flag.
#
# ── WHY THIS IS A TWO-STAGE BUILD ────────────────────────────────────────────
#
# `mls-rs`, `mls-rs-codec` and `mls-rs-core` each declare an UNCONDITIONAL,
# non-optional `wasm-bindgen` dependency for wasm32. Not behind a feature — there
# is nothing to switch off. So a module with MLS in it carries wasm-bindgen, and
# wasm-bindgen is a two-stage build: rustc emits placeholder imports plus describe
# machinery, and the `wasm-bindgen` CLI turns that into a real module plus glue.
# Skipping stage two is what made the module fail to instantiate with a LinkError.
#
# So the build runs the CLI and asserts what actually matters: that the module
# INSTANTIATES and its surface is present. A module that links but cannot stand up
# is the regression to catch.
#
# Requires: cargo, the wasm32-unknown-unknown target, and
# `cargo install wasm-bindgen-cli --version <the version in Cargo.lock>`.

set -euo pipefail
cd "$(dirname "$0")"

CORE="${CORE:-../../core}"

# Where the module lands. Overridable so this stays the ONE builder: the guard
# block below is the only place that knows which exports must exist and which
# imports must not, and a second copy of it in another script would rot in
# exactly the way a transcribed catalogue rots. Other surfaces pass OUT_DIR and
# get the same build and the same assertions.
OUT_DIR="${OUT_DIR:-docs}"
export PATH="$HOME/.cargo/bin:$PATH"

WBG_VERSION="$(grep -A1 '^name = "wasm-bindgen"$' "$CORE/Cargo.lock" | grep '^version' | head -1 | cut -d'"' -f2)"
if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "wasm-bindgen CLI not found. Install the version matching the crate:" >&2
  echo "  cargo install wasm-bindgen-cli --version ${WBG_VERSION}" >&2
  exit 1
fi
HAVE="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$HAVE" != "$WBG_VERSION" ]; then
  echo "  WARNING: wasm-bindgen CLI is $HAVE but the crate is $WBG_VERSION." >&2
  echo "  A mismatch produces a module that fails to instantiate. Reinstall to match." >&2
fi

echo "building core-wasm WITH mls (release, wasm32-unknown-unknown)…"
( cd "$CORE" && cargo build -p core-wasm --target wasm32-unknown-unknown --release )

echo "post-processing with wasm-bindgen ${HAVE}…"
rm -rf "$OUT_DIR/core"
wasm-bindgen --target web --out-dir "$OUT_DIR/core" --out-name core_wasm \
  "$CORE/target/wasm32-unknown-unknown/release/core_wasm.wasm"

printf "  %s  %s bytes\n" "$OUT_DIR/core/core_wasm_bg.wasm" \
  "$(stat -f%z "$OUT_DIR/core/core_wasm_bg.wasm")"

# What replaces the no-imports assertion: the module must STAND UP, and its MLS
# surface must be there. A LinkError here is the regression that used to ship.
node -e '
  const fs = require("fs");
  const p = "'"$OUT_DIR"'/core/core_wasm_bg.wasm";
  const m = new WebAssembly.Module(fs.readFileSync(p));
  const ex = WebAssembly.Module.exports(m).map(e => e.name);
  const mls = ex.filter(n => n.startsWith("mls_"));
  const need = ["mls_init","mls_create_group","mls_key_package","mls_add_member",
                "mls_join","mls_encrypt","mls_decrypt","mls_epoch_keys",
                "mls_snapshot","mls_restore",
                // pairing as a person, and the history a new leaf cannot decrypt
                "mls_contact_bundle","mls_join_intro","seal_open","seal_seal",
                // the WRAP: the seed under the passkey, and the only thing
                // served to anyone who asks. NO APOSTROPHES IN HERE: this block
                // is a single-quoted shell string, and one ends it.
                "wrap_key","wrap_seal","wrap_open",
                // the other door: 24 words, which follow the person rather than
                // the passkey
                "recovery_words","recovery_seed","identity_key",
                // THE SPINE READ (7a3fdf5): what a device that has never seen
                // the account walks to name its objects. The history blob these
                // replaced (history_key/seal/open/build/peek/unpack) left the
                // core in a58798c, when the escrow retired on 14 Sep did.
                "spine_addresses","spine_read","spine_place",
                // THE DOOR: fold one live object. Authoring left the browser in
                // 5636ac4: it is the Door, through the core one authoring door.
                "fold_object","mint_ops","ops_on",
                // THE INTRO (G1): an invite by contact card delivers the Welcome
                // with the gen floor, and a join from it keeps the floor.
                "bundle_read","intro_seal","mls_join_intro",
                // THE ANCHOR. Without these three a browser can make an account
                // and never write a head, which is an account whose chain cannot
                // be shown to be whole — see the block above head_seal in
                // core-wasm/src/lib.rs.
                "head_seal","head_open","head_check",
                // THE ADDRESSES. Without these a page derives its own HKDF, and
                // a page one byte out files records where the other devices of
                // that account never look — nothing logged, no test red.
                // (No apostrophes: see the note in the block above.)
                "locators","arc_write_sign","arc_write_verify","chain_next_tag",
                "mls_rekey","mls_rekey_apply","mls_rekey_drop"];
  const missing = need.filter(n => !ex.includes(n));
  if (missing.length) {
    console.error("  REGRESSION: MLS surface missing —", missing.join(" "));
    process.exit(1);
  }
  const imports = WebAssembly.Module.imports(m);
  const unresolved = imports.filter(i => i.module !== "env" && !i.module.startsWith("./"));
  if (unresolved.length) {
    console.error("  REGRESSION: imports no glue will satisfy —", JSON.stringify(unresolved));
    process.exit(1);
  }
  // THE getrandom BACKEND GUARD. mls-rs hard-codes getrandom ["js","custom"] for
  // wasm32 and js WINS the cascade, which makes our registered host-entropy hook
  // dead code and drags ~25 JS-crypto shims in with it. The [patch] in core/Cargo.toml
  // drops js. That patch is VERSION-PINNED and fails SILENTLY when mls-rs moves —
  // it already did once, 0.55.2 -> 0.55.4. This is what catches it: with js gone the
  // module imports entropy exactly once, from us.
  if (!imports.some(i => i.module === "env" && i.name === "pacific_fill_random")) {
    console.error("  REGRESSION: no host entropy import — the custom backend is not wired");
    process.exit(1);
  }
  const jsCrypto = imports.filter(i => /getRandomValues|randomFillSync|__wbg_crypto|msCrypto/.test(i.name));
  if (jsCrypto.length) {
    console.error("  REGRESSION: the getrandom js backend is back —", jsCrypto.length,
                  "JS-crypto shims. The mls-rs patch in core/Cargo.toml has stopped");
    console.error("  applying, almost certainly because the version moved. Re-vendor it.");
    process.exit(1);
  }
  console.log("  mls surface:", mls.length, "exports · imports:", imports.length,
              "· entropy: host · instantiable: yes");
'
# Every other surface is its own origin and cannot fetch docs/ across one, so
# each carries its own staged copy of the core AND of the account module that
# calls it. One script knows the list.
#
# Only for THIS tree's own build. A caller that redirected OUT_DIR elsewhere is
# building for a surface outside app/web, and fanning that out to app/web's
# staging targets would overwrite them with a module they did not ask for.
if [ "$OUT_DIR" = "docs" ]; then
  ./account/stage.sh
else
  echo "  OUT_DIR is ${OUT_DIR} — skipping account/stage.sh (that stages app/web's own surfaces)"
fi
