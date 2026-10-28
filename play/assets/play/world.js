/* world.js — the how-it-works scene, built.
 *
 * assets/how/ceremony.js DRAWS this: an egg at the left, a sunflower planted at
 * the right, six boards on bamboo poles between them, deep soil under it all.
 * Here the same scene is a block of voxels, one to an art pixel, so it can be
 * walked through and seen from any side.
 *
 * THE COLOURS, THE WORDS AND THE PARTS ARE CEREMONY.JS'S (Kenjin.Ceremony.art).
 * Its palette, its noise, its egg's profile and panels, its key, seed, poles,
 * leaves, blooms and deltas are read from it; nothing here restates a colour.
 * What is this file's own is the third dimension: a cross-section became a
 * block, a silhouette became a solid of revolution, a bush became a mound.
 *
 * THE CUT FACES ARE THE PICTURE. Each side of the block is ceremony.js's
 * ground, painted by its rules: topsoil threaded with grass roots, dirt with
 * clay, pebbles, ore and the odd worm, a seam of stone holding the wordmark's
 * colours, bedrock. The front face also keeps the bones, the skull and the
 * chest, where the picture has them. Inside is plain: nobody sees it.
 *
 * THE ROOTS GROW ON THE FRONT FACE, a voxel at a time, like an ant farm's glass:
 * that is where the picture shows them, and a root inside the block would be a
 * root nobody watched.
 *
 * A DRAWING, AS CEREMONY.JS IS. No key, seed, group or delta here is real, and
 * nothing a person does in it is kept: no Delta carries any of it.
 */
