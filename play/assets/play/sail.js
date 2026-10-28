/* sail.js — the sea, sailed.
 *
 * ocean.js says what the sea, the wind and a boat are doing; this shows it and
 * puts the tiller and the sheet in your hands. An island with a light on it, a
 * dinghy off its jetty, and a course of two marks laid to the wind: the first
 * dead upwind, where no boat can sail straight, the second across and down it,
 * then home.
 *
 * THE SEA IS DRAWN AS IT IS SAILED. Each frame ocean.js's `sample` fills a
 * square of the sea round the view, a column of water to a block of land.js's
 * (eight art pixels), its top anywhere to the art pixel; the GPU stands the
 * columns up. Nothing about a wave is worked out twice. Past the square the
 * sea is flat, and the square goes level at its edge to meet it.
 *
 * WHAT TELLS YOU THE WIND: the water (dark where a puff is on it, white where
 * it breaks), the clouds (they go where the wind goes), the flag at the
 * masthead (the wind as the boat feels it, which is not the same), and the
 * sail (it flaps when it is doing nothing). The dial says it again in numbers:
 * heading, speed in knots, the wind, and how the next mark bears and how far.
 *
 * THREE CONTROLS. Left and right are the tiller; up hauls the sheet in and
 * down lets it out; the button lets it fly, for a puff. Keys, or hud.js's stick
 * and button under the thumbs.
 *
 * THE COLOURS ARE CEREMONY.JS'S and the island's stuff is land.js's. The sea is
 * the wordmark's blue. Nothing here states a colour.
 *
 * MADE UP, AND NOT KEPT. The island, the marks and the time you took are gone
 * when you leave, on this device and every other. No Delta carries any of it.
 *
 * ?v= IS LOAD-BEARING. assets/ is served immutable for a year; bump the number
 * in sail.html in the same change as any edit here.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin = root.Kenjin || {}, En = K.Engine, Ocean = K.Ocean, Vox = root.Vox;
  var clamp = En.clamp, lerp = En.lerp, PX = Ocean.PX, DEG = Ocean.DEG, TAU = Math.PI * 2;
  var G = 8;                                  // art pixels to a block: land.js's, and a column of sea is one
  var AHEAD = 150;                            // the square of sea sits this far ahead of what the view is on
  var FETCH = 60000;                          // open water: this much of it to windward, in metres

  var STUFF = {
    sea: { colours: ['gem:4', 'gem:6', ['gem:4', 0.2], ['gem:4', -0.22]] },      // itself, a glint, in the light, out of it
    foam: { colours: ['cloud', 'cloudLight', 'cloudShade', 'gem:6'] },
    band: { colours: ['gem:5', ['gem:5', 0.15], ['gem:5', -0.12], ['gem:5', -0.35]], pattern: 1 }
  };

  /* ── THE SURFACE, ON THE GPU ──
     A unit cube to every cell (vox.js's own, so its faces look the way the
     engine's shades expect), stood as high as the cell's byte says. A side
     stops where the column next door starts; the underside is not drawn.
     What a face looks like is its stuff, as in engine.js: four colours and a
     rule, dithered by ceremony.js's matrix where one gives way to another. */
  var VS = '#version 300 es\nprecision highp float; precision highp int; precision highp usampler2D;\n' +
    'layout(location=0) in uvec3 aPos; layout(location=1) in uvec3 aAux;\n' +
    'uniform mat4 uVP; uniform usampler2D uSea; uniform ivec2 uAt; uniform int uN; uniform float uCell; uniform float uLevel; uniform vec3 uFar;\n' +
    'flat out int vN; flat out ivec3 vOn; flat out float vH; out vec3 vW;\n' +
    'void main(){ vN = int(aAux.y); vH = 0.0; vec3 w;\n' +
    '  if (uFar.z > 0.0) { vOn = ivec3(0, 128, 0); w = vec3(uFar.x + (float(aPos.x) - 0.5) * uFar.z, uLevel, uFar.y + (float(aPos.z) - 0.5) * uFar.z); }\n' +
    '  else { ivec2 c = ivec2(gl_InstanceID % uN, gl_InstanceID / uN); uvec4 s = texelFetch(uSea, c, 0); float y = float(s.r); vH = y - 128.0;\n' +
    '    if (aPos.y == 0u) { ivec2 d = vN == 0 ? ivec2(-1, 0) : vN == 1 ? ivec2(1, 0) : vN == 4 ? ivec2(0, -1) : ivec2(0, 1);\n' +
    '      y = min(y, float(texelFetch(uSea, clamp(c + d, ivec2(0), ivec2(uN - 1)), 0).r)); }\n' +
    '    vOn = ivec3(s.gba); w = vec3(float(uAt.x + c.x) + float(aPos.x), 0.0, float(uAt.y + c.y) + float(aPos.z)) * uCell; w.y = uLevel + y - 128.0; }\n' +
    '  vW = w; gl_Position = vN == 2 ? vec4(2.0, 2.0, 2.0, 1.0) : uVP * vec4(w, 1.0); }';
  var FS = '#version 300 es\nprecision highp float; precision highp int;\n' +
    'uniform sampler2D uPal; uniform float uB[16]; uniform float uTime; uniform ivec2 uStuff; uniform vec4 uHole; uniform vec3 uFar; uniform vec3 uMid; uniform float uTall;\n' +
    'flat in int vN; flat in ivec3 vOn; flat in float vH; in vec3 vW; out vec4 o;\n' +
    'const float SH[6] = float[6](0.84, 0.74, 0.5, 1.0, 0.62, 0.92);\n' +
    'const vec3 NV[6] = vec3[6](vec3(-1,0,0), vec3(1,0,0), vec3(0,-1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1));\n' +
    'float hash(vec3 p){ p = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419)); p *= 17.0; return fract(p.x * p.y * p.z * (p.x + p.y + p.z)); }\n' +
    'vec3 pal(int id, int k){ return texelFetch(uPal, ivec2(id & 255, (id >> 8) + 16 * k), 0).rgb; }\n' +
    'void main(){ bool far = uFar.z > 0.0;\n' +
    '  if (far && vW.x > uHole.x && vW.z > uHole.y && vW.x < uHole.z && vW.z < uHole.w) discard;\n' +                 // the square is drawn in its own right
    '  ivec3 a = ivec3(floor(vW - NV[vN] * 0.5)); ivec2 uv = vN < 2 ? ivec2(a.z, a.y) : vN < 4 ? ivec2(a.x, a.z) : ivec2(a.x, a.y);\n' +
    '  float th = (uB[(uv.y & 3) * 4 + (uv.x & 3)] + 0.5) / 16.0;\n' +
    '  float near = far ? 0.0 : clamp((uMid.z - distance(vW.xz, uMid.xy)) / (0.3 * uMid.z), 0.0, 1.0);\n' +
    '  float tall = vH / uTall, light = tall * 0.55 - (float(vOn.y) - 128.0) / 127.0 * 1.3; int id = uStuff.x, k = 0;\n' +   // a crest catches the light; a trough, and a puff of wind, lose it
    '  if (light > th + 0.25) k = 2; else if (-light > th + 0.25) k = 3;\n' +
    '  if (vN == 3 && hash(vec3(float(uv.x >> 2), float(uv.y), floor(uTime * 2.0))) > 1.0 - 0.03 * near * clamp(tall, 0.0, 1.0)) k = 1;\n' +   // glints on the crests, in whole steps
    '  if (vN != 3 && k != 3) k = 0;\n' +
    '  if (float(max(vOn.x, vOn.z)) / 255.0 > th) { id = uStuff.y; k = hash(vec3(a)) > 0.75 ? 2 : 0; }\n' +            // a breaking crest, or a wake
    '  o = vec4(pal(id, k) * SH[vN], 1.0); }';

  function program(gl, vs, fs) {
    function sh(type, src) {
      var s = gl.createShader(type);
      gl.shaderSource(s, src); gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error('sail: ' + gl.getShaderInfoLog(s));
      return s;
    }
    var p = gl.createProgram();
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vs)); gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error('sail: ' + gl.getProgramInfoLog(p));
    var u = {}, n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
    for (var i = 0; i < n; i++) { var name = gl.getActiveUniform(p, i).name.replace(/\[0\]$/, ''); u[name] = gl.getUniformLocation(p, name); }
    return { p: p, u: u };
  }
  function surface(E, n, bayerRows, sea, foam) {
    var gl = E.gl, p = program(gl, VS, FS), cube = E.part({ mesh: Vox.meshCubes([0, 0, 0, 1]) }).gpu;
    var data = new Uint8Array(n * n * 4), tex = gl.createTexture(), bayer = new Float32Array(16);
    bayerRows.forEach(function (row, j) { row.forEach(function (v, i) { bayer[j * 4 + i] = v; }); });
    gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8UI, n, n, 0, gl.RGBA_INTEGER, gl.UNSIGNED_BYTE, data);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.activeTexture(gl.TEXTURE0);
    return {
      n: n, data: data,
      /* VP: the view. at: the square's corner, in cells. level: the sea's, and
         tall: what counts as a tall wave on it; both in art pixels. */
      draw: function (VP, at, level, tall, time) {
        var u = p.u, mid = [(at[0] + n / 2) * G, (at[1] + n / 2) * G];
        gl.useProgram(p.p);
        gl.uniformMatrix4fv(u.uVP, false, VP); gl.uniform1i(u.uPal, 0); gl.uniform1i(u.uSea, 1);
        gl.uniform2i(u.uAt, at[0], at[1]); gl.uniform1i(u.uN, n); gl.uniform1f(u.uCell, G); gl.uniform1f(u.uLevel, level); gl.uniform1f(u.uTall, tall);
        gl.uniform1fv(u.uB, bayer); gl.uniform1f(u.uTime, time); gl.uniform2i(u.uStuff, sea, foam);
        gl.uniform3f(u.uMid, mid[0], mid[1], n / 2 * G); gl.uniform4f(u.uHole, at[0] * G, at[1] * G, (at[0] + n) * G, (at[1] + n) * G);
        gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, n, n, gl.RGBA_INTEGER, gl.UNSIGNED_BYTE, data);
        gl.bindVertexArray(cube.vao);
        gl.uniform3f(u.uFar, 0, 0, 0); gl.drawElementsInstanced(gl.TRIANGLES, cube.count, gl.UNSIGNED_INT, 0, n * n);
        gl.uniform3f(u.uFar, mid[0], mid[1], 14000); gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_INT, 3 * 6 * 4);     // the cube's top, as wide as the view is deep
        gl.bindVertexArray(null); gl.activeTexture(gl.TEXTURE0);
      }
    };
  }

  /* A part whose own x, y and z lie along X, Y and Z, held by its point v at p. */
  function pose(p, X, Y, Z, v) {
    return new Float32Array([X[0], X[1], X[2], 0, Y[0], Y[1], Y[2], 0, Z[0], Z[1], Z[2], 0,
      p[0] - (X[0] * v[0] + Y[0] * v[1] + Z[0] * v[2]), p[1] - (X[1] * v[0] + Y[1] * v[1] + Z[1] * v[2]), p[2] - (X[2] * v[0] + Y[2] * v[1] + Z[2] * v[2]), 1]);
  }
  function along(p, a, s, b, t, c, u) { return [p[0] + a[0] * s + b[0] * t + c[0] * u, p[1] + a[1] * s + b[1] * t + c[1] * u, p[2] + a[2] * s + b[2] * t + c[2] * u]; }

  /* o: { canvas, mark, dial, words, as, seed, wind (degrees it comes from), knots, t (the moment to start the sea at) } */
  function start(cast, o) {
    var Tama = root.Tama, Ceremony = K.Ceremony, A = Ceremony.art, P = A.P, PW = K.PlayWorld, Land = K.Land;
    var WB = 1024, world = new Vox.World(WB, 64, WB), pal = new PW.Palette(world), C = WB / 2, MID = C * G;   // the block; its middle is where metres are counted from
    var LEVEL = Land.SEA * G, SB = Land.SEA;                                                                 // the sea's level: land.js's
    function colour(c) {
      if (typeof c !== 'string') return A.shade(colour(c[0]), c[1]);
      var m = /^(gem|sky):(\d)$/.exec(c);
      return m ? (m[1] === 'gem' ? A.GEMS : A.SKY)[+m[2]] : P[c];
    }
    var ids = {};
    function M(name) { var spec = STUFF[name] || Land.stuffOf(name); return ids[name] || (ids[name] = pal.mix(spec.colours.map(colour), spec)); }
    function px(m) { return MID + m * PX; }

    /* ── THE SEA AND ITS WIND: the seed's, unless the page says ── */
    var seed = K.Geo.fnv(String(o.seed || 'wallflowers'));
    var from = o.wind !== null && o.wind !== undefined && o.wind !== '' ? +o.wind : Math.floor(A.hash(seed, 3) * 36) * 10;
    var sea = Ocean.sea({ seed: seed, from: from, speed: (+o.knots || 12.5) * Ocean.KNOT, fetch: FETCH });
    var epoch = o.t !== null && o.t !== undefined && o.t !== '' ? +o.t : Date.now() / 1000 % 86400;           // the same moment on every device that keeps the time

    /* ── THE ISLAND: a low dome of sand and grass, a light on it, a jetty ── */
    var IR = 22;
    function top(bx, bz) {
      var r = Math.hypot(bx - C, bz - C) / IR;
      return r > 1.2 ? 0 : Math.max(0, Math.round(SB - 3 + 6.5 * (1 - r * r) + (A.noise2(bx, bz, 6, 5) - 0.5) * 1.6));
    }
    var JETTY = [C + 13, C + 31, C + 1, C + 4], home = { x: (JETTY[1] + 4 - C) * G / PX, z: (JETTY[2] + 1.5 - C) * G / PX };
    (function island() {
      var x, y, z, t, r;
      for (z = C - 28; z <= C + 28; z++) for (x = C - 28; x <= C + 28; x++) {
        t = top(x, z);
        for (y = 0; y < t; y++) world.set(x, y, z, y === 0 ? M('bed') : y < t - 4 ? M('stone') : t <= SB + 1 ? M('sand') : y < t - 1 ? M('dirt') : M('grass'));
      }
      var y0 = top(C, C);
      for (y = 0; y < 20; y++) for (z = -4; z <= 4; z++) for (x = -4; x <= 4; x++) {
        r = Math.hypot(x, z);
        var stuff = y < 14 ? (r <= 2.4 ? (((y >> 1) & 1) ? 'band' : 'plaster') : null) : y === 14 ? (r <= 3.5 ? 'trim' : null)
                  : y < 18 ? (r <= 1.6 ? 'lit' : null) : r <= (y === 18 ? 2.4 : 1.2) ? 'copper' : null;
        if (stuff) world.set(C + x, y0 + y, C + z, M(stuff));
      }
      for (y = 0; y < 3; y++) world.set(C, y0 + y, C + 2, M('door'));
      [[-8, 5], [7, -7], [-3, -10], [9, 6]].forEach(function (q, k) {
        var tx = C + q[0], tz = C + q[1], base = top(tx, tz), tall = 3 + (k & 1);
        for (y = 0; y < tall; y++) world.set(tx, base + y, tz, M('trunk'));
        for (z = -1; z <= 1; z++) for (x = -1; x <= 1; x++) {
          world.set(tx + x, base + tall, tz + z, M('leaf'));
          if (!x || !z) world.set(tx + x, base + tall + 1, tz + z, M('leaf'));
        }
      });
      for (x = JETTY[0]; x < JETTY[1]; x++) for (z = JETTY[2]; z < JETTY[3]; z++) {
        world.set(x, SB + 1, z, M('timber'));
        if ((x - JETTY[0]) % 5 === 4 && z !== JETTY[2] + 1) for (y = top(x, z); y <= SB + 1; y++) world.set(x, y, z, M('trunk'));
      }
    })();
    /* Too shoal to float in, or the jetty: metres in, yes or no out. */
    function aground(x, z) {
      var bx = Math.floor(C + x * PX / G), bz = Math.floor(C + z * PX / G);
      return top(bx, bz) >= SB - 1 || (bx >= JETTY[0] && bx < JETTY[1] && bz >= JETTY[2] && bz < JETTY[3]);
    }

    var E = En.create(o.canvas, { art: A, Tama: Tama, cast: cast, pal: pal, world: world, grain: G, mark: o.mark });
    var gl = E.gl, cam = E.cam, part = E.part;
    var N = root.matchMedia && root.matchMedia('(pointer: coarse)').matches ? 128 : 192;
    var water = surface(E, N, A.BAYER, M('sea'), M('foam'));

    /* ── THE BOAT, at one voxel to an art pixel like everything that moves ── */
    var who = Object.keys(cast), meId = cast[o.as] ? o.as : who[Math.floor(Math.random() * who.length)];
    var spec = Ocean.DINGHY, HL = Math.round(spec.length * PX), HB = Math.round(spec.beam * PX), HH = 11, FIN = Math.round(spec.draft * PX);
    var BOOM = Math.round(spec.boom * PX), LUFF = 78, MAST = LUFF + 12, WL = 3;                // the waterline is this far up her side
    var mine = A.GEMS[K.Geo.fnv(meId) % A.GEMS.length];                                       // her colour: one of the wordmark's, by who sails her
    function hull() {
      var m = new Vox.World(HL, HH + FIN, HB), x, y, z;
      for (x = 0; x < HL; x++) {
        var t = x / (HL - 1), half = HB / 2 * (t < 0.45 ? 0.8 + 0.2 * t / 0.45 : Math.sqrt(Math.max(0, 1 - Math.pow((t - 0.45) / 0.55, 2))));
        var keel = t > 0.7 ? Math.round((t - 0.7) / 0.3 * 3) : 0;                             // the bow lifts
        for (y = keel; y < HH; y++) for (z = 0; z < HB; z++) {
          var off = Math.abs(z + 0.5 - HB / 2), wide = half * (0.55 + 0.45 * y / (HH - 1));
          if (off > wide) continue;
          var skin = off > wide - 1.5 || y === keel, well = t > 0.08 && t < 0.6 && off < wide - 2.5;
          if (well && y > 4) continue;                                                        // where the crew sit
          m.set(x, FIN + y, z, pal.id(y === HH - 1 ? (skin ? mine : P.dirtHi) : well ? P.boneShade : y < WL ? P.panelHi : y === HH - 2 && skin ? P.boneShade : P.paper));
        }
      }
      for (y = 0; y < FIN; y++) for (x = 0; x < 6; x++) {                                     // the board, and the rudder at her stern
        if (y >= x) m.set(HL / 2 + 2 + x, y, HB / 2, pal.id(P.boneShade));
        if (y > FIN - 11 && x < 4) m.set(x, y, HB / 2, pal.id(P.chest));
      }
      return part({ mesh: Vox.meshAll(m) });
    }
    function mast() {
      var m = new Vox.World(2, MAST, 2), x, y, z;
      for (y = 0; y < MAST; y++) for (z = 0; z < 2; z++) for (x = 0; x < 2; x++) m.set(x, y, z, pal.id(y % 13 === 12 ? P.caneNode : x + z ? P.cane : P.caneLight));
      return part({ mesh: Vox.meshAll(m) });
    }
    function sail() {
      var m = new Vox.World(BOOM + 1, LUFF, 1), x, y;
      for (y = 0; y < LUFF; y++) {
        var chord = y < 2 ? BOOM : Math.round(BOOM * Math.pow(1 - y / LUFF, 0.85));
        for (x = 0; x <= chord; x++)
          m.set(x, y, 0, pal.id(y < 2 ? P.cane : y > LUFF * 0.7 ? (x === chord ? A.shade(mine, -0.25) : mine) : y % 13 === 0 || x === chord ? P.boneShade : x === 0 ? P.bone : P.paper));
      }
      return part({ mesh: Vox.meshAll(m) });
    }
    function flag() {
      var m = new Vox.World(10, 5, 1), x, y;
      for (y = 0; y < 5; y++) for (x = 0; x < 10 - Math.abs(y - 2) * 3; x++) m.set(x, y, 0, pal.id(x < 2 ? P.petalLight : P.berry));
      return part({ mesh: Vox.meshAll(m) });
    }
    function buoy(n) {
      var m = new Vox.World(12, 26, 12), x, y, z, c = A.GEMS[[3, 1, 5][n % 3]];
      for (y = 0; y < 26; y++) for (z = 0; z < 12; z++) for (x = 0; x < 12; x++) {
        var r = Math.hypot(x - 5.5, z - 5.5);
        if (y < 11 ? r <= 5.6 - (y < 2 ? 2 - y : 0) : y < 22 ? r <= 1 : r <= 2.3)
          m.set(x, y, z, pal.id(y < 4 ? P.panelHi : y < 11 ? ((y - 4) % 4 < 2 ? c : P.paper) : y < 22 ? P.caneDry : P.petalLight));
      }
      return part({ mesh: Vox.meshAll(m) });
    }
    var hullM = hull(), mastM = mast(), sailM = sail(), flagM = flag();

    /* ── THE COURSE: laid to the wind, from the island ── */
    var marks = Ocean.course(sea.from, [[0, 150], [125, 110]]).map(function (m, i) { m.gpu = buoy(i); m.name = 'Mark ' + (i + 1); return m; });
    marks.push({ x: home.x, z: home.z, gpu: buoy(2), name: 'Home' });
    var LEGS = ['First mark: dead upwind. Nothing sails straight at the wind, so zigzag.',
                'Second mark: turn away from the wind and let the sail out.',
                'Now home, to the mark off the jetty.'];
    var leg = 0, took = 0, said = '', until = 0, stuck = 0, flap = 0;
    function say(text, secs) { if (secs) until = E.clock() + secs; if (text !== said) o.words(said = text); }

    /* she starts off the jetty, the wind on her beam, heading for open water */
    var east = Math.sin(sea.from + Math.PI / 2) >= 0 ? 90 : -90;
    var boat = Ocean.boat({ x: home.x + 8, z: home.z + 9, hdg: sea.from / DEG + east, sheet: 0.42 }), helm = { rudder: 0, sheet: 0.42, fly: false };
    boat.vx = Math.sin(boat.hdg) * 2; boat.vz = -Math.cos(boat.hdg) * 2;
    var me = E.person(meId, 0, 0, 0), hands = { mx: 0, mz: 0, jump: false }, T = {}, trail = [], laid = 0;
    me.ground = true;

    var Q = PW.parts(Ceremony, Tama, pal), clouds = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map(function (n) {
      var c = part(Q.cloud(n));
      c.x = (A.hash(n, 33) - 0.5) * 3600; c.z = (A.hash(n, 36) - 0.5) * 3600; c.top = LEVEL + 300 + c.y * 260;
      return c;
    });

    /* How she lies: forward, up and to starboard, as she has them. */
    function lie(hdg, pitch, heel) {
      var sh = Math.sin(hdg), ch = Math.cos(hdg), sp = Math.sin(pitch), cp = Math.cos(pitch), sr = Math.sin(heel), cr = Math.cos(heel);
      var f = [sh * cp, sp, -ch * cp], u = [-sh * sp, cp, ch * sp], s = [ch, 0, sh];
      return { f: f, u: [u[0] * cr + s[0] * sr, u[1] * cr, u[2] * cr + s[2] * sr], s: [s[0] * cr - u[0] * sr, -u[1] * sr, s[2] * cr - u[2] * sr] };
    }
    function where() { return [px(boat.x), LEVEL + boat.y * PX, px(boat.z)]; }

    /* ── THE DIAL: she is bow up; the wind comes in from where she feels it; the dot is the mark ── */
    var ap = 2, drawn = -1;
    function dial() {
      var w = 134, h = 58, art = A.art(w, h), cx = 29, cy = 29, R = 22, i, a;
      function at(ang, r) { return [Math.round(cx + Math.sin(ang) * r), Math.round(cy - Math.cos(ang) * r)]; }
      A.panelBox(art, 0, 0, w, h, false);
      for (i = 0; i < 96; i++) { a = at(i / 96 * TAU, R); art.put(a[0], a[1], i % 12 ? P.panelLine : P.panelHi); }
      ['..x..', '.xxx.', '.xxx.', 'xxxxx', 'xxxxx', 'xxxxx', '.xxx.'].forEach(function (row, j) {
        for (i = 0; i < 5; i++) if (row[i] === 'x') art.put(cx - 2 + i, cy - 3 + j, P.panelText);
      });
      a = at(-boat.hdg, R - 5); A.putText(art, 'N', a[0] - 2, a[1] - 3, 1, P.panelText, P.panelShade);
      for (i = 6; i < R - 1; i++) { a = at(boat.awa, i); art.put(a[0], a[1], P.panelArrow); }                 // the wind, as she feels it
      [-0.35, 0.35].forEach(function (d) { for (i = 0; i < 5; i++) { a = at(boat.awa + d * i / 5, 6 + i); art.put(a[0], a[1], P.panelArrow); } });
      var c = A.GEMS[[3, 1, 5][leg % 3]];
      a = at(T.off || 0, R);
      for (i = 0; i < 9; i++) art.put(a[0] - 1 + i % 3, a[1] - 1 + (i / 3 | 0), leg < marks.length ? c : P.panelLine);
      function num(v, n) { return ('000' + Math.round(v)).slice(-n); }
      [['HDG', num(boat.hdg / DEG % 360, 3)], ['KN', (Math.max(0, boat.speed) / Ocean.KNOT).toFixed(1)],
       ['WIND', num((boat.from / DEG % 360 + 360) % 360, 3) + ' ' + Math.round(boat.tws / Ocean.KNOT)],
       leg < marks.length ? ['MARK', num((T.bearing || 0) / DEG % 360, 3) + ' ' + Math.round(T.range || 0)] : ['HOME', clock(took)]
      ].forEach(function (row, j) {
        A.putText(art, row[0], 58, 8 + j * 11, 1, P.panelArrowOff, null);
        A.putText(art, row[1], w - 7 - A.textW(row[1], 1), 8 + j * 11, 1, P.panelText, P.panelShade);
      });
      ap = root.innerWidth >= 900 ? 3 : 2;
      o.dial.width = w; o.dial.height = h; o.dial.getContext('2d').drawImage(art.c, 0, 0);
      o.dial.style.width = w * ap + 'px'; o.dial.style.height = h * ap + 'px';
      o.dial.setAttribute('aria-label', 'Heading ' + num(boat.hdg / DEG % 360, 3) + ', ' + (Math.max(0, boat.speed) / Ocean.KNOT).toFixed(1) + ' knots, ' + Ocean.point(boat.twa));
    }
    function clock(s) { return Math.floor(s / 60) + ':' + ('0' + Math.floor(s % 60)).slice(-2); }

    var view = { yaw: -boat.hdg, set: 0, look: -0.6, loc: null, prog: null };              // from off her quarter, so her side and her sail show
    cam.pitch = 0.2; cam.yaw = view.set = view.yaw + view.look;
    var p0 = where();
    cam.tx = p0[0]; cam.ty = p0[1] + 26; cam.tz = p0[2];
    say('Left and right steer. Up hauls the sail in, down lets it out.', 9);

    E.run({
      update: function (dt) {
        var t = epoch + E.clock(), px0 = boat.x, pz0 = boat.z;
        E.control(hands);                                                     // the engine's hands, turned back from the view's way to the stick's
        var s = Math.sin(cam.yaw), co = Math.cos(cam.yaw), ix = co * hands.mx - s * hands.mz, iy = -s * hands.mx - co * hands.mz;
        helm.rudder = ix; helm.sheet = clamp(helm.sheet - iy * 0.55 * dt, 0, 1); helm.fly = hands.jump;
        Ocean.step(boat, sea, helm, dt, t);
        var f = [Math.sin(boat.hdg), -Math.cos(boat.hdg)], bow = spec.length * 0.45;
        if (aground(boat.x + f[0] * bow, boat.z + f[1] * bow) || aground(boat.x, boat.z) || aground(boat.x - f[0] * bow, boat.z - f[1] * bow)) {
          boat.x = px0; boat.z = pz0; boat.vx *= -0.3; boat.vz *= -0.3; boat.r *= 0.5;       // she touches, and comes off
        }

        /* the course */
        if (leg < marks.length) {
          took += dt;
          Ocean.to(boat, marks[leg].x, marks[leg].z, T);
          if (T.range < 9) {
            leg++; E.feel(me, leg < marks.length ? 'happy' : 'laugh', leg < marks.length ? 1.5 : 5);
            say(leg < marks.length ? LEGS[leg] : 'Home, in ' + clock(took) + '.', 8);
          }
        }
        /* what she is telling you, when it is worth saying */
        stuck = Math.abs(boat.twa) < 32 * DEG && boat.speed < 0.6 ? stuck + dt : 0;
        flap = boat.luff && Math.abs(boat.twa) >= 32 * DEG && boat.speed < 1.5 ? flap + dt : 0;
        if (boat.over > 0) say('Over she goes. In a puff, let the sail fly.', 6);
        else if (stuck > 3) say('Head to wind she will not sail. Hold the tiller over and wait.', 5);
        else if (flap > 3) say('The sail is flapping. Haul it in, or turn away from the wind.', 5);
        else if (E.clock() > until && leg < marks.length) say(LEGS[leg]);
        if (!(me.moodT > 0)) me.mood = boat.over > 0 ? 'scared' : Math.abs(boat.heel) > 0.5 ? 'surprised' : boat.speed > 3.2 ? 'happy' : null;

        /* her wake: where she has been, for a while */
        laid += dt;
        if (laid > 0.2) { laid = 0; trail.push([boat.x - f[0] * 1.9, boat.z - f[1] * 1.9, E.clock(), clamp(Math.abs(boat.speed) / 3, 0, 1)]); if (trail.length > 45) trail.shift(); }

        /* the view rides behind her; whatever you have dragged it round by, it keeps */
        view.look += cam.yaw - view.set;
        view.yaw += Ocean.turn(view.yaw, -boat.hdg) * (1 - Math.exp(-dt * 2.2));
        cam.yaw = view.set = view.yaw + view.look;
        var p = where(), k = 1 - Math.exp(-dt * 6);
        cam.dist = root.innerWidth < root.innerHeight ? 560 : 460;
        cam.tx = lerp(cam.tx, p[0], k); cam.tz = lerp(cam.tz, p[2], k); cam.ty = lerp(cam.ty, LEVEL + 26, k);

        var wx = -Math.sin(sea.from) * sea.speed * PX * 0.3, wz = Math.cos(sea.from) * sea.speed * PX * 0.3;      // clouds go where the wind goes
        clouds.forEach(function (c) {
          c.x += wx * dt; c.z += wz * dt;
          if (c.x - p[0] > 1800) c.x -= 3600; else if (c.x - p[0] < -1800) c.x += 3600;
          if (c.z - p[2] > 1800) c.z -= 3600; else if (c.z - p[2] < -1800) c.z += 3600;
        });
      },
      parts: function (model) {
        /* engine.js keeps the view to itself, so it is asked of the program the
           block was just drawn with. One read a frame; the engine handing its
           view to `parts` would be the better arrangement. */
        var prog = gl.getParameter(gl.CURRENT_PROGRAM), now = E.clock(), t = epoch + now;
        if (view.prog !== prog) { view.prog = prog; view.loc = gl.getUniformLocation(prog, 'uVP'); }
        var VP = gl.getUniform(prog, view.loc);

        var p = where(), L = lie(boat.hdg, boat.pitch, boat.heel), deck = along(p, L.u, HH - WL, L.f, 0, L.s, 0);
        var foot = along(deck, L.f, spec.mast * PX, L.u, 0, L.s, 0), swing = boat.boom + (boat.luff ? Math.sin(Math.floor(now * 14) * 2.4) * 0.12 : 0);
        var cb = Math.cos(swing), sb = Math.sin(swing), boom = along([0, 0, 0], L.f, -cb, L.s, sb, L.u, 0), face = along([0, 0, 0], L.s, -cb, L.f, -sb, L.u, 0);
        model(hullM, pose(p, L.f, L.u, L.s, [HL / 2, FIN + WL, HB / 2]));
        model(mastM, pose(foot, L.f, L.u, L.s, [1, 0, 1]));
        model(sailM, pose(along(foot, L.u, 9, L.f, 0, L.s, 0), boom, L.u, face, [-1, 0, 0.5]));
        var away = boat.hdg + boat.awa + Math.PI, fx = [Math.sin(away), 0, -Math.cos(away)];                    // the flag streams down the wind she feels
        model(flagM, pose(along(foot, L.u, MAST, L.f, 0, L.s, 0), fx, [0, 1, 0], [-fx[2], 0, fx[0]], [0, 0, 0.5]));
        /* the crew, in the well, up on whichever side holds her up */
        var seat = along(p, L.f, -10, L.s, boat.hike * (HB / 2 - 3), L.u, 5 - WL);
        me.x = seat[0]; me.y = seat[1]; me.z = seat[2]; me.hx = L.f[0]; me.hz = L.f[2]; me.hidden = boat.over > 0;

        var S = {};
        marks.forEach(function (m) {
          sea.surface(m.x, m.z, t, S, 1);
          var up = [-S.sx, 1, -S.sz], n = Math.hypot(up[0], up[1], up[2]), X = [1, S.sx, 0], xn = Math.hypot(1, S.sx);
          up = [up[0] / n, up[1] / n, up[2] / n]; X = [X[0] / xn, X[1] / xn, 0];
          model(m.gpu, pose([px(m.x), LEVEL + S.y * PX, px(m.z)], X, up, [X[1] * up[2] - X[2] * up[1], X[2] * up[0] - X[0] * up[2], X[0] * up[1] - X[1] * up[0]], [6, 4, 6]));
        });
        clouds.forEach(function (c) { model(c, En.place([c.x, c.top, c.z])); });

        /* the sea itself, last: a square of it round a point ahead of the view */
        var at = [Math.floor((cam.tx - Math.sin(cam.yaw) * AHEAD) / G) - N / 2, Math.floor((cam.tz - Math.cos(cam.yaw) * AHEAD) / G) - N / 2];
        sea.sample(water.data, N, G / PX, (at[0] * G - MID) / PX, (at[1] * G - MID) / PX, t);
        var ci = px(boat.x) / G - at[0], cj = px(boat.z) / G - at[1], dry = clamp(128 + Math.round(boat.y * PX) + 1, 0, 255), di, dj;
        for (dj = -5; dj <= 5; dj++) for (di = -5; di <= 5; di++) {                                          // the sea does not stand inside a boat
          var i2 = Math.floor(ci) + di, j2 = Math.floor(cj) + dj, ox = (i2 + 0.5 - ci) * G, oz = (j2 + 0.5 - cj) * G;
          if (i2 < 0 || j2 < 0 || i2 >= N || j2 >= N || Math.abs(ox * L.f[0] + oz * L.f[2]) > HL / 2 || Math.abs(ox * L.s[0] + oz * L.s[2]) > HB / 2) continue;
          var q2 = (j2 * N + i2) * 4;
          if (water.data[q2] > dry) water.data[q2] = dry;
          water.data[q2 + 1] = 0;
        }
        trail.forEach(function (w) {
          var i = Math.floor(px(w[0]) / G) - at[0], j = Math.floor(px(w[1]) / G) - at[1], age = (now - w[2]) / 9, v = Math.round(255 * w[3] * (1 - age));
          if (age >= 1 || i < 1 || j < 1 || i >= N - 1 || j >= N - 1) return;
          for (var dj = -1; dj <= 1; dj++) for (var di = -1; di <= 1; di++) { var q = ((j + dj) * N + i + di) * 4 + 3, e = di && dj ? v >> 1 : v; if (water.data[q] < e) water.data[q] = e; }
        });
        water.draw(VP, at, LEVEL, sea.hs * PX / 2, now);
        gl.useProgram(prog);

        if (o.dial && Math.floor(now * 8) !== drawn) { drawn = Math.floor(now * 8); dial(); }
      }
    });

    return {
      input: E.input, cam: cam, stop: E.stop,
      /* Read-only, for whoever is watching. */
      state: function () {
        return { as: me.id, at: [boat.x, boat.z], hdg: boat.hdg / DEG, speed: boat.speed, heel: boat.heel / DEG, boom: boat.boom / DEG, sheet: helm.sheet,
                 awa: boat.awa / DEG, twa: boat.twa / DEG, point: Ocean.point(boat.twa), luff: boat.luff, over: boat.over > 0,
                 wind: { from: boat.from / DEG, knots: boat.tws / Ocean.KNOT, mean: sea.from / DEG }, sea: { hs: sea.hs, tp: sea.tp, seed: sea.seed },
                 leg: leg, mark: leg < marks.length ? { name: marks[leg].name, bearing: T.bearing / DEG, range: T.range } : null, took: took,
                 marks: marks.map(function (m) { return [m.x, m.z]; }), words: said, frames: E.frames(), colours: pal.n, chunks: E.chunks(), cells: N * N };
      }
    };
  }

  K.Sail = { start: start };
  if (root.document) root.addEventListener('DOMContentLoaded', function () {
    var doc = root.document, q = new root.URLSearchParams(root.location.search);
    function $(id) { return doc.getElementById(id); }
    if (doc.body.getAttribute('data-play') !== 'sail') return;
    K.Hud.cast().then(function (cast) {
      K.Sail.now = start(cast, { canvas: $('world'), mark: $('mark'), dial: $('dial'), words: K.Hud.words,
                                 as: q.get('as'), seed: q.get('seed'), wind: q.get('wind'), knots: q.get('knots'), t: q.get('t') });
      K.Hud.bind(K.Sail.now);
    });
  });
})(typeof self !== 'undefined' ? self : this);
