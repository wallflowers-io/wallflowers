/* folk.js — the cast, as figures.
 *
 * tama.js draws each of the cast as a sprite, from a spec: a build, a head, a
 * way of wearing their hair, what they have on and in what colours. This reads
 * the same spec and stands the same person up in the round: a big head on a
 * small body, as they are drawn, in their own colours (Tama.palette). No sprite
 * is used.
 *
 * FIVE PIECES: the body with its head, two arms, two legs. A limb is made
 * hanging from 0, 0, 0, so whoever draws it swings it by turning it there
 * (town.js); `at` says where each hangs from. Facing south, feet at 0.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin = root.Kenjin || {}, Mesh = K.TownMesh.Mesh;
  var BUILD = { baby: [19, 0.52], kid: [24, 0.45], teen: [28, 0.39], adult: [31, 0.35], elder: [29, 0.37] };   // how tall, and how much of that is head
  function dim(c, f) { return [c[0] * f, c[1] * f, c[2] * f].map(Math.round); }

  function build(Tama, spec) {
    var s = Tama.normalize(spec), P = Tama.palette(spec), MAT = Tama.MAT, b = BUILD[s.body.build] || BUILD.kid;
    var tall = b[0], R = tall * b[1] / 2 + (s.head.size || 0) * 0.3, legs = tall * 0.27, torso = tall - 2 * R * 0.94 - legs, wide = R * 1.45, deep = R * 0.95;
    var skin = P[MAT.SKIN].map(Math.round), hair = P[MAT.HAIR].map(Math.round), top = P[MAT.OUTFIT].map(Math.round), shoes = P[MAT.SHOES].map(Math.round), eyes = P[MAT.EYES];
    var bare = s.body.outfit === 'dress' || s.body.outfit === 'diaper', lower = bare ? skin : dim(top, 0.62);
    var body = new Mesh(), hy = legs + torso + R * 0.9, style = s.hair.style;
    body.box(-wide / 2, legs - 0.5, -deep / 2, wide / 2, legs + torso, deep / 2, top);
    if (s.body.outfit === 'dress') body.cyl(0, 0, wide * 0.5, legs + torso * 0.5, legs - legs * 0.45, 8, top, 0, wide * 0.78);      // a skirt
    body.ball(0, hy, 0, R, R, R * 0.96, skin);
    [-1, 1].forEach(function (e) { body.box(e * R * 0.36 - 0.6, hy - R * 0.12, R * 0.9, e * R * 0.36 + 0.6, hy + R * 0.2, R * 0.99, eyes); });
    body.box(-R * 0.2, hy - R * 0.5, R * 0.86, R * 0.2, hy - R * 0.4, R * 0.95, dim(skin, 0.6));
    if (style !== 'bald') {                       // hair: over the top and down the back, and then whatever the style adds
      var big = style === 'afro' || style === 'curls' ? 1.34 : 1.1;
      body.ball(0, hy + R * 0.06, -R * 0.05, R * big, R * big, R * big, hair, 0, 0, 0, function (v) { return v[1] > 0.22 || v[2] < -0.25 || (big > 1.2 && v[2] < 0.5); });
      if (/long|braids|bob|pigtails|cornrows/.test(style)) body.box(-R * 0.86, hy - R * (style === 'bob' ? 0.7 : 1.5), -R * 1.02, R * 0.86, hy + R * 0.3, -R * 0.5, hair);
      if (style === 'pigtails') [-1, 1].forEach(function (e) { body.ball(e * R * 1.15, hy - R * 0.1, -R * 0.1, R * 0.38, R * 0.5, R * 0.38, hair); });
      if (style === 'bun') body.ball(0, hy + R * 1.12, -R * 0.2, R * 0.42, R * 0.42, R * 0.42, hair);
      if (/mohawk|spiky|tall|flattop/.test(style)) body.box(-R * (style === 'mohawk' ? 0.16 : 0.62), hy + R * 0.8, -R * 0.6, R * (style === 'mohawk' ? 0.16 : 0.62), hy + R * 1.4, R * 0.5, hair);
    }
    if (s.hat.style && s.hat.style !== 'none') body.cyl(0, 0, R * 0.92, hy + R * 0.78, hy + R * 1.2, 8, P[MAT.HAT].map(Math.round), 0, R * 0.8);
    function limb(w, len, d, upper, low, foot) {  // hanging from 0, 0, 0
      var m = new Mesh();
      m.box(-w / 2, -len * 0.5, -d / 2, w / 2, 0, d / 2, upper);
      m.box(-w / 2, -len + (foot ? 2 : 0), -d / 2, w / 2, -len * 0.5, d / 2, low);
      if (foot) m.box(-w / 2 - 0.2, -len, -d / 2, w / 2 + 0.2, -len + 2, d / 2 + 1.4, foot);
      return m.done();
    }
    var arm = limb(2.3, torso * 0.92, 2.3, top, skin), leg = limb(2.9, legs, 3, lower, lower, shoes);
    return { tall: tall, body: body.done(), arm: arm, leg: leg,
             at: { arm: [wide / 2 + 1.2, legs + torso - 0.6, 0], leg: [wide * 0.26, legs, 0] } };
  }

  K.Folk = { build: build };
})(typeof self !== 'undefined' ? self : this);
