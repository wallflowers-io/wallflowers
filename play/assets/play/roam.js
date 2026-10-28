/* roam.js — a generated land, walked.
 *
 * /map lets you choose real places; land.js makes the land between them; this
 * puts you down at the first and lets you walk to the rest. Each place has its
 * landmark, and a board that comes up on its pole when you reach it — the Site
 * board of /play, in a shell colour of its own.
 *
 * SEEN AS THE SIMS SEES A LOT, FOUND AS SAN ANDREAS FINDS A MISSION. The view
 * stands high and back, and turns a quarter at a time (Q and E; a drag turns it
 * freely). Whatever stands between it and you is cut away, and the sun casts,
 * so a street reads as a street (engine.js). A green diamond hangs over you; a
 * gold one over every place you have not reached, high enough to show over the
 * roofs. The radar turns with the view: its rim holds a blip for every place,
 * so the way to each is always on screen. No clouds: seen from above, a cloud
 * is a thing in the way.
 *
 * THE NAMES ARE THE PAGE'S. A place's name is whatever script its country
 * writes in, and ceremony.js's font is Latin; so this says which name belongs
 * where (`opts.names`, `opts.say`) and the page writes it. One name is up at a
 * time: the nearest place you have not reached.
 *
 * MADE UP, AND NOT KEPT. The land is generated from the chosen geohashes each
 * time; which boards you have raised is forgotten when you leave. No Delta
 * carries a chosen place, a land, or a visit.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin = root.Kenjin || {}, En = K.Engine, lerp = En.lerp, place = En.place;
  var G = 8;                                  // art pixels to a block

  function start(cast, sites, opts) {
    var Tama = root.Tama, Ceremony = K.Ceremony, A = Ceremony.art, P = A.P, PW = K.PlayWorld;
    var pal = new PW.Palette({ soft: new Uint8Array(65536) });
    function colour(name) { var m = /^(gem|sky):(\d)$/.exec(name); return m ? (m[1] === 'gem' ? A.GEMS : A.SKY)[+m[2]] : P[name]; }
    var L = K.Land.generate(sites, { stuff: function (name, spec) { return pal.mix(spec.colours.map(colour), spec); } });
    var E = En.create(opts.canvas, { art: A, Tama: Tama, cast: cast, pal: pal, world: L.world, grain: G, step: G, mark: opts.mark });
    var Q = PW.parts(Ceremony, Tama, pal), poleLen = PW.RAISE + Math.round(PW.TILE * 0.55), poleM = E.part(Q.pole(poleLen));
    var mine = E.part(Q.gem(P.grassTip, P.grassLight, P.grass)), theirs = E.part(Q.gem(P.keyLight, P.key, P.keyDark));
    var cam = E.cam;
    E.sun(L.tops, L.size);

    var spots = L.spots.map(function (s, i) {
      return { name: s.name, kind: s.kind, as: s.as, x: (s.board[0] + 0.5) * G, y: s.level * G, z: (s.board[1] + 0.5) * G, roof: s.top * G,
               cx: (s.lot ? s.lot[0] + s.lot[2] / 2 : s.x + 0.5) * G, cz: (s.lot ? s.lot[1] + s.lot[3] / 2 : s.z - 8) * G,      // the middle of its landmark
               bx: s.x + 0.5, bz: s.z + 0.5, up: 0, seen: false, board: E.part(Q.board(0, null, i)) };
    });
    var ids = Object.keys(cast), first = L.spots[0];
    var me = E.person(cast[opts.as] ? opts.as : ids[Math.floor(Math.random() * ids.length)], (first.x - 1.5) * G, first.level * G + 170, (first.z + 1.5) * G);
    var done = false;
    if (opts.at) { me.x = opts.at[0]; me.z = opts.at[1]; me.y = E.floor(me.x, 9999, me.z) + 1; }       // carried over from another look
    spots.forEach(function (s, i) { if (opts.seen && opts.seen[i]) { s.seen = true; s.up = 1; E.solids.push([s.x - 2.5, s.y, s.z - 5, s.x + 2.5, s.y + poleLen, s.z]); } });
    function arrive(s) {
      s.seen = true;
      E.feel(me, 'happy', 1.2);
      if (opts.say) opts.say(s.name);
      E.tween(0.85, function (u) { s.up = En.outBack(u); }, function () {
        s.up = 1;
        E.solids.push([s.x - 2.5, s.y, s.z - 5, s.x + 2.5, s.y + poleLen, s.z]);
        if (!done && spots.every(function (t) { return t.seen; })) { done = true; E.feel(me, 'laugh', 4); }
      });
    }
    function next() {                           // the nearest place not yet reached
      var best = null;
      spots.forEach(function (s) { if (!s.seen && (!best || Math.hypot(s.x - me.x, s.z - me.z) < Math.hypot(best.x - me.x, best.z - me.z))) best = s; });
      return best;
    }

    cam.tx = me.x; cam.ty = first.level * G + 20; cam.tz = me.z; cam.pitch = 0.8; cam.fov = 0.5;
    E.run({
      update: function (dt) {
        E.control(me); E.move(me, dt);
        spots.forEach(function (s) { if (!s.seen && me.ground && Math.hypot(me.x - s.x, me.z - s.z) < 42) arrive(s); });
        cam.dist = root.innerWidth < root.innerHeight ? 560 : 470;
        cam.tx = lerp(cam.tx, me.x, 1 - Math.exp(-dt * 5)); cam.tz = lerp(cam.tz, me.z, 1 - Math.exp(-dt * 5));
        cam.ty = lerp(cam.ty, me.y + 20, 1 - Math.exp(-dt * 4));
        E.cut = [me.x, me.y + 6, me.z];
      },
      parts: function (model) {
        var t = E.clock(), to = next();
        spots.forEach(function (s) {
          if (!s.seen) model(theirs, place([s.cx, s.roof + 26 + Math.sin(t * 2.2 + s.x) * 3, s.cz], t * 1.6, 0, [4.5, 0, 4.5]));
          if (s.up <= 0) return;
          var y = s.y + PW.RAISE - (1 - s.up) * (PW.RAISE + PW.TILE + 6);
          model(poleM, place([s.x - 2.5, y - PW.RAISE - 3, s.z - 5]));
          model(s.board, place([s.x - PW.TILE / 2, y, s.z]));
        });
        model(mine, place([me.x, me.y + 38 + Math.sin(t * 3) * 1.5, me.z], t * 2.2, 0, [4.5, 0, 4.5]));
        if (opts.names) opts.names(to ? [{ name: to.name, at: E.project([to.cx, to.roof + 48, to.cz]) || [root.innerWidth / 2, 0] }] : []);
        if (opts.radar) opts.radar({ map: L.map, size: L.size, x: me.x / G, z: me.z / G, hx: me.hx, hz: me.hz, yaw: cam.yaw,
                                     spots: spots.map(function (s) { return { x: s.bx, z: s.bz, seen: s.seen, next: s === to }; }) });
      }
    });

    return {
      input: E.input, cam: cam, stop: E.stop, turn: E.turn,
      /* Read-only, for whoever is watching. */
      state: function () {
        return { phase: done ? 'done' : 'roam', as: me.id, at: [me.x, me.y, me.z], ground: me.ground, frames: E.frames(), size: L.size, seed: L.seed,
                 lots: L.lots.length, trees: L.trees, cars: L.cars, chunks: E.chunks(), colours: pal.n, skin: 'blocks',
                 spots: spots.map(function (s) { return { name: s.name, as: s.as, at: [s.x, s.y, s.z], seen: s.seen, up: s.up }; }) };
      }
    };
  }

  K.Roam = { start: start };
})(typeof self !== 'undefined' ? self : this);
