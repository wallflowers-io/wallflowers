/* tile.js — one tile of the grid, as a Tamagotchi: bezel, glass, and a 32x32
 * picture on its LCD.
 *
 *     Kenjin.Tile.paint(canvas, R, pressed[, colours])   R: a Uint8Array(32*32) of tama.js tones
 *
 * Moved out of site.html (2026-09-25) so that anything else drawing the grid's
 * tiles draws them with this, not a copy: site.html paints the cast and the
 * ceremony's boards through it, and branding's docs/instagram-grid.html paints
 * the how page's boards through it. Needs tama.js (Tama.LCD, Tama.paint) by the
 * time it paints, not when it loads.
 *
 * ?v= IS LOAD-BEARING. assets/ is served immutable for a year; bump the number
 * in site.html in the same change as any edit here.
 */
(function (root) {
  'use strict';

  var PX = 7, SLOT = 256;

  /* The tile's outline at a given size: the grid's power-4 superellipse. */
  function shape(g, c, half) {
    g.beginPath();
    for (var i = 0; i <= 96; i++) {
      var t = i / 96 * Math.PI * 2, co = Math.cos(t), si = Math.sin(t);
      var x = c + half * Math.sign(co) * Math.sqrt(Math.abs(co)), y = c + half * Math.sign(si) * Math.sqrt(Math.abs(si));
      if (i) g.lineTo(x, y); else g.moveTo(x, y);
    }
    g.closePath();
  }
  /* One device, the way a real one is built: a thin bezel of neutral plastic
     round a recessed screen. The bezel is 4% of the tile each side (the screen
     is 92%), lit from the top left like every tile's shadow on this page, with
     a fine groove where it meets the glass; the screen sits below it, so its
     top-left inner edge is in shadow. The renderer clips the whole picture to
     the tile's outline. Pressed, the bezel dims — the renderer's own press was
     drawn in the band this page has turned off. Colour is not at rest on the
     tile at all: it comes up only as the glow on hover and focus (.tile-ring).

     THE STILL PARTS ARE DRAWN ONCE. The bezel (with the screen's glass already
     in it) and the recess's shadow are two cached canvases; a repaint is those
     two stamped round the sprite, which tama.js paints in one pass. */
  var SCREEN = 0.46;          // half the screen's width, as a fraction of the tile
  var layers = null;
  function canvas() { var c = document.createElement('canvas'); c.width = c.height = SLOT; return c; }
  function still() {
    if (layers) return layers;
    var S = SLOT, half = S * SCREEN, lcd = root.Tama.LCD;
    var bezel = canvas(), g = bezel.getContext('2d');
    var lit = g.createLinearGradient(0, 0, S, S);
    lit.addColorStop(0, '#F1EEE7'); lit.addColorStop(1, '#D5D1C7');
    g.fillStyle = lit; g.fillRect(0, 0, S, S);
    shape(g, S / 2, half + 1.5);
    g.lineWidth = 2; g.strokeStyle = 'rgba(40,38,32,.22)'; g.stroke();          // the groove
    shape(g, S / 2, half);
    g.fillStyle = 'rgb(' + lcd.bg.join(',') + ')'; g.fill();                       // the glass
    var recess = canvas(), r = recess.getContext('2d');
    r.save(); shape(r, S / 2, half); r.clip();
    r.translate(3, 3); shape(r, S / 2, half);                                  // shadow on the upper and left inner edges
    r.lineWidth = 7; r.strokeStyle = 'rgba(30,34,24,.26)'; r.stroke(); r.restore();
    return (layers = { bezel: bezel, recess: recess });
  }
  /* `colours`, when given, paints the picture in full colour instead of on the
     LCD: { bg, 1: ink, 2..5: the picture's tones } as CSS colours, no grid. */
  function paint(c, R, pressed, colours) {
    var g = c.getContext('2d'), S = SLOT, L = still();
    g.drawImage(L.bezel, 0, 0);
    if (pressed) { g.fillStyle = 'rgba(20,24,30,.12)'; g.fillRect(0, 0, S, S); }
    g.save();
    shape(g, S / 2, S * SCREEN);
    g.clip();
    var N = root.Tama.N, off = (S - N * PX) / 2;
    if (colours) {
      g.fillStyle = colours.bg; g.fillRect(0, 0, S, S);
      for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
        var col = colours[R[y * N + x]];
        if (col) { g.fillStyle = col; g.fillRect(off + x * PX, off + y * PX, PX, PX); }
      }
    } else root.Tama.paint(g, R, { px: PX, x: off, y: off, grid: 0.22 });
    g.restore();
    g.drawImage(L.recess, 0, 0);
  }

  var K = root.Kenjin = root.Kenjin || {};
  K.Tile = { PX: PX, SLOT: SLOT, SCREEN: SCREEN, shape: shape, canvas: canvas, paint: paint };
})(typeof self !== 'undefined' ? self : this);
