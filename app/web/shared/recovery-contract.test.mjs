/* ═══════════════════════════════════════════════════════════════════════════
   recovery-contract.test.mjs — spine_read says what the contract says.

   recovery-contract.json is what three sessions agreed a fresh device draws
   about what came back. This holds core-wasm's spine_read to it, against the
   REAL module. The names are read out of the contract, not restated here, so
   the contract and this test cannot drift apart: rename a field in either the
   core or the contract and this goes red.

   WHAT IT CAN AND CANNOT REACH. Nothing in the browser build seals a spine
   entry, so no case here has an entry that OPENS. What it pins is the shape,
   the head-0 trap, the tail judged apart from the holes, and that a blob which
   will not open is counted as unopened rather than found. The semantics of
   opened entries are pinned where entries can be made, in core's own tests.

   Run:  node app/web/shared/recovery-contract.test.mjs
   ═══════════════════════════════════════════════════════════════════════════ */
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const here = path.dirname(fileURLToPath(import.meta.url));
const CORE = path.join(here, '..', 'docs', 'core', 'core_wasm_bg.wasm');
const contract = JSON.parse(fs.readFileSync(path.join(here, 'recovery-contract.json'), 'utf8'));

if (!fs.existsSync(CORE)) {
  console.error(`missing ${CORE}\n  run app/web/build-wasm.sh`);
  process.exit(2);
}

let fails = 0;
const ok = (what, cond, detail) => {
  if (!cond) fails++;
  console.log(`${cond ? '  ok  ' : ' FAIL '} ${what}${cond || !detail ? '' : `\n         ${detail}`}`);
};
const same = (what, got, want) => ok(what, JSON.stringify(got) === JSON.stringify(want),
  `got  ${JSON.stringify(got)}\n         want ${JSON.stringify(want)}`);

/* The real core, with the host entropy it declares as an import. */
const mod = await WebAssembly.compile(fs.readFileSync(CORE));
let inst;
const imports = {};
for (const i of WebAssembly.Module.imports(mod)) {
  (imports[i.module] ||= {})[i.name] = i.name === 'pacific_fill_random'
    ? (ptr, len) => { crypto.getRandomValues(new Uint8Array(inst.exports.memory.buffer, ptr, len)); return 0; }
    : () => 0;
}
inst = await WebAssembly.instantiate(mod, imports);
const core = inst.exports;
const enc = new TextEncoder(), dec = new TextDecoder();

/* The ABI every export shares. A refusal returns 0 and leaves its words in the
   out buffer, so they are read by out_len(), never by the returned length. */
function call(fn, arg) {
  const b = enc.encode(JSON.stringify(arg));
  const p = core.alloc(b.length);
  new Uint8Array(core.memory.buffer, p, b.length).set(b);
  const n = core[fn](b.length);
  if (core.erred()) {
    return { refused: dec.decode(new Uint8Array(core.memory.buffer, core.out_ptr(), core.out_len())) };
  }
  return JSON.parse(dec.decode(new Uint8Array(core.memory.buffer, core.out_ptr(), n)));
}

/* Everything the contract names, taken from the contract. */
const recoveryFields = Object.keys(contract.per_account.fields).sort();
/* Only the leading `"a" | "b" | …` enumeration: the prose after it quotes values too. */
const tailEnum = (contract.per_account.fields.tail.match(/^\s*"[a-z_]+"(?:\s*\|\s*"[a-z_]+")*/) || [''])[0];
const tailValues = [...tailEnum.matchAll(/"([a-z_]+)"/g)].map(m => m[1]);
const readOut = Object.keys(contract.exports.spine_read.out).sort();
const seed = '11'.repeat(32);
const read = (head, fetched) => call('spine_read', { seed, head, fetched });

console.log('\nthe exports are in the module the web ships');
const exported = WebAssembly.Module.exports(mod).map(e => e.name);
for (const n of Object.keys(contract.exports).filter(k => !k.startsWith('_') && k !== 'superseded')) {
  ok(`${n} is exported`, exported.includes(n), 'rebuild with app/web/build-wasm.sh');
}

console.log('\nspine_read returns the contract, name for name');
const empty = read(null, []);
same('its top level is the contract\'s', Object.keys(empty).sort(), readOut);
same('its recovery record is the contract\'s', Object.keys(empty.recovery || {}).sort(), recoveryFields);
ok('the tail values come from the contract', tailValues.length === 4, JSON.stringify(tailValues));

console.log('\nTHE HEAD-0 TRAP: a head that never advanced proves nothing');
same('no head: tail unknown, nothing found, nothing claimed', empty.recovery,
     { extra: 0, found: 0, holes: [], tail: 'unknown' });
const zero = read({ position: 0, tail: '00'.repeat(32) }, []);
same('position 0, which is every account today: tail unknown', zero.recovery.tail, 'unknown');
ok('and it is never read as intact', zero.recovery.tail !== 'intact');

console.log('\nthe tail is judged apart from the holes');
const three = read({ position: 3, tail: 'ab'.repeat(32) }, []);
same('a head at 3 with nothing fetched: the tail is missing', three.recovery.tail, 'missing');
same('and the holes stop below the tail index', three.recovery.holes, [0, 1]);
ok('every tail value is one the contract lists', [empty, zero, three].every(r => tailValues.includes(r.recovery.tail)));

console.log('\na blob that will not open is not found');
const junk = read(null, [{ index: 0, blobs: ['AAAAAAAA'] }]);
same('it is counted as unopened', junk.unopened, 1);
same('and not as found', junk.recovery.found, 0);
same('and names no object', junk.objects, []);

console.log('\nspine_addresses gives one unrelated address per index');
const addr = call('spine_addresses', { seed, gen: 0, from: 0, count: 3 });
same('indices in order', (addr.addresses || []).map(a => a.index), [0, 1, 2]);
ok('each tag is 64 hex', (addr.addresses || []).every(a => /^[0-9a-f]{64}$/.test(a.tag)));
ok('no two alike', new Set((addr.addresses || []).map(a => a.tag)).size === 3);
const over = call('spine_addresses', { seed, gen: 0, from: 0, count: 257 });
ok('over one batch it is refused', !!over.refused);
ok('and the refusal is in words', !!(over.refused && over.refused.length), JSON.stringify(over));

console.log(fails ? `\n${fails} FAILED — spine_read and the contract disagree\n` : '\nspine_read keeps the contract\n');
process.exit(fails ? 1 : 0);
