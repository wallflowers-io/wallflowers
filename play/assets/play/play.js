/* play.js — HOW IT WORKS, played.
 *
 * /play is ceremony.js's story with you walking it. The scene is world.js's
 * block of voxels; engine.js draws it and moves the cast through it.
 *
 * WHAT YOU DO, AND IT IS CEREMONY.JS'S ELEVEN BEATS IN ORDER:
 *   0  you come down on the grass, a key in your hand
 *   1  walk the key to the egg: you step in, it turns, the egg lights and
 *      gives you your seed
 *   2  carry the seed to the turned soil at the right: roots first, then a
 *      sunflower through the grass
 *  3–8 the roots go looking; where one comes up for air, stand by it and the
 *      board comes up on its pole and the head grows a leaf. Six of them,
 *      coming back towards the egg
 *   9  four friends run in. Jump under a board and bump it from below: every
 *      bump mints a delta with its author's face, stacked on that board
 *  10  when every board holds what it was going to, the head blooms
 * The words are ceremony.js's BEATS, in its panel and its font. Nothing here
 * restates them.
 *
 * TWO CONTROLS: move, and jump. Reaching a thing is using it. Keys: WASD or the
 * arrows, Space. Touch: a stick under the left thumb, a button under the right.
 * Drag turns the view; the wheel or a pinch brings it nearer.
 *
 * ONE BUMP AT A TIME, PER BOARD, as in the picture: a board is not bumped again
 * until its last delta has landed, so a stack is in the order it was minted.
 *
 * A DRAWING. No key, seed, group or delta here is real, and nothing you do is
 * kept: reload and it is gone, on this device and every other. No Delta carries
 * any of it, so none of it is WallFlowers state.
 *
 * ?v= IS LOAD-BEARING. assets/ is served immutable for a year; bump the number
 * in play.html in the same change as any edit here.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin = root.Kenjin || {}, En = K.Engine;
  var clamp = En.clamp, lerp = En.lerp, outCubic = En.outCubic, inOut = En.inOut, outBack = En.outBack, hump = En.hump, place = En.place;
  var HW = En.HW, HD = En.HD, BH = En.BH;
  var LANE = 10;                              // in front of the boards: where you run under them
  var ROOM = 12;                              // the most a stack holds
  var KEY_STEP = 14, KEY_DEPTH = 10, KEY_LEN = 19;

  function start(cast, opts) {
    var Tama = root.Tama, Ceremony = K.Ceremony, A = Ceremony.art;
    var S = K.PlayWorld.build(Ceremony, Tama, cast), GY = S.GY, LZ = S.LZ;
    var E = En.create(opts.canvas, { art: A, Tama: Tama, cast: cast, pal: S.pal, world: S.world, grain: 1, mark: opts.mark });
    var part = E.part, tween = E.tween, chain = E.chain, feel = E.feel, cam = E.cam, people = E.people, solids = E.solids;

    /* ── WHAT STANDS IN THE WAY besides the block: the boards, their poles, the stem ── */
    var links = [];
    var poleLen = S.RAISE + Math.round(S.TILE * 0.55), flowerH = 0, stage = -1, bloom = -1;
    /* Something new to stand in the way: whoever is where it lands steps out, towards you. */
    function solid(b) {
      solids.push(b);
      people.forEach(function (c) {
        if (c.x + HW > b[0] && c.x - HW < b[3] && c.y + BH > b[1] && c.y < b[4] && c.z + HD > b[2] && c.z - HD < b[5]) c.z = b[5] + HD + 0.01;
      });
    }
    /* The board a rising head meets: anywhere in front of it counts, so you need
       not line up in depth to bump it. */
    E.onHead = function (c, x, y, z) {
      for (var i = 0; i < links.length; i++) {
        var l = links[i], foot = GY + S.RAISE;
        if (l.up < 1 || Math.abs(x - l.x) > S.TILE / 2 + HW - 2 || z < LZ - 8) continue;
        if (y + BH > foot && y + BH < foot + 8) { bumped(l, c); return true; }
      }
      return false;
    };

    /* ── THE STORY ── */
    var ids = Object.keys(cast), meId = cast[opts.as] ? opts.as : ids[Math.floor(Math.random() * ids.length)];
    var me = E.person(meId, S.colX(1), GY + 170, LZ + LANE), friends = [];
    var G = { beat: -1, phase: 'arrive', hold: null, lock: true, done: false };
    var keyM = part(S.key()), seedM = part(S.seed()), stemM = part(S.stem()), shootM = part(S.shoot());
    var blooms = [0, 1, 2, 3].map(function (j) { return part(S.bloom(j)); });
    var leaves = [0, 1, 2, 3, 4, 5].map(function (k) {
      return { fresh: part(S.leaf(k, A.LEAF_NEW)), rest: part(S.leaf(k, A.leafRest(k))), y: S.flower.leaves[k].y,
               yaw: [0, Math.PI, -Math.PI / 2, Math.PI / 2][k % 4], dx: [1, -1, 0, 0][k % 4], dz: [0, 0, 1, -1][k % 4] };
    });
    /* Clouds, behind the block and above the boards, so nothing built ever stands in one. */
    var clouds = [0, 1, 2, 3, 4, 5, 6, 7].map(function (n) {
      var c = part(S.cloud(n));
      c.x = A.hash(n, 33) * (S.W + 320) - 160; c.z = -30 - A.hash(n, 36) * 150; c.top = GY + 120 + c.y * 110;
      return c;
    });
    var roots = [], keyPose = null, seedPose = null, slabs = {}, flying = [];
    var egg = S.egg, out = [Math.sin(egg.turn), Math.cos(egg.turn)];           // the way the keyhole faces, in x and z

    function beat(n) { if (n === G.beat) return; G.beat = n; opts.words(A.BEATS[n]); }

    /* 1 — the key goes in with you: you step in, it turns, the egg gives you the seed */
    function unlock() {
      G.lock = true; G.phase = 'egg'; beat(1);
      var from = [me.x, me.z], far = KEY_LEN + 7 + KEY_STEP - KEY_DEPTH, roll = 0, inn = 0;
      function at(u) {                                                         // you, u of the way in
        var r = far - KEY_STEP * u;
        return [egg.hole[0] + out[0] * r, egg.hole[2] + out[1] * r];
      }
      function pose() {
        var r = far - KEY_STEP * inn - KEY_LEN - 7;
        keyPose = place([egg.hole[0] + out[0] * r, egg.hole[1], egg.hole[2] + out[1] * r], Math.PI / 2 + egg.turn, roll, [KEY_LEN, 4, 1]);
      }
      me.mx = me.mz = 0; me.hx = -out[0]; me.hz = -out[1];
      chain([
        [0.4, function (u) { var a = at(0); me.x = lerp(from[0], a[0], inOut(u)); me.z = lerp(from[1], a[1], inOut(u)); me.moving = u < 1; pose(); }],
        [0.5, function (u) { inn = inOut(u); var a = at(inn); me.x = a[0]; me.z = a[1]; me.moving = u < 1; pose(); }],
        [0.4, function (u) { roll = inOut(u) * Math.PI / 2; pose(); }, function () { egg.light(true); feel(me, 'surprised', 1.2); }],
        [0.75, function (u) {
          var a = egg.top, e = inOut(u);
          seedPose = place([lerp(a[0], me.x, e), lerp(a[1], me.y + 30, e) + hump(u) * 22, lerp(a[2], me.z, e)], cam.yaw, 0, [2, 0, 1]);
        }, function () { G.hold = 'seed'; seedPose = null; feel(me, 'happy', 1.2); }],
        [0.4, function (u) { inn = 1 - inOut(u); var a = at(inn); me.x = a[0]; me.z = a[1]; me.moving = u < 1; pose(); },
          function () { keyPose = null; G.phase = 'seed'; G.lock = false; beat(2); }]
      ]);
    }

    /* 2 — into the turned soil: roots first, then the stem through the grass */
    var stemBox = [S.HEAD_X - 1.5, GY, S.PLOT_Z - 1.5, S.HEAD_X + 1.5, GY + 400, S.PLOT_Z + 1.5];
    function plant() {
      G.phase = 'planting'; G.hold = null;
      var from = [me.x, me.y + 26, me.z], tap = part(S.tap());
      tap.shown = 0; roots.push(tap);
      feel(me, 'happy', 1.5);
      chain([
        [0.45, function (u) { seedPose = place([lerp(from[0], S.HEAD_X, u), lerp(from[1], GY - 5, u * u) + hump(u) * 12, lerp(from[2], S.PLOT_Z, u)], cam.yaw, 0, [2, 0, 1]); },
          function () { seedPose = null; }],
        [1.0, function (u) { tap.shown = Math.round(tap.n * u); }],
        [0.7, function (u) { flowerH = S.flower.Hs * A.GROW[0] * outBack(u); }, function () { stage = 0; solid(stemBox); G.phase = 'chain'; forage(0, [S.HEAD_X - 2, 22]); }]
      ]);
    }

    /* 3–8 — the roots go looking for the next board, and one comes up for air */
    function forage(i, from) {
      var f = S.forage(from, S.linkX(i)), fronts = f.fronts.map(function (mesh, j) { var r = part({ mesh: mesh, n: f.n[j] }); r.shown = 0; roots.push(r); return r; });
      var cord = part({ mesh: f.cord, n: f.cordN }); cord.shown = 0; roots.push(cord);
      chain([
        [1.3, function (u) { fronts.forEach(function (r) { r.shown = Math.round(r.n * u); }); }],
        [0.6, function (u) {
          cord.shown = Math.round(cord.n * u);
          fronts.forEach(function (r, j) { if (j !== f.finder) r.shown = Math.round(r.n * lerp(1, 0.45, inOut(u))); });   // the rest are reeled in, part of the way
        }, function () { links[i].air = 0; links[i].end = f.end; tween(0.25, function (u) { links[i].air = outBack(u); }); }]
      ]);
    }
    function raise(i) {
      var l = links[i];
      l.air = -1; l.rising = true; beat(A.CHAIN_BEAT + i);
      var h0 = flowerH, h1 = S.flower.Hs * A.GROW[i + 1];
      tween(0.85, function (u) { l.up = outBack(u); }, function () {
        l.up = 1; l.rising = false;
        solid([l.x - S.TILE / 2, GY + S.RAISE, LZ - 3, l.x + S.TILE / 2, GY + S.RAISE + S.TILE, LZ + 3]);
        solid([l.x - 2.5, GY, LZ - 8, l.x + 2.5, GY + poleLen, LZ - 3]);
      });
      tween(0.7, function (u) { flowerH = lerp(h0, h1, inOut(u)); }, function () {
        stage = i + 1;
        if (i < 5) forage(i + 1, l.end); else tween(0.9, null, write);
      });
    }

    /* 9 — your friends come back, and everyone writes */
    function write() {
      G.phase = 'write'; beat(A.WRITE_BEAT);
      var rest = ids.filter(function (id) { return id !== me.id; });
      rest.sort(function (a, b) { return A.hash(ids.indexOf(a), 5 + ids.indexOf(me.id)) - A.hash(ids.indexOf(b), 5 + ids.indexOf(me.id)); });
      rest.slice(0, A.FRIENDS).forEach(function (id, n) {
        var f = E.person(id, S.W - 14 - n * 9, GY + 150 + n * 40, LZ + LANE + n * 4);
        f.n = n; f.speed = En.RUN; f.state = 'wait'; f.wait = 0.25 * n; f.job = null; f.hx = -1; f.hz = 0;
        friends.push(f);
      });
    }
    function need(l) { return l.target - l.count - (l.taken ? 1 : 0); }
    function choose(f) {
      var best = null;
      links.forEach(function (l) {
        if (l.up < 1 || l.taken || need(l) <= 0) return;
        if (!best || need(l) > need(best) || (need(l) === need(best) && Math.abs(l.x - f.x) < Math.abs(best.x - f.x))) best = l;
      });
      if (best) { best.taken = f; f.job = best; f.tx = best.x + (f.n - 1.5) * 4; f.tz = LZ + LANE - 2 + f.n * 3; }
      else { f.job = null; f.tx = S.colX(2) + f.n * 30; f.tz = LZ + 18 + (f.n & 1) * 9; }
      f.state = 'run';
    }
    function think(f, dt) {
      f.mx = f.mz = 0;
      if (f.state === 'wait') { f.wait -= dt; if (f.wait <= 0) choose(f); return; }
      if (f.state === 'rest') { if (!G.done && links.some(function (l) { return !l.taken && need(l) > 0; })) choose(f); return; }
      var dx = f.tx - f.x, dz = f.tz - f.z, far = Math.hypot(dx, dz);
      if (f.state === 'run') {
        if (far > 1.5) {
          f.mx = dx / far * Math.min(1, far / 4 + 0.3); f.mz = dz / far * Math.min(1, far / 4 + 0.3);
          f.slow = Math.hypot(f.x - f.px, f.z - f.pz) < f.speed * dt * 0.2 ? (f.slow || 0) + dt : 0;   // something is in the way:
          if (f.slow > 0.3) { f.mx = 0; f.mz = 1; }                                                    // step round it, towards you
          f.px = f.x; f.pz = f.z;
          return;
        }
        if (!f.job) { f.state = 'rest'; f.hx = 0; f.hz = 1; return; }
        if (f.ground && !f.job.busy) { f.jump = true; f.state = 'jump'; f.air = 0; }
        return;
      }
      if (f.state === 'jump') {
        f.air += dt;
        if (f.air > 0.2 && f.ground) { if (f.job && f.job.taken === f) f.job.taken = null; f.state = 'wait'; f.wait = 0.15 + A.hash(f.n, Math.floor(E.clock() * 7)) * 0.5; }
      }
    }
    /* A bump from below mints a delta: it comes out of the top of the board and
       lands on the stack, and the board is free again when it has. */
    function bumped(l, c) {
      tween(0.25, function (u) { l.hop = hump(u) * 5; }, function () { l.hop = 0; });
      if (l.busy || l.count >= ROOM) return;
      l.busy = true;
      if (!slabs[c.id]) slabs[c.id] = part(S.slab(c.spec, ids.indexOf(c.id)));
      var s = { gpu: slabs[c.id].gpu, x: l.x, y: 0, by: c.id }, y0 = GY + S.RAISE + S.TILE - A.SLAB_H, y1 = GY + S.RAISE + S.TILE + 1 + l.count * (A.SLAB_H + A.SLAB_GAP);
      flying.push(s);
      feel(c, 'happy', 0.7);
      tween(0.5, function (u) { s.y = lerp(y0, y1, outCubic(u)) + hump(u) * 14; }, function () {
        flying.splice(flying.indexOf(s), 1);
        l.stack.push(s.gpu); l.count++; l.busy = false;
        if (l.taken === c) l.taken = null;
        reprint(l, S.author(c.spec, ids.indexOf(c.id)));
        if (!G.done && links.every(function (k) { return k.count >= k.target; })) flower();
      });
    }
    /* The picture on a board's screen, reprinted in its author's colour for a
       moment, in whole steps: the palette is animated, never blended over. */
    function reprint(l, col) {
      var lit = part(S.board(l.i, col));
      l.board = lit;
      tween(0.45, null, function () { if (l.board === lit) l.board = l.plain; lit.gpu.free(); });
    }
    /* 10 — and the head blooms */
    function flower() {
      G.done = true; G.phase = 'bloom'; beat(A.LAST_BEAT);
      var h0 = flowerH, h1 = S.flower.Hs;
      chain([
        [0.8, function (u) { flowerH = lerp(h0, h1, inOut(u)); }, function () { stage = 7; bloom = 0; }],
        [0.45, null, function () { bloom = 1; }], [0.35, null, function () { bloom = 2; }],
        [0.35, null, function () { bloom = 3; people.forEach(function (c) { feel(c, 'laugh', 4); }); G.phase = 'done'; }]
      ]);
    }

    for (var li = 0; li < 6; li++) {
      var plain = part(S.board(li));
      links.push({ i: li, x: S.linkX(li), up: 0, air: -1, hop: 0, count: 0, busy: false, taken: null, stack: [],
                   target: 7 + Math.floor(A.hash(li, 91) * 6), plain: plain, board: plain });
    }
    var poleM = part(S.pole(poleLen));

    function near(x, z, rx, rz) { return Math.abs(me.x - x) < rx && Math.abs(me.z - z) < rz; }
    function story() {
      if (G.lock) return;
      if (G.phase === 'key' && me.ground && Math.hypot(me.x - egg.x, me.z - egg.z) < egg.r + 22) unlock();
      else if (G.phase === 'seed' && me.ground && near(S.HEAD_X, S.PLOT_Z, 18, 44)) plant();
      else if (G.phase === 'chain') links.forEach(function (l) { if (l.air >= 1 && near(l.x, LZ, 16, 44)) raise(l.i); });
    }

    cam.tx = me.x; cam.ty = GY + 26; cam.tz = LZ;
    var IDENT = place([0, 0, 0]);
    E.run({
      update: function (dt) {
        if (!G.lock) E.control(me); else { me.jump = false; if (G.phase !== 'arrive') me.mx = me.mz = 0; }
        if (G.phase === 'arrive' || !G.lock) E.move(me, dt);
        if (G.phase === 'arrive' && me.ground) { G.phase = 'key'; G.hold = 'key'; G.lock = false; }
        friends.forEach(function (f) { think(f, dt); E.move(f, dt); });
        story();
        clouds.forEach(function (c) { c.x += c.v * dt; if (c.x > S.W + 160) c.x = -160 - c.w; });
        /* The view stands back as the head grows, and leans toward the middle of
           what has been built, so the whole of it and the soil under it stay in
           the picture. */
        var tall = clamp(flowerH, 0, 190);
        cam.tx = lerp(cam.tx, lerp(clamp(me.x, 40, S.W - 40), (egg.x + S.HEAD_X) / 2, tall / 190 * 0.45), 1 - Math.exp(-dt * 5));
        cam.dist = lerp(cam.dist, (root.innerWidth < root.innerHeight ? 340 : 300) + tall * 0.75, 1 - Math.exp(-dt * 1.5));
        cam.ty = lerp(cam.ty, Math.max(GY, me.y) + 26 + tall * 0.3, 1 - Math.exp(-dt * 4));
        cam.tz = lerp(cam.tz, me.z - 6, 1 - Math.exp(-dt * 5));
      },
      parts: function (model) {
        var gl = E.gl, clock = E.clock();
        roots.forEach(function (r) { model(r, IDENT, undefined, r.shown * 36); });
        links.forEach(function (l) {
          if (l.air > 0) model(shootM, place([l.x - 3, GY - 6 + 6 * l.air, LZ - 6]));
          if (l.up <= 0) return;
          var y = GY + S.RAISE - (1 - l.up) * (S.RAISE + S.TILE + 6) + l.hop;
          model(poleM, place([l.x - 2.5, y - S.RAISE - 3 - l.hop, LZ - 8]));
          model(l.board, place([l.x - S.TILE / 2, y, LZ - 3]));
          l.stack.forEach(function (g, n) { model(g, place([l.x - S.SLAB_W / 2, y + S.TILE + 1 + n * (A.SLAB_H + A.SLAB_GAP), LZ - S.SLAB_D / 2])); });
        });
        flying.forEach(function (s) { model(s.gpu, place([s.x - S.SLAB_W / 2, s.y, LZ - S.SLAB_D / 2])); });
        if (flowerH > 0) {
          gl.disable(gl.CULL_FACE);
          model(stemM, place([S.HEAD_X - 1.5, GY, S.PLOT_Z - 1.5]), GY + flowerH);
          leaves.forEach(function (lf, k) {
            if (k >= stage) return;
            var m = k === stage - 1 && stage < 7 ? lf.fresh : lf.rest;
            model(m, place([S.HEAD_X + lf.dx * 1.5, GY + lf.y, S.PLOT_Z + lf.dz * 1.5], lf.yaw, 0, [m.bx, m.by + 0.5, 0.5]));
          });
          if (bloom >= 0) { var b = blooms[bloom]; model(b, place([S.HEAD_X, GY + flowerH, S.PLOT_Z + 1], 0, 0, [b.w / 2, b.h / 2, 0])); }
          gl.enable(gl.CULL_FACE);
        }
        clouds.forEach(function (c) { model(c, place([c.x, c.top, c.z])); });
        if (keyPose) model(keyM, keyPose);
        else if (G.hold === 'key') model(keyM, E.hand(me, KEY_LEN, 4));
        if (seedPose) model(seedM, seedPose);
        else if (G.hold === 'seed') model(seedM, place([me.x, me.y + 28 + Math.sin(clock * 5) * 1.2, me.z], cam.yaw, 0, [2, 0, 1]));
      }
    });
    beat(0);

    return {
      input: E.input, cam: cam,
      /* Read-only, for whoever is watching. */
      state: function () {
        return { beat: G.beat, phase: G.phase, hold: G.hold, as: me.id, at: [me.x, me.y, me.z], ground: me.ground, frames: E.frames(),
                 flower: flowerH, stage: stage, bloom: bloom, friends: friends.map(function (f) { return f.id; }),
                 links: links.map(function (l) { return { up: l.up, air: l.air, count: l.count, target: l.target }; }),
                 colours: S.pal.n, chunks: E.chunks() };
      }
    };
  }

  K.Play = { start: start };
})(typeof self !== 'undefined' ? self : this);
