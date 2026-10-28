/* engine.js — what /play and /map share: the GPU, how a body moves, the view,
 * and the hands on the controls. A scene (play.js, roam.js) brings the block,
 * the parts and the story.
 *
 * ONE ART PIXEL THROUGHOUT. A block of voxels is drawn at a GRAIN: one art
 * pixel to a voxel on the island (world.js), eight to a voxel on a generated
 * land (land.js) — Terraria's blocks. A sprite pixel is an art pixel in both,
 * so the cast stand in either at the same size.
 *
 * WHAT A BLOCK'S FACE LOOKS LIKE IS ITS STUFF. A palette id is four colours and
 * a way of laying them: scattered, in courses like brick, in boards, as a
 * window, as water, in stripes, as a road's centre line, as paving. At a grain
 * of one there is one art pixel to a face and all of that comes to a single
 * colour. No gradients: flat shades, one to a side, the light where
 * ceremony.js has it.
 *
 * SO THAT A LAND CAN BE SEEN. Three things a grain above one turns on. A
 * pattern is only drawn where an art pixel is at least most of a screen pixel:
 * further off, a face is its plain colour, which is what stops a distant wall
 * from crawling. The sun casts: a face is in shade where something taller
 * stands between it and the light (a height to a column, marched in the
 * shader). And what stands between you and the view is cut away, as The Sims
 * takes its walls down: a cone from the eye to you, dithered at its edge.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin = root.Kenjin || {}, Vox = root.Vox;

  /* ── EASING ── */
  function clamp(x, a, b) { return x < a ? a : x > b ? b : x; }
  function lerp(a, b, t) { return a + (b - a) * t; }
  function outCubic(x) { return 1 - Math.pow(1 - x, 3); }
  function inOut(x) { return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; }
  function outBack(x) { var c1 = 1.3, c3 = c1 + 1; return 1 + c3 * Math.pow(x - 1, 3) + c1 * Math.pow(x - 1, 2); }
  function hump(x) { return x <= 0 || x >= 1 ? 0 : Math.sin(x * Math.PI); }

  /* ── MATRICES: column-major, as WebGL takes them ── */
  function mul(a, b) {
    var o = new Float32Array(16);
    for (var c = 0; c < 4; c++) for (var r = 0; r < 4; r++)
      o[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
    return o;
  }
  function perspective(fovy, aspect, near, far) {
    var f = 1 / Math.tan(fovy / 2), o = new Float32Array(16);
    o[0] = f / aspect; o[5] = f; o[10] = (far + near) / (near - far); o[11] = -1; o[14] = 2 * far * near / (near - far);
    return o;
  }
  function lookAt(e, t) {
    var zx = e[0] - t[0], zy = e[1] - t[1], zz = e[2] - t[2], l = Math.hypot(zx, zy, zz);
    zx /= l; zy /= l; zz /= l;
    var xx = zz, xz = -zx, xl = Math.hypot(xx, xz);                 // up is +y
    xx /= xl; xz /= xl;
    var yx = zy * xz, yy = zz * xx - zx * xz, yz = -zy * xx;
    return new Float32Array([xx, yx, zx, 0, 0, yy, zy, 0, xz, yz, zz, 0,
      -(xx * e[0] + xz * e[2]), -(yx * e[0] + yy * e[1] + yz * e[2]), -(zx * e[0] + zy * e[1] + zz * e[2]), 1]);
  }
  /* A part's place: at `p`, turned `yaw` about the upright, rolled about its own
     length, and held by its `pivot`. */
  function place(p, yaw, roll, pivot) {
    var c = Math.cos(yaw || 0), s = Math.sin(yaw || 0), cr = Math.cos(roll || 0), sr = Math.sin(roll || 0);
    var m = new Float32Array([c, 0, -s, 0, s * sr, cr, c * sr, 0, s * cr, -sr, c * cr, 0, 0, 0, 0, 1]);
    var v = pivot || [0, 0, 0];
    m[12] = p[0] - (m[0] * v[0] + m[4] * v[1] + m[8] * v[2]);
    m[13] = p[1] - (m[1] * v[0] + m[5] * v[1] + m[9] * v[2]);
    m[14] = p[2] - (m[2] * v[0] + m[6] * v[1] + m[10] * v[2]);
    return m;
  }

  /* ── THE GPU ── */
  var VS_VOX = '#version 300 es\n' +
    'layout(location=0) in uvec3 aPos; layout(location=1) in uvec3 aAux;\n' +
    'uniform mat4 uVP, uModel; flat out int vId; flat out int vN; out vec3 vWorld; out vec3 vLocal;\n' +
    'void main(){ vec4 w = uModel * vec4(vec3(aPos), 1.0); vWorld = w.xyz; vLocal = vec3(aPos);\n' +
    '  vId = int(aAux.x); vN = int(aAux.y); gl_Position = uVP * w; }';
  var FS_VOX = '#version 300 es\nprecision highp float; precision highp int;\n' +
    'uniform sampler2D uPal; uniform sampler2D uTop; uniform float uClipY; uniform float uGrain; uniform float uTime;\n' +
    'uniform vec3 uSun; uniform float uSize; uniform vec3 uEye; uniform vec3 uFocus; uniform float uCut;\n' +
    'flat in int vId; flat in int vN; in vec3 vWorld; in vec3 vLocal; out vec4 o;\n' +
    'const float SH[6] = float[6](0.84, 0.74, 0.5, 1.0, 0.62, 0.92);\n' +
    'const vec3 NV[6] = vec3[6](vec3(-1,0,0), vec3(1,0,0), vec3(0,-1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1));\n' +
    'float hash(vec3 p){ p = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419)); p *= 17.0; return fract(p.x * p.y * p.z * (p.x + p.y + p.z)); }\n' +
    'vec4 pal(int k){ return texelFetch(uPal, ivec2(vId & 255, (vId >> 8) + 16 * k), 0); }\n' +
    'void main(){ if (vWorld.y > uClipY) discard;\n' +
    '  if (uCut > 0.0 && uGrain > 1.0 && vWorld.y > uFocus.y - 4.0) {\n' +                                 // what stands between the eye and you
    '    vec3 ef = uFocus - uEye; float t = dot(vWorld - uEye, ef) / dot(ef, ef);\n' +
    '    if (t > 0.0 && t < 0.97) { float d = length(vWorld - (uEye + ef * t)), r = uCut * (0.3 + 0.7 * t);\n' +
    '      ivec2 fc = ivec2(gl_FragCoord.xy); if (d < r || (d < r * 1.3 && ((fc.x + fc.y) & 1) == 0)) discard; } }\n' +
    '  vec3 q = (vLocal - NV[vN] * (0.5 / uGrain)) * uGrain; ivec3 a = ivec3(floor(q)); int k = 0;\n' +   // the art pixel under this fragment
    '  float sun = 1.0;\n' +
    '  if (uGrain > 1.0) {\n' +
    '    vec3 fw = fwidth(q);\n' +
    '    if (max(fw.x, max(fw.y, fw.z)) < 1.5) {\n' +                                                        // near enough to tell one art pixel from the next
    '      int pat = int(pal(1).a * 255.0 + 0.5); ivec2 uv = vN < 2 ? ivec2(a.z, a.y) : vN < 4 ? ivec2(a.x, a.z) : ivec2(a.x, a.y);\n' +
    '      float h = hash(vec3(a >> 1)); int few = h < 0.8 ? 0 : h < 0.9 ? 1 : h < 0.96 ? 2 : 3;\n' +          // mostly itself, flecked in twos
    '      if (pat == 0) k = few;\n' +
    '      else if (pat == 1) { int row = uv.y >> 2; int col = uv.x + (row & 1) * 4;\n' +                    // courses, four pixels high, joints staggered
    '        k = (uv.y & 3) == 0 || (col & 7) == 0 ? 3 : int(hash(vec3(float(row), float(col >> 3), 7.0)) * 3.0); }\n' +
    '      else if (pat == 2) k = (uv.x & 3) == 0 ? 3 : int(hash(vec3(float(uv.x >> 2), 3.0, 1.0)) * 3.0);\n' +   // boards
    '      else if (pat == 3) { ivec2 p = uv & 7; k = p.y == 0 ? 3 : p.x == 0 || p.x == 7 || p.y == 7 || p.x == 4 ? 2 : (p.x + p.y == 9 || p.x + p.y == 10) && p.y > 3 ? 1 : 0; }\n' +   // a window
    '      else if (pat == 5) k = int(mod(float(uv.y) + floor(uTime * 1.5) + floor(hash(vec3(float(uv.x >> 2), 0.0, 1.0)) * 4.0), 4.0));\n' +   // water, in whole steps
    '      else if (pat == 6) k = ((uv.x >> 1) & 1) == 0 ? 0 : 1;\n' +                                        // stripes
    '      else if (pat == 7 || pat == 8) { ivec2 p = (pat == 7 ? uv : uv.yx) & 7; k = vN == 3 && (p.y == 3 || p.y == 4) && p.x < 5 ? 1 : few == 1 ? 0 : few; }\n' +   // a road, its centre line dashed along it
    '      else if (pat == 9) { ivec2 p = uv & 7; k = p.x == 0 || p.y == 0 ? 3 : few == 3 ? 0 : few; }\n' +   // paving: a joint round every slab
    '    }\n' +
    '    if (uSize > 0.0 && dot(NV[vN], uSun) > 0.0) {\n' +                                                  // is something taller between here and the sun
    '      vec3 p = vLocal + NV[vN] * 0.03;\n' +
    '      for (int i = 1; i <= 14; i++) { vec3 w = p + uSun * (float(i) * 0.9);\n' +
    '        if (w.y >= 63.0 || w.x < 0.0 || w.z < 0.0 || w.x >= uSize || w.z >= uSize) break;\n' +
    '        if (texelFetch(uTop, ivec2(w.xz), 0).r * 255.0 > w.y + 0.02) { sun = 0.72; break; } }\n' +
    '    }\n' +
    '  }\n' +
    '  vec4 c = pal(k); float f = SH[vN] * sun; float amt = (1.0 - pal(0).a) * 0.5;\n' +
    '  if (amt > 0.0) f *= 1.0 + (hash(floor(q / 8.0)) - 0.5) * amt;\n' +
    '  o = vec4(c.rgb * f, 1.0); }';
  var VS_SKY = '#version 300 es\nvoid main(){ vec2 p = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0); gl_Position = vec4(p, 1.0, 1.0); }';
  /* ceremony.js's sky: its six blues, dithered by its Bayer matrix, and the
     warm ring it puts behind the page's mark. */
  var FS_SKY = '#version 300 es\nprecision highp float;\n' +
    'uniform vec2 uRes; uniform float uPx; uniform vec3 uSky[6]; uniform vec3 uSun; uniform float uB[16]; out vec4 o;\n' +
    'void main(){ vec2 q = floor(vec2(gl_FragCoord.x, uRes.y - gl_FragCoord.y) / uPx);\n' +
    '  float th = (uB[int(mod(q.y, 4.0)) * 4 + int(mod(q.x, 4.0))] + 0.5) / 16.0;\n' +
    '  float f = clamp(q.y * uPx / uRes.y, 0.0, 1.0) * 5.0; int bi = int(floor(f));\n' +
    '  vec3 c = uSky[min(5, bi + (fract(f) > th ? 1 : 0))];\n' +
    '  if (uSun.z > 0.0) { float halo = max(4.0 * uPx, uSun.z * 0.8); float g = (distance(q * uPx, uSun.xy) - uSun.z) / halo;\n' +
    '    if (g > -0.2 && g < 1.0 && (1.0 - g) > th) c = vec3(1.0, 0.957, 0.784); }\n' +
    '  o = vec4(c, 1.0); }';
  var VS_SPR = '#version 300 es\nlayout(location=0) in vec2 aQ;\n' +
    'uniform mat4 uVP; uniform vec3 uPos; uniform vec3 uRight; uniform vec2 uSize; uniform vec4 uCell; out vec2 vUv;\n' +
    'void main(){ vec3 w = uPos + uRight * (aQ.x * uSize.x) + vec3(0.0, aQ.y * uSize.y, 0.0);\n' +
    '  vUv = uCell.xy + vec2(aQ.x + 0.5, 1.0 - aQ.y) * uCell.zw; gl_Position = uVP * vec4(w, 1.0); }';
  var FS_SPR = '#version 300 es\nprecision highp float; uniform sampler2D uTex; in vec2 vUv; out vec4 o;\n' +
    'void main(){ vec4 c = texture(uTex, vUv); if (c.a < 0.5) discard; o = vec4(c.rgb, 1.0); }';
  /* A shadow is every other art pixel of the ground under you, darkened. */
  var VS_SHD = '#version 300 es\nlayout(location=0) in vec2 aQ;\n' +
    'uniform mat4 uVP; uniform vec3 uPos; uniform vec2 uR; out vec2 vQ; out vec3 vW;\n' +
    'void main(){ vQ = vec2(aQ.x * 2.0, aQ.y * 2.0 - 1.0); vW = uPos + vec3(vQ.x * uR.x, 0.0, vQ.y * uR.y); gl_Position = uVP * vec4(vW, 1.0); }';
  var FS_SHD = '#version 300 es\nprecision highp float; in vec2 vQ; in vec3 vW; out vec4 o;\n' +
    'void main(){ if (dot(vQ, vQ) > 1.0) discard; ivec2 c = ivec2(floor(vW.xz)); if (((c.x + c.y) & 1) == 0) discard; o = vec4(0.16, 0.30, 0.10, 1.0); }';

  function program(gl, vs, fs) {
    function sh(type, src) {
      var s = gl.createShader(type);
      gl.shaderSource(s, src); gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error('engine: ' + gl.getShaderInfoLog(s));
      return s;
    }
    var p = gl.createProgram();
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vs)); gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error('engine: ' + gl.getProgramInfoLog(p));
    var u = {}, n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
    for (var i = 0; i < n; i++) { var name = gl.getActiveUniform(p, i).name.replace(/\[0\]$/, ''); u[name] = gl.getUniformLocation(p, name); }
    return { p: p, u: u };
  }
  function Mesh(gl, m) {
    this.gl = gl; this.count = m.i.length;
    this.vao = gl.createVertexArray(); gl.bindVertexArray(this.vao);
    this.vbo = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo); gl.bufferData(gl.ARRAY_BUFFER, m.v, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribIPointer(0, 3, gl.UNSIGNED_SHORT, 12, 0);
    gl.enableVertexAttribArray(1); gl.vertexAttribIPointer(1, 3, gl.UNSIGNED_SHORT, 12, 6);
    this.ibo = gl.createBuffer(); gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.ibo); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, m.i, gl.STATIC_DRAW);
    gl.bindVertexArray(null);
  }
  Mesh.prototype.draw = function (count) {
    var n = count === undefined ? this.count : Math.min(this.count, count);
    if (n <= 0) return;
    this.gl.bindVertexArray(this.vao); this.gl.drawElements(this.gl.TRIANGLES, n, this.gl.UNSIGNED_INT, 0);
  };
  Mesh.prototype.free = function () {
    this.gl.deleteBuffer(this.vbo); this.gl.deleteBuffer(this.ibo); this.gl.deleteVertexArray(this.vao);
  };

  /* ── HOW A BODY MOVES, in art pixels and seconds ── */
  var GRAV = 640, WALK = 78, RUN = 112, JUMP = 196;
  var HW = 6, HD = 4, BH = 26;                // a body: half its width, half its depth, its height

  /* o: { art: ceremony.js's, Tama, cast, pal, world, grain, mark } */
  function create(canvas, o) {
    var Tama = o.Tama, A = o.art, world = o.world, g = o.grain || 1;
    var gl = canvas.getContext('webgl2', { antialias: !!o.smooth, alpha: false });       // smooth: a scene that is not pixel art
    if (!gl) throw new Error('engine: no WebGL2');
    var pVox = program(gl, VS_VOX, FS_VOX), pSky = program(gl, VS_SKY, FS_SKY), pSpr = program(gl, VS_SPR, FS_SPR), pShd = program(gl, VS_SHD, FS_SHD);
    var IDENT = place([0, 0, 0]), GROWN = new Float32Array([g, 0, 0, 0, 0, g, 0, 0, 0, 0, g, 0, 0, 0, 0, 1]);
    var E = { gl: gl, grain: g, step: o.step || 0, people: [], solids: [], onHead: null, cut: null,       // cut: the point the view is cleared to, or null
              cam: { yaw: 0, pitch: 0.2, dist: 300, zoom: 1, fov: 0.62, tx: 0, ty: 0, tz: 0 }, input: { x: 0, y: 0, jump: false } };

    /* the palette, as a texture the fragment shader looks ids up in */
    var palTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, palTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    function palette() {
      if (!o.pal.changed) return;
      gl.bindTexture(gl.TEXTURE_2D, palTex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 256, 64, 0, gl.RGBA, gl.UNSIGNED_BYTE, o.pal.data);
      o.pal.changed = false;
    }
    /* the block, a mesh to a chunk, remade when a voxel in it changes */
    var chunks = new Array(world.chunks.length);
    function remesh() {
      for (var cy = 0; cy < world.ny; cy++) for (var cz = 0; cz < world.nz; cz++) for (var cx = 0; cx < world.nx; cx++) {
        var i = world.index(cx, cy, cz);
        if (!world.dirty[i]) continue;
        world.dirty[i] = 0;
        if (chunks[i]) { chunks[i].free(); chunks[i] = null; }
        var m = Vox.meshChunk(world, cx, cy, cz);
        if (m) chunks[i] = new Mesh(gl, m);
      }
    }
    E.part = function (m) { m.gpu = new Mesh(gl, m.mesh); return m; };
    /* How high each column of the block stands, for the sun to cast by. */
    var topTex = gl.createTexture(), topSize = 0, SUN = [-0.45, 0.62, 0.5].map(function (v) { return v / Math.hypot(0.45, 0.62, 0.5); });
    E.sun = function (tops, size) {
      gl.bindTexture(gl.TEXTURE_2D, topTex);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, size, size, 0, gl.RED, gl.UNSIGNED_BYTE, tops);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      topSize = size;
    };
    E.sun(new Uint8Array(1), 1); topSize = 0;        // something for the sampler to hold until a land gives its heights
    E.bindTops = function (at) { gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, topTex); gl.uniform1i(at, 1); gl.activeTexture(gl.TEXTURE0); };

    /* ── THE CAST: tama.js's sprites, an atlas cell to each pose as it is first needed ── */
    var atlas = gl.createTexture(), cells = {}, cellN = 0;
    gl.bindTexture(gl.TEXTURE_2D, atlas);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1024, 1024, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    function cell(c, emotion, facing, frame) {
      var key = c.id + '|' + emotion + '|' + facing + '|' + frame, n = cells[key];
      if (n !== undefined) return n;
      n = cells[key] = cellN++ % 1024;
      var L = Tama.renderLayers(c.spec, { emotion: emotion, facing: facing, frame: frame });
      gl.bindTexture(gl.TEXTURE_2D, atlas);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, (n & 31) * 32, (n >> 5) * 32, 32, 32, gl.RGBA, gl.UNSIGNED_BYTE, Tama.rgbaColour(L, c.spec));
      return n;
    }
    var quad = gl.createVertexArray(), quadBuf = gl.createBuffer();
    gl.bindVertexArray(quad); gl.bindBuffer(gl.ARRAY_BUFFER, quadBuf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-0.5, 0, 0.5, 0, 0.5, 1, -0.5, 0, 0.5, 1, -0.5, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);

    E.person = function (id, x, y, z) {
      var spec = Tama.compile(o.cast[id]), rest = spec.personality ? spec.personality.rest : 'neutral';
      var L = Tama.renderLayers(spec, { emotion: rest, facing: 'down', frame: 0 }), foot = 31;
      while (foot > 0 && !L.M.subarray(foot * 32, foot * 32 + 32).some(Boolean)) foot--;
      var c = { id: id, spec: spec, rest: rest, mood: null, moodT: 0, drop: 31 - foot, x: x, y: y, z: z, vy: 0,
                hx: 0, hz: 1, ground: false, moving: false, mx: 0, mz: 0, speed: WALK, jump: false, hidden: false };
      E.people.push(c);
      return c;
    };
    E.feel = function (c, mood, s) { c.mood = mood; c.moodT = s; };

    /* ── WHAT STOPS A BODY: the block, and whatever a scene says is solid ── */
    E.blocked = function (x, y, z) {
      if (world.hits((x - HW) / g, y / g, (z - HD) / g, (x + HW) / g, (y + BH) / g, (z + HD) / g)) return true;
      for (var i = 0; i < E.solids.length; i++) {
        var b = E.solids[i];
        if (x + HW > b[0] && x - HW < b[3] && y + BH > b[1] && y < b[4] && z + HD > b[2] && z - HD < b[5]) return true;
      }
      return false;
    };
    E.floor = function (x, y, z) { return world.floorUnder(x / g, y / g, z / g) * g; };
    E.move = function (c, dt) {
      c.vy -= GRAV * dt;
      if (c.jump && c.ground) { c.vy = JUMP; c.ground = false; }
      c.jump = false;
      var d = [c.mx * c.speed * dt, c.vy * dt, c.mz * c.speed * dt], axis, left, step;
      c.moving = Math.hypot(c.mx, c.mz) > 0.05;
      if (c.moving) { var hl = Math.hypot(c.mx, c.mz); c.hx = c.mx / hl; c.hz = c.mz / hl; }
      for (axis = 0; axis < 3; axis++) {
        left = d[axis];
        while (left !== 0) {
          step = Math.abs(left) > 1 ? Math.sign(left) : left; left -= step;
          var nx = c.x + (axis === 0 ? step : 0), ny = c.y + (axis === 1 ? step : 0), nz = c.z + (axis === 2 ? step : 0);
          if (axis === 1 && step > 0 && E.onHead && E.onHead(c, nx, ny, nz)) { c.vy = -60; break; }
          if (E.blocked(nx, ny, nz)) {
            if (axis === 1) { if (step < 0) c.ground = true; c.vy = 0; break; }
            if (!E.step || !c.ground || E.blocked(nx, c.y + E.step, nz)) break;     // a step up is taken in stride
            ny = c.y + E.step;
          }
          c.x = nx; c.y = ny; c.z = nz;
          if (axis === 1) c.ground = false;
        }
      }
    };

    /* ── TIME: things that take a while ── */
    var tweens = [], clock = 0;
    E.tween = function (dur, fn, then) { tweens.push({ t: 0, dur: dur, fn: fn, then: then }); };
    E.chain = function (steps) {
      var i = 0;
      (function next() {
        if (i >= steps.length) return;
        var s = steps[i++];
        E.tween(s[0], s[1], function () { if (s[2]) s[2](); next(); });
      })();
    };
    function tick(dt) {
      var list = tweens; tweens = [];
      list.forEach(function (w) {
        w.t += dt;
        var u = w.dur <= 0 ? 1 : Math.min(1, w.t / w.dur);
        if (w.fn) w.fn(u);
        if (u < 1) tweens.push(w); else if (w.then) w.then();
      });
    }
    E.clock = function () { return clock; };

    /* ── THE VIEW, AND THE HANDS ── */
    var cam = E.cam, input = E.input, keys = {}, VP = IDENT;
    E.control = function (c) {
      var kx = (keys.KeyD || keys.ArrowRight ? 1 : 0) - (keys.KeyA || keys.ArrowLeft ? 1 : 0);
      var ky = (keys.KeyW || keys.ArrowUp ? 1 : 0) - (keys.KeyS || keys.ArrowDown ? 1 : 0);
      var ix = clamp(kx + input.x, -1, 1), iy = clamp(ky + input.y, -1, 1), n = Math.hypot(ix, iy);
      if (n > 1) { ix /= n; iy /= n; }
      var s = Math.sin(cam.yaw), co = Math.cos(cam.yaw);
      c.mx = co * ix - s * iy; c.mz = -s * ix - co * iy;                       // right and forward, as the view has them
      c.jump = !!(keys.Space || input.jump);
    };
    E.facing = function (c) {
      var s = Math.sin(cam.yaw), co = Math.cos(cam.yaw), hf = -s * c.hx - co * c.hz, hr = co * c.hx - s * c.hz;
      return Math.abs(hf) > Math.abs(hr) * 1.3 ? (hf > 0 ? 'up' : 'down') : (hr > 0 ? 'right' : 'left');   // sideways unless plainly not
    };
    /* What you carry is held out on the side you are heading, edge on to the view. */
    E.hand = function (c, len, axisY) {
      var side = E.facing(c) === 'left' ? -1 : 1, s = Math.sin(cam.yaw), co = Math.cos(cam.yaw);
      var hx = c.x + co * side * 7 + s * 2, hz = c.z - s * side * 7 + co * 2;
      return place([hx + co * side * len, c.y + 11 + (c.moving ? Math.sin(clock * 14) : 0), hz - s * side * len],
                   cam.yaw + (side < 0 ? Math.PI : 0), 0, [len, axisY, 1]);
    };
    /* Where a point of the scene is on the page, in CSS pixels; null behind you. */
    E.project = function (p) {
      var w = VP[3] * p[0] + VP[7] * p[1] + VP[11] * p[2] + VP[15];
      if (w <= 0) return null;
      return [((VP[0] * p[0] + VP[4] * p[1] + VP[8] * p[2] + VP[12]) / w + 1) / 2 * root.innerWidth,
              (1 - (VP[1] * p[0] + VP[5] * p[1] + VP[9] * p[2] + VP[13]) / w) / 2 * root.innerHeight];
    };

    var dpr = 1, sun = [0, 0, 0];
    function resize() {
      dpr = Math.min(2, root.devicePixelRatio || 1);
      var w = Math.max(1, Math.round(root.innerWidth * dpr)), h = Math.max(1, Math.round(root.innerHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
      var m = o.mark && o.mark.getBoundingClientRect();
      sun = m && m.width ? [(m.left + m.width / 2) * dpr, (m.top + m.height / 2) * dpr, m.width / 2 * dpr] : [0, 0, 0];
    }
    function model(m, mat, clipY, count) {
      gl.uniformMatrix4fv(pVox.u.uModel, false, mat);
      gl.uniform1f(pVox.u.uClipY, clipY === undefined ? 1e9 : clipY);
      m.gpu ? m.gpu.draw(count) : m.draw(count);
    }
    var skyRGB = new Float32Array(18), bayer = new Float32Array(16);
    A.SKY.forEach(function (c, i) { skyRGB[i * 3] = c[0] / 255; skyRGB[i * 3 + 1] = c[1] / 255; skyRGB[i * 3 + 2] = c[2] / 255; });
    A.BAYER.forEach(function (row, j) { row.forEach(function (v, i) { bayer[j * 4 + i] = v; }); });

    function look() {                               // where the view is, and the matrix that puts the scene on the page
      var W = canvas.width, H = canvas.height, aspect = W / H;
      var fovy = aspect >= 1 ? cam.fov : 2 * Math.atan(Math.tan(cam.fov / 2) / aspect);
      var cp = Math.cos(cam.pitch), far = cam.dist * cam.zoom;
      var eye = [cam.tx + far * Math.sin(cam.yaw) * cp, cam.ty + far * Math.sin(cam.pitch), cam.tz + far * Math.cos(cam.yaw) * cp];
      VP = mul(perspective(fovy, aspect, 8, 6000), lookAt(eye, [cam.tx, cam.ty, cam.tz]));
      return { VP: VP, eye: eye, W: W, H: H, dpr: dpr, near: 8, far: 6000, clock: clock, right: [Math.cos(cam.yaw), 0, -Math.sin(cam.yaw)] };
    }
    function draw(scene) {
      var v = look(), W = v.W, H = v.H, eye = v.eye, right = v.right;
      gl.viewport(0, 0, W, H);
      gl.disable(gl.DEPTH_TEST); gl.disable(gl.CULL_FACE);
      gl.useProgram(pSky.p);
      gl.uniform2f(pSky.u.uRes, W, H); gl.uniform1f(pSky.u.uPx, Math.max(2, Math.round(dpr * 3)));
      gl.uniform3fv(pSky.u.uSky, skyRGB); gl.uniform3f(pSky.u.uSun, sun[0], sun[1], sun[2]); gl.uniform1fv(pSky.u.uB, bayer);
      gl.bindVertexArray(null); gl.drawArrays(gl.TRIANGLES, 0, 3);

      gl.enable(gl.DEPTH_TEST); gl.clear(gl.DEPTH_BUFFER_BIT); gl.enable(gl.CULL_FACE);
      palette();
      gl.useProgram(pVox.p);
      gl.uniformMatrix4fv(pVox.u.uVP, false, VP); gl.uniform1f(pVox.u.uTime, clock);
      gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, palTex); gl.uniform1i(pVox.u.uPal, 0);
      gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, topTex); gl.uniform1i(pVox.u.uTop, 1); gl.activeTexture(gl.TEXTURE0);
      gl.uniform1f(pVox.u.uSize, topSize); gl.uniform3f(pVox.u.uSun, SUN[0], SUN[1], SUN[2]);
      gl.uniform3f(pVox.u.uEye, eye[0], eye[1], eye[2]);
      if (E.cut) gl.uniform3f(pVox.u.uFocus, E.cut[0], E.cut[1], E.cut[2]);
      gl.uniform1f(pVox.u.uCut, E.cut ? 46 : 0);
      gl.uniformMatrix4fv(pVox.u.uModel, false, GROWN); gl.uniform1f(pVox.u.uClipY, 1e9); gl.uniform1f(pVox.u.uGrain, g);
      for (var i = 0; i < chunks.length; i++) if (chunks[i]) chunks[i].draw();
      gl.uniform1f(pVox.u.uGrain, 1);
      scene.parts(model);

      gl.disable(gl.CULL_FACE);
      gl.useProgram(pShd.p); gl.uniformMatrix4fv(pShd.u.uVP, false, VP); gl.bindVertexArray(quad);
      E.people.forEach(function (c) {
        if (c.hidden) return;
        var fy = E.floor(c.x, c.y + 2, c.z), k = clamp(1 - (c.y - fy) / 90, 0.35, 1);
        gl.uniform3f(pShd.u.uPos, c.x, fy + 0.08, c.z); gl.uniform2f(pShd.u.uR, 9 * k, 5 * k);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      });
      gl.useProgram(pSpr.p); gl.uniformMatrix4fv(pSpr.u.uVP, false, VP);
      gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, atlas); gl.uniform1i(pSpr.u.uTex, 0);
      gl.uniform3f(pSpr.u.uRight, right[0], right[1], right[2]); gl.uniform2f(pSpr.u.uSize, 32, 32);
      E.people.forEach(function (c) {
        if (c.hidden) return;
        var mood = c.mood || (c.y - E.floor(c.x, c.y + 2, c.z) > 60 ? 'surprised' : c.rest);     // a long way up is a long way down
        var n = cell(c, mood, E.facing(c), c.moving && c.ground ? Math.floor(clock * 7) & 1 : 0);
        gl.uniform3f(pSpr.u.uPos, c.x, c.y - c.drop, c.z);
        gl.uniform4f(pSpr.u.uCell, (n & 31) / 32, (n >> 5) / 32, 1 / 32, 1 / 32);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      });
      gl.bindVertexArray(null);
    }

    /* ── INPUT ── */
    var heard = [];
    function hear(el, type, fn, opt) { el.addEventListener(type, fn, opt); heard.push([el, type, fn, opt]); }
    /* A quarter turn of the view, eased: Q and E, as The Sims has its two keys. */
    var turnTo = null;
    E.turn = function (way) { turnTo = (turnTo === null ? Math.round(cam.yaw / (Math.PI / 2)) * (Math.PI / 2) : turnTo) + way * Math.PI / 2; };
    hear(root, 'keydown', function (e) {
      if (e.metaKey || e.ctrlKey || e.altKey || canvas.hidden) return;
      if (!e.repeat && (e.code === 'KeyQ' || e.code === 'KeyE')) E.turn(e.code === 'KeyQ' ? 1 : -1);
      keys[e.code] = true;
      if (/^(Arrow|Space)/.test(e.code)) e.preventDefault();
    });
    hear(root, 'keyup', function (e) { keys[e.code] = false; });
    hear(root, 'blur', function () { keys = {}; });
    var drags = {};
    function span() { var k = Object.keys(drags); return k.length < 2 ? 0 : Math.hypot(drags[k[0]].x - drags[k[1]].x, drags[k[0]].y - drags[k[1]].y); }
    hear(canvas, 'pointerdown', function (e) {
      drags[e.pointerId] = { x: e.clientX, y: e.clientY };
      try { canvas.setPointerCapture(e.pointerId); } catch (err) { /* a synthetic pointer has nothing to capture */ }
    });
    hear(canvas, 'pointermove', function (e) {
      var d = drags[e.pointerId];
      if (!d) return;
      var before = span(), dx = e.clientX - d.x, dy = e.clientY - d.y;
      d.x = e.clientX; d.y = e.clientY;
      if (before) { var after = span(); if (after) cam.zoom = clamp(cam.zoom * before / after, 0.35, 2.2); return; }
      turnTo = null;
      cam.yaw -= dx * 0.006; cam.pitch = clamp(cam.pitch + dy * 0.005, 0.02, 1.25);
    });
    function drop(e) { delete drags[e.pointerId]; }
    hear(canvas, 'pointerup', drop); hear(canvas, 'pointercancel', drop);
    hear(canvas, 'wheel', function (e) { e.preventDefault(); cam.zoom = clamp(cam.zoom * Math.exp(e.deltaY * 0.0012), 0.35, 2.2); }, { passive: false });
    hear(root, 'resize', resize);

    /* ── GO, AND STOP ── */
    var raf = 0, frames = 0;
    E.frames = function () { return frames; };
    /* A scene with a `draw` of its own draws itself (town.js); the block and the sprites are then not this file's business. */
    E.run = function (scene) {
      if (!scene.draw) { world.dirty.fill(1); remesh(); }
      resize();
      var last = 0, acc = 0, STEP = 1 / 120;
      (function frame(now) {
        var dt = Math.min(0.1, last ? (now - last) / 1000 : 0);
        last = now; acc += dt;
        while (acc >= STEP) {
          clock += STEP; tick(STEP); scene.update(STEP);
          if (turnTo !== null) { cam.yaw = lerp(cam.yaw, turnTo, 1 - Math.exp(-STEP * 12)); if (Math.abs(cam.yaw - turnTo) < 0.002) { cam.yaw = turnTo; turnTo = null; } }
          E.people.forEach(function (c) { if (c.moodT > 0) { c.moodT -= STEP; if (c.moodT <= 0) c.mood = null; } });
          acc -= STEP;
        }
        if (scene.draw) scene.draw(look()); else { remesh(); draw(scene); }
        frames++;
        raf = root.requestAnimationFrame(frame);
      })(0);
    };
    E.stop = function () {
      root.cancelAnimationFrame(raf);
      heard.forEach(function (h) { h[0].removeEventListener(h[1], h[2], h[3]); });
      var lose = gl.getExtension('WEBGL_lose_context');
      if (lose) lose.loseContext();
    };
    E.chunks = function () { return chunks.filter(Boolean).length; };
    return E;
  }

  K.Engine = { create: create, place: place, program: program, clamp: clamp, lerp: lerp, outCubic: outCubic, inOut: inOut, outBack: outBack, hump: hump,
               GRAV: GRAV, WALK: WALK, RUN: RUN, JUMP: JUMP, HW: HW, HD: HD, BH: BH };
})(typeof self !== 'undefined' ? self : this);
