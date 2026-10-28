/* town.js — a generated land, walked: in shapes, and in any of several looks.
 *
 * roam.js walks land.js's land as blocks, with the cast as sprites. This walks
 * the same land with neither: townmesh.js's shapes for the ground, the
 * buildings and what grows; folk.js's figures for the cast. What you do is the
 * same — you come down at the first place and walk to the rest, each one's
 * sign coming up as you reach it — and so are the view, the cutaway, the
 * diamonds, the radar (roam.js says why).
 *
 * A LOOK IS A SKIN (skins.js), AND IT IS ONLY NUMBERS: a colour to every stuff,
 * where the sun is, how hard a shadow falls, how thick the air, whether light
 * comes in bands, whether things are inked round. Nothing is rebuilt to change
 * one, so `skin(id)` flicks between two frames, mid-stride.
 *
 * HOW A FACE IS LIT. A triangle carries no normal: its slant is worked out from
 * how its place in the scene changes across the picture. So every face is flat,
 * which is the look. The sun casts as it does in engine.js: a height to each
 * column of the land, marched toward the light.
 *
 * INK IS DRAWN AFTERWARDS. For a skin that asks for it, the picture is drawn
 * off screen first, then put up with a line wherever its colour or its depth
 * jumps: round a wall, along a roof's edge, round you. A skin of lines only
 * keeps the lines and throws the picture away.
 *
 * WHAT STOPS YOU is townmesh.js's rectangles and circles, and the ground is its
 * rolling surface: no block is consulted.
 *
 * MADE UP, AND NOT KEPT, as roam.js is. No Delta carries a chosen place, a
 * land, a visit, or which look you were wearing.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin = root.Kenjin || {}, En = K.Engine, lerp = En.lerp, place = En.place, Skins = K.Skins, TM = K.TownMesh;
  var G = 8;

  var VS = '#version 300 es\nlayout(location=0) in vec3 aPos; layout(location=1) in uvec4 aM;\n' +
    'uniform mat4 uVP, uModel; out vec3 vW; flat out ivec4 vM;\n' +
    'void main(){ vec4 w = uModel * vec4(aPos, 1.0); vW = w.xyz; vM = ivec4(aM); gl_Position = uVP * w; }';
  var FS = '#version 300 es\nprecision highp float; precision highp int;\n' +
    'uniform sampler2D uTop; uniform float uSize; uniform vec3 uPal[48]; uniform float uGlow[48];\n' +
    'uniform vec3 uSun, uSunCol, uAir, uFog, uEye, uFocus; uniform float uThick, uShade, uCel, uCut, uTime, uLevel, uLampGain;\n' +
    'uniform vec3 uLamp[12]; uniform int uLamps;\n' +
    'in vec3 vW; flat in ivec4 vM; out vec4 o;\n' +
    'void main(){\n' +
    '  int m = vM.x; bool own = m == 255, ground = m <= ' + Skins.index.pave + ';\n' +
    '  if (uCut > 0.0 && !ground && vW.y > uFocus.y + 3.0) {\n' +                                          // what stands between the eye and you: never the land itself
    '    vec3 ef = uFocus - uEye; float t = dot(vW - uEye, ef) / dot(ef, ef);\n' +
    '    if (t > 0.0 && t < 0.97 && length(vW - (uEye + ef * t)) < uCut * (0.3 + 0.7 * t)) discard; }\n' +
    '  vec3 n = normalize(cross(dFdx(vW), dFdy(vW)));\n' +                                                 // the face\'s slant, from the picture itself
    '  if (ground) n = normalize(mix(n, vec3(0.0, 1.0, 0.0), uLevel));\n' +                                // how much a hillside\'s facets are let show

    '  vec3 base = own ? vec3(vM.yzw) / 255.0 : uPal[m] * (0.88 + float(vM.y) / 255.0 * 0.24);\n' +
    '  float l = max(dot(n, uSun), 0.0);\n' +
    '  if (uSize > 0.0 && l > 0.0) { vec3 p = vW / 8.0 + n * 0.06;\n' +                                    // is something taller between here and the sun
    '    for (int i = 1; i <= 16; i++) { vec3 w = p + uSun * (float(i) * 0.9);\n' +
    '      if (w.y >= 63.0 || w.x < 0.0 || w.z < 0.0 || w.x >= uSize || w.z >= uSize) break;\n' +
    '      if (texelFetch(uTop, ivec2(w.xz), 0).r * 63.75 > w.y + 0.35) { l *= 1.0 - uShade; break; } } }\n' +
    '  if (uCel > 0.0) l = floor(l * uCel + 0.5) / uCel;\n' +                                              // light in bands
    '  vec3 col = base * (uAir + uSunCol * l);\n' +
    '  if (m == ' + Skins.index.water + ') col += uSunCol * 0.06 * sin(vW.x * 0.11 + uTime * 1.2) * sin(vW.z * 0.09 - uTime * 0.9);\n' +
    '  for (int i = 0; i < uLamps; i++) { vec3 d = uLamp[i] - vW; float a = 1.0 / (1.0 + dot(d, d) * 0.0006);\n' +   // the street lamps, after dark
    '    col += base * vec3(1.0, 0.8, 0.5) * a * uLampGain * (0.35 + 0.65 * max(dot(n, normalize(d)), 0.0)); }\n' +
    '  if (!own) col = mix(col, base * 1.15, uGlow[m]);\n' +                                               // what shines by itself
    '  col = mix(col, uFog, 1.0 - exp(-length(vW - uEye) * uThick));\n' +
    '  o = vec4(col, 1.0); }';
  var VS_FULL = '#version 300 es\nvoid main(){ gl_Position = vec4(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0, 1.0, 1.0); }';
  var FS_SKY = '#version 300 es\nprecision highp float; uniform vec3 uTopCol, uFootCol; uniform float uH; out vec4 o;\n' +
    'void main(){ o = vec4(mix(uFootCol, uTopCol, clamp(gl_FragCoord.y / uH, 0.0, 1.0)), 1.0); }';
  var FS_INK = '#version 300 es\nprecision highp float; uniform sampler2D uCol, uDep; uniform vec2 uInv; uniform float uWide, uInk, uLines, uNear, uFar;\n' +
    'uniform vec3 uInkCol, uPaper; out vec4 o;\n' +
    'float far(float d){ return 2.0 * uNear * uFar / (uFar + uNear - (d * 2.0 - 1.0) * (uFar - uNear)); }\n' +
    'void main(){ vec2 uv = gl_FragCoord.xy * uInv; vec3 c = texture(uCol, uv).rgb; float d = far(texture(uDep, uv).r), e = 0.0;\n' +
    '  for (int i = 0; i < 4; i++) { vec2 at = uv + vec2(i == 0 ? 1.0 : i == 1 ? -1.0 : 0.0, i == 2 ? 1.0 : i == 3 ? -1.0 : 0.0) * uWide * uInv;\n' +
    '    float d2 = far(texture(uDep, at).r);\n' +
    '    e = max(e, max(smoothstep(0.05, 0.14, length(texture(uCol, at).rgb - c)), smoothstep(0.012, 0.035, abs(d2 - d) / min(d, d2)))); }\n' +
    '  o = vec4(uLines > 0.5 ? mix(uPaper, uInkCol, e) : mix(c, uInkCol, e * uInk), 1.0); }';

  function Shape(gl, m) {
    this.gl = gl; this.n = m.n;
    this.vao = gl.createVertexArray(); gl.bindVertexArray(this.vao);
    var a = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, a); gl.bufferData(gl.ARRAY_BUFFER, m.p, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    var b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, m.m, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(1); gl.vertexAttribIPointer(1, 4, gl.UNSIGNED_BYTE, 0, 0);
    gl.bindVertexArray(null);
  }

  function start(cast, sites, opts) {
    var Tama = root.Tama, A = K.Ceremony.art, n = 0;
    var L = K.Land.generate(sites, { stuff: function () { return ++n; } }), T = TM.build(L);
    var E = En.create(opts.canvas, { art: A, Tama: Tama, cast: cast, pal: { changed: false }, world: L.world, grain: G, smooth: true, mark: opts.mark });
    var gl = E.gl, cam = E.cam, pScene = En.program(gl, VS, FS), pSky = En.program(gl, VS_FULL, FS_SKY), pInk = En.program(gl, VS_FULL, FS_INK);
    function shape(m) { return new Shape(gl, m); }
    var land = shape(T.mesh), mine = shape(TM.diamond('green')), theirs = shape(TM.diamond('gold'));
    E.sun(T.tops, L.size);
    var skin = Skins.resolve(opts.skin, A.GEMS);

    var spots = L.spots.map(function (s, i) {
      var x = (s.board[0] + 0.5) * G, z = (s.board[1] + 0.5) * G;
      return { name: s.name, as: s.as, x: x, z: z, y: T.heightAt(x, z), roof: s.top * G, bx: s.x + 0.5, bz: s.z + 0.5, up: 0, seen: false, sign: shape(TM.sign(i)),
               cx: (s.lot ? s.lot[0] + s.lot[2] / 2 : s.x + 0.5) * G, cz: (s.lot ? s.lot[1] + s.lot[3] / 2 : s.z - 8) * G };
    });
    var ids = Object.keys(cast), first = L.spots[0], who = cast[opts.as] ? opts.as : ids[Math.floor(Math.random() * ids.length)], fig = K.Folk.build(Tama, cast[who]);
    var body = shape(fig.body), arm = shape(fig.arm), leg = shape(fig.leg);
    var me = { id: who, x: (first.x - 1.5) * G, z: (first.z + 1.5) * G, y: 0, vy: 0, hx: 0, hz: 1, mx: 0, mz: 0, jump: false, ground: false, stride: 0 };
    me.y = T.heightAt(me.x, me.z) + 170;
    if (opts.at) { me.x = opts.at[0]; me.z = opts.at[1]; me.y = T.heightAt(me.x, me.z) + 1; }      // carried over from another look
    var done = false, R = 4.5;
    function stood(s) { T.rounds.push([s.x, s.z - 3, 2.5]); }
    spots.forEach(function (s, i) { if (opts.seen && opts.seen[i]) { s.seen = true; s.up = 1; stood(s); } });
    function arrive(s) {
      s.seen = true;
      if (opts.say) opts.say(s.name);
      E.tween(0.85, function (u) { s.up = En.outBack(u); }, function () {
        s.up = 1; stood(s);
        if (spots.every(function (t) { return t.seen; })) done = true;
      });
    }
    function next() {
      var best = null;
      spots.forEach(function (s) { if (!s.seen && (!best || Math.hypot(s.x - me.x, s.z - me.z) < Math.hypot(best.x - me.x, best.z - me.z))) best = s; });
      return best;
    }

    /* ── the picture off screen, for a skin that inks ── */
    var fbo = null, shot = null, depth = null, sized = '';
    function offscreen(W, H) {
      if (sized === W + 'x' + H) return;
      sized = W + 'x' + H;
      if (!fbo) { fbo = gl.createFramebuffer(); shot = gl.createTexture(); depth = gl.createTexture(); }
      [[shot, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, gl.LINEAR], [depth, gl.DEPTH_COMPONENT24, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, gl.NEAREST]].forEach(function (t) {
        gl.bindTexture(gl.TEXTURE_2D, t[0]);
        gl.texImage2D(gl.TEXTURE_2D, 0, t[1], W, H, 0, t[2], t[3], null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, t[4]); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, t[4]);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      });
      gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, shot, 0);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, depth, 0);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    }

    var IDENT = place([0, 0, 0]), lampBuf = new Float32Array(36);
    function put(sh, mat) { gl.uniformMatrix4fv(pScene.u.uModel, false, mat); gl.bindVertexArray(sh.vao); gl.drawArrays(gl.TRIANGLES, 0, sh.n); }
    function turned(v, yaw) { var c = Math.cos(yaw), s = Math.sin(yaw); return [c * v[0] + s * v[2], v[1], -s * v[0] + c * v[2]]; }

    cam.tx = me.x; cam.ty = T.heightAt(me.x, me.z) + 20; cam.tz = me.z; cam.pitch = 0.8; cam.fov = 0.5;
    E.run({
      update: function (dt) {
        E.control(me);
        var moving = Math.hypot(me.mx, me.mz) > 0.05;
        if (moving) { var hl = Math.hypot(me.mx, me.mz); me.hx = me.mx / hl; me.hz = me.mz / hl; me.stride += dt * 11; }
        var nx = me.x + me.mx * En.WALK * dt, nz = me.z + me.mz * En.WALK * dt;
        if (!T.inTheWay(nx, me.z, R)) me.x = nx;
        if (!T.inTheWay(me.x, nz, R)) me.z = nz;
        me.vy -= En.GRAV * dt;
        if (me.jump && me.ground) { me.vy = En.JUMP; me.ground = false; }
        me.y += me.vy * dt;
        var floor = T.heightAt(me.x, me.z);
        if (me.y <= floor + (me.ground ? 3 : 0)) { me.y = floor; me.vy = 0; me.ground = true; } else me.ground = false;     // feet stay on a slope
        me.moving = moving;
        spots.forEach(function (s) { if (!s.seen && me.ground && Math.hypot(me.x - s.x, me.z - s.z) < 42) arrive(s); });
        cam.dist = root.innerWidth < root.innerHeight ? 560 : 470;
        cam.tx = lerp(cam.tx, me.x, 1 - Math.exp(-dt * 5)); cam.tz = lerp(cam.tz, me.z, 1 - Math.exp(-dt * 5));
        cam.ty = lerp(cam.ty, me.y + 20, 1 - Math.exp(-dt * 4));
      },
      draw: function (v) {
        var W = v.W, H = v.H, inked = skin.ink > 0 || skin.lines, t = v.clock, to = next(), u = pScene.u;
        if (inked) offscreen(W, H);
        gl.bindFramebuffer(gl.FRAMEBUFFER, inked ? fbo : null);
        gl.viewport(0, 0, W, H);
        gl.disable(gl.DEPTH_TEST); gl.disable(gl.CULL_FACE); gl.depthMask(false);
        gl.useProgram(pSky.p); gl.uniform3fv(pSky.u.uTopCol, skin.sky[0]); gl.uniform3fv(pSky.u.uFootCol, skin.sky[1]); gl.uniform1f(pSky.u.uH, H);
        gl.bindVertexArray(null); gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.depthMask(true); gl.enable(gl.DEPTH_TEST); gl.clear(gl.DEPTH_BUFFER_BIT);

        gl.useProgram(pScene.p);
        gl.uniformMatrix4fv(u.uVP, false, v.VP);
        gl.uniform3fv(u.uPal, skin.pal); gl.uniform1fv(u.uGlow, skin.glow);
        gl.uniform3fv(u.uSun, skin.sun); gl.uniform3fv(u.uSunCol, skin.sunCol); gl.uniform3fv(u.uAir, skin.air); gl.uniform3fv(u.uFog, skin.fog);
        gl.uniform1f(u.uThick, skin.thick); gl.uniform1f(u.uShade, skin.shade); gl.uniform1f(u.uCel, skin.cel); gl.uniform1f(u.uTime, t);
        gl.uniform1f(u.uLevel, skin.level); gl.uniform1f(u.uLampGain, 2.6);
        gl.uniform3fv(u.uEye, v.eye); gl.uniform3f(u.uFocus, me.x, me.y, me.z); gl.uniform1f(u.uSize, L.size);
        E.bindTops(u.uTop);
        var lit = 0;
        if (skin.lamps) T.lamps.slice().sort(function (a, b) { return Math.hypot(a[0] - me.x, a[2] - me.z) - Math.hypot(b[0] - me.x, b[2] - me.z); })
          .slice(0, 12).forEach(function (l) { lampBuf.set(l, lit++ * 3); });
        gl.uniform3fv(u.uLamp, lampBuf); gl.uniform1i(u.uLamps, lit);
        gl.uniform1f(u.uCut, 46); put(land, IDENT);
        gl.uniform1f(u.uCut, 0);
        spots.forEach(function (s) {
          if (!s.seen) put(theirs, place([s.cx, s.roof + 30 + Math.sin(t * 2.2 + s.x) * 3, s.cz], t * 1.6));
          if (s.up > 0) put(s.sign, place([s.x, s.y - (1 - s.up) * 74, s.z]));
        });
        /* you: the body, and each limb turned where it hangs */
        var yaw = Math.atan2(me.hx, me.hz), swing = me.ground ? (me.moving ? Math.sin(me.stride) * 0.75 : 0) : -0.5, bob = me.moving && me.ground ? Math.abs(Math.sin(me.stride)) * 0.9 : 0;
        var at = [me.x, me.y + bob, me.z];
        put(body, place(at, yaw));
        [-1, 1].forEach(function (side) {
          var a = turned([fig.at.arm[0] * side, fig.at.arm[1], 0], yaw), l = turned([fig.at.leg[0] * side, fig.at.leg[1], 0], yaw);
          put(arm, place([at[0] + a[0], at[1] + a[1], at[2] + a[2]], yaw, me.ground ? -swing * side : -2.6));
          put(leg, place([at[0] + l[0], at[1] + l[1], at[2] + l[2]], yaw, swing * side));
        });
        put(mine, place([me.x, me.y + fig.tall + 12 + Math.sin(t * 3) * 1.5, me.z], t * 2.2));

        if (inked) {
          gl.bindFramebuffer(gl.FRAMEBUFFER, null);
          gl.disable(gl.DEPTH_TEST);
          gl.useProgram(pInk.p);
          gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, shot); gl.uniform1i(pInk.u.uCol, 0);
          gl.activeTexture(gl.TEXTURE2); gl.bindTexture(gl.TEXTURE_2D, depth); gl.uniform1i(pInk.u.uDep, 2); gl.activeTexture(gl.TEXTURE0);
          gl.uniform2f(pInk.u.uInv, 1 / W, 1 / H); gl.uniform1f(pInk.u.uWide, Math.max(1, v.dpr)); gl.uniform1f(pInk.u.uInk, skin.ink);
          gl.uniform1f(pInk.u.uLines, skin.lines ? 1 : 0); gl.uniform1f(pInk.u.uNear, v.near); gl.uniform1f(pInk.u.uFar, v.far);
          gl.uniform3fv(pInk.u.uInkCol, skin.inkCol); gl.uniform3fv(pInk.u.uPaper, skin.paper);
          gl.bindVertexArray(null); gl.drawArrays(gl.TRIANGLES, 0, 3);
        }
        if (opts.names) opts.names(to ? [{ name: to.name, at: E.project([to.cx, to.roof + 52, to.cz]) || [root.innerWidth / 2, 0] }] : []);
        if (opts.radar) opts.radar({ map: L.map, size: L.size, x: me.x / G, z: me.z / G, hx: me.hx, hz: me.hz, yaw: cam.yaw,
                                     spots: spots.map(function (s) { return { x: s.bx, z: s.bz, seen: s.seen, next: s === to }; }) });
      }
    });

    return {
      input: E.input, cam: cam, stop: E.stop, turn: E.turn,
      /* The look, changed between two frames. */
      skin: function (id) { skin = Skins.resolve(id, A.GEMS); return skin.name; },
      /* Read-only, for whoever is watching. */
      state: function () {
        return { phase: done ? 'done' : 'roam', skin: skin.id, as: me.id, at: [me.x, me.y, me.z], ground: me.ground, frames: E.frames(), size: L.size, seed: L.seed,
                 lots: L.lots.length, trees: L.trees, cars: L.cars, corners: T.mesh.n,
                 spots: spots.map(function (s) { return { name: s.name, as: s.as, at: [s.x, s.y, s.z], seen: s.seen, up: s.up }; }) };
      }
    };
  }

  K.Town = { start: start };
})(typeof self !== 'undefined' ? self : this);
