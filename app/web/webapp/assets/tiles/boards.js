/* boards.js — the six boards as things with names and colours.
 *
 *     Kenjin.Boards.NAMES[key]              'Chat Rooms', 'Articles', 'Trade', …
 *     Kenjin.Boards.colours(key)            { bg, 1..5 } for Kenjin.Tile.paint(c, R, false, colours)
 *     Kenjin.Boards.paint(canvas, key, lcd) a board, in full colour unless lcd
 *     Kenjin.Boards.banner(key)             { name, w, h, gap } of its name panel, in panel pixels
 *     Kenjin.Boards.bannerArt(key)          that panel, one canvas pixel to a panel pixel
 *
 * The pictures are ceremony.js's (Kenjin.Ceremony.PICTURES); the tile is
 * tile.js's. This file holds only what the boards ARE beyond their pictures:
 * the launch deck's words for them, and their colours. Moved out of branding's
 * docs/instagram-grid.html (2026-09-25) so the webapp's site panel draws its
 * section icons with the same code as the Instagram grid, not a copy. Needs
 * tama.js, tile.js and ceremony.js loaded before it is called.
 *
 * ?v= IS LOAD-BEARING. assets/ is served immutable for a year; bump the number
 * everywhere this is loaded in the same change as any edit here.
 */
(function (root) {
  'use strict';

  /* The launch deck's names (business/finance/deck/wallflowers-launch.html),
     with Site renamed Communities and Marketplace renamed Trade by Ralph. The
     keys are the pictures' keys. */
  var NAMES = { communities: 'Communities', forums: 'Chat Rooms', publications: 'Articles',
                marketplace: 'Trade', events: 'Events', treasury: 'Treasury' };

  /* FULL COLOUR, from the walls of Vista Valley (Ralph's photographs, 25 Sep
     2026): spray paint on white, outlined in black — sunflower yellow, sky
     blue, bubblegum pink, lilac, lime, red, orange. A board's tones (tama.js:
     1 ink, 2 mid, 3 pale, 4 tone, 5 deep) each take one; 0 is the wall. */
  var WALL = '#F7F4EE', INK = '#1B1B1F';
  var SPRAY = { yellow: '#F7C51E', gold: '#E39A16', blue: '#4FA3E0', pink: '#F29BC0', lilac: '#B9A3E3',
                lime: '#9ACD3F', red: '#E8423A', orange: '#F08A2C', brown: '#5C351B' };
  var COLOURS = {
    communities: { 2: SPRAY.blue, 3: SPRAY.pink, 4: SPRAY.yellow, 5: SPRAY.brown },
    forums: { 2: SPRAY.blue, 3: SPRAY.lilac, 5: SPRAY.pink },
    publications: { 2: SPRAY.orange, 4: SPRAY.lilac, 5: SPRAY.blue },
    marketplace: { 2: SPRAY.red, 3: SPRAY.yellow, 5: SPRAY.gold },
    events: { 2: SPRAY.lime, 5: SPRAY.red },
    treasury: { 2: SPRAY.gold, 3: SPRAY.yellow, 4: SPRAY.gold, 5: '#B8871E' }
  };
  function colours(key) {
    var c = { bg: WALL, 1: INK }, m = COLOURS[key] || {};
    for (var t in m) c[t] = m[t];
    return c;
  }
  function paint(canvas, key, lcd) {
    root.Kenjin.Tile.paint(canvas, root.Kenjin.Ceremony.PICTURES[key](), false, lcd ? null : colours(key));
    return canvas;
  }

  /* THE NAME UNDER A BOARD, on the how page's own panel (ceremony.js: panelBox,
     putText) without its outer edge: each as wide as its name needs. */
  var bannerCache = {};
  function banner(key) {
    var name = NAMES[key] || key, A = root.Kenjin.Ceremony.art;
    return { name: name, w: A.textW(name, 1) + 9, h: 16, gap: 3 };
  }
  function bannerArt(key) {
    if (bannerCache[key]) return bannerCache[key];
    var A = root.Kenjin.Ceremony.art, P = A.P, B = banner(key), T = A.art(B.w, B.h), x, y;
    A.panelBox(T, -1, -1, B.w + 2, B.h + 2, false);                             // its edge falls outside: gone
    for (x = 2; x < B.w - 2; x++) { T.put(x, 2, P.panelLine); T.put(x, B.h - 3, P.panelLine); }   // the inner line
    for (y = 2; y < B.h - 2; y++) { T.put(2, y, P.panelLine); T.put(B.w - 3, y, P.panelLine); }
    A.putText(T, B.name, 4, 4, 1, P.panelText, P.panelShade);
    return (bannerCache[key] = T.c);
  }

  var K = root.Kenjin = root.Kenjin || {};
  K.Boards = { NAMES: NAMES, WALL: WALL, INK: INK, SPRAY: SPRAY, COLOURS: COLOURS,
               colours: colours, paint: paint, banner: banner, bannerArt: bannerArt };
})(typeof self !== 'undefined' ? self : this);
