#!/usr/bin/env node
/*
 * What /map runs that this repository did not write (assets/map/): every file
 * is the bytes its lock names, and carries its licence's text beside it.
 *
 * The tile archive is not committed. Present, it is held to the lock too;
 * absent, this says so and passes — /map has no map until scripts/cut-tiles.sh
 * has been run, and that is a step for whoever ships the page.
 *
 * Run: node play/tests/test_vendor.js
 */
'use strict';
var fs = require('fs');
var path = require('path');
var crypto = require('crypto');
var assert = require('assert');
var map = path.join(__dirname, '..', 'assets', 'map');
var lock = JSON.parse(fs.readFileSync(path.join(map, 'VENDOR.json'), 'utf8'));
function sha(file) { return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'); }

assert.ok(lock.code.length >= 5);
lock.code.forEach(function (f) {
  var file = path.join(map, f.path);
  assert.ok(fs.existsSync(file), f.path + ' is in the lock and not in the tree');
  assert.strictEqual(sha(file), f.sha256, f.path + ' is not the ' + f.package + ' ' + f.version + ' the lock names');
  assert.ok(f.from.indexOf(f.package + '@' + f.version) > 0, f.path + ' says one version and was fetched as another');
  assert.ok(fs.statSync(path.join(map, f.licence_text)).size > 500, f.path + ' has no licence text beside it');
});
/* nothing unlisted: a file dropped in here is third-party code nobody pinned */
var listed = lock.code.map(function (f) { return f.path; }).concat(lock.code.map(function (f) { return f.licence_text; }), ['VENDOR.json', 'tiles']);
fs.readdirSync(map).forEach(function (name) { assert.ok(listed.indexOf(name) >= 0, 'assets/map/' + name + ' is not in the lock'); });

var tiles = path.join(map, lock.tiles.path);
if (!fs.existsSync(tiles)) { console.log('ok: assets/map code. TILES NOT STAGED: /map draws no map until scripts/cut-tiles.sh is run'); return; }
assert.strictEqual(fs.readFileSync(tiles).subarray(0, 7).toString(), 'PMTiles');
assert.strictEqual(sha(tiles), lock.tiles.sha256, lock.tiles.path + ' is not the cut the lock names');
console.log('ok: assets/map code and tiles (' + lock.tiles.path + ', build ' + lock.tiles.build + ')');
