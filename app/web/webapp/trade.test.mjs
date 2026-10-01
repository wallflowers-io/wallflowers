/* trade.test.mjs — W-98 Trade in the webapp: what the Trade board and a listing's page write
   is what the ICD declares (group.publishListing, group.removeListing, thing.setProfile,
   thing.addPhoto, thing.removePhoto, thing.setDisposition), held to the real ICD.

   Run: node --test app/web/webapp/trade.test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const Trade = createRequire(import.meta.url)('./trade.js');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
/* A Site's listings as core serialises them (core tests/w98_site_listings.rs writes and holds
   this file), never typed here: a key the UI reads is a key core serves. */
const LISTINGS = JSON.parse(readFileSync(new URL('./trade.listings.json', import.meta.url), 'utf8'));
const MINE = LISTINGS.find((l) => !l.site), THEIRS = LISTINGS.find((l) => l.site);
const op = (kind, name) => ICD.kinds[kind].ops[name];
const ME = 'c'.repeat(64), OTHER = 'e'.repeat(64), THING = 'd'.repeat(64);
const NOW = 1_790_000_000_000;

/* Every key the ICD declares for the op, every required one present but `gen` (the Door's). */
function declared(kind, name, args) {
  const spec = op(kind, name).args;
  for (const k of Object.keys(args)) assert.ok(k in spec, `${name} declares no \`${k}\``);
  for (const [k, s] of Object.entries(spec)) if (s.required && k !== 'gen') assert.ok(k in args, `${name} needs \`${k}\``);
}

const thing = (view) => ({ id: THING, kind: 'thing', owner: ME, view: { name: 'A bike', descriptor: 'Blue, 54 cm', category: 'artifact', reach: 'network', photos: [], ...view } });

test('a Thing with a posture lists on a Site as the ICD says; one with none cannot', () => {
  const a = Trade.listArgs(thing({ posture: 'Selling', price: '£40', area: 'Mill Road', photos: [{ id: '0000000000000001', mime: 'image/png', data: 'cHJvYmU=', at: 1 }] }), NOW);
  declared('group', 'group.publishListing', a);
  assert.deepEqual([a.thingId, a.title, a.posture, a.price, a.area, a.photo, a.photoMime], [THING, 'A bike', 'Selling', '£40', 'Mill Road', 'cHJvYmU=', 'image/png']);
  assert.ok(a.rev >= 0 && !('withdrawn' in a));
  assert.equal(Trade.listArgs(thing({ posture: null }), NOW), null);
});

test('withdrawing a listing restates it, withdrawn, at a later rev', () => {
  const a = Trade.withdrawArgs(MINE, NOW);
  declared('group', 'group.publishListing', a);
  assert.equal(a.withdrawn, 1);
  assert.deepEqual([a.thingId, a.posture, a.title, a.reach], [MINE.thingId, MINE.posture, MINE.title, MINE.reach]);
  assert.ok(a.rev > MINE.rev);
});

test('the owner or an admin removes a member\'s listing by (author, Thing)', () => {
  const a = Trade.removeArgs(THEIRS);
  declared('group', 'group.removeListing', a);
  assert.deepEqual(a, { author: THEIRS.author, thingId: THEIRS.thingId });
  assert.equal(Trade.mayRemove('owner'), true);
  assert.equal(Trade.mayRemove('admin'), true);
  assert.equal(Trade.mayRemove('member'), false);
});

test('the board is the Site\'s live listings, newest first, each marked yours or not', () => {
  const site = { id: 'a'.repeat(64), view: { listings: LISTINGS } };
  const byGen = LISTINGS.slice().sort((a, b) => b.gen - a.gen);
  const rows = Trade.board(site, MINE.author);
  assert.deepEqual(rows.map((r) => [r.title, r.mine]), byGen.map((l) => [l.title, l.author === MINE.author]));
  assert.deepEqual(Trade.board({ view: {} }, ME), []);
  assert.equal(Trade.listedOn(site, MINE.thingId, MINE.author).title, MINE.title);
  assert.equal(Trade.listedOn(site, THEIRS.thingId, MINE.author), undefined, 'another member\'s listing is not yours');
  assert.equal(Trade.picture(MINE), 'data:' + MINE.photoMime + ';base64,' + MINE.photo, 'a row draws its snapshot\'s photo');
  assert.equal(Trade.picture(THEIRS), null, 'none without one');
});

