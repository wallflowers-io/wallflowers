/* geo.js — where a real place is: as a geohash, and as a spot on a flat plan.
 *
 * A GEOHASH IS THE PLACE'S NAME. Nine characters is a cell under five metres
 * across: what the iPhone app mints a venue at (Geohash.swift, venuePrecision),
 * and the same alphabet and bit order as core's (pacific-core, thing.rs
 * `geohash`). This is that arithmetic again, for a page that does not load the
 * core; tests/test_land.js holds it to the published vectors.
 *
 * NOTHING HERE TOUCHES THE DOM, so node can run it.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else (root.Kenjin = root.Kenjin || {}).Geo = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';
  var B32 = '0123456789bcdefghjkmnpqrstuvwxyz';

  function encode(lat, lng, n) {
    var la = [-90, 90], lo = [-180, 180], out = '', bit = 0, ch = 0, even = true;
    while (out.length < n) {
      var r = even ? lo : la, v = even ? lng : lat, mid = (r[0] + r[1]) / 2;
      if (v >= mid) { ch |= 16 >> bit; r[0] = mid; } else r[1] = mid;
      even = !even;
      if (++bit === 5) { out += B32[ch]; bit = 0; ch = 0; }
    }
    return out;
  }
  function bounds(hash) {
    var la = [-90, 90], lo = [-180, 180], even = true;
    for (var i = 0; i < hash.length; i++) {
      var ch = B32.indexOf(hash[i]);
      if (ch < 0) return null;
      for (var bit = 16; bit; bit >>= 1) {
        var r = even ? lo : la, mid = (r[0] + r[1]) / 2;
        if (ch & bit) r[0] = mid; else r[1] = mid;
        even = !even;
      }
    }
    return { s: la[0], n: la[1], w: lo[0], e: lo[1] };
  }
  function centre(hash) { var b = bounds(hash); return b ? [(b.s + b.n) / 2, (b.w + b.e) / 2] : null; }

  /* Metres east and south of an origin. Flat is near enough across a city. */
  function plan(lat0, lng0) {
    var kx = 111320 * Math.cos(lat0 * Math.PI / 180), kz = 110540;
    return function (lat, lng) { return [(lng - lng0) * kx, -(lat - lat0) * kz]; };
  }
  /* FNV-1a: a string to a seed, the same on every device. */
  function fnv(str) {
    var h = 0x811c9dc5;
    for (var i = 0; i < str.length; i++) { h ^= str.charCodeAt(i); h = Math.imul(h, 0x01000193); }
    return h >>> 0;
  }
  return { encode: encode, bounds: bounds, centre: centre, plan: plan, fnv: fnv };
});
