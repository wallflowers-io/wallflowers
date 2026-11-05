/* not-ready-for-merge.test.mjs — app/ios is NOT READY FOR MERGE while any test here fails.

   A. Content that must not reach a public repository, or that is wrong there. Each test
      scans app/ios's tracked files, names file:line, and passes once fixed; it then stays
      as a guard.
   B. Open work a scan cannot see. Fails unconditionally; delete an item when it is done,
      and the test when the list is empty.

   Run: node --test app/ios/not-ready-for-merge.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const IOS = dirname(fileURLToPath(import.meta.url));
const REPO = join(IOS, '..', '..');
const SELF = basename(fileURLToPath(import.meta.url));

const tracked = execFileSync('git', ['ls-files', '-z'], { cwd: IOS, encoding: 'utf8' })
  .split('\0').filter((f) => f && f !== SELF);

/* Every line of every tracked text file, as { at: 'file:line', text }. */
const lines = tracked.flatMap((f) => {
  const buf = readFileSync(join(IOS, f));
  if (buf.includes(0)) return [];
  return buf.toString('utf8').split('\n').map((text, i) => ({ at: `${f}:${i + 1}`, text }));
});

const hits = (re) => lines.filter((l) => re.test(l.text)).map((l) => `  ${l.at}  ${l.text.trim().slice(0, 120)}`);

function empty(found, why) {
  assert.equal(found.length, 0, `${why}\n${found.join('\n')}`);
}

const none = (re, why) => empty(hits(re), why);

test('A1 no third-party confidential material (YC batch data, Bookface)', () => {
  const file = 'Docs/YC-SYSTEM-TARGET-STATE.txt';
  // Docs/n1-journey.html names the founder's own batch; kept by Ralph, 2026-11-05.
  const kept = ['Docs/n1-journey.html'];
  empty([
    ...(tracked.includes(file) ? [`  ${file}  a connector design against YC's account API and Bookface`] : []),
    ...hits(/bookface|search\.companies|\bS26\b/i).filter((h) => !kept.some((k) => h.startsWith(`  ${k}:`))),
  ], 'YC batch enumeration and batch references:');
});

test('A2 no absolute paths on a developer machine', () => {
  none(/\/Users\/[A-Za-z]/, 'absolute local paths:');
});

test('A3 every image has recorded provenance or a ruling', () => {
  // Fixtures carry source, licence and credit in their manifest. The rest were reviewed one by
  // one and kept by Ralph, 2026-11-05.
  const ruled = ['Docs/Van_Gogh_Color_Pie_Chart.jpg', 'ui/SeedMedia/beachcrimes.jpg', 'ui/SeedMedia/upward.jpg',
                 'ui/App/Life/Space/kitchen.pspc', 'ui/App/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png'];
  const manifest = JSON.parse(readFileSync(join(IOS, 'ui/Fixtures/media/manifest.json'), 'utf8'));
  const credited = new Set(Object.values(manifest).filter((m) => m.source && m.license && m.credit)
    .map((m) => `ui/Fixtures/media/${m.file}`));
  const media = tracked.filter((f) => /\.(jpe?g|png|gif|heic|webp|pspc|usdz|mp4|mov)$/i.test(f));
  empty(media.filter((f) => !credited.has(f) && !ruled.includes(f)).map((f) => `  ${f}`),
    'images with no source, licence and credit, and no ruling:');
});

test('A4 no retired kenjin.cc endpoints', () => {
  none(/(wss?|https?):\/\/[a-z0-9.-]*kenjin\.cc/, 'kenjin.cc is retired; these still dial it:');
});

test('A5 no old brand shown to users', () => {
  none(/\? "P" : "K"/, 'theme switch labelled P/K (Pacific/Kenjin):');
});

test('A6 no references to files outside this repository', () => {
  const dead = lines.flatMap((l) => [...l.text.matchAll(/coordination\/([A-Za-z0-9._-]+\.(?:txt|md))/g)]
    .filter((m) => !existsSync(join(REPO, 'core', 'coordination', m[1])))
    .map((m) => `  ${l.at}  coordination/${m[1]}`));
  empty([...dead, ...hits(/\.claude\//)], 'references to files not in this repository:');
});

test('B open work', () => {
  assert.fail([
    'app/ios is NOT READY FOR MERGE. Open:',
    '  ICD: arc-resolver icd_graph fails 4 tests against core/coordination/delta-graph.icd.json',
    '       (cd app/ios/LocalStore/graph/arc-resolver && cargo test)',
    '  ICD: notes live only in LodeDB (ui/App/Notes/NotesStore.swift); note.write is never emitted',
    '  ICD: every op the app emits or folds, unaudited against the ICD',
    '  UI tests: 22 of 57 fail in WallFlowersUITests, untriaged (just test); unit 116/116 pass',
    '  Sign-in: BYO key with biometric Keychain; wallflowers.io passkey unverified end to end',
    '  Brand: "Sysiphus" in user-visible strings, undecided',
  ].join('\n'));
});