test('the profile restates what the Thing is, with its condition and description, as the ICD bounds them', () => {
  const a = Trade.profileArgs(thing({}).view, { condition: 'good', description: 'Serviced in May.\nNew chain.' }, ICD);
  declared('thing', 'thing.setProfile', a);
  assert.deepEqual(a, { name: 'A bike', descriptor: 'Blue, 54 cm', category: 'artifact', condition: 'good', description: 'Serviced in May.\nNew chain.' });
  const words = Object.keys(op('thing', 'thing.setProfile').args.condition.vocabulary);
  assert.deepEqual(Trade.conditions(ICD), words, 'the conditions are the ICD\'s words, in its order');
  assert.throws(() => Trade.profileArgs(thing({}).view, { condition: 'mint' }, ICD), /condition/);
  const max = op('thing', 'thing.setProfile').args.description.maxLength;
  assert.throws(() => Trade.profileArgs(thing({}).view, { description: 'x'.repeat(max + 1) }, ICD), /description/);
  assert.equal('condition' in Trade.profileArgs(thing({}).view, { condition: '' }, ICD), false, 'unstated is absent');
});

test('a photo is a 16-hex id, a still within the ICD\'s inline ceiling, and its time', () => {
  const a = Trade.photoArgs('data:image/jpeg;base64,/9j/4A==', ICD, NOW);
  declared('thing', 'thing.addPhoto', a);
  assert.match(a.id, new RegExp(op('thing', 'thing.addPhoto').args.id.pattern));
  assert.deepEqual([a.photo, a.photoMime, a.at], ['/9j/4A==', 'image/jpeg', NOW]);
  for (const mime of ['image/webp', 'image/gif'])
    assert.equal(Trade.photoArgs('data:' + mime + ';base64,UklGRg==', ICD, NOW), null, mime + ' is not a still to core (pacific-media MediaKind::Still)');
  const max = op('thing', 'thing.addPhoto').args.photo.maxLength;
  assert.equal(Trade.photoArgs('data:image/jpeg;base64,' + 'A'.repeat(max + 4), ICD, NOW), null, 'over the ceiling: not sent');
  assert.equal(Trade.photoArgs('data:text/html;base64,PGI+', ICD, NOW), null, 'not a still');
  declared('thing', 'thing.removePhoto', Trade.removePhotoArgs('0000000000000001'));
  assert.equal(Trade.roomForPhotos(thing({ photos: Array(op('thing', 'thing.addPhoto').maxLive).fill({}) }).view, ICD), false);
  assert.equal(Trade.roomForPhotos(thing({}).view, ICD), true);
});

test('a disposition is the ICD\'s word; reserved names the member and when the hold ends', () => {
  const a = Trade.dispositionArgs('available', NOW, null, null, ICD);
  declared('thing', 'thing.setDisposition', a);
  assert.deepEqual(a, { state: 'available', at: NOW });
  const r = Trade.dispositionArgs('reserved', NOW, OTHER, NOW + 86_400_000, ICD);
  declared('thing', 'thing.setDisposition', r);
  assert.deepEqual(r, { state: 'reserved', at: NOW, for: OTHER, until: NOW + 86_400_000 });
  assert.throws(() => Trade.dispositionArgs('gone', NOW, null, null, ICD), /state/);
  assert.throws(() => Trade.dispositionArgs('reserved', NOW, null, null, ICD), /reserved/);
  assert.deepEqual(Trade.dispositions(ICD), Object.keys(op('thing', 'thing.setDisposition').args.state.vocabulary));
});