(function (root) {
  'use strict';
  var Vox = root.Vox;

  var PITCH = 44, TILE = 38, COLS = 8;       // a column, and a board, in voxels
  var W = 448, H = 320, D = 96;
  var GY = 112;                              // the grass line: below it is ground
  var LZ = 54;                               // the plane the poles, the flower and the egg stand in
  var X0 = 92;                               // column 0's centre
  var SOIL = 62, STONE = 22;
  var RAISE = PITCH;                         // a board's foot above the grass: one row up
  var BOARD_D = 6, SLAB_W = 30, SLAB_D = 4;
  var EGG_X = 38;
  function colX(c) { return X0 + c * PITCH; }
  function lerp(a, b, t) { return a + (b - a) * t; }

  /* ids to stuff: four colours and a way of laying them (engine.js, the
     patterns). The first colour's alpha is how far a block of eight art pixels
     may shade from its neighbours (ceremony.js's `tone`); the second's is the
     pattern. One colour four times over is a plain voxel, and at a grain of one
     that is all there is. */
  function Palette(world) {
    this.data = new Uint8Array(256 * 64 * 4); this.n = 1; this.map = {}; this.world = world; this.changed = true;
  }
  function byte(v) { return Math.max(0, Math.min(255, Math.round(v))); }
  Palette.prototype.id = function (c, tone, soft) { return this.mix([c, c, c, c], { tone: tone, soft: soft }); };
  Palette.prototype.mix = function (cs, o) {
    var t = Math.round((o.tone || 0) * 510), pat = o.pattern || 0;
    var key = cs.map(function (c) { return byte(c[0]) << 16 | byte(c[1]) << 8 | byte(c[2]); }).join('/') + '/' + t + '/' + pat + (o.soft ? 's' : ''), id = this.map[key];
    if (id) return id;
    if (this.n >= 4096) return 1;
    id = this.map[key] = this.n++;
    for (var k = 0; k < 4; k++)
      this.data.set([byte(cs[k][0]), byte(cs[k][1]), byte(cs[k][2]), k === 0 ? 255 - t : k === 1 ? pat : 255], ((k * 16 + (id >> 8)) * 256 + (id & 255)) * 4);
    if (o.soft) this.world.soft[id] = 1;
    this.changed = true;
    return id;
  };

  /* A picture's pixels, top row first. */
  function pixels(canvas) {
    var w = canvas.width, h = canvas.height, d = canvas.getContext('2d').getImageData(0, 0, w, h).data;
    return { w: w, h: h, at: function (x, y) {
      if (x < 0 || y < 0 || x >= w || y >= h) return null;
      var i = (y * w + x) * 4;
      return d[i + 3] > 127 ? [d[i], d[i + 1], d[i + 2]] : null;
    } };
  }
  /* A picture given depth: `inner(x, y, z)` may say what lies behind the front. */
  function extrude(pal, pix, depth, inner) {
    var m = new Vox.World(pix.w, pix.h, depth);
    for (var y = 0; y < pix.h; y++) for (var x = 0; x < pix.w; x++) {
      var c = pix.at(x, y);
      if (!c) continue;
      for (var z = 0; z < depth; z++) {
        var cz = z === depth - 1 || !inner ? c : inner(x, y, z, c);
        if (cz) m.set(x, pix.h - 1 - y, z, pal.id(cz));
      }
    }
    return { mesh: Vox.meshAll(m), w: pix.w, h: pix.h, d: depth };
  }

  /* ── THE PARTS THAT MOVE: each its own small block, at a grain of one ── */
  function parts(Ceremony, Tama, pal) {
    var A = Ceremony.art, P = A.P, hash = A.hash, shade = A.shade;
    function board(i, lit, tint) {                                              // tint: a shell colour other than the picture's own
      var R = Ceremony.PICTURES[A.CHAIN[i]](), LCD = Tama.LCD, shell = A.GEMS[(tint === undefined ? i : tint) % A.GEMS.length];
      var rim = shade(shell, -0.45), hi = shade(shell, 0.3), lo = shade(shell, -0.22), m = new Vox.World(TILE, TILE, BOARD_D), o = (TILE - 32) / 2;
      function tone(v) {
        if (v === 0) return LCD.bg;
        var t = v === 1 ? 1 : v === 2 ? null : Tama.SKIN_T[v - 2];
        if (lit) return v === 1 ? lit : v === 2 ? shade(lit, 0.3) : shade(lit, 0.55);
        return t === null ? LCD.mid : [0, 1, 2].map(function (j) { return lerp(LCD.bg[j], LCD.ink[j], t); });
      }
      for (var y = 0; y < TILE; y++) for (var x = 0; x < TILE; x++) {
        var ex = Math.min(x, TILE - 1 - x), ey = Math.min(y, TILE - 1 - y);
        if (ex + ey < 2) continue;                                              // rounded corners
        var c = ex === 0 || ey === 0 || ex + ey === 2 ? rim : ex === 1 || ey === 1 ? (x === 1 || y === 1 ? hi : lo) : shell;
        var screen = x >= o && y >= o && x < o + 32 && y < o + 32;
        for (var z = 0; z < BOARD_D; z++) {
          if (z === BOARD_D - 1 && screen) continue;                            // the screen sits a voxel back in its bezel
          m.set(x, TILE - 1 - y, z, pal.id(z === BOARD_D - 2 && screen ? tone(R[(y - o) * 32 + (x - o)]) : c));
        }
      }
      return { mesh: Vox.meshAll(m), w: TILE, h: TILE, d: BOARD_D };
    }
    function pole(len) {
      var pix = pixels(A.poleArt(len).c), m = new Vox.World(5, pix.h, 5);
      for (var y = 0; y < pix.h; y++) for (var z = 0; z < 5; z++) for (var x = 0; x < 5; x++) {
        if ((x === 0 || x === 4) && (z === 0 || z === 4)) continue;
        m.set(x, pix.h - 1 - y, z, pal.id(pix.at(z === 0 || z === 4 ? Math.min(x, 1) : x, y)));
      }
      return { mesh: Vox.meshAll(m), w: 5, h: pix.h, d: 5 };
    }
    function author(spec, n) {
      return spec.colour && /^#[0-9a-f]{6}$/i.test(spec.colour.outfit || '')
        ? [1, 3, 5].map(function (k) { return parseInt(spec.colour.outfit.slice(k, k + 2), 16); }) : A.GEMS[n % A.GEMS.length];
    }
    function slab(spec, n) {
      var col = author(spec, n), pix = pixels(A.deltaArt(col, A.miniFace(Tama, spec), SLAB_W, A.SLAB_H));
      return extrude(pal, pix, SLAB_D, function (x, y) { return pix.at(x === 0 || x === SLAB_W - 1 ? x : SLAB_W - 2, y); });
    }
    var F = A.flowerArt(256 / 7);
    function stem() {
      var pix = pixels(F.stem), m = new Vox.World(3, F.Hs, 3);
      for (var y = 0; y < F.Hs; y++) for (var z = 0; z < 3; z++) for (var x = 0; x < 3; x++) m.set(x, y, z, pal.id(pix.at(x, 0)));
      return { mesh: Vox.meshAll(m), w: 3, h: F.Hs, d: 3 };
    }
    function leaf(k, deg) {
      var L = A.leafArt(F.len, F.wid, deg, 1, 1), e = extrude(pal, pixels(L.c), 1);
      e.bx = L.bx; e.by = e.h - 1 - L.by;                                       // where it joins the stem
      return e;
    }
    function shoot() {                                                          // a root come up for air
      var m = new Vox.World(6, 6, 2), rows = ['l....l', '.l..l.', '..dd..', '..pp..', '..pp..', '..pp..'];
      var c = { l: P.paleLight, d: P.paleDark, p: P.pale };
      rows.forEach(function (row, y) { for (var x = 0; x < 6; x++) if (c[row[x]]) for (var z = 0; z < 2; z++) m.set(x, 5 - y, z, pal.id(c[row[x]])); });
      return { mesh: Vox.meshAll(m), w: 6, h: 6, d: 2 };
    }
    function cloud(n) {
      var w = Math.round(PITCH * (0.7 + hash(n, 31) * 0.8)), h = Math.round(w * (0.3 + hash(n, 32) * 0.1)), d = Math.round(w * 0.6);
      var k, count = 3 + Math.floor(hash(n, 1) * 3), lobes = [], m = new Vox.World(w, h, d);
      for (k = 0; k < count; k++)
        lobes.push([w * (0.18 + 0.64 * k / (count - 1)) + (hash(n, k + 5) - 0.5) * 4, h * 0.62, d * (0.5 + (hash(n, k + 15) - 0.5) * 0.4), h * (0.34 + hash(n, k + 9) * 0.28)]);
      function inside(x, y, z) {
        if (y > h * 0.86) return false;
        return lobes.some(function (l) { return Math.hypot(x - l[0], (y - l[1]) * 1.15, (z - l[2]) * 0.8) <= l[3]; });
      }
      for (var y = 0; y < h; y++) for (var z = 0; z < d; z++) for (var x = 0; x < w; x++) {
        if (!inside(x + 0.5, y + 0.5, z + 0.5)) continue;
        var under = !inside(x + 0.5, y + 2.5, z + 0.5), over = !inside(x + 0.5, y - 1.5, z + 0.5);
        m.set(x, h - 1 - y, z, pal.id(under ? P.cloudShade : over && x < w * 0.6 ? P.cloudLight : P.cloud));
      }
      return { mesh: Vox.meshAll(m), w: w, h: h, d: d, v: (0.0018 + hash(n, 35) * 0.0028) * 1000, y: hash(n, 34) };
    }
    /* A diamond: what The Sims hangs over whoever you are, and here over where you are going too. */
    function gem(light, mid, dark) {
      var R = 4, T = 7, m = new Vox.World(2 * R + 1, 2 * T + 1, 2 * R + 1);
      for (var y = -T; y <= T; y++) {
        var r = Math.round(R * (1 - Math.abs(y) / (T + 0.5)));
        for (var z = -r; z <= r; z++) for (var x = -r; x <= r; x++)
          if (Math.abs(x) + Math.abs(z) <= r) m.set(x + R, y + T, z + R, pal.id(y > 1 ? (x + z < 0 ? light : mid) : x + z < 0 ? mid : dark));
      }
      return { mesh: Vox.meshAll(m), w: 2 * R + 1, h: 2 * T + 1, d: 2 * R + 1 };
    }
    return { board: board, pole: pole, author: author, slab: slab, flower: F, stem: stem, leaf: leaf, shoot: shoot, cloud: cloud, gem: gem,
             bloom: function (j) { return extrude(pal, pixels(F.blooms[j]), 3); },
             key: function () { return extrude(pal, pixels(A.keyArt()), 2); },
             seed: function () { return extrude(pal, pixels(A.seedArt()), 2); } };
  }

  function build(Ceremony, Tama, cast) {
    var A = Ceremony.art, P = A.P, hash = A.hash, n2 = A.noise2, shade = A.shade;
    var world = new Vox.World(W, H, D), pal = new Palette(world), ids = {};
    function id(name, tone, soft) {
      var k = name + (tone || '') + (soft ? 's' : '');
      return ids[k] || (ids[k] = pal.id(P[name], tone, soft));
    }
    function h3(x, y, z) { return hash(x + z * 7919, y + z * 104729); }

    /* ── THE GROUND ── */
    function strata(x, z) {
      var soilEnd = SOIL + Math.round((n2(x, z, 13, 3) - 0.5) * 10);
      return { soilEnd: soilEnd, stoneEnd: soilEnd + STONE + Math.round((n2(x, z, 9, 5) - 0.5) * 8),
               topEnd: Math.round(SOIL * 0.24 + (n2(x, z, 11, 6) - 0.5) * 8),
               drip: h3(x, 21, z) < 0.4 ? 1 + Math.floor(h3(x, 22, z) * 3) : 0 };
    }
    (function fill() {
      var x, z, k, r, v, plain = [id('top', 0.1), id('dirt', 0.1), id('stone', 0.08), id('bed')];
      var tip = id('grassTip'), light = id('grassLight'), grass = id('grass'), dark = id('grassDark'), deep = id('grassDeep'), topDark = id('topDark');
      for (z = 0; z < D; z++) for (x = 0; x < W; x++) {
        var s = strata(x, z);
        for (k = 0; k < GY; k++) {
          if (k > 6) v = k < s.topEnd ? plain[0] : k < s.soilEnd ? plain[1] : k < s.stoneEnd ? plain[2] : plain[3];
          else {
            r = h3(x, k, z);
            if (k === 0) v = r < 0.5 ? tip : light;
            else if (k === 1) v = r < 0.3 ? light : grass;
            else if (k === 2) v = r < 0.4 ? grass : dark;
            else if (k < 3 + s.drip) v = dark;
            else if (k === 3 + s.drip) v = r < 0.5 ? deep : topDark;
            else v = plain[0];
          }
          world.set(x, GY - 1 - k, z, v);
        }
      }
    })();
    /* One cut face, by ceremony.js's groundTexture: `a` runs along it, `k` down
       from the grass line; `at(a, t)` is the column t voxels in from the face. */
    function face(len, at, salt, keeps) {
      var mat = new Uint8Array(len * GY), a, k, r, f;
      function put(a2, k2, c, m, tone) {
        if (a2 < 0 || k2 < 7 || a2 >= len || k2 >= GY) return;
        var v = pal.id(c, tone);
        for (var t = 0; t < 2; t++) { var p = at(a2, t); world.set(p[0], GY - 1 - k2, p[1], v); }
        if (m !== undefined) mat[k2 * len + a2] = m;
      }
      function m(a2, k2) { return a2 < 0 || k2 < 0 || a2 >= len || k2 >= GY ? 0 : mat[k2 * len + a2]; }
      function rr(a2, k2) { return hash(a2 + salt * 5003, k2); }
      for (a = 0; a < len; a++) {
        var p0 = at(a, 0), s = strata(p0[0], p0[1]);
        for (k = 7; k < GY; k++) {
          r = rr(a, k);
          if (k < s.soilEnd) {
            if (k < s.topEnd) put(a, k, r < 0.28 ? P.topDark : r < 0.5 ? P.topLight : P.top, 2, 0.1);
            else if (n2(a, k, 10, 31 + salt) > 0.76) put(a, k, r < 0.22 ? P.clayDark : r < 0.36 ? P.clayLight : P.clay, 4, 0.08);
            else {
              var clump = n2(a, k, 5, 41 + salt) > 0.74;
              put(a, k, clump ? (r < 0.5 ? P.dirtDark : P.topLight)
                    : r < 0.16 ? P.dirtDark : r < 0.27 ? P.dirtLight : r < 0.3 ? P.dirtHi : P.dirt, 3, 0.1);
            }
          } else if (k < s.stoneEnd) put(a, k, r < 0.2 ? P.stoneDark : r < 0.32 ? P.stoneLight : P.stone, 5, 0.08);
          else put(a, k, r < 0.16 ? P.bedDark : r < 0.22 ? P.bedLight : P.bed, 6);
        }
      }
      for (k = 7; k < GY; k++) for (a = 0; a < len; a++) {
        var mm = m(a, k);
        if (mm === 2 && rr(a * 3 + 7, k * 5 + 1) < 0.004)                       // a grass root, down from the lip
          for (f = 0; f < 3 + Math.floor(rr(a, k + 5) * 5); f++) if (m(a, k + f) === 2) put(a, k + f, P.fibre);
        if ((mm === 2 || mm === 3) && rr(a * 5 + 1, k * 7 + 3) < 0.0035) {      // a pebble
          put(a, k, P.pebbleLight); put(a + 1, k, P.pebble); put(a, k + 1, P.pebble); put(a + 1, k + 1, P.pebbleDark);
          if (rr(a, k + 3) < 0.5) { put(a + 2, k, P.pebble); put(a + 2, k + 1, P.pebbleDark); }
        }
        if (mm === 3 && rr(a * 13 + 5, k * 11 + 9) < 0.0005)                    // a worm
          [0, -1, -1, 0, 0].forEach(function (o, i) { put(a + i, k + o, i === 0 || i === 4 ? P.wormDark : P.worm); });
        if ((mm === 3 || mm === 4) && k > SOIL * 0.5 && rr(a * 17 + 3, k * 19 + 1) < 0.0009) {
          var cu = rr(a, k + 7) < 0.6;                                          // a seam of copper, or of iron
          [[0, 0], [1, 0], [0, 1], [1, 1], [2, 1], [1, 2]].forEach(function (q, i) {
            put(a + q[0], k + q[1], i === 0 ? (cu ? P.copperLight : P.ironLight) : (cu ? P.copper : P.iron));
          });
        }
        if (mm === 5 && rr(a * 3 + 1, k * 5 + 2) < 0.012) {                     // a gem
          var gem = A.GEMS[Math.floor(rr(a, k + 99) * A.GEMS.length)];
          put(a, k, shade(gem, 0.3)); put(a + 1, k, gem); put(a, k + 1, gem); put(a + 1, k + 1, shade(gem, -0.2));
        }
        if (mm === 5 && (a & 7) === 0 && (k & 7) === 0 && rr(a, k + 91) < 0.4)  // a crack in a stone block
          for (f = 0; f < 3; f++) if (m(a + 2 + f, k + 2 + f) === 5) put(a + 2 + f, k + 2 + f, P.crack);
        if (mm === 6 && rr(a * 7 + 3, k * 3 + 8) < 0.0015) put(a, k, P.bedGlint);
      }
      if (!keeps) return;
      function stamp(rows, a0, k0, colours) {                                   // what the ground has kept
        rows.forEach(function (row, j) {
          for (var i = 0; i < row.length; i++) if (colours[row[i]]) put(a0 + i, k0 + j, colours[row[i]]);
        });
      }
      var BONE = { b: P.bone, s: P.boneShade, e: P.boneEdge };
      stamp(['.ee....ee.', 'ebbeeeebbe', 'essbbbbsse', '.ee....ee.'], Math.round(len * 0.16), Math.round(SOIL * 0.62), BONE);
      stamp(['.ee.', 'ebbe', '.eb.', '.eb.', '.eb.', '.eb.', 'ebbe', '.ee.'], Math.round(len * 0.47), Math.round(SOIL * 0.78), BONE);
      stamp(['..eeee..', '.ebbbbe.', 'ebbbbbbe', 'eb.bb.be', 'ebbbbbbe', '.ebssbe.', '..eeee..'], Math.round(len * 0.33), Math.round(SOIL * 0.7), BONE);
      var CH = { o: P.chestDark, w: P.chest, g: P.gold, l: P.goldLight, d: P.goldDark };
      var cx0 = Math.round(len * 0.74), ck0 = Math.round(SOIL * 0.8);
      stamp(['.oooooooooooo.', 'owwwwwgwwwwwwo', 'owwwwwgwwwwwwo', 'oggggggggggggo', 'owwwwldwwwwwwo', 'owwwwwdwwwwwwo', 'owwwwwgwwwwwwo', 'oooooooooooooo'], cx0, ck0, CH);
      [[-4, 6], [-2, 7], [16, 7], [18, 6], [15, 5], [-3, 3]].forEach(function (q, i) {
        put(cx0 + q[0], ck0 + q[1], i & 1 ? P.gold : P.goldLight); put(cx0 + q[0] + 1, ck0 + q[1], P.goldDark);
      });
    }
    face(W, function (a, t) { return [a, D - 1 - t]; }, 0, true);
    face(W, function (a, t) { return [W - 1 - a, t]; }, 1);
    face(D, function (a, t) { return [t, a]; }, 2);
    face(D, function (a, t) { return [W - 1 - t, D - 1 - a]; }, 3);

    /* ── THE EGG: ceremony.js's two paintings are of this solid ── */
    var castH = 0, castN = 0;
    Object.keys(cast).forEach(function (key) {
      var L = Tama.renderLayers(cast[key], { emotion: cast[key].personality ? cast[key].personality.rest : 'neutral', facing: 'down', frame: 0 });
      var top = -1, foot = -1;
      for (var i = 0; i < L.M.length; i++) if (L.M[i]) { if (top < 0) top = i >> 5; foot = i >> 5; }
      castH += foot - top + 1; castN++;
    });
    var EGG_H = Math.round((castN ? castH / castN : 26) * 5 / 3);            // five thirds as tall as the cast
    var E = A.eggArt(EGG_H), S = E.solid, EGG_Z = LZ - 6;
    function turnFrom(th, to) { var d = th - to; while (d > Math.PI) d -= 2 * Math.PI; while (d < -Math.PI) d += 2 * Math.PI; return d; }
    function egg(lit) {
      var C = S.C, R = Math.ceil(S.a) + 1, bands = [C.dark, C.shadow, C.mid, C.light, C.hi], L = S.LIGHT, y, dx, dz;
      for (y = 0; y < EGG_H; y++) {
        var Y = S.Yof(y), w = S.half(Y);
        for (dz = -R; dz < R; dz++) for (dx = -R; dx < R; dx++) {
          var px = dx + 0.5, pz = dz + 0.5, rad = Math.hypot(px, pz);
          if (rad > w) continue;
          var col = C.mid;
          if (rad > w - 1.6) {
            var th = Math.atan2(px, pz), n0 = Math.sin(th) * 0.9, n1 = Y * 0.55, nz = Math.cos(th);
            var d = (n0 * L[0] + n1 * L[1] + nz * L[2]) / Math.hypot(n0, n1, nz);
            col = bands[d > 0.86 ? 4 : d > 0.62 ? 3 : d > 0.34 ? 2 : d > 0.08 ? 1 : 0];
            var u = turnFrom(th, S.TURN) * w, seam = S.rings.indexOf(y) >= 0;
            S.meridians.forEach(function (mer) {
              if (Math.abs(turnFrom(th, mer + S.TURN)) * w < 0.55) seam = seam ? 'rivet' : true;
            });
            if (seam) col = seam === 'rivet' ? C.rivet : C.seam;
            var v = y + 0.5 - S.syc;                                            // the screen
            if (Math.abs(u) <= S.sw / 2 && Math.abs(v) <= S.sh / 2) {
              var edge = Math.abs(u) > S.sw / 2 - 1 || Math.abs(v) > S.sh / 2 - 1;
              col = edge ? C.bezel : lit ? C.lit : C.glass;
              if (!edge && !lit && Math.abs(v) < 0.5 && Math.floor(u + 100) % 3 !== 0 && Math.abs(u) < S.sw / 2 - 2) col = C.text;
            }
            var vk = y - E.ky, rk = Math.hypot(u, vk);                          // the keyhole
            if (rk <= 1.2 || (vk > 0 && vk <= 3 && Math.abs(u) < 0.6)) col = C.hole;
            else if (rk <= 2.3 && vk <= 0) col = C.brass;
          }
          world.set(EGG_X + dx, GY + EGG_H - y, EGG_Z + dz, pal.id(col));
        }
      }
      var br = Math.round(S.W * 0.46) / 2;                                      // the base it sits on
      for (y = EGG_H - 2; y <= EGG_H; y++) for (dz = -R; dz < R; dz++) for (dx = -R; dx < R; dx++) {
        if (Math.hypot(dx + 0.5, dz + 0.5) > br - (y === EGG_H ? 1 : 0)) continue;
        world.set(EGG_X + dx, GY + EGG_H - y, EGG_Z + dz, pal.id(y === EGG_H - 2 ? C.light : y === EGG_H - 1 ? C.mid : C.shadow));
      }
    }
    egg(false);
    var kw = S.half(S.Yof(E.ky));
    var eggAt = { x: EGG_X, z: EGG_Z, h: EGG_H, r: Math.ceil(S.a), turn: S.TURN,
                  hole: [EGG_X + Math.sin(S.TURN) * kw, GY + EGG_H - E.ky + 0.5, EGG_Z + Math.cos(S.TURN) * kw],
                  top: [EGG_X, GY + EGG_H + 1, EGG_Z], light: egg };

    /* ── WHERE THE HEAD GOES: a patch of turned soil ── */
    var HEAD_X = colX(COLS - 1), PLOT_Z = LZ;
    (function plot() {
      for (var dz = -7; dz <= 7; dz++) for (var dx = -7; dx <= 7; dx++) {
        var r = Math.hypot(dx, dz);
        if (r > 6.6) continue;
        var q = h3(HEAD_X + dx, 61, PLOT_Z + dz), soil = r < 1.5 ? id('topDark') : q < 0.3 ? id('dirtLight') : q < 0.6 ? id('dirt') : id('dirtDark');
        world.set(HEAD_X + dx, GY - 1, PLOT_Z + dz, soil);
        if (r > 2.2 && r < 5.4) world.set(HEAD_X + dx, GY, PLOT_Z + dz, soil);    // heaped round the hole the seed goes in
      }
    })();
    function linkX(i) { return colX(COLS - 2 - i); }

    /* ── ON THE LIP: blades, a flower here and there, bushes behind the lane ── */
    function clear(x, z) {                       // where something stands, or will
      if (Math.hypot(x - EGG_X, z - EGG_Z) < eggAt.r + 3) return false;
      if (Math.hypot(x - HEAD_X, z - PLOT_Z) < 9) return false;
      for (var i = 0; i < 6; i++) if (Math.abs(x - linkX(i)) < 5 && Math.abs(z - LZ) < 6) return false;
      return true;
    }
    (function lip() {
      var x, z, b;
      for (z = 1; z < D - 1; z++) for (x = 1; x < W - 1; x++) {
        if (h3(x, 7, z) > 0.1 || !clear(x, z)) continue;
        var hb = h3(x, 8, z);
        var h = hb < 0.42 ? 1 + Math.floor(h3(x, 9, z) * 2) : hb < 0.86 ? 3 + Math.floor(h3(x, 9, z) * 3) : 6 + Math.floor(h3(x, 10, z) * 4);
        if (z > LZ - 5 && z < LZ + 26) h = Math.min(h, 2);                      // short where people walk, so their feet show
        var lean = h > 3 ? (h3(x, 12, z) < 0.5 ? -1 : 1) : 0;
        for (b = 1; b <= h; b++)
          world.set(x + (b > h - 2 && b > 2 ? lean : 0), GY - 1 + b, z, b === h ? id('grassTip', 0, true) : b > h * 0.5 ? id('grassLight', 0, true) : id('grass', 0, true));
        if (h >= 4 && h3(x, 13, z) < 0.11) {
          var fc = pal.id(A.TINY[Math.floor(h3(x, 14, z) * A.TINY.length)], 0, true), fx = x + lean, fy = GY + h;
          [[0, 1, 0], [0, -1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]].forEach(function (o) { world.set(fx + o[0], fy + o[1], z + o[2], fc); });
          world.set(fx, fy, z, id('petalLight', 0, true));
        }
      }
      for (var n = 0, bx = 14; bx < W - 20; n++, bx += 20 + Math.floor(hash(n, 71) * 30)) {
        if (hash(n, 72) < 0.25) continue;
        var bz = 6 + Math.floor(hash(n, 77) * (LZ - 34)), bw = 10 + Math.floor(hash(n, 73) * 9), bh = 6 + Math.floor(hash(n, 74) * 6), lobes = [];
        for (var q = 0; q < 3; q++) lobes.push([bx + bw * (0.25 + 0.25 * q), bh * 0.45, bz + (hash(n, 78 + q) - 0.5) * bw * 0.5, bh * (0.5 + hash(n, 75 + q) * 0.3)]);
        var inside = function (xx, yy, zz) {
          return yy >= 0 && lobes.some(function (l) { return Math.hypot(xx - l[0], (yy - l[1]) * 1.2, zz - l[2]) <= l[3]; });
        };
        var berry = hash(n, 76) < 0.5;
        for (var y = 0; y < bh + 3; y++) for (z = bz - bw; z < bz + bw; z++) for (x = bx - 3; x < bx + bw + 3; x++) {
          if (!inside(x + 0.5, y + 0.5, z + 0.5) || !clear(x, z)) continue;
          var out = !inside(x + 0.5, y + 1.5, z + 0.5), side = !inside(x - 0.5, y + 0.5, z + 0.5) || !inside(x + 1.5, y + 0.5, z + 0.5) ||
                    !inside(x + 0.5, y + 0.5, z - 0.5) || !inside(x + 0.5, y + 0.5, z + 1.5);
          var r = h3(x, y + 200, z), c = out && y > 2 ? P.bushLight : r < 0.18 || y < 3 ? P.bushDark : P.bush;
          if ((out || side) && h3(x * 3, y * 7, z) < 0.04) c = berry ? P.berry : A.TINY[Math.floor(r * A.TINY.length)];
          world.set(x, GY + y, z, pal.id(c, 0, true));
        }
      }
    })();

    var Q = parts(Ceremony, Tama, pal);

    /* ── THE ROOTS: lists of voxels on the front face, in the order they grow ── */
    var seq = 0;
    function rnd() { return hash(seq++, 4177); }
    var RC = { root: pal.id(P.root), mid: pal.id(P.rootMid), dark: pal.id(P.rootDark), hair: pal.id(P.hair), hypha: pal.id(P.hypha) };
    function vox(list, a, k, c, out) { if (a > 0 && a < W - 1 && k > 3 && k < SOIL - 4) list.push(a, GY - 1 - k, D + (out || 0), c); }
    function weave(lists) {                      // several growing at once
      var out = [], n = Math.max.apply(null, lists.map(function (l) { return l.length; }));
      for (var i = 0; i < n; i += 4) lists.forEach(function (l) { if (i < l.length) out.push(l[i], l[i + 1], l[i + 2], l[i + 3]); });
      return out;
    }
    function hairs(list, a, k) {
      var dir = rnd() < 0.5 ? -1 : 1, n = 3 + Math.floor(rnd() * 5);
      for (var s = 1; s <= n; s++) vox(list, a + dir * s, k + Math.floor(s / 2), RC.hair);
    }
    function tap() {                             // the seed's own roots: down first
      return weave([-2, 0, 3].map(function (off, j) {
        var list = [], a = HEAD_X + off, k = 4, n = 24 + Math.floor(rnd() * 20);
        for (var s = 0; s < n; s++) {
          vox(list, a, k, s < n * 0.45 ? RC.root : RC.mid);
          if (s < n * 0.3) vox(list, a + 1, k, RC.mid);
          if (rnd() < 0.74) k++; else a += rnd() < 0.5 + (j - 1) * 0.3 ? 1 : -1;
          if (rnd() < 0.08) hairs(list, a, k);
        }
        return list;
      }));
    }
    /* The way a slime mold forages: fine fronts go looking, one finds the ground
       under the next board and thickens into a cord, and one root rises to it. */
    function forage(from, toA) {
      var fronts = [], finder = Math.floor(rnd() * 3), end = null, path = [];
      for (var f = 0; f < 3; f++) {
        var list = [], a = from[0], k = from[1], want = 12 + Math.floor(rnd() * 30), reach = f === finder ? 1 : 0.45 + rnd() * 0.4;
        var stop = Math.round(lerp(from[0], toA, reach));
        while (a > stop) {
          vox(list, a, k, RC.hypha);
          if (f === finder) path.push([a, k]);
          if (rnd() < 0.8) a--; else k += k < want ? 1 : -1;
          if (rnd() < 0.3) k += k < want ? 1 : k > want ? -1 : 0;
          k = Math.max(6, Math.min(SOIL - 8, k));
          if (rnd() < 0.05) hairs(list, a, k);
        }
        if (f === finder) end = [a, k];
        else for (var s = 0; s < 5 + rnd() * 6; s++) { vox(list, a, k, RC.hypha); k++; if (rnd() < 0.4) a--; }
        fronts.push(list);
      }
      var cord = [];
      path.forEach(function (p) { vox(cord, p[0], p[1], RC.root, 1); vox(cord, p[0], p[1] + 1, RC.mid, 1); });
      for (var kk = end[1]; kk >= 4; kk--) { vox(cord, end[0], kk, RC.root, 1); vox(cord, end[0] + 1, kk, RC.dark, 1); }
      return { fronts: fronts.map(Vox.meshCubes), n: fronts.map(function (l) { return l.length / 4; }), finder: finder,
               cord: Vox.meshCubes(cord), cordN: cord.length / 4, end: end };
    }

    var scene = {
      world: world, pal: pal, art: A, W: W, H: H, D: D, GY: GY, LZ: LZ, PITCH: PITCH, TILE: TILE, RAISE: RAISE,
      BOARD_D: BOARD_D, SLAB_W: SLAB_W, SLAB_D: SLAB_D, HEAD_X: HEAD_X, PLOT_Z: PLOT_Z, colX: colX, linkX: linkX,
      egg: eggAt, tap: function () { var l = tap(); return { mesh: Vox.meshCubes(l), n: l.length / 4 }; }, forage: forage
    };
    for (var q in Q) scene[q] = Q[q];
    return scene;
  }

  (root.Kenjin = root.Kenjin || {}).PlayWorld = { build: build, parts: parts, Palette: Palette, TILE: TILE, RAISE: RAISE, BOARD_D: BOARD_D };
})(typeof self !== 'undefined' ? self : this);
