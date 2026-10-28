/* land.js — the land between chosen sites, generated.
 *
 * GIVEN real places (a geohash, a latitude and longitude, a name, a kind),
 * this lays them out as they lie on the map and makes up everything between
 * them: hills and water, the roads that join them, the buildings along those
 * roads, the trees. Each chosen place gets a landmark of its own, by its kind.
 *
 * ARRANGED AS THE MAP HAS THEM, NOT TO THE MAP'S SCALE. North is north and
 * who is left of whom is true; how far is not. The span of the places is fitted
 * to a block you can walk across, and places that would stand on each other
 * are eased apart. A kilometre of real street becomes a short road.
 *
 * THE SAME PLACES MAKE THE SAME LAND, on every device, with nothing sent and
 * in whatever order they were chosen: the seed is a hash of the chosen
 * geohashes, and every choice below is drawn from it. Choose one more place
 * and it is a different land.
 *
 * A CITY'S STREETS, NOT A VILLAGE'S LANES. A road is five blocks of asphalt
 * with a dashed line down it and a pavement either side; palms and lamps stand
 * on the pavement and a car or two at the kerb. Along it, shops stand on paving
 * and houses behind a hedge on their own grass.
 *
 * BUILT OF BLOCKS, eight art pixels to a side — Terraria's arrangement, and
 * ceremony.js's. A block is one voxel here; the renderer paints its eight by
 * eight from the stuff it is made of (engine.js, the patterns).
 *
 * WHAT A THING IS MADE OF IS A NAME, NOT A COLOUR. STUFF lists, for each
 * material, four entries of ceremony.js's palette and how they are laid.
 * The caller turns a name into an id (`opts.stuff`); nothing here states a
 * colour, and nothing here touches the DOM, so node can run it.
 *
 * WHAT WAS BUILT IS ALSO SAID IN WORDS. Besides the block, this returns each
 * building, tree, palm, lamp, car and hedge as what it is and where (`builds`,
 * `plants`, `props`, `parked`, `hedges`, and the heights and kinds of the
 * ground), so a renderer that does not draw blocks can draw the same land
 * (townmesh.js).
 *
 * MADE UP. The roads, buildings and hills are not the real ones, and nothing
 * here is WallFlowers state: no Delta carries a chosen place or a land.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory(require('./vox.js'), require('./geo.js'));
  else (root.Kenjin = root.Kenjin || {}).Land = factory(root.Vox, root.Kenjin.Geo);
})(typeof self !== 'undefined' ? self : this, function (Vox, Geo) {
  'use strict';

  /* Patterns: 0 scattered, 1 courses (brick, tile, dressed stone), 2 boards,
     3 a window (pane, glint, frame, sill), 5 water, 6 stripes. */
  var STUFF = {
    grass: { colours: ['grass', 'grassLight', 'grassDark', 'grassTip'], tone: 0.06 },
    dirt: { colours: ['dirt', 'dirtDark', 'dirtLight', 'dirt'], tone: 0.1 },
    stone: { colours: ['stone', 'stoneDark', 'stoneLight', 'stone'], tone: 0.08 },
    bed: { colours: ['bed', 'bedDark', 'bed', 'bedLight'] },
    sand: { colours: ['fibre', 'seedStripe', 'rootMid', 'fibre'] },
    water: { colours: ['sky:0', 'sky:1', 'sky:0', 'sky:2'], pattern: 5, soft: true },
    asphalt: { colours: ['panelLo', 'panelFill2', 'panelFill', 'panelHi'], tone: 0.04 },
    roadX: { colours: ['panelLo', 'key', 'panelFill', 'panelHi'], pattern: 7 },             // its centre line runs east and west
    roadZ: { colours: ['panelLo', 'key', 'panelFill', 'panelHi'], pattern: 8 },             // north and south
    pave: { colours: ['pebbleLight', 'ironLight', 'pebble', 'pebble'], pattern: 9 },
    brick: { colours: ['clay', 'clayLight', 'clayDark', 'boneEdge'], pattern: 1 },
    plaster: { colours: ['bone', 'paper', 'boneShade', 'bone'], tone: 0.04 },
    timber: { colours: ['chest', 'dirtHi', 'chestDark', 'chestDark'], pattern: 2 },
    ashlar: { colours: ['stoneLight', 'pebbleLight', 'stone', 'crack'], pattern: 1 },
    roof: { colours: ['panelLo', 'panelHi', 'panelFill', 'panelEdge'], pattern: 1 },
    copper: { colours: ['copper', 'copperLight', 'copper', 'topDark'], pattern: 1 },
    glass: { colours: ['panelFill', 'sky:2', 'bone', 'boneShade'], pattern: 3 },
    lit: { colours: ['petalLight', 'white', 'bone', 'boneShade'], pattern: 3 },
    door: { colours: ['chestDark', 'chest', 'chestDark', 'wallDark'], pattern: 2 },
    trim: { colours: ['boneShade', 'bone', 'boneShade', 'boneEdge'] },
    trunk: { colours: ['chestDark', 'chest', 'chestDark', 'wallDark'], pattern: 2 },
    leaf: { colours: ['bush', 'bushLight', 'bushDark', 'bush'], soft: true },
    hedge: { colours: ['bushDark', 'bush', 'bushDark', 'bushLight'] },
    palm: { colours: ['rootDark', 'fibre', 'rootMid', 'boneEdge'], pattern: 1 },
    frond: { colours: ['stem', 'stemLight', 'stemDark', 'stem'], soft: true },
    post: { colours: ['panelHi', 'panelHi', 'panelLo', 'panelLo'] },
    lamp: { colours: ['petalLight', 'white', 'petalLight', 'petalLight'] }
  };
  function stuffOf(name) {
    var m = /^(awning|car)(\d)$/.exec(name), gem = m && 'gem:' + m[2];   // a shop's stripes, a car's paint: in one of the wordmark's colours
    return !m ? STUFF[name] : m[1] === 'car' ? { colours: [gem, gem, gem, 'white'] } : { colours: [gem, 'white', gem, 'white'], pattern: 6 };
  }

  /* What kind of landmark a place gets, from the map's own word for it. */
  var KINDS = {
    green: /park|garden|forest|wood|playground|nature|pitch|golf|cemetery|beach|zoo|grass|meadow|common/,
    shop: /cafe|restaurant|bar|pub|fast_food|bakery|convenience|supermarket|shop|market|food|ice_cream|biergarten|mall|store|books|clothes/,
    hall: /station|museum|theatre|library|school|university|college|hospital|townhall|worship|stadium|arts|gallery|cinema|attraction|airport|aerodrome|hotel|bank|post/
  };
  function landmark(kind) {
    var k = String(kind || '');
    return KINDS.green.test(k) ? 'green' : KINDS.shop.test(k) ? 'shop' : KINDS.hall.test(k) ? 'hall' : 'tower';
  }

  var SEA = 5, TALL = 64, FLOOR = 4;                // water's level, the block's height, a storey: in blocks
  var FREE = 0, ROAD = 1, SITE = 2, BUILT = 3, TREE = 4, PAVE = 5;

  function generate(chosen, opts) {
    /* In the order of their geohashes, whatever order they were chosen in: the
       same places are the same land. The order chosen is only where you start. */
    var sites = chosen.slice().sort(function (a, b) { return a.id < b.id ? -1 : a.id > b.id ? 1 : 0; });
    var n = sites.length, seed = Geo.fnv(sites.map(function (s) { return s.id; }).join(','));
    var N = n <= 2 ? 112 : n <= 4 ? 144 : n <= 6 ? 176 : 208, EDGE = 22, APART = 34;
    var world = new Vox.World(N, TALL, N), ids = {};
    function M(name) {
      if (!ids[name]) { var spec = stuffOf(name); ids[name] = opts.stuff(name, spec); if (spec.soft) world.soft[ids[name]] = 1; }
      return ids[name];
    }
    function h(a, b) {
      var x = (Math.imul(a | 0, 374761393) + Math.imul(b | 0, 668265263) + seed) | 0;
      x = Math.imul(x ^ (x >>> 13), 1274126177);
      return ((x ^ (x >>> 16)) >>> 0) / 4294967296;
    }
    function lerp(a, b, t) { return a + (b - a) * t; }
    function clamp(v, a, b) { return v < a ? a : v > b ? b : v; }
    function ease(t) { t = clamp(t, 0, 1); return t * t * (3 - 2 * t); }
    function noise(x, z, period, salt) {
      var i = Math.floor(x / period), j = Math.floor(z / period), fx = ease(x / period - i), fz = ease(z / period - j);
      return lerp(lerp(h(i * 31 + salt, j), h((i + 1) * 31 + salt, j), fx), lerp(h(i * 31 + salt, j + 1), h((i + 1) * 31 + salt, j + 1), fx), fz);
    }

    /* ── WHERE THE PLACES STAND: as on the map, fitted to the block, eased apart ── */
    var lat0 = 0, lng0 = 0;
    sites.forEach(function (s) { lat0 += s.lat / n; lng0 += s.lng / n; });
    var plan = Geo.plan(lat0, lng0), m = sites.map(function (s) { return plan(s.lat, s.lng); });
    var lo = [1e12, 1e12], hi = [-1e12, -1e12];
    m.forEach(function (p) { lo[0] = Math.min(lo[0], p[0]); lo[1] = Math.min(lo[1], p[1]); hi[0] = Math.max(hi[0], p[0]); hi[1] = Math.max(hi[1], p[1]); });
    var room = N - 2 * EDGE, scale = Math.min(room / Math.max(1, hi[0] - lo[0]), room / Math.max(1, hi[1] - lo[1]));
    var pos = m.map(function (p) {
      return [N / 2 + (p[0] - (lo[0] + hi[0]) / 2) * scale, N / 2 + (p[1] - (lo[1] + hi[1]) / 2) * scale];
    });
    for (var pass = 0; pass < 80; pass++) for (var a = 0; a < n; a++) for (var b = a + 1; b < n; b++) {
      var dx = pos[b][0] - pos[a][0], dz = pos[b][1] - pos[a][1], d = Math.hypot(dx, dz);
      if (d >= APART) continue;
      if (d < 0.01) { var t = h(a, b + 900) * 6.283; dx = Math.cos(t); dz = Math.sin(t); d = 1; }
      var push = (APART - d) / 2 + 0.01;
      pos[a][0] -= dx / d * push; pos[a][1] -= dz / d * push; pos[b][0] += dx / d * push; pos[b][1] += dz / d * push;
      [a, b].forEach(function (i) { pos[i][0] = clamp(pos[i][0], EDGE, N - EDGE); pos[i][1] = clamp(pos[i][1], EDGE + 6, N - EDGE); });
    }
    pos = pos.map(function (p) { return [Math.round(p[0]), Math.round(p[1])]; });

    /* ── THE GROUND: hills, and hollows that hold water ── */
    var H = new Uint8Array(N * N), occ = new Uint8Array(N * N), x, z, i, j;
    function at(x2, z2) { return z2 * N + x2; }
    function inside(x2, z2, pad) { return x2 >= pad && z2 >= pad && x2 < N - pad && z2 < N - pad; }
    for (z = 0; z < N; z++) for (x = 0; x < N; x++) {
      var hills = 4.5 * noise(x, z, 38, 1) + 2.5 * noise(x, z, 15, 2), hollow = Math.max(0, noise(x, z, 64, 3) - 0.6);
      H[at(x, z)] = clamp(Math.round(SEA + 1.5 + hills - hollow * 30), SEA - 1, 30);
    }
    var level = pos.map(function (p) {              // each place on a level of its own, the land easing up to it
      var sum = 0, c = 0;
      for (var dz2 = -5; dz2 <= 5; dz2++) for (var dx2 = -5; dx2 <= 5; dx2++) if (inside(p[0] + dx2, p[1] + dz2, 0)) { sum += H[at(p[0] + dx2, p[1] + dz2)]; c++; }
      return Math.max(SEA + 1, Math.round(sum / c));
    });
    for (z = 0; z < N; z++) for (x = 0; x < N; x++) for (i = 0; i < n; i++) {
      var far = Math.hypot(x - pos[i][0], z - (pos[i][1] - 4));
      if (far < 22) H[at(x, z)] = Math.round(lerp(H[at(x, z)], level[i], ease(1 - (far - 10) / 12)));
    }

    /* ── EACH PLACE'S OWN GROUND: a landmark behind a forecourt, facing you ── */
    var keep = new Uint8Array(N * N), pond = new Uint8Array(N * N);
    function lot(x0, z0, w, d2, base, what) {
      for (var z2 = z0; z2 < z0 + d2; z2++) for (var x2 = x0; x2 < x0 + w; x2++) { H[at(x2, z2)] = base; occ[at(x2, z2)] = what; }
    }
    var spots = sites.map(function (s, k) {
      var p = pos[k], kind = landmark(s.kind), base = level[k], w, d2, x0, z0, ring = [], x2, z2;
      if (kind === 'green') {                       // a pond, and trees round it
        for (z2 = -2; z2 <= 2; z2++) for (x2 = -3; x2 <= 3; x2++) if (Math.hypot(x2 * 0.7, z2) < 2.2) {
          var c = at(p[0] + x2, p[1] - 8 + z2);
          H[c] = base - 1; pond[c] = 1; occ[c] = SITE; keep[c] = 1;
        }
        for (var t = 0; t < 7; t++) {
          var ang = t / 7 * 6.283 + h(k, 71) * 3, tx = Math.round(p[0] + Math.cos(ang) * 8), tz = Math.round(p[1] - 8 + Math.sin(ang) * 5);
          if (inside(tx, tz, 3) && occ[at(tx, tz)] === FREE && Math.hypot(tx - p[0], tz - p[1]) > 5) { occ[at(tx, tz)] = SITE; keep[at(tx, tz)] = 1; ring.push([tx, tz]); }
        }
      } else {
        w = kind === 'hall' ? 11 : kind === 'shop' ? 8 : 7; d2 = kind === 'tower' ? 7 : 6;
        x0 = p[0] - Math.floor(w / 2); z0 = p[1] - 3 - d2;
        for (z2 = z0 - 1; z2 < z0 + d2 + 1; z2++) for (x2 = x0 - 1; x2 < x0 + w + 1; x2++) if (inside(x2, z2, 0)) { H[at(x2, z2)] = base; occ[at(x2, z2)] = SITE; keep[at(x2, z2)] = 1; }
        lot(x0, z0, w, d2, base, BUILT);
      }
      return { id: s.id, name: s.name, kind: s.kind, as: kind, x: p[0], z: p[1], level: base, lot: kind === 'green' ? null : [x0, z0, w, d2], trees: ring,
               board: [p[0] + 3, p[1] + 1] };
    });

    /* ── THE ROADS: the shortest set that joins every place, and one more to close a loop ── */
    var edges = [], linked = [0], best, a2, b2;
    while (linked.length < n) {
      best = null;
      linked.forEach(function (p) { for (var q = 0; q < n; q++) if (linked.indexOf(q) < 0) { var len = Math.hypot(pos[p][0] - pos[q][0], pos[p][1] - pos[q][1]); if (!best || len < best[2]) best = [p, q, len]; } });
      edges.push(best); linked.push(best[1]);
    }
    if (n >= 4) {
      best = null;
      for (a2 = 0; a2 < n; a2++) for (b2 = a2 + 1; b2 < n; b2++) {
        if (edges.some(function (g) { return (g[0] === a2 && g[1] === b2) || (g[0] === b2 && g[1] === a2); })) continue;
        var len2 = Math.hypot(pos[a2][0] - pos[b2][0], pos[a2][1] - pos[b2][1]);
        if (!best || len2 < best[2]) best = [a2, b2, len2];
      }
      if (best) edges.push(best);
    }
    /* A road finds its own way, on the square: it runs east-west or north-south
       and pays to turn, so streets come out straight with corners, as a city's
       are. It would rather stay level and dry, it goes round what is built, and
       it joins a road that is already there. */
    function route(A, B) {
      var open = [], cost = new Float32Array(N * N * 4).fill(1e9), from = new Int32Array(N * N * 4).fill(-1), done = new Uint8Array(N * N * 4);
      var DX = [1, -1, 0, 0], DZ = [0, 0, 1, -1], TURN = 7;
      function push(c, f) {
        open.push([f, c]);
        for (var k = open.length - 1; k > 0;) { var up = (k - 1) >> 1; if (open[up][0] <= open[k][0]) break; var t = open[up]; open[up] = open[k]; open[k] = t; k = up; }
      }
      function pop() {
        var top = open[0], last = open.pop();
        if (open.length) {
          open[0] = last;
          for (var k = 0; ;) {
            var l = k * 2 + 1, r = l + 1, m2 = k;
            if (l < open.length && open[l][0] < open[m2][0]) m2 = l;
            if (r < open.length && open[r][0] < open[m2][0]) m2 = r;
            if (m2 === k) break;
            var t = open[m2]; open[m2] = open[k]; open[k] = t; k = m2;
          }
        }
        return top;
      }
      var start = at(A[0], A[1]), goal = at(B[0], B[1]), end = -1, d;
      for (d = 0; d < 4; d++) { cost[start * 4 + d] = 0; push(start * 4 + d, 0); }
      while (open.length) {
        var cur = pop()[1], cell = cur >> 2, dir = cur & 3;
        if (cell === goal) { end = cur; break; }
        if (done[cur]) continue;
        done[cur] = 1;
        var cx = cell % N, cz = (cell / N) | 0;
        for (d = 0; d < 4; d++) {
          var x2 = cx + DX[d], z2 = cz + DZ[d], ni = at(x2, z2), ns = ni * 4 + d;
          if (!inside(x2, z2, 6) || done[ns] || keep[ni]) continue;
          var step = 1 + (H[ni] < SEA ? 2.5 : 0) + 0.7 * Math.abs(H[ni] - H[cell]) - (occ[ni] === ROAD ? 0.5 : 0) + (d !== dir && cell !== start ? TURN : 0);
          if (cost[cur] + step < cost[ns]) { cost[ns] = cost[cur] + step; from[ns] = cur; push(ns, cost[ns] + Math.abs(x2 - B[0]) + Math.abs(z2 - B[1])); }
        }
      }
      var path = [];                                // each step: where, and which way it was heading (1 east-west, 2 north-south)
      for (var c = end; c >= 0; c = from[c]) { path.push([(c >> 2) % N, ((c >> 2) / N) | 0, (c & 3) < 2 ? 1 : 2]); if ((c >> 2) === start) break; }
      return path.reverse();
    }
    var near = new Float32Array(N * N).fill(99), roadH = new Uint8Array(N * N), lane = new Uint8Array(N * N), paths = [];
    edges.forEach(function (g) {
      var path = route(pos[g[0]], pos[g[1]]);
      /* a road keeps its feet dry and climbs a block at a time */
      var r = path.map(function (p) { return Math.max(SEA + 1, H[at(p[0], p[1])]); }), sm = r.slice();
      for (i = 0; i < r.length; i++) { var sum = 0, c = 0; for (j = Math.max(0, i - 5); j <= Math.min(r.length - 1, i + 5); j++) { sum += r[j]; c++; } sm[i] = Math.round(sum / c); }
      sm[0] = level[g[0]]; sm[sm.length - 1] = level[g[1]];
      for (i = 1; i < sm.length - 1; i++) sm[i] = clamp(sm[i], sm[i - 1] - 1, sm[i - 1] + 1);
      for (i = sm.length - 2; i > 0; i--) sm[i] = clamp(sm[i], sm[i + 1] - 1, sm[i + 1] + 1);
      path.forEach(function (p, q) {
        occ[at(p[0], p[1])] = ROAD; lane[at(p[0], p[1])] = p[2];
        for (var dz2 = -7; dz2 <= 7; dz2++) for (var dx2 = -7; dx2 <= 7; dx2++) {
          var x2 = p[0] + dx2, z2 = p[1] + dz2, d2 = Math.hypot(dx2, dz2);
          if (inside(x2, z2, 0) && d2 < near[at(x2, z2)]) { near[at(x2, z2)] = d2; roadH[at(x2, z2)] = sm[q]; }
        }
      });
      paths.push(path);
    });
    for (i = 0; i < N * N; i++) {
      if (keep[i] || pond[i]) continue;
      if (near[i] <= 2.5) { H[i] = roadH[i]; occ[i] = ROAD; }
      else if (near[i] <= 3.6) { H[i] = roadH[i]; occ[i] = PAVE; }
      else if (near[i] <= 7) H[i] = Math.round(lerp(roadH[i], H[i], (near[i] - 3.6) / 3.4));
    }
    /* each place's forecourt, where its road arrives */
    pos.forEach(function (p, k) {
      for (var dz2 = -2; dz2 <= 3; dz2++) for (var dx2 = -4; dx2 <= 4; dx2++) {
        var c = at(p[0] + dx2, p[1] + dz2);
        if (!inside(p[0] + dx2, p[1] + dz2, 1) || Math.hypot(dx2 * 0.8, dz2) > 3.4 || occ[c] === BUILT) continue;
        H[c] = level[k]; occ[c] = PAVE;
      }
    });

    /* ── THE BLOCK ITSELF: bedrock, stone, soil, and what lies on top ── */
    function wet(x2, z2) { return inside(x2, z2, 0) && (H[at(x2, z2)] < SEA || pond[at(x2, z2)]); }
    function ground() {
      for (z = 0; z < N; z++) for (x = 0; x < N; x++) {
        var c = at(x, z), top = H[c], water = top < SEA || pond[c];
        var shore = !water && occ[c] !== ROAD && occ[c] !== PAVE && (wet(x - 1, z) || wet(x + 1, z) || wet(x, z - 1) || wet(x, z + 1));
        for (var y = 0; y < top; y++)
          world.set(x, y, z, y === 0 ? M('bed') : y < top - 4 ? M('stone') : y < top - 1 ? M('dirt')
            : occ[c] === ROAD ? M(lane[c] === 1 ? 'roadX' : lane[c] === 2 ? 'roadZ' : 'asphalt') : occ[c] === PAVE ? M('pave') : water || shore ? M('sand') : M('grass'));
        if (water) world.set(x, top, z, M('water'));
      }
    }
    function free(x0, z0, w, d2, pad) {             // a lot nothing stands on, dry and near enough level
      var min = 99, max = 0;
      for (var z2 = z0 - pad; z2 < z0 + d2 + pad; z2++) for (var x2 = x0 - pad; x2 < x0 + w + pad; x2++) {
        if (!inside(x2, z2, 2) || occ[at(x2, z2)] !== FREE || H[at(x2, z2)] < SEA) return -1;
        min = Math.min(min, H[at(x2, z2)]); max = Math.max(max, H[at(x2, z2)]);
      }
      return max - min > 3 ? -1 : Math.round((min + max) / 2);
    }

    /* ── A BUILDING: walls, a window to every other block of each storey, a door, a roof ── */
    var builds = [];
    function house(x0, z0, w, d2, base, face, o) {  // face: which side the door is on: 0 +z, 1 -z, 2 +x, 3 -x
      var wall = M(o.wall), top = base + o.floors * FLOOR, x2, y, z2, f, k, panes = [];
      builds.push({ x0: x0, z0: z0, w: w, d: d2, base: base, top: top, face: face, wall: o.wall, roof: o.roof, floors: o.floors, gable: !!o.gable,
                    awning: o.awning || null, columns: !!o.columns, door: o.door === undefined ? 2 : o.door, mark: !!o.mark, panes: panes });
      for (y = base; y < top; y++) for (z2 = z0; z2 < z0 + d2; z2++) for (x2 = x0; x2 < x0 + w; x2++) world.set(x2, y, z2, wall);
      function side(s, k2) {                        // the k2-th block along side s, in the wall's own face
        return s === 0 ? [x0 + k2, z0 + d2 - 1] : s === 1 ? [x0 + k2, z0] : s === 2 ? [x0 + w - 1, z0 + k2] : [x0, z0 + k2];
      }
      for (var s = 0; s < 4; s++) {
        var len = s < 2 ? w : d2;
        for (f = 0; f < o.floors; f++) for (k = 1; k < len - 1; k++) {
          if ((k + (len & 1 ? 0 : f)) % 2 === 0) continue;
          var c = side(s, k), lit = h(c[0] * 7 + f, c[1] * 13 + s + o.salt) < 0.3, big = f === 0 && o.tallGlass;
          if (!(s === face && f === 0 && k >= Math.floor(((s < 2 ? w : d2) - (o.door === undefined ? 2 : o.door)) / 2) && k < Math.floor(((s < 2 ? w : d2) - (o.door === undefined ? 2 : o.door)) / 2) + (o.door === undefined ? 2 : o.door)))
            panes.push([s, k, f, lit, big]);          // not where the door is
          for (y = big ? 1 : 2; y < 3; y++) world.set(c[0], base + f * FLOOR + y, c[1], M(lit ? 'lit' : 'glass'));
        }
      }
      var span = face < 2 ? w : d2, wide = o.door === undefined ? 2 : o.door, d0 = Math.floor((span - wide) / 2);
      var out = face === 0 ? [0, 1] : face === 1 ? [0, -1] : face === 2 ? [1, 0] : [-1, 0];
      for (k = 0; k < wide; k++) { var dc = side(face, d0 + k); for (y = 0; y < 3; y++) world.set(dc[0], base + y, dc[1], M('door')); }
      if (o.awning) for (k = 0; k < span; k++) { var ac = side(face, k); world.set(ac[0] + out[0], base + FLOOR, ac[1] + out[1], M(o.awning)); }   // over heads, not at them
      if (o.columns) for (k = 0; k < span; k += 2) {
        var cc = side(face, k);
        if (k >= d0 - 1 && k <= d0 + wide) continue;                              // none across the door
        for (y = base; y < top; y++) world.set(cc[0] + out[0], y, cc[1] + out[1], M('trim'));
        if (inside(cc[0] + out[0], cc[1] + out[1], 0)) occ[at(cc[0] + out[0], cc[1] + out[1])] = BUILT;
      }
      if (o.gable) {                                // a ridge along the longer way
        var long = w >= d2, across = long ? d2 : w;
        for (k = 0; k * 2 < across; k++) for (z2 = z0 + (long ? k : 0); z2 < z0 + d2 - (long ? k : 0); z2++) for (x2 = x0 + (long ? 0 : k); x2 < x0 + w - (long ? 0 : k); x2++)
          world.set(x2, top + k, z2, M(o.roof));
        return top + Math.ceil(across / 2);
      }
      for (z2 = z0; z2 < z0 + d2; z2++) for (x2 = x0; x2 < x0 + w; x2++)
        world.set(x2, top, z2, x2 === x0 || z2 === z0 || x2 === x0 + w - 1 || z2 === z0 + d2 - 1 ? M('trim') : M(o.roof));
      return top + 1;
    }
    var plants = [];
    function tree(x2, z2) {
      var base = H[at(x2, z2)], tall = 2 + Math.floor(h(x2 * 3, z2 * 5 + 77) * 2), y;
      plants.push([x2, z2, tall]);
      for (y = 0; y < tall; y++) world.set(x2, base + y, z2, M('trunk'));
      for (var dz2 = -1; dz2 <= 1; dz2++) for (var dx2 = -1; dx2 <= 1; dx2++) {
        world.set(x2 + dx2, base + tall, z2 + dz2, M('leaf'));
        if (dx2 === 0 || dz2 === 0) world.set(x2 + dx2, base + tall + 1, z2 + dz2, M('leaf'));
      }
      world.set(x2, base + tall + 2, z2, M('leaf'));
      occ[at(x2, z2)] = BUILT;                      // a trunk is in the way; its leaves are not
    }

    /* ── THE BUILDINGS BETWEEN: lots along the roads, either side, two deep.
       A shop stands on paving; a house on its own grass, behind a hedge. ── */
    var lots = [], hedges = [];
    paths.forEach(function (path, pk) {
      for (var q = 5, gap; q < path.length - 5; q += gap) {
        gap = 5 + Math.floor(h(pk * 31 + q, 211) * 4);
        var p = path[q], tx = path[q + 3][0] - path[q - 3][0], tz = path[q + 3][1] - path[q - 3][1], tl = Math.hypot(tx, tz) || 1;
        if (pos.some(function (s) { return Math.hypot(s[0] - p[0], s[1] - (p[1] - 4)) < 12; })) continue;
        [1, -1].forEach(function (sd) {
          var nx = -tz / tl * sd, nz = tx / tl * sd, out = 4.7;
          for (var row = 0; row < 2; row++) {
            var w = 4 + Math.floor(h(pk * 17 + q, 223 + sd + row * 50) * 4), d2 = 4 + Math.floor(h(pk * 19 + q, 227 + sd + row * 50) * 3);
            var half = (Math.abs(nx) * w + Math.abs(nz) * d2) / 2, x0 = Math.round(p[0] + nx * (out + half) - w / 2), z0 = Math.round(p[1] + nz * (out + half) - d2 / 2);
            var base = free(x0, z0, w, d2, 1), salt = pk * 131 + q * 7 + sd + row * 977, home = h(salt, 421) < 0.5;
            out += half * 2 + 2.5;
            if (base < 0) continue;
            var face = Math.abs(nx) > Math.abs(nz) ? (nx > 0 ? 3 : 2) : (nz > 0 ? 1 : 0);
            lot(x0, z0, w, d2, base, BUILT);
            for (var z2 = z0 - 1; z2 <= z0 + d2; z2++) for (var x2 = x0 - 1; x2 <= x0 + w; x2++) {     // the strip round it
              if (occ[at(x2, z2)] !== FREE) continue;
              var front = face === 0 ? z2 === z0 + d2 : face === 1 ? z2 === z0 - 1 : face === 2 ? x2 === x0 + w : x2 === x0 - 1;
              H[at(x2, z2)] = base;
              if (home && !front) { occ[at(x2, z2)] = TREE; hedges.push([x2, z2]); } else occ[at(x2, z2)] = PAVE;
            }
            lots.push([x0, z0, w, d2, base, face, salt, home]);
          }
        });
      }
    });
    /* ── ON THE PAVEMENT AND AT THE KERB: palms, a lamp to every third, a car here and there ── */
    var props = [], cars = [];
    paths.forEach(function (path, pk) {
      for (var q = 4, n2 = 0; q < path.length - 4; q += 5, n2++) {
        var p = path[q], tx = path[q + 2][0] - path[q - 2][0], tz = path[q + 2][1] - path[q - 2][1], tl = Math.hypot(tx, tz) || 1, sd = n2 & 1 ? 1 : -1;
        var x2 = Math.round(p[0] - tz / tl * sd * 3.2), z2 = Math.round(p[1] + tx / tl * sd * 3.2), c = at(x2, z2);
        if (inside(x2, z2, 3) && occ[c] === PAVE && !pos.some(function (s) { return Math.hypot(s[0] - x2, s[1] - z2) < 7; })) { occ[c] = BUILT; props.push([x2, z2, n2 % 3 === 2 ? 'lamp' : 'palm']); }
        var way = lane[at(p[0], p[1])];
        if (path[q - 1][2] !== way || path[q + 1][2] !== way || n2 % 2 || h(pk * 41 + q, 607) > 0.75) continue;   // a car parks on the straight
        var cells = [], ok = true;
        for (var along = -1; along <= 1; along++) for (var across = 1; across <= 2; across++) {
          var cx = way === 1 ? p[0] + along : p[0] - sd * across, cz = way === 1 ? p[1] - sd * across : p[1] + along;
          if (occ[at(cx, cz)] !== ROAD || near[at(cx, cz)] < 0.9 || H[at(cx, cz)] !== H[at(p[0], p[1])]) ok = false;
          cells.push([cx, cz, along === 0]);
        }
        if (ok) { cells.forEach(function (k) { occ[at(k[0], k[1])] = BUILT; }); cars.push([cells, Math.floor(h(pk * 43 + q, 613) * 7)]); }
      }
    });
    /* ── TREES: scattered, and thick where the land says grove ── */
    var trees = [];
    for (z = 3; z < N - 3; z++) for (x = 3; x < N - 3; x++) {
      if (occ[at(x, z)] !== FREE || H[at(x, z)] < SEA || near[at(x, z)] < 5) continue;
      if (h(x * 7 + 1, z * 13 + 5) < 0.004 + 0.06 * Math.max(0, noise(x, z, 22, 9) - 0.62)) {
        trees.push([x, z]);
        for (var dz3 = -1; dz3 <= 1; dz3++) for (var dx3 = -1; dx3 <= 1; dx3++) if (occ[at(x + dx3, z + dz3)] === FREE) occ[at(x + dx3, z + dz3)] = TREE;
      }
    }

    ground();
    spots.forEach(function (s, k) {
      s.top = s.level + 6;
      if (s.as === 'tower') {
        var top = house(s.lot[0], s.lot[1], 7, 7, s.level, 0, { salt: k * 37, roof: 'roof', wall: 'ashlar', floors: 4 + Math.floor(h(k, 301) * 3), mark: true });
        s.top = house(s.lot[0] + 1, s.lot[1] + 1, 5, 5, top, 0, { salt: k, roof: 'copper', wall: 'ashlar', floors: 1, gable: true, door: 0, mark: true });
      } else if (s.as === 'hall') s.top = house(s.lot[0], s.lot[1], s.lot[2], s.lot[3], s.level, 0, { salt: k, roof: 'copper', wall: 'plaster', floors: 2, gable: true, columns: true, door: 3, mark: true });
      else if (s.as === 'shop') s.top = house(s.lot[0], s.lot[1], s.lot[2], s.lot[3], s.level, 0, { salt: k, roof: 'roof', wall: 'brick', floors: 2, awning: 'awning' + (k % 7), tallGlass: true, mark: true });
      s.trees.forEach(function (t) { tree(t[0], t[1]); });
    });
    lots.forEach(function (l) {                     // a house: plaster or timber under a ridge; a shop: brick or stone, flat-topped, glass to the street
      var r = h(l[6], 401), r2 = h(l[6], 409), home = l[7];
      house(l[0], l[1], l[2], l[3], l[4], l[5], { salt: l[6], wall: home ? (r < 0.5 ? 'plaster' : 'timber') : (r < 0.5 ? 'brick' : 'ashlar'), roof: r2 < 0.5 ? 'roof' : 'copper',
                                                  floors: home ? 1 + Math.floor(h(l[6], 419) * 2) : 2 + Math.floor(h(l[6], 419) * 3), gable: home, tallGlass: !home,
                                                  awning: !home && h(l[6], 431) < 0.5 ? 'awning' + Math.floor(r2 * 7) : null });
    });
    hedges.forEach(function (k) { world.set(k[0], H[at(k[0], k[1])], k[1], M('hedge')); });
    props.forEach(function (k) {
      var base = H[at(k[0], k[1])], y;
      if (k[2] === 'lamp') { k[3] = 4; for (y = 0; y < 4; y++) world.set(k[0], base + y, k[1], M('post')); world.set(k[0], base + 4, k[1], M('lamp')); return; }
      var tall = k[3] = 5 + Math.floor(h(k[0] * 3, k[1] * 7 + 31) * 3);
      for (y = 0; y < tall; y++) world.set(k[0], base + y, k[1], M('palm'));
      [[0, 0, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1], [2, -1, 0], [-2, -1, 0], [0, -1, 2], [0, -1, -2], [0, 1, 0]].forEach(function (f) { world.set(k[0] + f[0], base + tall + f[1], k[1] + f[2], M('frond')); });
    });
    cars.forEach(function (car) {                   // six blocks of paint, and a cab of glass over the middle two
      car[0].forEach(function (k) {
        var base = H[at(k[0], k[1])];
        world.set(k[0], base, k[1], M('car' + car[1]));
        if (k[2]) world.set(k[0], base + 1, k[1], M('glass'));
      });
    });
    trees.forEach(function (t) { tree(t[0], t[1]); });

    /* How high each column stands, for the sun to cast by; and what each is, for a radar to show. */
    var tops = new Uint8Array(N * N), map = new Uint8Array(N * N);
    for (z = 0; z < N; z++) for (x = 0; x < N; x++) {
      var y2 = TALL - 1;
      while (y2 > 0 && !world.get(x, y2, z)) y2--;
      tops[at(x, z)] = y2 + 1;
      map[at(x, z)] = H[at(x, z)] < SEA || pond[at(x, z)] ? 4 : occ[at(x, z)] === ROAD ? 1 : occ[at(x, z)] === PAVE ? 2 : occ[at(x, z)] === TREE ? 5 : 0;
    }
    lots.map(function (l) { return l.slice(0, 4); }).concat(spots.filter(function (s) { return s.lot; }).map(function (s) { return s.lot; })).forEach(function (l) {
      for (var z2 = l[1]; z2 < l[1] + l[3]; z2++) for (var x2 = l[0]; x2 < l[0] + l[2]; x2++) map[at(x2, z2)] = 3;
    });

    return {
      world: world, size: N, tall: TALL, sea: SEA, seed: seed, scale: scale, paths: paths, lots: lots, trees: trees.length, cars: cars.length, props: props.length,
      tops: tops, map: map, heights: H, lanes: lane,
      builds: builds, plants: plants, props: props, hedges: hedges,
      parked: cars.map(function (car) {              // where a car stands, which way it points, what colour it is
        var xs = car[0].map(function (k) { return k[0]; }), zs = car[0].map(function (k) { return k[1]; });
        var x0 = Math.min.apply(null, xs), z0 = Math.min.apply(null, zs), x1 = Math.max.apply(null, xs) + 1, z1 = Math.max.apply(null, zs) + 1;
        return { x0: x0, z0: z0, x1: x1, z1: z1, alongX: x1 - x0 > z1 - z0, paint: car[1], level: H[at(x0, z0)] };
      }),
      spots: chosen.map(function (c) { return spots[sites.indexOf(c)]; }),
      height: function (x2, z2) { return inside(x2, z2, 0) ? H[at(x2, z2)] : 0; },
      /* Can a body stand here: ground under it, and nothing built on it. */
      open: function (x2, z2) { return inside(x2, z2, 0) && occ[at(x2, z2)] !== BUILT; }
    };
  }

  return { generate: generate, landmark: landmark, STUFF: STUFF, stuffOf: stuffOf, SEA: SEA };
});
