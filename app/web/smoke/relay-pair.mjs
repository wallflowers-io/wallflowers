// Two wasm devices, paired over the REAL relay, message sent in the direction
// that matters: B (the browser's stand-in) → A (the phone's stand-in).
//
// WHY THIS EXISTS. `core-wasm.mjs` proves the same handshake IN PROCESS, passing
// the sealed Welcome hand to hand. That cannot fail the way a real pairing fails,
// because nothing crosses a wire. This one puts every byte through
// wss://arc.wallflowers.io/v1/relay — the exact host the phone dials — so a break in
// the browser's SEND path shows up here instead of on someone's phone.
//
//   node smoke/relay-pair.mjs
//   RELAY=ws://127.0.0.1:8080/v1/relay node smoke/relay-pair.mjs

import fs from "fs";
import { webcrypto as wc } from "node:crypto";

// WHICH RELAY. Production by default, because that is the host the phone dials
// and the one a break has to be found on. RELAY= points it at a local arc —
// `ws://127.0.0.1:8080/v1/relay` is the gateway this tree brings up — so the same
// bytes can be put through the arc under development before it is deployed.
const RELAY = process.env.RELAY || "wss://arc.wallflowers.io/v1/relay";
const WASM  = new URL("../docs/core/core_wasm_bg.wasm", import.meta.url);

let failures = 0;
const ok   = (c, m) => { console.log((c ? "  ok    " : "  FAIL  ") + m); if (!c) failures++; };
const head = (m) => console.log("\n" + m);

const enc = new TextEncoder(), dec = new TextDecoder();
const hex   = (u) => Buffer.from(u).toString("hex");
const unhex = (h) => new Uint8Array(Buffer.from(h, "hex"));
const b64   = (u) => Buffer.from(u).toString("base64");
const unb64 = (s) => new Uint8Array(Buffer.from(s, "base64"));

/* ── one wasm device ─────────────────────────────────────────────────────────
   Its own module instance, so its own linear memory and its own MLS state —
   two instances really are two devices, the same trick two ports play for two
   browser origins. */
async function device() {
  const mod = await WebAssembly.compile(fs.readFileSync(WASM));
  let inst = null; const at = () => inst;
  const HOST = {
    pacific_fill_random: () => (p, l) =>
      wc.getRandomValues(new Uint8Array(at().exports.memory.buffer, p, l)),
    __wbindgen_init_externref_table: () => () => {
      const t = at().exports.__wbindgen_externrefs, off = t.grow(4);
      t.set(0, undefined); t.set(off + 0, undefined); t.set(off + 1, null);
      t.set(off + 2, true); t.set(off + 3, false);
    },
    date_now: () => () => Date.now(),
  };
  const imports = {};
  for (const i of WebAssembly.Module.imports(mod)) {
    const mk = HOST[i.name];
    if (!mk) throw new Error("unsupplied import " + i.module + "." + i.name);
    (imports[i.module] ||= {})[i.name] = mk(at);
  }
  inst = await WebAssembly.instantiate(mod, imports);
  if (inst.exports.__wbindgen_start) inst.exports.__wbindgen_start();
  const E = inst.exports;
  // `fn` may be the export itself or its name — the callers use both.
  const call = (fn, bytes) => {
    const f = typeof fn === "function" ? fn : E[fn];
    if (typeof f !== "function") throw new Error("no such export: " + fn);
    const p = E.alloc(bytes.length);
    new Uint8Array(E.memory.buffer, p, bytes.length).set(bytes);
    const n = f(bytes.length);
    if (E.erred()) throw new Error(dec.decode(new Uint8Array(E.memory.buffer, E.out_ptr(), E.out_len())));
    return new Uint8Array(E.memory.buffer, E.out_ptr(), n).slice();
  };
  const text = (fn, s) => dec.decode(call(fn, enc.encode(s ?? "")));
  return {
    E, call, text,
    json: (fn, o) => JSON.parse(text(fn, typeof o === "string" ? o : JSON.stringify(o))),
  };
}

/* The IntroPayload the scanner seals — ciborium reads a text-keyed map in any
   order, and absent optional fields decode as None. Same encoder core-wasm.mjs
   uses, so the two tests cannot disagree about the wire. */
