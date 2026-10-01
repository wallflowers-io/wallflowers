// The core's own answer, for e2e.py to be checked against.
//
// WHY THIS EXISTS. e2e.py retypes constants from `pacific-core` and rebuilds
// three derivations on top of them. Two implementations that agree only
// because they share code agree about nothing — and the failure when they stop
// agreeing is SILENT: a wrong tag raises no error anywhere, it just puts two
// devices on different mailboxes where neither ever hears the other. So the
// Python is put against the Rust, through the same wasm the browser loads.
//
//   node smoke/core-vectors.mjs <seed-hex> <prf-hex> <host> <wrap-hex>
//
// Prints one JSON object. Every field is what THE CORE says; e2e.py compares.
// `opened_wrap` is the other direction — the core opening what Python sealed —
// because an envelope both sides can only write is not one either side can read.
// (The history pair went with the history blob, a58798c: the escrow retired on
// 14 Sep has no key and no reader left in the core to check against.)
//
// Loaded the way relay-pair.mjs loads it: the module's own import list, three
// host functions, no wasm-bindgen glue. The glue is an ES module importing the
// bare specifier "env", which needs an import map a page has and Node does not.
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { webcrypto as wc } from "node:crypto";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WASM = path.resolve(HERE, "..", "docs", "core", "core_wasm_bg.wasm");

const hex = (u) => Buffer.from(u).toString("hex");
const unhex = (h) => new Uint8Array(Buffer.from(h, "hex"));
const utf8 = (s) => new TextEncoder().encode(s);
const text = (u) => new TextDecoder().decode(u);

const mod = await WebAssembly.compile(fs.readFileSync(WASM));
let inst = null;
const at = () => inst;
const HOSTFNS = {
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
  const mk = HOSTFNS[i.name];
  if (!mk) throw new Error("unsupplied import " + i.module + "." + i.name);
  (imports[i.module] ||= {})[i.name] = mk(at);
}
inst = await WebAssembly.instantiate(mod, imports);
if (inst.exports.__wbindgen_start) inst.exports.__wbindgen_start();
const E = inst.exports;

function call(name, bytes) {
  const p = E.alloc(bytes.length);
  new Uint8Array(E.memory.buffer, p, bytes.length).set(bytes);
  const n = E[name](bytes.length);
  const out = new Uint8Array(E.memory.buffer, E.out_ptr(), E.out_len()).slice();
  if (E.erred()) throw new Error(text(out));
  return out.slice(0, n);
}
const callJson = (name, obj) => text(call(name, utf8(JSON.stringify(obj))));

const [seedHex, prfHex, host, wrapHex] = process.argv.slice(2);
const seed = unhex(seedHex);
const out = {
  identity_key: hex(call("identity_key", seed)),
  account_channel: hex(call("account_channel", seed)),
  wrap_key: hex(call("wrap_key", unhex(prfHex))),
};
// The core reading what Python wrote. A failure here is reported rather than
// thrown: "the core could not open it" is the answer, not a crash.
try {
  out.opened_wrap = callJson("wrap_open", { prf: prfHex, blob: wrapHex, host });
} catch (e) { out.opened_wrap = "ERR " + e.message; }

console.log(JSON.stringify(out));
