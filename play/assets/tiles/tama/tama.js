/* tama.js — the WallFlowers Tamagotchi engine.
 *
 * ONE FUNCTION DRAWS EVERY SPRITE:
 *
 *     Tama.render(spec, { emotion, facing, frame, blink, look }) -> Uint8Array(32*32)
 *
 * FACINGS ARE DIRECTIONS ON THE GRID, not camera angles: the cast stand on a 2D
 * plane and turn toward each other. `down` faces the viewer, `up` faces away,
 * `left` is `right` mirrored.
 *
 * The studio, the site grid, the demo grid and the bake script all call it, so a
 * character looks the same everywhere by construction rather than by copying.
 * Each pixel is OFF (0), INK (1), MID (2) or SKIN 1..5 (3..7, graded shades from
 * light to dark); how those look (an LCD, a tile, a PNG) is the caller's business —
 * see Tama.paint and Tama.rgba. Skin is a tone, not a colour: the LCD has no hue.
 *
 * A SPEC IS PARAMETERS, NOT PIXELS. A character is a handful of choices (build,
 * head, ears, hair, hat, face, outfit, what they hold, how they behave) and every
 * emotion, facing and frame is derived from those same choices. Where a derived
 * pixel is wrong, `overrides` corrects it by hand, keyed by facing/emotion/frame
 * with `*` wildcards — the parameters stay the source and the patch stays small.
 *
 * THE SCHEMA BELOW IS THE ONLY LIST OF PARAMETERS. The studio builds its controls
 * from it and the tests validate cast.json against it; nothing else restates it.
 *
 * FIXTURE ART, NOT PACIFIC STATE. The specs in cast.json belong to the demo's
 * cast, keyed by the same ids as world.js's profiles. No Delta carries a sprite
 * today. A member's own Tamagotchi would travel with their profile — a `sprite`
 * field on core's ContactCard, published by contact.publishProfile — and that
 * field does not exist yet.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.Tama = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var N = 32, OFF = 0, INK = 1, MID = 2;
  /* SKIN levels 1..5 are pixel values 3..7; level 0 is the paper itself (OFF). SKIN_T is
     how far each level sits from the LCD's background toward its ink. */
  var SKIN_T = [0, 0.12, 0.24, 0.36, 0.48, 0.60];
  function skinValue(level) { return level > 0 ? 2 + level : OFF; }
  var curSkin = OFF, curMark = MID, curLine = MID;   // set per render: the skin, and what reads on it

  /* MATERIALS. Every pixel also records WHAT it depicts — skin, hair, an eye's white,
     the tongue — in a layer beside the tones. The LCD ignores it (tones are the LCD);
     full colour (paintColour) is drawn from it, with each character's own colours. One
     drawing, two outputs, so a colour Tamagotchi is always the LCD one, coloured in. */
  var MAT = { BG: 0, INK: 1, SKIN: 2, SKIN_SHADE: 3, WHITE: 4, BLUSH: 5, TEAR: 6, FACELINE: 7, HAIR: 8,
              HAIR_LIGHT: 9, HAT: 10, HAT_DETAIL: 11, OUTFIT: 12, OUTFIT2: 13, PATTERN: 14, HELD: 15,
              TONGUE: 16, SWEAT: 17, FX_RED: 18, FX: 19, BROW: 20, GLASSES: 21, SHOES: 22, MID: 23,
              EYES: 24, TIES: 25, HELD_LIGHT: 26, FX_GOLD: 27, SCLERA: 28 };
  var ML = null;                       // the material layer being drawn, beside R
  var cur = {};                        // what '#', '+', 'o', a fill and an edge depict right now
  function mat(o) {
    cur = { ink: MAT.INK, mid: MAT.MID, off: MAT.BG, fill: MAT.MID, edge: MAT.INK, mark: MAT.BLUSH };
    for (var k in o) cur[k] = o[k];
  }
  function matOf(v) { return v === INK ? cur.ink : v === MID ? cur.mid : v === OFF ? cur.off : MAT.SKIN; }
  var EMOTIONS = ['neutral', 'happy', 'laugh', 'sad', 'cry', 'angry',
                  'surprised', 'scared', 'sleepy', 'love', 'smug', 'nervous'];
  var FACINGS = ['down', 'left', 'right', 'up'];
  var FRAMES = 2;

  /* ── THE SCHEMA ─────────────────────────────────────────────────────────── */
  function E(key, label, options, def) { return { key: key, label: label, type: 'enum', options: options, default: def === undefined ? options[0] : def }; }
  function C(key, label) { return { key: key, label: label, type: 'color', default: '' }; }
  function I(key, label, min, max, def) { return { key: key, label: label, type: 'int', min: min, max: max, default: def || 0 }; }
  var TONES = ['ink', 'mid', 'off'];

  var SCHEMA = [
    { group: 'Body', fields: [
      E('body.build', 'Build', ['baby', 'kid', 'teen', 'adult', 'elder'], 'kid'),
      I('body.width', 'Width', -2, 2),
      E('body.outfit', 'Outfit', ['tee', 'collar', 'hoodie', 'jacket', 'dress', 'overalls', 'diaper', 'suit', 'vest', 'apron'], 'tee'),
      E('body.tone', 'Outfit tone', TONES, 'mid'),
      E('body.emblem', 'Emblem', ['none', 'star', 'planet', 'heart', 'stripes', 'dots', 'pocket', 'tie', 'bowtie'], 'none'),
      E('body.feet', 'Feet', ['nubs', 'shoes', 'boots'], 'nubs')]},
    { group: 'Head', fields: [
      E('head.shape', 'Shape', ['round', 'oval', 'wide', 'long', 'bean'], 'round'),
      I('head.size', 'Size', -3, 3)]},
    { group: 'Skin', fields: [
      I('skin.tone', 'Skin tone', 0, 5, 1)]},
    { group: 'Ears', fields: [
      E('ears.style', 'Ears', ['none', 'small', 'big', 'aardvark', 'rabbit', 'bear', 'cat', 'floppy', 'mouse'], 'small')]},
    { group: 'Hair', fields: [
      E('hair.style', 'Style', ['bald', 'strands', 'cap', 'spiky', 'afro', 'bob', 'long', 'pigtails', 'bun', 'braids',
                                'cornrows', 'mohawk', 'curls', 'sides', 'flattop', 'tall'], 'cap'),
      E('hair.tone', 'Tone', TONES, 'ink'),
      I('hair.volume', 'Volume', -2, 3),
      I('hair.count', 'Strands', 1, 6, 3),
      E('hair.fringe', 'Fringe', ['none', 'straight', 'side', 'ragged', 'parted', 'eyes'], 'straight'),
      E('hair.ties', 'Ties', ['none', 'bows', 'bands'], 'none')]},
    { group: 'Hat', fields: [
      E('hat.style', 'Hat', ['none', 'sock', 'cap', 'beanie', 'bow', 'headband', 'crown', 'saucepan'], 'none'),
      E('hat.tone', 'Tone', TONES, 'ink')]},
    { group: 'Face', fields: [
      E('eyes.style', 'Eyes', ['dot', 'round', 'big', 'narrow', 'lashes'], 'round'),
      I('eyes.spacing', 'Eye spacing', -2, 2),
      I('eyes.y', 'Eye height', -2, 2),
      E('brows.style', 'Brows', ['none', 'thin', 'thick', 'uni'], 'none'),
      E('glasses.style', 'Glasses', ['none', 'round', 'square', 'big', 'shades'], 'none'),
      E('nose.style', 'Nose', ['none', 'dot', 'button', 'snout', 'long'], 'none'),
      E('mouth.rest', 'Resting mouth', ['smile', 'flat', 'smirk', 'grin', 'frown', 'o', 'wavy'], 'smile'),
      E('mouth.teeth', 'Teeth', ['none', 'gap', 'buck', 'three'], 'none'),
      I('mouth.y', 'Mouth height', -2, 2),
      E('face.cheeks', 'Cheeks', ['none', 'blush', 'freckles'], 'none'),
      E('face.extra', 'Extra', ['none', 'moustache', 'beard', 'stubble', 'wrinkles'], 'none')]},
    { group: 'Held', fields: [
      E('held', 'Holds', ['none', 'pacifier', 'bottle', 'book', 'pen', 'clipboard', 'screwdriver', 'cane', 'spoon',
                          'flower', 'ball', 'drumsticks', 'watercan', 'key', 'plank'], 'none')]},
    /* COLOUR is for the full-colour Tamagotchi only (paintColour); the LCD has no hue.
       Empty means derived: skin from skin.tone, hair from hair.tone, and so on. */
    { group: 'Colour', fields: [
      C('colour.skin', 'Skin'), C('colour.hair', 'Hair'), C('colour.outfit', 'Outfit'),
      C('colour.outfit2', 'Under / trim'), C('colour.pattern', 'Pattern'), C('colour.hat', 'Hat'),
      C('colour.ties', 'Ties / bows'), C('colour.glasses', 'Glasses'), C('colour.shoes', 'Shoes')]},
    { group: 'Personality', fields: [
      E('personality.temperament', 'Temperament', ['bold', 'anxious', 'bossy', 'calm', 'cheerful', 'grumpy', 'curious',
                                                   'dreamy', 'stern', 'sly', 'fussy', 'gentle'], 'calm'),
      E('personality.rest', 'At rest', EMOTIONS, 'neutral'),
      E('personality.poke', 'When poked', EMOTIONS, 'surprised'),
      E('personality.pet', 'When petted', EMOTIONS, 'happy'),
      I('personality.energy', 'Energy', 0, 10, 5),
      I('personality.blink', 'Blink rate', 0, 10, 5),
      I('personality.sociable', 'Turns to neighbours', 0, 10, 5)]}
  ];
  /* EXPRESSIONS: how THIS character does each emotion. Every part defaults to
     'auto' (the emotion's own look, in EMO below); setting one is how a bossy
     character's happy differs from an anxious one's. */
  var EXPR_PARTS = {
    eye:   ['auto', 'dot', 'round', 'big', 'narrow', 'lashes', 'closed', 'happy', 'laugh', 'squeeze', 'wide', 'tiny', 'heart', 'lidded', 'angry', 'star', 'sly', 'wink'],
    mouth: ['auto', 'smile', 'wide', 'flat', 'smirk', 'grin', 'frown', 'o', 'laugh', 'wail', 'wavy', 'tiny', 'grit', 'pout', 'shout', 'tongue'],
    brow:  ['auto', 'none', 'thin', 'thick', 'angry', 'sad', 'up'],
    arms:  ['auto', 'down', 'up', 'out', 'hips', 'wave', 'cross'],
    pose:  ['auto', 'still', 'bob', 'hop', 'shake', 'jitter', 'slump'],
    fx:    ['auto', 'none', 'tear', 'tears', 'vein', 'bang', 'sweat', 'shiver', 'zzz', 'heart', 'sparkle', 'question', 'steam', 'note']
  };
  EMOTIONS.forEach(function (emo) {
    SCHEMA.push({ group: 'Expression · ' + emo, expression: emo, fields: Object.keys(EXPR_PARTS).map(function (part) {
      return E('expr.' + emo + '.' + part, part, EXPR_PARTS[part], 'auto');
    }) });
  });
  var HEX = /^#[0-9a-fA-F]{6}$/;
  var FIELDS = {};
  SCHEMA.forEach(function (g) { g.fields.forEach(function (f) { FIELDS[f.key] = f; }); });

  function getKey(o, key) {
    var p = key.split('.'), v = o;
    for (var i = 0; i < p.length; i++) { if (v == null) return undefined; v = v[p[i]]; }
    return v;
  }
  function setKey(o, key, val) {
    var p = key.split('.'), v = o;
    for (var i = 0; i < p.length - 1; i++) { if (typeof v[p[i]] !== 'object' || v[p[i]] === null) v[p[i]] = {}; v = v[p[i]]; }
    v[p[p.length - 1]] = val;
  }

  /* Every field present and in range; unknown keys are reported, never kept. */
  function normalize(spec) {
    var out = {};
    Object.keys(FIELDS).forEach(function (k) {
      var f = FIELDS[k], v = spec ? getKey(spec, k) : undefined;
      if (f.type === 'enum') v = f.options.indexOf(v) >= 0 ? v : f.default;
      else if (f.type === 'color') v = HEX.test(v) ? v.toLowerCase() : '';
      else v = typeof v === 'number' && isFinite(v) ? Math.max(f.min, Math.min(f.max, Math.round(v))) : f.default;
      setKey(out, k, v);
    });
    out.overrides = spec && spec.overrides && typeof spec.overrides === 'object' ? spec.overrides : {};
    return out;
  }
  var META = { name: 1, profile: 1, overrides: 1, notes: 1 };
  function validate(spec) {
    var errs = [];
    (function walk(o, pre) {
      Object.keys(o).forEach(function (k) {
        var key = pre ? pre + '.' + k : k;
        if (!pre && META[k]) return;
        var v = o[k];
        if (v && typeof v === 'object' && !Array.isArray(v) && !FIELDS[key]) return walk(v, key);
        var f = FIELDS[key];
        if (!f) return errs.push('unknown parameter ' + key);
        if (f.type === 'enum' && f.options.indexOf(v) < 0) errs.push(key + ': ' + JSON.stringify(v) + ' is not one of ' + f.options.join('|'));
        if (f.type === 'color' && v !== '' && !HEX.test(v)) errs.push(key + ': ' + JSON.stringify(v) + ' is not #rrggbb');
        if (f.type === 'int' && (typeof v !== 'number' || v < f.min || v > f.max || v !== Math.round(v))) errs.push(key + ': ' + v + ' outside ' + f.min + '..' + f.max);
      });
    })(spec || {}, '');
    Object.keys((spec && spec.overrides) || {}).forEach(function (k) {
      var p = k.split('/');
      if (p.length !== 3 || (p[0] !== '*' && FACINGS.indexOf(p[0]) < 0) || (p[1] !== '*' && EMOTIONS.indexOf(p[1]) < 0) ||
          (p[2] !== '*' && !(+p[2] >= 0 && +p[2] < FRAMES))) errs.push('override key ' + k + ' is not facing/emotion/frame');
      (spec.overrides[k] || []).forEach(function (px) {
        if (!Array.isArray(px) || px.length !== 3 || px[0] < 0 || px[0] >= N || px[1] < 0 || px[1] >= N || !(px[2] >= 0 && px[2] <= 7 && px[2] === Math.round(px[2])))
          errs.push('override ' + k + ' has a bad pixel ' + JSON.stringify(px));
      });
    });
    return errs;
  }

  /* ── RASTER AND MASKS ───────────────────────────────────────────────────── */
  function mask() { return new Uint8Array(N * N); }
  function ellipse(m, cx, cy, rx, ry) {
    for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
      var dx = (x + 0.5 - cx) / rx, dy = (y + 0.5 - cy) / ry;
      if (dx * dx + dy * dy <= 1.0001) m[y * N + x] = 1;
    }
    return m;
  }
  function rect(m, x0, y0, x1, y1) {
    for (var y = Math.max(0, y0); y <= Math.min(N - 1, y1); y++)
      for (var x = Math.max(0, x0); x <= Math.min(N - 1, x1); x++) m[y * N + x] = 1;
    return m;
  }
  function clip(m, fn) { for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) if (m[y * N + x] && !fn(x, y)) m[y * N + x] = 0; return m; }
  function union(a, b) { var m = mask(); for (var i = 0; i < N * N; i++) m[i] = a[i] || b[i] ? 1 : 0; return m; }
  function minus(a, b) { var m = mask(); for (var i = 0; i < N * N; i++) m[i] = a[i] && !b[i] ? 1 : 0; return m; }
  function edge(m) {
    var e = mask();
    for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
      if (!m[y * N + x]) continue;
      if (x === 0 || y === 0 || x === N - 1 || y === N - 1 || !m[y * N + x - 1] || !m[y * N + x + 1] || !m[(y - 1) * N + x] || !m[(y + 1) * N + x]) e[y * N + x] = 1;
    }
    return e;
  }
  function fill(R, m, v, mt) { var t = mt === undefined ? matOf(v) : mt; for (var i = 0; i < N * N; i++) if (m[i]) { R[i] = v; if (ML) ML[i] = t; } }
  /* A shape: its inside in `tone` (material cur.fill), its edge in ink (cur.edge). */
  function shape(R, m, tone) { fill(R, m, tone, cur.fill); fill(R, edge(m), INK, cur.edge); }
  function put(R, x, y, v, mt) { x = Math.round(x); y = Math.round(y); if (x >= 0 && y >= 0 && x < N && y < N) { R[y * N + x] = v; if (ML) ML[y * N + x] = mt === undefined ? matOf(v) : mt; } }
  /* Stamps: '#' ink, '+' mid, 'o' off (erases), '.' leaves what is there. */
  function stamp(R, rows, x0, y0, flip) {
    x0 = Math.round(x0); y0 = Math.round(y0);
    for (var r = 0; r < rows.length; r++) for (var c = 0; c < rows[r].length; c++) {
      var ch = rows[r][flip ? rows[r].length - 1 - c : c];
      if (ch === '#') put(R, x0 + c, y0 + r, INK);
      else if (ch === '+') put(R, x0 + c, y0 + r, MID);
      else if (ch === 'o') put(R, x0 + c, y0 + r, OFF);
      else if (ch === 's') put(R, x0 + c, y0 + r, curSkin, MAT.SKIN);
      else if (ch === 'm') put(R, x0 + c, y0 + r, curMark, cur.mark);
    }
  }
  /* Eye whites: on the darkest skin, a one-pixel light surround around each eye, laid only
     over skin, so the eyes read as they do in the reference art. Never over hair, glasses
     or outline. */
  function eyeWhites(R, rows, x0, y0, flip) {
    x0 = Math.round(x0); y0 = Math.round(y0);
    var m = mask();
    for (var r = 0; r < rows.length; r++) for (var c = 0; c < rows[r].length; c++) {
      var ch = rows[r][flip ? rows[r].length - 1 - c : c], x = x0 + c, y = y0 + r;
      if ((ch === '#' || ch === '+') && x >= 0 && y >= 0 && x < N && y < N) m[y * N + x] = 1;
    }
    for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
      if (m[y * N + x]) continue;
      var near = (x > 0 && m[y * N + x - 1]) || (x < N - 1 && m[y * N + x + 1]) || (y > 0 && m[(y - 1) * N + x]) || (y < N - 1 && m[(y + 1) * N + x]);
      if (near && R[y * N + x] === curSkin) { R[y * N + x] = OFF; if (ML) ML[y * N + x] = MAT.SCLERA; }
    }
  }

  /* Centred on (cx, cy): the stamp's middle column lands on cx. */
  function stampC(R, rows, cx, cy, flip) {
    var w = rows[0].length, h = rows.length;
    stamp(R, rows, Math.round(cx - w / 2), Math.round(cy - h / 2), flip);
  }
  function mirror(R) {
    var o = new Uint8Array(N * N);
    for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) o[y * N + x] = R[y * N + (N - 1 - x)];
    return o;
  }
  function mirrorL(L) { return { R: mirror(L.R), M: mirror(L.M) }; }
  function shift(R, dx, dy) {
    var o = new Uint8Array(N * N);
    for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
      var sx = x - dx, sy = y - dy;
      if (sx >= 0 && sy >= 0 && sx < N && sy < N) o[y * N + x] = R[sy * N + sx];
    }
    return o;
  }
  function tone(t) { return t === 'ink' ? INK : t === 'mid' ? MID : OFF; }

  /* ── STAMPS ──────────────────────────────────────────────────────────────
     Eyes are drawn as the figure's LEFT eye (outer edge on the left); the right
     eye is the same stamp mirrored. Mouths are drawn centred. */
  var EYE = {
    dot:    ['##', '##'],
    round:  ['.##.', '####', '####', '.##.'],
    big:    ['.##.', '#o##', '####', '####', '.##.'],
    narrow: ['####'],
    lashes: ['#..#.', '.##..', '####.', '####.', '.##..'].map(function (r) { return r.slice(0, 4); }),
    closed: ['####'],
    happy:  ['.##.', '#..#'],
    laugh:  ['##..', '..##', '##..'],
    squeeze:['#..#', '.##.'],
    wide:   ['.##.', '#..#', '#..#', '.##.'],
    tiny:   ['#'],
    heart:  ['##.##', '#####', '.###.', '..#..'],
    lidded: ['####', '.###'],
    angry:  ['##..', '###.', '####'],
    star:   ['.#.', '###', '.#.'],
    sly:    ['####', '..##'],
    side:   ['##', '##']
  };
  var MOUTH = {
    smile: ['#....#', '.####.'],
    wide:  ['#......#', '.######.'],
    flat:  ['####'],
    smirk: ['.....#', '#####.'],
    grin:  ['######', '#oooo#', '.####.'],
    frown: ['.####.', '#....#'],
    o:     ['.##.', '#..#', '.##.'],
    laugh: ['######', '##++##', '.####.'],
    wail:  ['.####.', '##++##', '##++##', '.####.'],
    wavy:  ['#.#.#.', '.#.#.#'],
    tiny:  ['##'],
    grit:  ['######', '#o#o##', '######'],
    pout:  ['.##.', '.##.'],
    shout: ['########', '#++++++#', '.######.'],
    tongue:['######', '.#++#.', '..++..']
  };
  var BROW = {
    thin:  ['###'],
    thick: ['####', '####'],
    angry: ['##..', '..##'],
    sad:   ['..##', '##..']
  };
  var FX = {
    zbig:  ['####', '..#.', '.#..', '####'],
    zsmall:['###', '.#.', '###'],
    heart: ['##.##', '#####', '.###.', '..#..'],
    heart2:['#.#', '###', '.#.'],
    vein:  ['#.#', '.#.', '#.#'],
    bang:  ['#', '#', '#', '.', '#'],
    sweat: ['.+', '++', '++'],
    shiver:['#', '.', '#'],
    sparkle:['.#.', '#.#', '.#.'],
    question:['###', '..#', '.##', '...', '.#.'],
    steam: ['.#.', '#.#'],
    note:  ['.##', '.#.', '.#.', '##.', '##.']
  };

  /* ── BUILDS AND HEADS ──────────────────────────────────────────────────── */
  var BUILD = {                 // torso height, torso half-width, head scale
    baby:  { torso: 5, half: 5, head: 1.06 },
    kid:   { torso: 6, half: 5, head: 1.00 },
    teen:  { torso: 7, half: 5, head: 0.94 },
    adult: { torso: 8, half: 6, head: 0.90 },
    elder: { torso: 7, half: 6, head: 0.90 }
  };
  var HEAD = { round: [1, 1], oval: [0.9, 1.1], wide: [1.12, 0.9], long: [0.86, 1.18], bean: [1.05, 0.95] };

  /* EMOTION -> face and pose. `eye` null keeps the character's own eye style. */
  var EMO = {
    neutral:  { eye: null,      brow: null,    mouth: null,    arms: 'down', fx: null },
    happy:    { eye: 'happy',   brow: null,    mouth: 'smile', arms: 'up',   fx: null,     bob: -1 },
    laugh:    { eye: 'laugh',   brow: null,    mouth: 'laugh', arms: 'up',   fx: null,     bob: -1, shake: 1 },
    sad:      { eye: null,      brow: 'sad',   mouth: 'frown', arms: 'down', fx: 'tear',   slump: 1 },
    cry:      { eye: 'squeeze', brow: 'sad',   mouth: 'wail',  arms: 'up',   fx: 'tears',  shake: 1 },
    angry:    { eye: null,      brow: 'angry', mouth: 'grit',  arms: 'out',  fx: 'vein',   shake: 1 },
    surprised:{ eye: 'wide',    brow: 'up',    mouth: 'o',     arms: 'up',   fx: 'bang',   bob: -1 },
    scared:   { eye: 'tiny',    brow: 'sad',   mouth: 'wavy',  arms: 'up',   fx: 'shiver', shake: 1 },
    sleepy:   { eye: 'closed',  brow: null,    mouth: 'tiny',  arms: 'down', fx: 'zzz',    slump: 1 },
    love:     { eye: 'heart',   brow: null,    mouth: 'smile', arms: 'up',   fx: 'heart',  bob: -1 },
    smug:     { eye: 'lidded',  brow: null,    mouth: 'smirk', arms: 'out',  fx: null },
    nervous:  { eye: null,      brow: 'sad',   mouth: 'wavy',  arms: 'down', fx: 'sweat',  jitter: 1 }
  };

  /* ── THE RENDER ─────────────────────────────────────────────────────────── */
  /* COMPILE ONCE for a caller that draws the same character many times (the site's
     grid): a compiled spec skips normalize() on every render. A spec being edited
     (the studio) is passed raw and normalized each time, so no edit goes stale. */
  var COMPILED = typeof WeakSet !== 'undefined' ? new WeakSet() : null;
  function compile(spec) { var n = normalize(spec); if (COMPILED) COMPILED.add(n); return n; }
  function render(specIn, st) { return renderLayers(specIn, st).R; }
  /* Both layers: R, the tones (what the LCD shows), and M, the materials (what full
     colour is drawn from). render() is R alone. */
  function renderLayers(specIn, st) {
    st = st || {};
    var s = COMPILED && COMPILED.has(specIn) ? specIn : normalize(specIn);
    var emotion = EMOTIONS.indexOf(st.emotion) >= 0 ? st.emotion : 'neutral';
    var facing = FACINGS.indexOf(st.facing) >= 0 ? st.facing : 'down';
    var frame = ((st.frame | 0) % FRAMES + FRAMES) % FRAMES;
    var L;
    if (facing === 'left') {
      L = mirrorL(draw(s, emotion, 'right', frame, st));
      applyOverrides(L, s.overrides, 'left', emotion, frame, mirrorOverrides(s.overrides));
    } else {
      L = draw(s, emotion, facing, frame, st);
      applyOverrides(L, s.overrides, facing, emotion, frame);
    }
    return L;
  }

  /* A right-facing override also shapes the left view, mirrored, unless the left
     view has an override of its own for the same slot. */
  function mirrorOverrides(ov) {
    var m = {};
    Object.keys(ov).forEach(function (k) {
      var p = k.split('/');
      if (p[0] === 'right') m[k] = ov[k].map(function (px) { return [N - 1 - px[0], px[1], px[2]]; });
    });
    return m;
  }
  /* A painted pixel says a tone; its material is read from that tone. */
  function overrideMat(v) { return v === INK ? MAT.INK : v === MID ? MAT.MID : v === OFF ? MAT.BG : MAT.SKIN; }
  function applyOverrides(L, ov, facing, emotion, frame, inherited) {
    var R = L.R;
    var keys = ['*/*/*', '*/*/' + frame, '*/' + emotion + '/*', '*/' + emotion + '/' + frame,
                facing + '/*/*', facing + '/*/' + frame, facing + '/' + emotion + '/*', facing + '/' + emotion + '/' + frame];
    keys.forEach(function (k) {
      var rk = k.replace(/^left\//, 'right/');
      var list = ov[k] || (inherited && facing === 'left' ? inherited[rk] : null);
      (list || []).forEach(function (px) { R[px[1] * N + px[0]] = px[2]; L.M[px[1] * N + px[0]] = overrideMat(px[2]); });
    });
    return L;
  }

  function draw(s, emotion, facing, frame, st) {
    var R = new Uint8Array(N * N);
    ML = new Uint8Array(N * N);
    var base = EMO[emotion], x = (s.expr && s.expr[emotion]) || {};
    var e = { eye: base.eye, brow: base.brow, mouth: base.mouth, arms: base.arms, fx: base.fx,
              bob: base.bob, shake: base.shake, slump: base.slump, jitter: base.jitter };
    if (x.eye && x.eye !== 'auto') e.eye = x.eye;
    if (x.mouth && x.mouth !== 'auto') e.mouth = x.mouth;
    if (x.brow && x.brow !== 'auto') e.brow = x.brow;
    if (x.arms && x.arms !== 'auto') e.arms = x.arms;
    if (x.fx && x.fx !== 'auto') e.fx = x.fx === 'none' ? null : x.fx;
    if (x.pose && x.pose !== 'auto') {
      e.bob = e.shake = e.slump = e.jitter = 0;
      if (x.pose === 'bob') e.bob = -1; if (x.pose === 'hop') e.bob = -2; if (x.pose === 'shake') e.shake = 1;
      if (x.pose === 'jitter') e.jitter = 1; if (x.pose === 'slump') e.slump = 1;
    }
    var b = BUILD[s.body.build], hs = HEAD[s.head.shape];
    /* The skin, and the marks drawn on it: light marks (tears, blush) turn light on
       darker skin, and fine lines (stubble, wrinkles, beard) turn ink, so both read. */
    curSkin = skinValue(s.skin.tone);
    curMark = s.skin.tone >= 3 ? OFF : MID;
    curLine = s.skin.tone >= 3 ? INK : MID;
    var side = facing === 'right', back = facing === 'up';

    /* Pose: whole-figure bob, head slump, shake, jitter — all per frame. */
    var dy = 0, headDy = 0, dx = 0;
    if (frame === 1) {
      if (e.bob) dy = e.bob;
      else if (!e.slump) headDy = 1;          // breathing: the head settles on frame 1
      if (e.shake) dx = 1;
      if (e.jitter) dx = -1;
    }
    if (e.slump) headDy += 1;
    if (emotion === 'scared' && !(x.pose && x.pose !== 'auto')) dx = frame ? 1 : -1;

    /* Geometry. x runs 0..31 with the figure centred on the line x = 16, so a
       front view is exactly symmetric under x -> 31 - x. Feet stand on row 30. */
    var cx = 16;
    var torsoBottom = 28, torsoTop = torsoBottom - b.torso + 1;
    var half = b.half + s.body.width;
    var scale = b.head * (1 + 0.06 * s.head.size);
    var rx = Math.max(6, Math.round(10 * hs[0] * scale));
    var ry = Math.max(6, Math.round(9 * hs[1] * scale));
    var headBottom = torsoTop + 2;
    var cy = headBottom - ry + headDy;
    var eyeY = Math.round(cy + ry * 0.08) + s.eyes.y;
    var ex = Math.round(rx * 0.42) + s.eyes.spacing;
    var mouthY = Math.round(cy + ry * 0.52) + s.mouth.y;
    var faceX = side ? cx + Math.round(rx * 0.38) : cx;

    var headM = ellipse(mask(), cx, cy, rx, ry);
    var hairTone = tone(s.hair.tone);

    /* 1. things BEHIND the head: tall ears, long and wide hair */
    mat({ fill: MAT.SKIN });
    drawEarsBack(R, s, cx, cy, rx, ry, side, back);
    var hairBack = hairBackMask(s, cx, cy, rx, ry, side, back, torsoTop);
    mat({ fill: MAT.HAIR });
    if (hairBack) shape(R, hairBack, hairTone);

    /* 2. the body */
    mat({ fill: MAT.OUTFIT, off: MAT.OUTFIT2 });
    drawBody(R, s, e, cx, torsoTop, torsoBottom, half, side, back, frame);

    /* 3. the head, then side ears over its edge */
    mat({ fill: MAT.SKIN });
    shape(R, headM, curSkin);
    drawEarsSide(R, s, cx, cy, rx, ry, eyeY, side, back);

    /* 4. hair on the head, and the hat over it */
    mat({ fill: MAT.HAIR, ink: MAT.HAIR });
    drawHairFront(R, s, cx, cy, rx, ry, side, back, headM, hairTone, eyeY);
    mat({ fill: MAT.HAT, ink: MAT.HAT, mid: MAT.HAT, off: MAT.HAT_DETAIL });
    drawHat(R, s, cx, cy, rx, ry, side, back);

    /* 5. the face */
    mat({ ink: MAT.EYES, off: MAT.WHITE, mid: MAT.TONGUE, fill: MAT.SKIN });
    if (!back) drawFace(R, s, e, emotion, faceX, cx, cy, rx, ry, eyeY, ex, mouthY, side, st, frame);

    /* 6. what they hold */
    mat({ ink: MAT.HELD, mid: MAT.HELD_LIGHT, off: MAT.WHITE, fill: MAT.HELD });
    drawHeld(R, s, cx, torsoTop, half, side, back, mouthY, faceX, e);

    /* 7. the emotion's effects around the head */
    mat({ ink: MAT.FX, mid: MAT.SWEAT, mark: MAT.TEAR });
    if (e.fx) drawFx(R, e.fx, cx, cy, rx, ry, eyeY, ex, faceX, side, back, frame);

    var Mo = ML; ML = null;
    if (dx || dy) { R = shift(R, dx, dy); Mo = shift(Mo, dx, dy); }
    return { R: R, M: Mo };
  }

  function drawEarsBack(R, s, cx, cy, rx, ry, side, back) {
    var st = s.ears.style, m;
    function pair(ox, oy, erx, ery, inner) {
      var xs = side ? [cx - Math.round(rx * 0.2)] : [cx - ox, cx + ox];
      xs.forEach(function (x) {
        m = ellipse(mask(), x, cy - ry + oy, erx, ery);
        shape(R, m, curSkin);
        if (inner && !back) { var im = ellipse(mask(), x, cy - ry + oy + 1, Math.max(1, erx - 1.5), Math.max(1, ery - 2)); fill(R, minus(im, edge(m)), s.skin.tone >= 3 ? skinValue(s.skin.tone - 2) : MID, MAT.SKIN_SHADE); }
      });
    }
    if (st === 'aardvark') pair(rx * 0.45, -3, 2.6, 5, true);
    else if (st === 'rabbit') pair(rx * 0.35, -6, 2.2, 7.5, true);
    else if (st === 'bear') pair(rx * 0.72, 1, 3, 3, true);
    else if (st === 'mouse') pair(rx * 0.78, 0, 4.2, 4.2, true);
    else if (st === 'cat') {
      var xs = side ? [cx - 2] : [cx - rx * 0.55, cx + rx * 0.55];
      xs.forEach(function (x, i) {
        var tri = mask(), top = Math.round(cy - ry - 4);
        for (var y = top; y <= cy - ry + 3; y++) {
          var w = Math.round((y - top) * 0.7);
          rect(tri, Math.round(x - w), y, Math.round(x + w) - 1, y);
        }
        shape(R, tri, curSkin);
      });
    }
  }
  function drawEarsSide(R, s, cx, cy, rx, ry, eyeY, side, back) {
    var st = s.ears.style;
    if (['small', 'big', 'floppy'].indexOf(st) < 0) return;
    var er = st === 'big' ? 2.6 : 2;
    if (side && st !== 'floppy') {
      var ear = st === 'big' ? ['.##.', '#ss#', '#ss#', '#ss#', '.##.'] : ['.##.', '#ss#', '#ss#', '.##.'];
      stamp(R, ear, cx - Math.round(rx * 0.35) - 2, eyeY - Math.floor(ear.length / 2) + 1);
      return;
    }
    var xs = side ? [cx - 2] : [cx - rx - 0.5, cx + rx + 0.5];
    xs.forEach(function (x, i) {
      if (st === 'floppy') {
        var m = ellipse(mask(), x + (side ? 0 : (i ? -1 : 1)), cy - ry * 0.35, 2.5, 4.5);
        cur.fill = MAT.SKIN_SHADE; shape(R, m, curSkin === OFF ? MID : curSkin); cur.fill = MAT.SKIN;
      } else {
        var m2 = ellipse(mask(), x, eyeY + 0.5, er, er + 0.6);
        if (!side) clip(m2, function (px) { return i ? px >= cx + rx - 2 : px <= cx - rx + 1; });
        shape(R, m2, curSkin);
        if (side && !back) put(R, x, eyeY + 1, INK, MAT.INK);
      }
    });
  }

  function hairBackMask(s, cx, cy, rx, ry, side, back, torsoTop) {
    var st = s.hair.style, v = s.hair.volume, sh = side ? -2 : 0, m;
    if (st === 'afro') return ellipse(mask(), cx + (side ? -1 : 0), cy - ry * 0.3, Math.min(13, rx + 1.5 + v * 0.7), Math.min(12, ry * 0.85 + 2 + v * 0.6));
    if (st === 'bob') return clip(ellipse(mask(), cx + sh, cy - 1, rx + 1.5 + v * 0.5, ry + 1 + v * 0.5), function (x, y) { return y <= cy + ry * 0.55; });
    if (st === 'long') { m = ellipse(mask(), cx + sh, cy, rx + 1.5 + v * 0.5, ry + 1); return union(m, rect(mask(), cx + sh - rx - 1, cy, cx + sh + rx, torsoTop + 3 + v)); }
    if (st === 'pigtails' && !side) {
      m = mask();
      [-1, 1].forEach(function (d) { ellipse(m, cx + d * (rx + 2.5), cy - ry * 0.35, 2.4 + v * 0.3, 3.2 + v * 0.4); });
      return m;
    }
    if (st === 'pigtails' && side) return ellipse(mask(), cx - rx - 1.5, cy - ry * 0.35, 2.4, 3.2);
    if (st === 'braids') {
      m = mask();
      (side ? [-1] : [-1, 1]).forEach(function (d) { rect(m, Math.round(cx + d * (rx - 1)) - 1, Math.round(cy), Math.round(cx + d * (rx - 1)), torsoTop + 2 + v); });
      return m;
    }
    if (st === 'bun') return ellipse(mask(), cx + (side ? -3 : 0), cy - ry - 1.5, 3 + v * 0.4, 2.6 + v * 0.3);
    if (st === 'tall') {             // piled high, but never past row 3, so a hop keeps it on the screen
      var tcy = cy - ry * 0.55 - 1 - v * 0.6, try_ = Math.min(ry * 0.75 + 1 + v * 0.6, tcy - 3);
      return ellipse(mask(), cx + (side ? -2 : 0), tcy, rx * 0.8 + v * 0.3, try_);
    }
    return null;
  }

  function drawHairFront(R, s, cx, cy, rx, ry, side, back, headM, t, eyeY) {
    var st = s.hair.style, v = s.hair.volume, top = cy - ry;
    if (st === 'bald') return;
    if (st === 'strands') {
      var n = s.hair.count, span = Math.min(rx * 1.1, 2 + n * 2);
      for (var i = 0; i < n; i++) {
        var x = Math.round(cx - span / 2 + (n === 1 ? span / 2 : i * span / (n - 1))) - (side ? 2 : 0);
        var lean = x < cx - 1 ? -1 : x > cx ? 1 : 0;
        put(R, x, Math.round(top) - 1, INK); put(R, x + lean, Math.round(top) - 2, INK); put(R, x + lean * 2, Math.round(top) - 3, INK);
      }
      return;
    }
    if (st === 'sides') {
      (side ? [-1] : [-1, 1]).forEach(function (d) {
        var m = ellipse(mask(), cx + d * (rx - 0.5) + (side ? -1 : 0), cy - ry * 0.05, 2.2 + v * 0.3, 3 + v * 0.3);
        shape(R, m, t);
      });
      return;
    }
    /* The cap: the top of the head, down to a hairline shaped by the fringe. */
    var line = cy - ry * (back ? -0.25 : 0.38) + (st === 'afro' ? 1 : 0);
    var cap = ellipse(mask(), cx + (side ? -1 : 0), cy - 0.5, rx + 0.5 + Math.max(0, v) * 0.35, ry + 0.5 + Math.max(0, v) * 0.35);
    var fr = s.hair.fringe;
    clip(cap, function (x, y) {
      var l = line;
      if (back) return y <= l;
      if (side) return y <= (fr === 'eyes' ? eyeY + 1 : l) + (x < cx ? 3 : 0);      // side view: hair falls lower at the back
      if (fr === 'none') l -= 2;
      if (fr === 'side') l += (x - cx) * 0.35;
      if (fr === 'ragged') l += (x % 2) ? 1 : 0;
      if (fr === 'parted') l -= Math.max(0, 2 - Math.abs(x + 0.5 - cx)) * 1.2;
      if (fr === 'eyes') l = eyeY + 1 + ((x % 3) ? 0 : 1);
      return y <= l;
    });
    if (st === 'mohawk') clip(cap, function (x) { return Math.abs(x + 0.5 - cx) < 2.5; });
    if (st === 'flattop') { rect(cap, Math.round(cx - rx * 0.7), Math.round(top) - 2 - v, Math.round(cx + rx * 0.7) - 1, Math.round(top) + 1); }
    if (st === 'mohawk') rect(cap, cx - 2, Math.round(top) - 3 - v, cx + 1, Math.round(top));
    shape(R, cap, t);
    if (st === 'spiky') {
      var y0 = Math.round(top);
      for (var x = Math.round(cx - rx + 2); x < cx + rx - 2; x += 3) stamp(R, ['.#.', '###'], x - 1 + (side ? -1 : 0), y0 - 2 - (x % 2 && v > 0 ? 1 : 0));
    }
    if (st === 'curls') {
      for (var xx = Math.round(cx - rx); xx < cx + rx; xx += 2) put(R, xx, Math.round(line) + 1, t === OFF ? INK : t, MAT.HAIR_LIGHT);
    }
    if (st === 'cornrows' && t !== OFF) {
      for (var cxr = Math.round(cx - rx + 2); cxr < cx + rx - 1; cxr += 3)
        for (var y = Math.round(top) + 1; y < line; y++) if (cap[y * N + cxr] && !edge(cap)[y * N + cxr]) put(R, cxr, y, OFF, MAT.SKIN_SHADE);
    }
    if (st === 'pigtails' && s.hair.ties === 'bows' && !back) {
      (side ? [-1] : [-1, 1]).forEach(function (d) { cur.ink = MAT.TIES; stampC(R, ['#.#', '###', '#.#'], cx + d * (rx + 0.5) + (side ? -1 : 0), cy - ry * 0.45); cur.ink = MAT.HAIR; });
    }
  }

  function drawHat(R, s, cx, cy, rx, ry, side, back) {
    var st = s.hat.style, t = tone(s.hat.tone), top = Math.round(cy - ry);
    if (st === 'none') return;
    var sx = side ? -1 : 0, m;
    if (st === 'sock') {
      m = ellipse(mask(), cx + sx, cy - ry * 0.35, rx + 0.5, ry * 0.75);
      clip(m, function (x, y) { return y <= cy - ry * 0.3; });
      var ht = Math.max(1, Math.min(5, top - 4));        // as tall as the tile allows, leaving room to jump
      rect(m, Math.round(cx + sx - rx * 0.75), top - ht, Math.round(cx + sx + rx * 0.75) - 1, top + 1);
      var cap2 = ellipse(mask(), cx + sx, top - ht, rx * 0.75, 1.5);
      m = union(m, cap2);
      shape(R, m, t);
      if (!back) (side ? [-1] : [-1, 1]).forEach(function (d) {      // the white side stripes
        var x = Math.round(cx + sx + d * rx * 0.55) - (d > 0 ? 1 : 0);
        for (var y = top - ht + 1; y <= top + 1; y++) put(R, x, y, OFF);
      });
    } else if (st === 'cap') {
      m = clip(ellipse(mask(), cx + sx, cy - 1, rx + 0.5, ry + 0.5), function (x, y) { return y <= cy - ry * 0.3; });
      shape(R, m, t);
      var by = Math.round(cy - ry * 0.3);
      cur.ink = MAT.HAT_DETAIL;
      if (side) stamp(R, ['#######'], cx + 2, by);
      else if (!back) stamp(R, [new Array(Math.round(rx * 2) + 3).join('#')], Math.round(cx - rx - 1), by);
    } else if (st === 'beanie') {
      m = clip(ellipse(mask(), cx + sx, cy - 1, rx + 0.8, ry + 1), function (x, y) { return y <= cy - ry * 0.25; });
      shape(R, m, t);
      stampC(R, ['.#.', '###'], cx + sx, top - 2);
    } else if (st === 'bow') {
      stampC(R, ['##..##', '######', '##..##'].map(function (r) { return r.replace(/#/g, '#'); }), cx + (side ? -2 : 0), top - 1);
      put(R, cx - 1 + (side ? -2 : 0), top - 1, t === INK ? OFF : MID); put(R, cx + (side ? -2 : 0), top - 1, t === INK ? OFF : MID);
    } else if (st === 'headband') {
      var y = Math.round(cy - ry * 0.45);
      for (var x = 0; x < N; x++) {
        var dx = (x + 0.5 - cx) / (rx + 0.5), dyy = (y + 0.5 - cy) / (ry + 0.5);
        if (dx * dx + dyy * dyy <= 1) { put(R, x, y, t === OFF ? INK : t); put(R, x, y + 1, t === OFF ? INK : t); }
      }
    } else if (st === 'saucepan') {
      /* A pan worn as a hat: a flat slab resting on the crown, a little narrower
         than the head, and its handle out to one side. Side-on the handle points
         back, behind the head; left mirrors right as always. */
      var hw = Math.round(rx * 0.82), py = top;
      m = rect(mask(), cx + sx - hw, py - 3, cx + sx + hw - 1, py);
      shape(R, m, t);
      if (!back) for (var rim = cx + sx - hw + 1; rim < cx + sx + hw - 1; rim++) put(R, rim, py - 2, t === INK ? MID : INK, MAT.HAT_DETAIL);
      cur.ink = MAT.INK;
      if (side) stamp(R, ['##...', '.####'], cx + sx - hw - 5, py - 3);
      else stamp(R, ['...##', '####.'], cx + sx + hw, py - 3);
    } else if (st === 'crown') {
      stampC(R, ['#.#.#', '#####', '#####'], cx + sx, top - 1);
    }
  }

  function drawBody(R, s, e, cx, top, bottom, half, side, back, frame) {
    var t = tone(s.body.tone), o = s.body.outfit, m;
    var w = side ? half - 1 : half;
    var bx = side ? cx - 1 : cx;
    if (o === 'dress') {
      m = mask();
      for (var y = top; y <= bottom; y++) { var ww = w - 1 + Math.round((y - top) * 0.45); rect(m, bx - ww, y, bx + ww - 1, y); }
    } else {
      m = rect(mask(), bx - w, top, bx + w - 1, bottom);
      m[top * N + bx - w] = 0; m[top * N + bx + w - 1] = 0;          // round the shoulders
    }
    shape(R, m, t);
    var mid = t === MID ? OFF : MID, ink = t === INK ? OFF : INK;
    if (!back) {
      mat({ ink: MAT.INK, off: MAT.OUTFIT2, mid: MAT.PATTERN, fill: MAT.OUTFIT2 });
      if (o === 'diaper') { var dy0 = bottom - 2; rect(m = mask(), bx - w + 1, dy0, bx + w - 2, bottom - 1); fill(R, m, OFF); for (var x = bx - w; x < bx + w; x++) put(R, x, dy0 - 1, INK); if (!side) { put(R, bx - w + 1, dy0, INK); put(R, bx + w - 2, dy0, INK); } }
      if (o === 'collar') { if (side) put(R, bx + w - 2, top + 1, OFF); else stampC(R, ['o..o', '.oo.'], bx, top + 1); }
      if (o === 'hoodie') { stamp(R, [new Array(w * 2 - 1).join(ink === OFF ? 'o' : '#')], bx - w + 2, bottom - 2); }
      if (o === 'overalls') { put(R, bx - 2, top, ink); put(R, bx - 2, top + 1, ink); put(R, bx + 1, top, ink); put(R, bx + 1, top + 1, ink); rect(m = mask(), bx - 3, top + 2, bx + 2, bottom - 1); fill(R, edge(m), ink); }
      if (o === 'suit' && !side) { cur.ink = MAT.PATTERN; }
      if (o === 'suit' && !side) { stampC(R, ['o##o', '.oo.', '.##.', '.##.'].map(function (r) { return r; }), bx, top + 2); }
      if (o === 'vest') { for (var yv = top + 1; yv < bottom; yv++) put(R, bx, yv, ink); }
      if (o === 'jacket' && !side) { for (var yj = top + 1; yj <= bottom; yj++) { put(R, bx - 1, yj, OFF); put(R, bx, yj, OFF); put(R, bx - 2, yj, INK); put(R, bx + 1, yj, INK); } }
      if (o === 'apron') { rect(m = mask(), bx - w + 2, top + 2, bx + w - 3, bottom); fill(R, m, OFF); fill(R, edge(m), INK); }
      mat({ ink: MAT.PATTERN, mid: MAT.PATTERN, off: MAT.PATTERN });
      var em = s.body.emblem, ey = Math.round((top + bottom) / 2);
      if (em === 'star') stampC(R, ['.#.', '###', '.#.'].map(function (r) { return r.replace(/#/g, ink === OFF ? 'o' : '#'); }), bx, ey);
      if (em === 'planet') stampC(R, ['.##.', '####', '.##.'], bx, ey), put(R, bx - 3, ey, ink), put(R, bx + 2, ey - 1, ink);
      if (em === 'heart') stampC(R, ['#.#', '###', '.#.'], bx, ey);
      if (em === 'stripes') for (var ys = top + 1; ys < bottom; ys += 2) for (var xs = bx - w + 1; xs < bx + w - 1; xs++) put(R, xs, ys, mid);
      if (em === 'dots') for (var yd = top + 1; yd < bottom; yd += 2) for (var xd = bx - w + 1 + (yd % 2); xd < bx + w - 1; xd += 2) put(R, xd, yd, mid);
      if (em === 'pocket') { cur.ink = MAT.INK; stamp(R, ['###', '#.#'], bx + 1, ey); cur.ink = MAT.PATTERN; }
      if (em === 'tie') { stampC(R, ['##', '.#', '##', '##'].map(function (r) { return r; }), bx, top + 2); }
      if (em === 'bowtie') stampC(R, ['#.#', '###', '#.#'], bx, top + 1);
    }
    /* Arms: two stubs, their angle set by the emotion. */
    mat({});
    var ay = top + 1;
    var ARM = { down: [['#', '#'], 1], up: [['#', '#'], -2], out: [['##'], 0], hips: [['#.', '.#'], 0] };
    var a = e.arms;
    if (a === 'cross') {
      for (var xc = bx - w + 1; xc < bx + w - 1; xc++) put(R, xc, ay + 2, INK);
      if (!side) { put(R, bx - w, ay + 1, INK); put(R, bx + w - 1, ay + 1, INK); }
    } else if (!side) {
      var L = ARM[a === 'wave' ? 'down' : a] || ARM.down, Rt = ARM[a === 'wave' ? 'up' : a] || ARM.down;
      stamp(R, L[0], bx - w - L[0][0].length, ay + L[1]);
      stamp(R, Rt[0], bx + w, ay + Rt[1], true);
    } else {
      var S = ARM[a === 'wave' ? 'up' : a] || ARM.down;
      stamp(R, S[0], bx + w, ay + S[1], true);
    }
    /* Feet on row 30, alternating on frame 1 when walking side-on. */
    mat({ ink: MAT.SHOES });
    var f = s.body.feet, foot = f === 'boots' ? ['##', '##'] : f === 'shoes' ? ['###'] : ['##'];
    var fy = 30 - foot.length + 1;
    if (!side) {
      stamp(R, foot, bx - 3 - (foot[0].length - 2), fy);
      stamp(R, foot, bx + 1, fy);
      for (var yl = bottom + 1; yl < fy; yl++) { put(R, bx - 3, yl, INK, MAT.INK); put(R, bx + 2, yl, INK, MAT.INK); }
    } else {
      var step = frame ? 1 : 0;
      stamp(R, foot, bx - 2 - step, fy);
      stamp(R, foot, bx + 1 + step, fy);
      for (var yl2 = bottom + 1; yl2 < fy; yl2++) { put(R, bx - 2 - step, yl2, INK, MAT.INK); put(R, bx + 1 + step, yl2, INK, MAT.INK); }
    }
  }

  function drawFace(R, s, e, emotion, fx, cx, cy, rx, ry, eyeY, ex, mouthY, side, st, frame) {
    /* Eyes: the emotion's shape, or the character's own style at rest. */
    var hidden = s.hair.fringe === 'eyes' && s.hat.style === 'none';
    var whites = s.skin.tone >= 4;
    var eyeName = e.eye || s.eyes.style;
    if (st.blink && ['dot', 'round', 'big', 'narrow', 'lashes', 'wide', 'tiny', 'lidded', 'angry', 'sly', 'star'].indexOf(eyeName) >= 0) eyeName = 'closed';
    var wink = eyeName === 'wink';
    if (wink) eyeName = st.blink ? 'closed' : s.eyes.style;
    var eye = EYE[eyeName] || EYE.round;
    cur.ink = eyeName === 'heart' ? MAT.FX_RED : MAT.EYES;
    var look = st.look ? Math.max(-1, Math.min(1, Math.round(st.look))) : 0;
    var ew = eye[0].length, eh = eye.length;
    var eyeTop = eyeY - Math.floor(eh / 2);
    if (hidden) {
      /* the fringe covers the eyes: nothing to draw, and no brows either */
    } else if (side) {
      var se = wink ? EYE.closed : eye, sx0 = cx + Math.round(rx * 0.42) - Math.floor(ew / 2);
      if (whites) eyeWhites(R, se, sx0, eyeTop, true);
      stamp(R, se, sx0, eyeTop, true);
    } else {
      var lx = cx - ex - Math.round(ew / 2) + look;
      var re = wink ? EYE.closed : eye, rw = re[0].length;
      var rx0 = cx + ex - Math.round(rw / 2) + (rw % 2) + look, ry0 = eyeTop + (wink ? Math.floor(eh / 2) : 0);
      if (whites) { eyeWhites(R, eye, lx, eyeTop); eyeWhites(R, re, rx0, ry0, true); }
      stamp(R, eye, lx, eyeTop);
      stamp(R, re, rx0, ry0, true);
    }

    /* Brows. */
    cur.ink = MAT.BROW;
    var bs = s.brows.style, eb = e.brow;
    var browName = eb === 'angry' || eb === 'sad' || eb === 'thin' || eb === 'thick' ? eb : null;
    if (!hidden && eb !== 'none' && (bs !== 'none' || browName)) {
      var brow = browName ? BROW[browName] : BROW[bs === 'uni' ? 'thin' : bs];
      var by = eyeTop - 2 - (eb === 'up' ? 1 : 0) - (brow.length - 1);
      if (bs === 'uni' && !browName && !side) { for (var x = cx - ex - 2; x < cx + ex + 2; x++) put(R, x, by, INK, MAT.BROW); }
      else if (side) stamp(R, brow, fx + 1, by, true);
      else { stamp(R, brow, cx - ex - Math.round(brow[0].length / 2), by); stamp(R, brow, cx + ex - Math.round(brow[0].length / 2) + (brow[0].length % 2), by, true); }
    }

    /* Glasses frame the eyes; drawn after them, leaving the eye inside. */
    cur.ink = MAT.GLASSES;
    var g = s.glasses.style;
    if (g !== 'none' && !hidden) {
      var gw = g === 'big' ? 4 : 3, gh = g === 'big' ? 3 : 2;
      (side ? [cx + Math.round(rx * 0.42)] : [cx - ex, cx + ex]).forEach(function (gx) {
        var m = rect(mask(), Math.round(gx - gw), eyeY - gh, Math.round(gx + gw) - 1, eyeY + gh - (g === 'big' ? 0 : 1));
        if (g === 'round') { m[(eyeY - gh) * N + Math.round(gx - gw)] = 0; m[(eyeY - gh) * N + Math.round(gx + gw) - 1] = 0; m[(eyeY + gh - 1) * N + Math.round(gx - gw)] = 0; m[(eyeY + gh - 1) * N + Math.round(gx + gw) - 1] = 0; }
        if (g === 'shades') fill(R, m, INK); else fill(R, edge(m), INK);
      });
      if (!side) { put(R, cx - 1, eyeY - 1, INK); put(R, cx, eyeY - 1, INK); }
      else { for (var ax = cx - 3; ax < fx - gw + 2; ax++) put(R, ax, eyeY - 1, INK); }
    }

    /* Nose. */
    cur.ink = MAT.FACELINE; cur.fill = MAT.SKIN;
    var ns = s.nose.style, ny = eyeY + 3;
    if (ns === 'dot') put(R, side ? fx + 4 : cx - 1, ny, INK), (!side && put(R, cx, ny, INK));
    if (ns === 'button') stampC(R, ['.##.', '#..#'], side ? fx + 4 : cx, ny);
    if (ns === 'snout') {
      var sm = ellipse(mask(), side ? cx + rx - 1 : cx, ny + 1, side ? 4 : 4.5, 2.5);
      shape(R, sm, curSkin);
      if (!side) { put(R, cx - 2, ny + 1, INK, MAT.INK); put(R, cx + 1, ny + 1, INK, MAT.INK); } else put(R, cx + rx + 1, ny + 1, INK, MAT.INK);
      mouthY += 1;
    }
    if (ns === 'long') { if (side) stamp(R, ['###', '####'], cx + rx - 2, ny); else stampC(R, ['.##.', '####'], cx, ny); }

    /* Mouth: the emotion's, or the resting one; teeth only where there is room. */
    cur.ink = MAT.INK; cur.mid = MAT.TONGUE; cur.off = MAT.WHITE;
    var mName = e.mouth || s.mouth.rest;
    if (emotion === 'happy' && s.mouth.rest === 'grin') mName = 'grin';
    var mouth = MOUTH[mName] || MOUTH.smile;
    if (side) {
      var half = mouth.map(function (r) { return r.slice(Math.floor(r.length / 2)); });
      stamp(R, half, fx, mouthY - Math.floor(mouth.length / 2));
    } else {
      stampC(R, mouth, cx, mouthY);
      var t = s.mouth.teeth;
      if (mName === 'grin' && t === 'gap') { put(R, cx - 1, mouthY, INK); }
      if (mName === 'grin' && t === 'three') { put(R, cx - 2, mouthY, INK); put(R, cx, mouthY, INK); }
      if (t === 'buck' && ['smile', 'flat', 'smirk'].indexOf(mName) >= 0) stampC(R, ['#o#'], cx, mouthY + 1);
    }

    /* Cheeks and the rest. */
    cur.mark = MAT.BLUSH; cur.ink = MAT.FACELINE;
    var ch = s.face.cheeks, cyy = mouthY - 2;
    if (ch === 'blush' || emotion === 'love') (side ? [fx + 1] : [cx - ex - 2, cx + ex + 1]).forEach(function (x) { stamp(R, ['mm'], x, cyy); });
    if (ch === 'freckles') (side ? [fx] : [cx - ex - 1, cx + ex]).forEach(function (x) { stamp(R, ['#.#', '.#.'], x - 1, cyy); });
    var fe = s.face.extra;
    cur.ink = MAT.BROW;
    if (fe === 'moustache') { if (side) stamp(R, ['###'], fx + 2, mouthY - 1); else stampC(R, ['.####.', '##..##'], cx, mouthY - 1); }
    if (fe === 'beard') { var bm = clip(ellipse(mask(), cx + (side ? 2 : 0), cy + 1, rx - 0.5, ry + 0.5), function (x, y) { return y >= mouthY + 1; }); fill(R, minus(bm, edge(ellipse(mask(), cx, cy, rx, ry))), curLine, MAT.BROW); }
    if (fe === 'stubble') for (var sx2 = cx - 4; sx2 < cx + 4; sx2 += 2) put(R, sx2 + (side ? 3 : 0), mouthY + 2, curLine, MAT.FACELINE);
    if (fe === 'wrinkles') { (side ? [fx + 3] : [cx - ex - 1, cx + ex + 1]).forEach(function (x) { put(R, x, eyeY + 2, curLine, MAT.FACELINE); }); cur.ink = MAT.FACELINE; stampC(R, ['.###.'], cx + (side ? 2 : 0), eyeY - 4); }
  }

  function drawHeld(R, s, cx, top, half, side, back, mouthY, fx, e) {
    var h = s.held; if (h === 'none' || back) return;
    if (h === 'pacifier') { stampC(R, ['.##.', '#++#', '.##.'], side ? fx + 2 : cx, mouthY); return; }
    var hx = side ? cx + half + 1 : cx + half + 1, hy = top + (e.arms === 'up' ? -1 : 2);
    var art = {
      bottle:     ['.#.', '###', '#+#', '#+#', '###'],
      book:       ['####', '#++#', '#++#', '####'],
      pen:        ['#', '#', '#', '+'],
      clipboard:  ['.##.', '####', '#oo#', '#oo#', '####'],
      screwdriver:['#', '#', '+', '+', '+'],
      cane:       ['##', '.#', '.#', '.#', '.#', '.#'],
      spoon:      ['##', '##', '.#', '.#'],
      flower:     ['#.#', '.#.', '#.#', '.#.', '.#.'],
      ball:       ['.##.', '#++#', '#++#', '.##.'],
      drumsticks: ['#.#', '#.#', '.#.', '#.#'],
      watercan:   ['..##', '####', '#++#', '####'],
      key:        ['##', '##', '.#', '##'],
      plank:      ['###', '#o#', '#+#', '#+#', '#+#', '#+#', '###']
    }[h];
    if (art) stamp(R, art, hx, hy);
  }

  function drawFx(R, fx, cx, cy, rx, ry, eyeY, ex, faceX, side, back, frame) {
    var top = Math.round(cy - ry), rt = Math.min(N - 5, Math.round(cx + rx));
    if (fx === 'tear' && !back) { var tx = side ? faceX + 2 : cx + ex; put(R, tx, eyeY + 2 + frame, curMark, MAT.TEAR); put(R, tx, eyeY + 3 + frame, curMark, MAT.TEAR); }
    if (fx === 'tears' && !back) (side ? [faceX + 2] : [cx - ex - 1, cx + ex]).forEach(function (x) { for (var y = eyeY + 1; y <= eyeY + 3 + frame * 2; y++) put(R, x, y, curMark, MAT.TEAR); });
    cur.ink = (fx === 'vein' || fx === 'heart') ? MAT.FX_RED : fx === 'sparkle' ? MAT.FX_GOLD : MAT.FX;
    if (fx === 'vein') stamp(R, FX.vein, rt - 1, top + (frame ? 0 : 1));
    if (fx === 'bang') stamp(R, FX.bang, rt + 1, Math.max(0, top - 3));
    if (fx === 'sweat') stamp(R, FX.sweat, rt, top + 3 + frame);
    if (fx === 'shiver') { stamp(R, FX.shiver, cx - rx - 3, cy - 1); stamp(R, FX.shiver, cx + rx + 2, cy - 1); }
    if (fx === 'zzz') { if (frame) stamp(R, FX.zbig, rt - 1, Math.max(0, top - 4)); else stamp(R, FX.zsmall, rt, Math.max(0, top - 2)); }
    if (fx === 'heart') stamp(R, frame ? FX.heart : FX.heart2, rt - (frame ? 1 : 0), Math.max(0, top - 3));
    if (fx === 'sparkle') stamp(R, FX.sparkle, frame ? rt : cx - rx - 2, Math.max(0, top - 1 + frame));
    if (fx === 'question') stamp(R, FX.question, rt + 1, Math.max(0, top - 3 + frame));
    if (fx === 'steam') { stamp(R, FX.steam, cx - rx + 1 - frame, Math.max(0, top - 2 - frame)); stamp(R, FX.steam, rt - 1 + frame, Math.max(0, top - 2 - frame)); }
    if (fx === 'note') stamp(R, FX.note, rt + (frame ? 0 : 1), Math.max(0, top - 3 + frame));
  }

  /* ── OUTPUT ─────────────────────────────────────────────────────────────── */
  var LCD = { bg: [195, 204, 163], ink: [35, 42, 27], mid: [110, 122, 86] };

  function skinRGB(pal, level) {
    var t = SKIN_T[Math.max(0, Math.min(5, level))];
    return [0, 1, 2].map(function (i) { return Math.round(pal.bg[i] + (pal.ink[i] - pal.bg[i]) * t); });
  }

  /* Paint a sprite into a 2D context: an LCD by default, with the unlit segments
     faintly visible and a faint segment grid over everything.

     ONE PASS: the 32x32 pixels are written once into a small buffer and scaled up
     with smoothing off, and the segment grid is one cached overlay per size — two
     drawImage calls where there were a thousand fillRects, which is what lets thirty
     of these run on a page without being felt. The colours are the same as before:
     an unlit segment is the ink at `ghost` over the paper, worked out once. */
  var PAINT = { buf: null, img: null, pal: null, lut: null, grids: {} };
  function canvas(w, h) { return typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(w, h) : Object.assign(document.createElement('canvas'), { width: w, height: h }); }
  function paint(ctx, R, opts) {
    opts = opts || {};
    var px = opts.px || 8, x0 = opts.x || 0, y0 = opts.y || 0, pal = opts.palette || LCD;
    var ghost = opts.ghost === undefined ? 0.07 : opts.ghost, grid = opts.grid === undefined ? 0.25 : opts.grid;
    if (!PAINT.buf) { PAINT.buf = canvas(N, N); PAINT.img = PAINT.buf.getContext('2d').createImageData(N, N); }
    var key = pal.bg.join() + '|' + pal.ink.join() + '|' + pal.mid.join() + '|' + ghost;
    if (PAINT.pal !== key) {
      PAINT.pal = key; PAINT.lut = [];
      for (var v = 0; v <= 7; v++) {
        PAINT.lut[v] = v === INK ? pal.ink : v === MID ? pal.mid : v > MID ? skinRGB(pal, v - 2)
          : [0, 1, 2].map(function (i) { return Math.round(pal.bg[i] + (pal.ink[i] - pal.bg[i]) * ghost); });
      }
    }
    var d = PAINT.img.data;
    for (var i = 0; i < N * N; i++) {
      var c = PAINT.lut[R[i]] || PAINT.lut[0];
      d[i * 4] = c[0]; d[i * 4 + 1] = c[1]; d[i * 4 + 2] = c[2]; d[i * 4 + 3] = 255;
    }
    PAINT.buf.getContext('2d').putImageData(PAINT.img, 0, 0);
    var smooth = ctx.imageSmoothingEnabled;
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(PAINT.buf, x0, y0, N * px, N * px);
    ctx.imageSmoothingEnabled = smooth;
    if (grid > 0 && px >= 4) ctx.drawImage(gridOverlay(px, pal, grid), x0, y0);
  }
  function gridOverlay(px, pal, alpha) {
    var k = px + '|' + pal.bg.join() + '|' + alpha;
    if (PAINT.grids[k]) return PAINT.grids[k];
    var S = N * px + 1, c = canvas(S, S), g = c.getContext('2d'), gap = Math.max(1, Math.round(px / 12));
    g.fillStyle = 'rgba(' + pal.bg.join(',') + ',' + alpha + ')';
    for (var i = 0; i <= N; i++) { g.fillRect(i * px, 0, gap, S); g.fillRect(0, i * px, S, gap); }
    return (PAINT.grids[k] = c);
  }
  /* RGB bytes, for a caller with no canvas (the bake script). */
  function rgba(R, px, pal) {
    px = px || 1; pal = pal || LCD;
    var W = N * px, out = new Uint8Array(W * W * 3);
    for (var y = 0; y < W; y++) for (var x = 0; x < W; x++) {
      var v = R[Math.floor(y / px) * N + Math.floor(x / px)];
      var c = v === INK ? pal.ink : v === MID ? pal.mid : v > MID ? skinRGB(pal, v - 2) : pal.bg;
      if (v === OFF) c = [Math.round(pal.bg[0] * 0.93 + pal.ink[0] * 0.07), Math.round(pal.bg[1] * 0.93 + pal.ink[1] * 0.07), Math.round(pal.bg[2] * 0.93 + pal.ink[2] * 0.07)];
      if (px >= 4 && (x % px === 0 || y % px === 0)) c = [Math.round(c[0] * 0.75 + pal.bg[0] * 0.25), Math.round(c[1] * 0.75 + pal.bg[1] * 0.25), Math.round(c[2] * 0.75 + pal.bg[2] * 0.25)];
      var i = (y * W + x) * 3; out[i] = c[0]; out[i + 1] = c[1]; out[i + 2] = c[2];
    }
    return out;
  }

  /* ── FULL COLOUR ──────────────────────────────────────────────────────────
     The same drawing, coloured by material. Every colour is the character's own
     (spec.colour) or derived from their tones, so a spec with no colours at all
     still comes out as a sensible full-colour sprite. */
  var SKIN_RGB = ['#fbe9da', '#f6d3b3', '#e6b487', '#c48a5c', '#8a5a3c', '#583928'];
  var TONE_RGB = { hair: { ink: '#2b2521', mid: '#9a6538', off: '#e8e5de' },
                   hat: { ink: '#2a2a2e', mid: '#8a5a34', off: '#efece5' },
                   outfit: { ink: '#353a4c', mid: '#6f93be', off: '#f3f0e8' } };
  var HELD_RGB = { pacifier: ['#f29cc0', '#ffffff'], bottle: ['#7fb6e0', '#ffffff'], book: ['#c8423a', '#f4ecd8'],
                   pen: ['#2f5fd0', '#e8e8e8'], clipboard: ['#a8733f', '#ffffff'], screwdriver: ['#9aa0a8', '#f2c230'],
                   cane: ['#8a5a2a', '#b98a55'], spoon: ['#b8bcc4', '#e4e6ea'], flower: ['#e8508e', '#f7d64a'],
                   ball: ['#e8453c', '#ffffff'], drumsticks: ['#d8b070', '#f0dcb0'], watercan: ['#4fa05a', '#9fd3f5'],
                   key: ['#e0b020', '#f5dc7a'], plank: ['#b8864c', '#d9ad74'] };
  function hex(h) { return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)]; }
  function shade(c, k) { return c.map(function (v) { return Math.round(k < 0 ? v * (1 + k) : v + (255 - v) * k); }); }
  function palette(specIn) {
    var s = normalize(specIn), c = s.colour;
    var skin = hex(c.skin || SKIN_RGB[s.skin.tone]);
    var hair = hex(c.hair || TONE_RGB.hair[s.hair.tone]);
    var hat = hex(c.hat || TONE_RGB.hat[s.hat.tone]);
    var hatLum = hat[0] * 0.3 + hat[1] * 0.59 + hat[2] * 0.11;
    var held = HELD_RGB[s.held] || ['#9a9a9a', '#dcdcdc'];
    var P = [];
    P[MAT.BG] = null;
    P[MAT.INK] = [46, 38, 34];
    P[MAT.EYES] = [36, 30, 26];
    P[MAT.SKIN] = skin; P[MAT.SKIN_SHADE] = shade(skin, -0.14); P[MAT.FACELINE] = shade(skin, -0.32);
    P[MAT.WHITE] = [255, 255, 255];
    /* The eye surround on dark skin (eyeWhites): on the LCD it has to be the paper to
       read at all; in colour a full white ring reads as goggles, so it is the skin
       lifted most of the way toward white. */
    P[MAT.SCLERA] = shade(skin, 0.62); P[MAT.BLUSH] = [244, 150, 164]; P[MAT.TEAR] = [150, 208, 246];
    P[MAT.HAIR] = hair; P[MAT.HAIR_LIGHT] = shade(hair, 0.28);
    P[MAT.BROW] = hair[0] + hair[1] + hair[2] > 420 ? shade(hair, -0.45) : shade(hair, -0.15);
    P[MAT.HAT] = hat; P[MAT.HAT_DETAIL] = hatLum < 90 ? shade(hat, 0.85) : shade(hat, -0.3);
    P[MAT.OUTFIT] = hex(c.outfit || TONE_RGB.outfit[s.body.tone]);
    P[MAT.OUTFIT2] = hex(c.outfit2 || '#f6f3ec');
    P[MAT.PATTERN] = hex(c.pattern || '#e2b33c');
    P[MAT.HELD] = hex(held[0]); P[MAT.HELD_LIGHT] = hex(held[1]);
    P[MAT.TONGUE] = [224, 99, 110]; P[MAT.SWEAT] = [140, 203, 242];
    P[MAT.FX_RED] = [228, 72, 90]; P[MAT.FX] = [90, 95, 115]; P[MAT.FX_GOLD] = [242, 194, 48];
    P[MAT.GLASSES] = hex(c.glasses || '#2f2a26'); P[MAT.SHOES] = hex(c.shoes || '#3a302a');
    P[MAT.TIES] = hex(c.ties || c.hat || '#e0609a');
    P[MAT.MID] = [138, 138, 128];
    return P;
  }
  /* Paint a full-colour sprite: transparent where there is nothing, unless opts.bg. */
  function paintColour(ctx, L, spec, opts) {
    opts = opts || {};
    var px = opts.px || 8, x0 = opts.x || 0, y0 = opts.y || 0, P = palette(spec);
    if (opts.bg) { ctx.fillStyle = opts.bg; ctx.fillRect(x0, y0, N * px, N * px); }
    for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
      var c = P[L.M[y * N + x]];
      if (!c) continue;
      ctx.fillStyle = 'rgb(' + c[0] + ',' + c[1] + ',' + c[2] + ')';
      ctx.fillRect(x0 + x * px, y0 + y * px, px, px);
    }
  }
  /* RGBA bytes of a region, for a caller with no canvas (the bake script). */
  function rgbaColour(L, spec, px, box) {
    px = px || 1; box = box || [0, 0, N, N];
    var P = palette(spec), W = box[2] * px, H = box[3] * px, out = new Uint8Array(W * H * 4);
    for (var y = 0; y < H; y++) for (var x = 0; x < W; x++) {
      var sx = box[0] + Math.floor(x / px), sy = box[1] + Math.floor(y / px);
      var c = sx >= 0 && sy >= 0 && sx < N && sy < N ? P[L.M[sy * N + sx]] : null, i = (y * W + x) * 4;
      if (c) { out[i] = c[0]; out[i + 1] = c[1]; out[i + 2] = c[2]; out[i + 3] = 255; }
    }
    return out;
  }

  return {
    N: N, OFF: OFF, INK: INK, MID: MID, MAT: MAT,
    renderLayers: renderLayers, palette: palette, paintColour: paintColour, rgbaColour: rgbaColour,
    EMOTIONS: EMOTIONS, FACINGS: FACINGS, FRAMES: FRAMES,
    SCHEMA: SCHEMA, FIELDS: FIELDS, LCD: LCD, SKIN_T: SKIN_T, skinValue: skinValue,
    render: render, normalize: normalize, validate: validate, compile: compile,
    get: getKey, set: setKey, paint: paint, rgba: rgba
  };
});