function cbor(v) {
  const out = [];
  const hdr = (major, n) => {
    if (n < 24) out.push((major << 5) | n);
    else if (n < 0x100) out.push((major << 5) | 24, n);
    else if (n < 0x10000) out.push((major << 5) | 25, n >> 8, n & 0xff);
    else out.push((major << 5) | 26, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
  };
  const e = (x) => {
    if (x instanceof Uint8Array) { hdr(2, x.length); out.push(...x); }
    else if (typeof x === "string") { const b = enc.encode(x); hdr(3, b.length); out.push(...b); }
    else if (x && typeof x === "object") {
      const k = Object.keys(x); hdr(5, k.length);
      for (const key of k) { e(key); e(x[key]); }
    } else throw new Error("cbor: unsupported " + typeof x);
  };
  e(v);
  return Uint8Array.from(out);
}

/* ── one device's relay socket ───────────────────────────────────────────── */
function socket(name) {
  const ws = new WebSocket(RELAY);
  const handlers = [];
  const seen = new Set();          // (tag,seq) — the dedupe the page lacked
  let dupes = 0;
  const ready = new Promise((res, rej) => {
    ws.onopen = () => res();
    ws.onerror = () => rej(new Error(name + ": relay unreachable"));
  });
  ws.onmessage = (ev) => {
    let f; try { f = JSON.parse(ev.data); } catch { return; }
    if (f.t !== "msg") return;
    const k = f.tag + ":" + f.seq;
    if (seen.has(k)) { dupes++; return; }
    seen.add(k);
    handlers.forEach((h) => h(f));
  };
  return {
    ready,
    sub: (tag) => ws.send(JSON.stringify({ t: "sub", tags: [tag], since: 0, v: 1 })),
    // A signed pub: the relay refuses one its tag did not sign (pacific_wire::address).
    pub: (signed, blob) => ws.send(JSON.stringify({ t: "pub", tag: signed.tag, blob, sig: signed.sig })),
    on: (h) => handlers.push(h),
    dupes: () => dupes,
    close: () => { try { ws.close(); } catch {} },
  };
}

const waitFor = (sock, tag, ms, why) => new Promise((res, rej) => {
  const timer = setTimeout(() => rej(new Error("timed out waiting for " + why)), ms);
  sock.on((f) => { if (f.tag === tag) { clearTimeout(timer); res(f); } });
});

/* ── go ──────────────────────────────────────────────────────────────────── */
head("two devices, one real relay");
const A = await device();                       // the phone's stand-in (scanner)
const B = await device();                       // the browser's stand-in
const a = A.json("mls_init", { seed: "a1".repeat(32) });
const b = B.json("mls_init", { seed: "b2".repeat(32) });
ok(a.cred !== b.cred, "two independent identities");

const sa = socket("A"), sb = socket("B");
await Promise.all([sa.ready, sb.ready]);
ok(true, "both sockets open on " + RELAY);

head("pairing over the wire (B shows a bundle, A scans it)");
// B's intro value is a SEED: the mailbox is the address it names, and only a holder
// of the value (anyone B gave the bundle to) can publish there.
const introTag = hex(B.call(B.E.relay_address, unhex(b.intro)));
sb.sub(introTag);
await new Promise((r) => setTimeout(r, 400));   // let the sub land before publishing

const kp   = B.json("mls_key_package", "").key_package;
const gid  = A.json("mls_create_group", "Relay pair").group_id;
const add  = A.json("mls_add_member", { group_id: gid, key_package: kp });
const intro = cbor({
  scanner_pk: unhex(a.cred), scanner_name: "A",
  welcome: unhex(add.welcome), kind: "connection", owner: unhex(a.cred),
});
const sealedIntro = A.text("seal_seal", JSON.stringify({
  inner: hex(intro), tag: b.intro, secret: b.intro,
}));
const introWait = waitFor(sb, introTag, 12000, "the Welcome on B's intro address");
const introBlob = b64(unhex(sealedIntro));
sa.pub(A.json("relay_sign_pub", { seed: b.intro, blob: introBlob }), introBlob);
const introFrame = await introWait;
ok(true, "the Welcome crossed the relay (seq " + introFrame.seq + ")");

const joined = B.json("mls_join_intro", { blob_b64: introFrame.blob });
ok(joined.group_id === gid, "B joined the group A created, at epoch " + joined.epoch);

head("both sides address the same mailbox");
const ka = A.json("mls_epoch_keys", gid);
const kb = B.json("mls_epoch_keys", gid);
ok(ka.tag === kb.tag && ka.secret === kb.secret,
   "same tag and seal secret at epoch " + ka.epoch + " (" + ka.tag.slice(0, 12) + "…)");

sa.sub(ka.tag); sb.sub(kb.tag);
await new Promise((r) => setTimeout(r, 400));

head("THE DIRECTION UNDER TEST — B → A");
const msgWait = waitFor(sa, ka.tag, 12000, "B's message on the group tag");
const delta  = B.call(B.E.forum_post, enc.encode(JSON.stringify({
  text: "Hello from the browser", gen: 1, epoch: kb.epoch,
})));
const encd   = B.json("mls_encrypt", { group_id: gid, delta: hex(delta) });
const sealed = B.text("seal_seal", JSON.stringify({
  inner: encd.message, tag: kb.tag, secret: kb.secret,
}));
const groupBlob = b64(unhex(sealed));
sb.pub(B.json("relay_sign_pub", { seed: kb.address_seed, blob: groupBlob }), groupBlob);
ok(true, "B published " + unhex(sealed).length + " sealed bytes to " + kb.tag.slice(0, 12) + "…");

const frame = await msgWait;
const inner = A.text("seal_open", JSON.stringify({
  blob: hex(unb64(frame.blob)), tag: ka.tag, secret: ka.secret,
}));
ok(true, "A unsealed it (" + unhex(inner).length + " bytes of MLS)");
const got = A.json("mls_decrypt", { group_id: gid, message: inner });
ok(got.kind === "application", "A decrypted it as an application message");
ok(got.sender === b.cred, "attributed to B's leaf, not carried in the payload");
ok(got.delta === hex(delta), "the delta is byte-identical to what B authored");

head("relay behaviour");
ok(true, "duplicate frames suppressed by (tag,seq): A " + sa.dupes() + ", B " + sb.dupes());

sa.close(); sb.close();
console.log("\n" + (failures ? failures + " FAILED" : "all passed — the browser's send path works over the real relay"));
process.exit(failures ? 1 : 0);
