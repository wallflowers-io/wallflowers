/* skins.js — the looks a land can wear, and what each is made of.
 *
 * A LAND IS DRAWN FROM STUFF, NOT COLOURS. townmesh.js says "this triangle is
 * brick"; a skin says what brick looks like under it, where the sun is, how
 * hard a shadow falls, whether there is ink round things. So the same land,
 * the same walk, can be flicked from one look to the next between two frames.
 *
 * ONE LIST OF STUFF (MATS), in an order the shader's palette is indexed by.
 * ONE SET OF PLAIN COLOURS (BASE), which each skin bends — warmer, brighter,
 * all to clay, down to night — and then overrides where it wants its own.
 * The seven `gem` entries are the wordmark's colours and are handed in, not
 * written here (ceremony.js, GEMS): awnings, cars and signs are painted in them.
 *
 * NOTHING HERE TOUCHES THE DOM OR THE GPU, so node can run it.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else (root.Kenjin = root.Kenjin || {}).Skins = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var MATS = ['grass', 'sand', 'dirt', 'rock', 'water', 'asphalt', 'line', 'pave', 'brick', 'plaster', 'timber', 'stone', 'mark', 'markRoof',
              'roofTile', 'roofSlate', 'roofCopper', 'trim', 'glass', 'lit', 'door', 'trunk', 'leaf', 'leaf2', 'palm', 'frond', 'post', 'lamp',
              'hedge', 'tyre', 'white', 'dark', 'gold', 'green', 'gem0', 'gem1', 'gem2', 'gem3', 'gem4', 'gem5', 'gem6'];
  var BASE = {
    grass: '#6FA545', sand: '#D9C58C', dirt: '#8A6B47', rock: '#8B8B90', water: '#4FA3D8', asphalt: '#3D3E46', line: '#F2C83C', pave: '#B9B6AE',
    brick: '#B5573B', plaster: '#EBDFC6', timber: '#9B6A3F', stone: '#A9A7A2', mark: '#E4D8BC', markRoof: '#B9502F',
    roofTile: '#C4552E', roofSlate: '#4A4C59', roofCopper: '#5E9E86', trim: '#F3EEE2', glass: '#2D3A4F', lit: '#FFE9A6', door: '#5B3A22',
    trunk: '#6B4A2E', leaf: '#3F8A3A', leaf2: '#5FA84A', palm: '#9C8156', frond: '#4C9A3E', post: '#54565F', lamp: '#FFF1B8',
    hedge: '#3A7A36', tyre: '#1F1F24', white: '#FAF7F0', dark: '#23232A', gold: '#F0BC38', green: '#7BD957'
  };

  /* sky: top and foot. sun: where it is (east, up, south) and its colour. air:
     the light in shade. shade: how much of the sun a shadow keeps out. level:
     how far a hillside's facets are smoothed away, 0 to 1. fog:
     colour and thickness. bend: [warmth r, g, b, saturation, brightness] laid
     over BASE. cel: bands of light, 0 for smooth. ink: an outline's strength
     and colour. lines: nothing but the outlines, on `paper`. glow: stuff that
     shines by itself. lamps: the street lamps light the ground. */
  var LIST = [
    { id: 'santos', name: 'Los Santos', sky: ['#5E9FD6', '#F7C99B'], sun: [-0.6, 0.46, 0.45], sunCol: [1.08, 0.9, 0.7], air: [0.46, 0.48, 0.58], shade: 1, level: 0.45, fog: ['#F3C9A2', 0.00042],
      bend: [1.05, 0.98, 0.88, 0.86, 1], own: { grass: '#84A850', leaf: '#577F3A', leaf2: '#7D9A45', plaster: '#EBD3A8', pave: '#C4B9A6', asphalt: '#4A4744' } },
    { id: 'dollhouse', name: 'Dollhouse', sky: ['#3FA9F5', '#D6EEFB'], sun: [-0.4, 0.78, 0.48], sunCol: [0.98, 0.96, 0.9], air: [0.5, 0.54, 0.62], shade: 1, level: 0.5, fog: ['#D6EEFB', 0.00008],
      bend: [1, 1, 1, 1.28, 1.05], own: { grass: '#7CCB4E', roofTile: '#E0492F', roofSlate: '#3F6FB5', plaster: '#FFF3DC', brick: '#D9694A' } },
    { id: 'ink', name: 'Ink', sky: ['#EFE6D0', '#F8F2E4'], sun: [-0.5, 0.62, 0.55], sunCol: [0.9, 0.9, 0.88], air: [0.52, 0.52, 0.55], shade: 1, level: 0.96, fog: ['#F8F2E4', 0.00005],
      bend: [1, 1, 1, 0.9, 1.06], cel: 2, ink: [1, '#1B1A22'], own: { water: '#86C6EA', asphalt: '#5A5A66', grass: '#9CC56A' } },
    { id: 'maquette', name: 'Maquette', sky: ['#D9D4CA', '#F3F0EA'], sun: [-0.45, 0.7, 0.5], sunCol: [0.62, 0.61, 0.58], air: [0.6, 0.6, 0.62], shade: 1, level: 0.8, fog: ['#F3F0EA', 0.00018],
      clay: '#ECE5D6', keep: ['mark', 'markRoof', 'gold', 'green', 'gem0', 'gem1', 'gem2', 'gem3', 'gem4', 'gem5', 'gem6'], ink: [0.35, '#8C8576'],
      own: { mark: '#E9B43A', markRoof: '#D9952B', water: '#C6D6DE', glass: '#C3C8CC', lit: '#F7EBC6', asphalt: '#D2CCBF', line: '#F3EEE2' } },
    { id: 'night', name: 'After dark', sky: ['#060918', '#1C2150'], sun: [0.4, 0.7, 0.35], sunCol: [0.34, 0.42, 0.7], air: [0.34, 0.37, 0.56], shade: 0.8, level: 0.6, fog: ['#0E1233', 0.0004],
      bend: [0.78, 0.86, 1.15, 0.75, 0.86], glow: ['lit', 'lamp', 'gold', 'green', 'gem0', 'gem1', 'gem2', 'gem3', 'gem4', 'gem5', 'gem6'], lamps: true,
      own: { lit: '#FFD98A', lamp: '#FFE7A8', water: '#1B3A66' } },
    { id: 'blueprint', name: 'Blueprint', sky: ['#1D4C8A', '#1D4C8A'], sun: [-0.5, 0.62, 0.55], sunCol: [0.9, 0.9, 0.9], air: [0.5, 0.5, 0.5], shade: 0, level: 1, fog: ['#1D4C8A', 0],
      bend: [1, 1, 1, 1.2, 1], cel: 2, lines: true, ink: [1, '#EAF3FF'], paper: '#1D4C8A', own: {} }
  ];

  function hex(h) { return [parseInt(h.slice(1, 3), 16) / 255, parseInt(h.slice(3, 5), 16) / 255, parseInt(h.slice(5, 7), 16) / 255]; }
  function unit(v) { var l = Math.hypot(v[0], v[1], v[2]); return [v[0] / l, v[1] / l, v[2] / l]; }
  /* A skin, worked out: a colour to every stuff in MATS's order, and the rest as
     numbers. `gems` are the wordmark's seven, as [r, g, b] out of 255. */
  function resolve(id, gems) {
    var s = LIST.filter(function (k) { return k.id === id; })[0] || LIST[0], own = s.own || {}, bend = s.bend || [1, 1, 1, 1, 1];
    var pal = new Float32Array(48 * 3), glow = new Float32Array(48);
    MATS.forEach(function (name, i) {
      var gem = /^gem(\d)$/.exec(name), c = own[name] ? hex(own[name]) : gem ? gems[+gem[1]].map(function (v) { return v / 255; }) : hex(BASE[name]);
      if (!own[name]) {
        if (s.clay && s.keep.indexOf(name) < 0) c = hex(s.clay);
        else {
          var grey = c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11;
          c = c.map(function (v, k) { return Math.max(0, Math.min(1, (grey + (v - grey) * bend[3]) * bend[k] * bend[4])); });
        }
      }
      pal.set(c, i * 3);
      glow[i] = s.glow && s.glow.indexOf(name) >= 0 ? 1 : 0;
    });
    return { id: s.id, name: s.name, pal: pal, glow: glow, sky: [hex(s.sky[0]), hex(s.sky[1])], sun: unit(s.sun), sunCol: s.sunCol, air: s.air, shade: s.shade, level: s.level || 0,
             fog: hex(s.fog[0]), thick: s.fog[1], cel: s.cel || 0, ink: s.ink ? s.ink[0] : 0, inkCol: hex(s.ink ? s.ink[1] : '#000000'),
             lines: !!s.lines, paper: hex(s.paper || '#FFFFFF'), lamps: !!s.lamps };
  }
  var index = {};
  MATS.forEach(function (name, i) { index[name] = i; });

  return { MATS: MATS, index: index, BASE: BASE, LIST: LIST, resolve: resolve };
});
