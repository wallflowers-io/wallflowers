/* ceremony.js — HOW IT WORKS, played on the grid.
 *
 * Pressing a character on site.html plays this — on /site, on / once the
 * entrance has played, and on /how, which is the same page under a title of
 * its own. Until 2026-09-19 that was /how alone, and a press on /site opened
 * the webapp's demo instead; the demo is no longer opened from the site at all.
 *
 * The character drops out through the bottom of their own screen, behind their
 * tile, and lands on the grass in full colour; everyone else leaves; a sky
 * opens and the ground comes up under the board; and the story is told on it,
 * a step at a time:
 *
 *    ☼   ~ clouds ~        ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭       deltas, each with its
 *    birds                 ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭       author's face: every
 *                          ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭    ▭▭▭       group's own chain
 *                        [mkt]-[pub]-[frm]-[evt]-[ DM ]-[site]       YOUR CHAIN, right to left
 *                          |      |      |      |      |     |    ✿  on bamboo poles
 *   (egg)  you             |      |      |      |      |     |    |  the head, planted at the right
 *   ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔|▔▔▔▔▔▔|▔▔▔▔▔▔|▔▔▔▔▔▔|▔▔▔▔▔▔|▔▔▔▔▔|▔▔▔▔|▔▔  grass, bushes
 *     soil, a web of roots — from the seed on the right, leftwards — bones, a chest
 *     stone
 *     bedrock ────────────  [<]  what is happening, in words  [>]  ──────────────
 *
 * AT THE LEFT EDGE stands an egg on its base — panelled copper, a small screen
 * and a keyhole, turned 45 degrees about its long axis so they face you; five
 * thirds as tall as the cast (EGG). It is your Arc: it kept your seed sealed,
 * and your key, slid all the way in and turned, opens it.
 *
 * THE STEPS. 0 you arrive · 1 you put your key in the egg and turn it, and it
 * gives you your seed · 2 you carry it across the board and plant it at the
 * right: its roots go down first, and then a sunflower — your head — bursts up
 * through the grass block · 3–8 your chain, a feature a step, coming back
 * towards you from the right: the roots go looking the way a slime mold
 * forages, one front
 * finds the ground under the next board and thickens into a cord while the
 * rest are pulled back part of the way (what is left of them stays, a fine web
 * through the soil), a single root rises to the board, the board comes up on a
 * bamboo pole, and the head grows a leaf — six links, six leaves · 9 you
 * and your friends run under the links and bump them from below, and every
 * bump mints a delta stamped with its author's face, stacked on that group's
 * own chain, until every group holds seven to twelve · 10 all of it, back, and
 * the head blooms. Each step plays and then holds long enough to read; the
 * buttons (and the arrow keys) step forward and back, playing the scene as they
 * go. Everything is a pure function of the time t, so stepping back plays it
 * backwards. Escape, and Home in the page's own corner, hand the board back:
 * the scene plays itself out to the frame it was taken at and lets go.
 *
 * ONE BUMP AT A TIME, PER LINK. The runs are planned before anything plays: a
 * link is not bumped again until its last delta has landed, so a stack is
 * always in the order its deltas were minted, newest on top.
 *
 * A CARD TO EACH LINK. Once a board is up it can be pressed (a button laid over
 * it, with a ? in its corner), and a card says in a line what that feature is,
 * and the one thing all six have in common: each is a GroupObject — an MLS
 * group whose members are who may read and write it, and an ordered log of
 * signed deltas that folds into what you see. The scene waits while a card is
 * open. Nothing on a card states an op, an id or a count: those belong to
 * core/coordination/delta-graph.icd.json, and a page that copies them out is
 * a second source of truth with a delay on it.
 *
 * WHAT IS BUILT AND WHAT IS DRAWN. The first half is working code:
 * a sign-in (the Door's session, or the iOS app) takes the passkey's PRF, fetches
 * the wrap by public key, opens it to the seed, and reads the head. The chain is the DESIGN: it is
 * not in core (locator.rs has its address and nothing seals an entry), so the
 * head never moves past 0, and no door writes `group.joinedObject` when a
 * person joins something. The page carried a standing note saying so until
 * 2026-09-20, when the owner had it taken off; the fact is unchanged, and it is
 * recorded here because nothing on screen says it any more. No key, ciphertext
 * or position here is computed — this file draws.
 *
 * THE GROUND IS A PICTURE, NOT A MAP. It stands for the relay. The wrap and the
 * head are really rows on your Arc, and the chain's entries sealed blobs on the
 * relay, so the words name the Arc for the seed and the relay for the chain. The
 * seed is planted only in the picture: it never leaves the phone — what reaches
 * the ground is an address derived from it, and the words say that too.
 *
 * ONE PIXEL SIZE FOR EVERYTHING THAT IS NOT A TILE. The LCD's pixel — a tile's
 * width x 7/256, rounded to whole device pixels so it stays crisp — is the unit
 * of the sky, the soil, the roots, the sunflower, the egg, the stalks, the
 * deltas, and the people and what they hold. It is Terraria's own arrangement:
 * blocks of 8x8 art pixels, and players drawn at the same grain as the world.
 * The tiles keep their own size throughout, and are drawn SMOOTHED: a tile's
 * 256px picture brought down to its box by nearest neighbour aliases its LCD
 * grid differently at every sub-pixel offset, which flickers as it moves.
 *
 * WHY A LAYER AND NOT THE RENDERER. grid-gpu.js draws tiles and nothing else;
 * which tiles exist is fixed when it mounts, and the only motion it knows is a
 * row or column turning. So the board is paused square, this layer draws every
 * tile at the cell it already occupies, and only then is the grid's canvas
 * hidden — the paper under it is the renderer's own ground, so there is no seam.
 * The page's header (the mark, Sign up, Sign in) stays above it.
 *
 * ?v= IS LOAD-BEARING. assets/ is served immutable for a year; bump the number
 * in site.html in the same change as any edit here.
 */
(function (root) {
  'use strict';

  var N = 32, OFF = 0, INK = 1, MID = 2;
  /* Three more tones a picture can be drawn in. On the board they are shades of
     the screen's own green (tama.js paints 3..5 as steps from the ground toward
     the ink); when a delta prints one of these boards, each tone takes a colour
     of its own — see litPic. Five tones, five colours, which is what makes a
     printed board look printed rather than tinted. */
  var PALE = 3, TONE = 4, DEEP = 5;
  /* Your chain, left to right. Two kinds repeat, drawn differently, because they
     are different sites and different people. */
  /* The six, in the order they come up — right to left, the nearest to the
     flower first. Each is a feature, and each has a card. */
  var CHAIN = ['site', 'dm', 'events', 'forums', 'publications', 'marketplace'];
  var LINK = {
    site: { says: 'Site', card: 'Your place on the web: a page anyone can visit, with a room behind it only the people in it can read.' },
    dm: { says: 'Direct messages', card: 'One to one, or a few of you: a conversation sealed to the people in it.' },
    events: { says: 'Events', card: 'Something happening — when it is, where it is, and who is coming.' },
    forums: { says: 'Forums', card: 'Threads a group keeps together, room by room.' },
    publications: { says: 'Publications', card: 'Something you publish and can hand on. It keeps its own readers wherever it ends up.' },
    marketplace: { says: 'Marketplace', card: 'What you are offering, and what you are looking for, among the people you share it with.' }
  };
  /* The one line under all six: what every one of them is made of. */
  var UNDER = 'Underneath, all six are the same thing: a GroupObject — an MLS group, whose members are who may read and write it, and an ordered log of signed deltas that folds into what you see.';
  var BEATS = [
    'You, on a new browser or phone',
    'Fetch your seed from the Arc, or recover directly with your 24 Words',
    'Only you can unlock your seed, the Arc doesn’t know who you are.',
    'Reconstruct your account one group at a time:: a Site you belong to.',
    'Then the next:: a Conversation.',
    'An Event you’re hosting.',
    'A Forum you are part of.',
    'An Article your friend published.',
    'Something you are selling.',
    'Connect online, and live offline.',
    'A better Social Network: Encrypted, Private, Secure.'
  ];
  var CHAIN_BEAT = 3, WRITE_BEAT = 9, LAST_BEAT = 10;
  var SLAB_H = 8, SLAB_GAP = 1;    // a delta on a stack, and the air above it
  var LEAD = 18;                  // art pixels above the grass line in the ground's picture: the plants on it
  var KEY_STEP = 14;               // art pixels you step in toward the egg with the key
  var KEY_DEPTH = 10;              // how far it goes in: the whole shaft, to the bow
  var FRIENDS = 4;                 // how many of the cast come back to write
  var RUN = 0.0042, JUMP = 420;    // lattice columns a ms; a jump's length
  /* How tall the sunflower's stem is after each stage it grows at: planted (a
     bare shoot — the roots came first), then a leaf for each link its roots
     find, and the last, when it buds and blooms. */
  var GROW = [0.14, 0.27, 0.40, 0.53, 0.66, 0.79, 0.92, 1];

  /* ── EASING AND NOISE ────────────────────────────────────────────────── */
  function clamp01(x) { return x < 0 ? 0 : x > 1 ? 1 : x; }
  function seg(t, at, dur) { return dur <= 0 ? (t >= at ? 1 : 0) : clamp01((t - at) / dur); }
  function lerp(a, b, x) { return a + (b - a) * x; }
  function inQuad(x) { return x * x; }
  function outCubic(x) { return 1 - Math.pow(1 - x, 3); }
  function inOutCubic(x) { return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; }
  function outBack(x) { var c1 = 1.3, c3 = c1 + 1; return 1 + c3 * Math.pow(x - 1, 3) + c1 * Math.pow(x - 1, 2); }
  function bump(x) { return x <= 0 || x >= 1 ? 0 : Math.sin(x * Math.PI); }
  /* Deterministic, so the ground, the roots and the debris are the same on every
     frame and the same played backwards. */
  function hash(x, y) {
    var h = (Math.imul(x | 0, 374761393) + Math.imul(y | 0, 668265263)) | 0;
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  }
  function noise1(x, period, salt) {
    var i = Math.floor(x / period), f = x / period - i, s = f * f * (3 - 2 * f);
    return lerp(hash(i, salt), hash(i + 1, salt), s);
  }
  function noise2(x, y, period, salt) {
    var i = Math.floor(x / period), j = Math.floor(y / period), fx = x / period - i, fy = y / period - j;
    var sx = fx * fx * (3 - 2 * fx), sy = fy * fy * (3 - 2 * fy);
    return lerp(lerp(hash(i * 31 + salt, j), hash((i + 1) * 31 + salt, j), sx),
                lerp(hash(i * 31 + salt, j + 1), hash((i + 1) * 31 + salt, j + 1), sx), sy);
  }

  /* ── THE LCD PICTURES: your chain's tiles ────────────────────────────── */
  function lcd() { return new Uint8Array(N * N); }
  function px(R, x, y, v) { if (x >= 0 && y >= 0 && x < N && y < N) R[y * N + x] = v; }
  function box(R, x, y, w, h, v) { for (var j = 0; j < h; j++) for (var i = 0; i < w; i++) px(R, x + i, y + j, v); }
  function glyph(R, rows, x, y, s, v) {
    for (var j = 0; j < rows.length; j++)
      for (var i = 0; i < rows[j].length; i++)
        if (rows[j][i] === '#') box(R, x + i * s, y + j * s, s, s, v);
  }
  function frame(R, x, y, w, h) {
    box(R, x, y, w, 1, INK); box(R, x, y + h - 1, w, 1, INK);
    box(R, x, y, 1, h, INK); box(R, x + w - 1, y, 1, h, INK);
  }
  var PICTURES = {
    site: function () {                          // a page with a picture and two lines of text
      var R = lcd();
      frame(R, 4, 7, 24, 19); box(R, 4, 7, 24, 4, INK);
      box(R, 6, 8, 2, 2, OFF); box(R, 9, 8, 2, 2, OFF); box(R, 12, 8, 2, 2, OFF);
      box(R, 7, 13, 18, 5, MID);
      box(R, 7, 20, 14, 1, DEEP); box(R, 7, 22, 10, 1, DEEP);
      return R;
    },
    communities: function () {                   // three flowers, big and overlapping, the sunflower in front, and a bee
      var R = lcd();
      /* One flower: n petals round a plain heart, round-tipped or, for the
         sunflower, pointed. Each pixel is in if most of sixteen samples are,
         and the left half is mirrored onto the right about the centre (a
         pixel's centre), so every flower is exactly symmetric. */
      function flower(cx, cy, r, n, petal, heart, hr, pointed) {   // its shape, and what it draws at a pixel
        function rim(a) { var c = Math.abs(Math.cos(n * a / 2)); return r * (0.38 + 0.62 * (pointed ? c : Math.sqrt(c))); }
        function hit(f, x, y) { var k = 0; for (var j = 0; j < 4; j++) for (var i = 0; i < 4; i++) k += f(x + (i + 0.5) / 4 - cx, y + (j + 0.5) / 4 - cy) ? 1 : 0; return k >= 8; }
        var F = {}, H = {};
        function key(x, y) { return x + ',' + y; }
        function inF(x, y) { if (x + 0.5 > cx) x = Math.round(2 * cx - 1 - x); return !!F[key(x, y)]; }
        function inH(x, y) { if (x + 0.5 > cx) x = Math.round(2 * cx - 1 - x); return !!H[key(x, y)]; }
        for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
          if (x + 0.5 > cx) continue;
          F[key(x, y)] = hit(function (X, Y) { return Math.hypot(X, Y) <= rim(Math.atan2(Y, X) + Math.PI / 2); }, x, y);
          H[key(x, y)] = hit(function (X, Y) { return Math.hypot(X, Y) <= hr; }, x, y);
        }
        return {
          covers: function (x, y) { return inF(x, y) || inH(x, y); },
          at: function (x, y) {
            if (inH(x, y)) return !inH(x - 1, y) || !inH(x + 1, y) || !inH(x, y - 1) || !inH(x, y + 1) ? INK : heart;
            return !inF(x - 1, y) || !inF(x + 1, y) || !inF(x, y - 1) || !inF(x, y + 1) ? INK : petal;
          }
        };
      }
      /* WHICH IS IN FRONT is decided pair by pair, per pixel (OVER), so any
         order can be asked for, a ring included. All three are the same flower. */
      var BL = flower(10.5, 21.5, 7.5, 5, PALE, MID, 2.6, false);
      var BR = flower(21.5, 21.5, 7.5, 5, MID, PALE, 2.6, false);
      var TOP = flower(16.5, 12.5, 9.5, 5, TONE, DEEP, 3.3, false);     // the sunflower, the biggest, its heart the darkest; its own tone, for colour
      var OVER = [[BL, TOP], [BL, BR], [TOP, BR]];                      // [above, below]: the bottom left in front of both
      for (var y = 0; y < N; y++) for (var x = 0; x < N; x++) {
        var on = [TOP, BR, BL].filter(function (f) { return f.covers(x, y); });
        if (!on.length) continue;
        var win = on.filter(function (f) { return !on.some(function (g) { return OVER.some(function (o) { return o[0] === g && o[1] === f; }); }); })[0] || on[0];
        px(R, x, y, win.at(x, y));
      }
      var BEE = ['.##.##...', '#oo#oo#..', '.##.##...', '.######..', '#p#p#p##.', '#p#p#pd#.', '.######..'];   // wings, and a striped body
      var BT = { '#': INK, o: OFF, p: TONE, d: DEEP };
      BEE.forEach(function (row, j) { for (var i = 0; i < row.length; i++) if (BT[row[i]] !== undefined) px(R, 5 + i, 2 + j, BT[row[i]]); });   // landing on the sunflower
      return R;
    },
    dm: function () {                            // a speech bubble, someone typing
      var R = lcd();
      box(R, 7, 7, 18, 1, INK); box(R, 7, 21, 18, 1, INK);
      box(R, 5, 9, 1, 11, INK); box(R, 26, 9, 1, 11, INK);
      px(R, 6, 8, INK); px(R, 25, 8, INK); px(R, 6, 20, INK); px(R, 25, 20, INK);
      box(R, 8, 22, 3, 1, INK); box(R, 8, 23, 2, 1, INK); px(R, 8, 24, INK);
      box(R, 10, 13, 2, 2, DEEP); box(R, 15, 13, 2, 2, DEEP); box(R, 20, 13, 2, 2, DEEP);
      return R;
    },
    events: function () {                        // a calendar, on today's date
      var R = lcd();
      frame(R, 6, 8, 20, 19); box(R, 6, 8, 20, 5, INK);
      box(R, 10, 5, 2, 5, MID); box(R, 20, 5, 2, 5, MID);
      glyph(R, ['.#.', '##.', '.#.', '.#.', '###'], 9, 15, 2, DEEP);
      glyph(R, ['###', '#.#', '###', '..#', '###'], 17, 15, 2, DEEP);
      return R;
    },
    forums: function () {                        // Chat Rooms: a thread, as a tree — a post, and the replies hung off it
      var R = lcd();
      function card(x0, y0, w, h) {              // a post: its card, who wrote it, what they said
        frame(R, x0, y0, w, h); box(R, x0 + 1, y0 + 1, w - 2, h - 2, PALE);
        box(R, x0 + 2, y0 + 1 + (h > 5 ? 1 : 0), 3, 3, MID);
        box(R, x0 + 6, y0 + 2, w - 8, 1, INK);
        if (h > 5) box(R, x0 + 6, y0 + 4, w - 11, 1, DEEP);
      }
      card(5, 5, 22, 7);                         // the post
      box(R, 7, 12, 1, 16, INK);                 // the stem, down to the last reply
      [[13, 12], [19, 16], [25, 14]].forEach(function (r) { box(R, 8, r[0] + 2, 2, 1, INK); card(10, r[0], r[1], 5); });   // a pixel apart, each its own length
      return R;
    },
    publications: function () {                  // a printed page: masthead, a column, a picture
      var R = lcd();
      frame(R, 5, 6, 22, 21);
      box(R, 8, 9, 16, 2, INK);                  // the masthead
      box(R, 8, 12, 16, 1, TONE);
      box(R, 8, 15, 7, 1, DEEP); box(R, 8, 17, 7, 1, DEEP); box(R, 8, 19, 6, 1, DEEP); box(R, 8, 21, 7, 1, DEEP); box(R, 8, 23, 4, 1, DEEP);
      box(R, 17, 15, 7, 6, MID);                 // the picture it runs with
      box(R, 17, 23, 7, 1, DEEP);
      return R;
    },
    marketplace: function () {                   // Trade: a market stall, its striped awning, goods on the counter
      var R = lcd(), x, y;
      for (x = 3; x < 29; x++) { px(R, x, 5, INK); box(R, x, 6, 1, 6, ((x - 3) >> 2) & 1 ? OFF : MID); }
      for (x = 3; x < 29; x += 4) [[1, 12], [2, 12], [0, 11], [3, 11]].forEach(function (p) { if (x + p[0] <= 28) px(R, x + p[0], p[1], INK); });   // scalloped, inside the awning
      box(R, 3, 6, 1, 6, INK); box(R, 28, 6, 1, 6, INK);
      box(R, 5, 12, 2, 16, INK); box(R, 25, 12, 2, 16, INK);                          // the posts
      frame(R, 8, 15, 6, 6); box(R, 9, 16, 4, 4, MID); box(R, 11, 16, 1, 4, PALE); box(R, 9, 17, 4, 1, PALE);   // a parcel,
      for (y = 13; y < 23; y++) for (x = 15; x < 25; x++) {                                                      // and a coin
        var d = Math.hypot(x + 0.5 - 19.5, y + 0.5 - 18);
        if (d <= 2.2) px(R, x, y, PALE); else if (d <= 3.3) px(R, x, y, INK);
      }
      frame(R, 4, 21, 24, 4); box(R, 5, 22, 22, 2, MID);                            // the counter
      return R;
    },
    treasury: function () {                      // three stacks of coins at three depths, the nearer in front
      var R = lcd(), TONES = { '#': INK, p: PALE, t: TONE, d: DEEP };
      /* Eleven wide, each coin a short cylinder seen from a little above: the
         top one shows its face, an oval; every coin has straight reeded walls
         and an ELLIPTICAL underside. A coin is four rows but stands three
         above the one below: its curved foot lies over that coin's top row,
         so the lower coin's walls show at the curve's ends and its top reads
         curved too. Drawn bottom coin first, each laid over the last. */
      var FACE = ['...#####...', '.##ppppp##.', '#ppppppppp#'];
      var COIN = ['#tdtdtdtdt#', '#tdtdtdtdt#', '.#dtdtdtd#.', '..#######..'];
      function stack(x0, bottom, shifts) {        // shifts: one per coin, top coin first
        var n = shifts.length, top = bottom - (FACE.length + 3 * n);
        function row(str, y, dx) { for (var i = 0; i < str.length; i++) if (TONES[str[i]] !== undefined) px(R, x0 + dx + i, y, TONES[str[i]]); }
        for (var k = n - 1; k >= 0; k--) COIN.forEach(function (r, j) { row(r, top + FACE.length + 3 * k + j, shifts[k]); });
        FACE.forEach(function (r, j) { row(r, top + j, shifts[0]); });
      }
      stack(11, 22, [0, 0, 1, 0, 0]);            // behind, five, on the centre line
      stack(4, 26, [0, 0, 0, 1]);                // four, to its left
      stack(18, 28, [0, -1, 0]);                 // nearest, three, to its right: the group spans 4..28, 4..28
      return R;
    }
  };

  /* ── THE LETTERS ─────────────────────────────────────────────────────────
     The words under the scene are drawn, not set: the page's own face is a
     marker hand and the scene is a pixel picture, so the explainer is a pixel
     display with a pixel face. Five wide by eight — seven for a capital and a
     row under the line for a descender — one glyph to eight characters here,
     each carrying a row's five bits (48 = an empty row). Spacing is the ink's:
     a glyph is as wide as it is drawn, plus one. */
  var FONT_KEYS = ' ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.,:’!?-()·/';
  var FONT_BITS =
    '00000000>AAOAAA0NAANAAN0>A@@@A>0NAAAAAN0O@@N@@O0O@@N@@@0>A@GAA>0' +
    'AAAOAAA0>44444>072222B<0ABDHDBA0@@@@@@O0AKEEAAA0AIEECAA0>AAAAA>0' +
    'NAAN@@@0>AAAEB=0NAANDBA0?@@>11N0O4444440AAAAAA>0AAAAA:40AAAEEKA0' +
    'AA:4:AA0AA:44440O1248@O000>1?A?0@@NAAAN000>@@@>011?AAA?000>AO@>0' +
    '68N8888000?AA?1>@@NAAAA040<444>0206222B<@@BDHDB0<44444>000JEEEE0' +
    '00NAAAA000>AAA>000NAAN@@00?AA?1100FH@@@000?@>1N088N8896000AAAA?0' +
    '00AAA:4000AEEE:000A:4:A000AAA?1>00O248O0>ACEIA>04<4444>0>A1248O0' +
    'N11>11N026:BO220O@N11A>0>A@NAA>0O1248880>AA>AA>0>AA?1A>000000<<0' +
    '00000<<800<<0<<0<<80000044444040>A124040000O00002488842084222480' +
    '0004000011248@@0';
  var FONT = (function () {
    var m = {}, i, j, x;
    for (i = 0; i < FONT_KEYS.length; i++) {
      var rows = [], lo = 5, hi = -1;
      for (j = 0; j < 8; j++) {
        var b = FONT_BITS.charCodeAt(i * 8 + j) - 48;
        rows.push(b);
        for (x = 0; x < 5; x++) if (b & (1 << (4 - x))) { if (x < lo) lo = x; if (x > hi) hi = x; }
      }
      m[FONT_KEYS.charAt(i)] = { rows: rows, lo: hi < 0 ? 0 : lo, w: hi < 0 ? 2 : hi - lo + 1 };
    }
    m["'"] = m['’']; m['—'] = m['-']; m['–'] = m['-'];      // the shapes a copy editor's quotes and dashes land on
    return m;
  })();
  function glyphOf(ch) { return FONT[ch] || FONT[' ']; }
  function textW(str, s) {
    var w = 0;
    for (var i = 0; i < str.length; i++) w += (glyphOf(str.charAt(i)).w + 1) * s;
    return Math.max(0, w - s);
  }
  /* Into an art canvas, at s art pixels to the font's one, with a shadow under
     it the way a game's display carries one. */
  function putText(A, str, x0, y0, s, col, shade) {
    var x = x0, i, j, k, dx2, dy2;
    for (i = 0; i < str.length; i++) {
      var gl = glyphOf(str.charAt(i));
      for (j = 0; j < 8; j++) {
        for (k = 0; k < 5; k++) {
          if (!(gl.rows[j] & (1 << (4 - k)))) continue;
          for (dy2 = 0; dy2 < s; dy2++) for (dx2 = 0; dx2 < s; dx2++) {
            if (shade) A.put(x + (k - gl.lo) * s + dx2 + s, y0 + j * s + dy2 + s, shade);
            A.put(x + (k - gl.lo) * s + dx2, y0 + j * s + dy2, col);
          }
        }
      }
      x += (gl.w + 1) * s;
    }
  }
  /* EVERY STEP IS TWO LINES, broken where the two come nearest the same
     length, because one long line over a short one reads as a mistake and a
     panel that changes shape under the reader reads as a fault. A step whose
     words will not go on two lines at this size takes as many as it needs. */
  function twoLines(str, w, s) {
    var words = str.split(' '), best = null, i;
    for (i = 1; i < words.length; i++) {
      var a = words.slice(0, i).join(' '), b = words.slice(i).join(' ');
      var wa = textW(a, s), wb = textW(b, s), worst = Math.max(wa, wb);
      if (w && worst > w) continue;
      if (!best || worst < best[0]) best = [worst, a, b];
    }
    return best ? [best[1], best[2]] : null;
  }
  /* The width the wider of those two lines wants, with no limit set. */
  function twoWidth(str, s) {
    var two = twoLines(str, 0, s);
    return two ? Math.max(textW(two[0], s), textW(two[1], s)) : textW(str, s);
  }
  function wrapText(str, w, s) {
    var two = str.indexOf(' ') > 0 ? twoLines(str, w, s) : null;
    if (two) return two;
    var words = str.split(' '), lines = [], line = '';
    words.forEach(function (word) {
      var next = line ? line + ' ' + word : word;
      if (line && textW(next, s) > w) { lines.push(line); line = word; }
      else line = next;
    });
    if (line) lines.push(line);
    return lines;
  }

  /* ── THE ART: colour, at the art pixel ───────────────────────────────── */
  function hex(h) { return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)]; }
  var P = {}, SRC = {
    grassTip: '#A8DA6C', grassLight: '#86C24F', grass: '#67A33A', grassDark: '#4B842A', grassDeep: '#36661E',
    top: '#4A2E1D', topDark: '#3B2416', topLight: '#5E3B25', fibre: '#9A8660',
    dirt: '#7A4B2C', dirtDark: '#633C22', dirtLight: '#8E5B36', dirtHi: '#A26C42',
    clay: '#8A4E33', clayDark: '#733F29', clayLight: '#9E5D3E',
    pebble: '#8C8881', pebbleLight: '#B2AEA6', pebbleDark: '#6A665F',
    worm: '#DB8E8C', wormDark: '#B46B6A',
    copper: '#B8693A', copperLight: '#E39C60', iron: '#A0948A', ironLight: '#CFC3B8',
    stone: '#77767D', stoneDark: '#606066', stoneLight: '#8F8E95', crack: '#4D4C53',
    bed: '#29272E', bedDark: '#1F1D23', bedLight: '#34313A', bedGlint: '#4B4557',
    wall: '#3A2415', wallDark: '#2C1B10',
    root: '#E6D5A6', rootMid: '#CCB787', rootDark: '#A99668', hair: '#D8C697', hypha: '#C2AF84',
    pale: '#C4DC98', paleDark: '#9FC06F', paleLight: '#DCEBBE',
    stemDark: '#3A7426', stem: '#569C37', stemLight: '#7CBD51',
    leafDark: '#326921', leaf: '#4E9232', leafLight: '#73B64A', vein: '#94CC66',
    bushDark: '#2E6427', bush: '#3F8634', bushLight: '#5CA845', berry: '#D8434A',
    petal: '#F6C21D', petalLight: '#FFE068', petalShade: '#E29F13', petalInner: '#D68511', petalEdge: '#AE5F0D',
    disc: '#5C351B', discDark: '#3D2311', discLight: '#7C4925', discRim: '#2C190B',
    seed: '#4A3A2A', seedEdge: '#2A1E14', seedStripe: '#D8C89E',
    key: '#F0BC38', keyLight: '#FFE48A', keyDark: '#A57416',
    caneDark: '#6F8A33', cane: '#9CB84A', caneLight: '#C6DC7E', caneNode: '#57701F', caneDry: '#B9A85A',
    panelEdge: '#15131A', panelHi: '#5C5568', panelLo: '#38333F', panelLine: '#4B4557',
    panelFill: '#26232C', panelFill2: '#2B2833', panelText: '#F3EFE6', panelShade: '#15131A',
    panelArrow: '#EADD3E', panelArrowOff: '#5C5868',
    bone: '#EDE6D2', boneShade: '#C4BA9E', boneEdge: '#8F8670',
    gold: '#F2C744', goldLight: '#FFE68A', goldDark: '#B8871E', chest: '#7A4A22', chestDark: '#4F2E14',
    bird: '#3A4450', cloud: '#F6F9FB', cloudLight: '#FFFFFF', cloudShade: '#D9E5EE',
    sparkle: '#FFF6C8', white: '#FFFFFF', crackDark: '#1E140C', paper: '#FFF8E8'
  };
  for (var k0 in SRC) P[k0] = hex(SRC[k0]);
  /* The wordmark's colours, sealed in the rock, and what a delta falls back to
     when its author has no outfit colour of their own. */
  var GEMS = ['#8BA1C8', '#EADD3E', '#D793B5', '#DC7629', '#546CAC', '#CD5B7A', '#A6C8E0'].map(hex);
  var TINY = ['#F28AB2', '#FFE36A', '#FFFFFF', '#9DB8F0'].map(hex);
  var SKY = ['#8EC5EA', '#A2CFEE', '#B6DAF2', '#CAE4F4', '#DCEDF4', '#EAF4F2'].map(hex);
  function css(c) { return 'rgb(' + (c[0] | 0) + ',' + (c[1] | 0) + ',' + (c[2] | 0) + ')'; }
  function shade(c, f) { return c.map(function (v) { return f >= 0 ? v + (255 - v) * f : v * (1 + f); }); }

  /* A small canvas to paint art into, a pixel at a time. */
  function art(w, h) {
    var c = document.createElement('canvas');
    c.width = Math.max(1, w); c.height = Math.max(1, h);
    var g = c.getContext('2d');
    return { c: c, g: g, put: function (x, y, col) {
      if (x < 0 || y < 0 || x >= w || y >= h) return;
      g.fillStyle = css(col); g.fillRect(x, y, 1, 1);
    } };
  }

  /* ── THE SKY: a dithered gradient, clouds, birds ─────────────────────── */
  var BAYER = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
  function skyArt(cols, rows) {
    var c = document.createElement('canvas');
    c.width = cols; c.height = rows;
    var g = c.getContext('2d'), img = g.createImageData(cols, rows), d = img.data;
    for (var y = 0; y < rows; y++) {
      var f = y / Math.max(1, rows - 1) * (SKY.length - 1), bi = Math.floor(f), fr = f - bi;
      for (var x = 0; x < cols; x++) {
        var col = SKY[Math.min(SKY.length - 1, bi + (fr > (BAYER[y & 3][x & 3] + 0.5) / 16 ? 1 : 0))], i = (y * cols + x) * 4;
        d[i] = col[0]; d[i + 1] = col[1]; d[i + 2] = col[2]; d[i + 3] = 255;
      }
    }
    g.putImageData(img, 0, 0);
    return c;
  }
  /* A puffy Terraria cloud: a few round lobes on a flat underside. */
  function cloudArt(w, h, salt) {
    var A = art(w, h), n = 3 + Math.floor(hash(salt, 1) * 3), lobes = [];
    for (var k = 0; k < n; k++) {
      lobes.push([w * (0.18 + 0.64 * k / (n - 1)) + (hash(salt, k + 5) - 0.5) * 4, h * 0.62, h * (0.34 + hash(salt, k + 9) * 0.28)]);
    }
    function inside(x, y) {
      if (y > h * 0.86) return false;
      return lobes.some(function (l) { return Math.hypot(x - l[0], (y - l[1]) * 1.15) <= l[2]; });
    }
    for (var y = 0; y < h; y++) for (var x = 0; x < w; x++) {
      if (!inside(x + 0.5, y + 0.5)) continue;
      var under = !inside(x + 0.5, y + 2.5), over = !inside(x + 0.5, y - 1.5);
      A.put(x, y, under ? P.cloudShade : over && x < w * 0.6 ? P.cloudLight : P.cloud);
    }
    return A.c;
  }
  var BIRD = [['#...#', '.#.#.', '..#..'], ['.....', '##.##', '..#..']];
  /* WHERE THE CLOUDS ARE, in art pixels down from the top of the window. One
     answer for two readers: the sky draws them, and the stacks are not allowed
     to reach them. */
  function cloudPlan(cellAp) {
    return [0, 1, 2, 3, 4, 5].map(function (n) {
      var w = Math.round(cellAp * (0.7 + hash(n, 31) * 0.8));
      return { w: w, h: Math.round(w * (0.3 + hash(n, 32) * 0.1)),
               y: Math.round(4 + hash(n, 34) * cellAp * 1.6), v: 0.0018 + hash(n, 35) * 0.0028 };
    });
  }
  /* THE SUN IS BEHIND THE PAGE'S OWN MARK (2026-09-19). It was a second
     drawing of the mark — brand/wallflowers-icon.svg brought down to the art
     pixel — standing in the sky at four fifths across, while the real one stood
     in the header, centre top, above this layer. Two of the mark on one screen,
     and the sky's was the copy. Now the sky draws only the glow, a dithered warm
     ring as wide as the disc it rings, around wherever the page says its mark
     is (o.sun), and the header's mark is the sun's face. No mark, no sun: the
     page has hidden its mark, so the sky has nothing to put behind. */
  var SUN_ART = {};
  function sunArt(size) {
    if (SUN_ART[size]) return SUN_ART[size];
    var halo = Math.max(4, Math.round(size * 0.4)), S = size + 2 * halo, A = art(S, S), x, y;
    var glow = hex('#FFF4C8');
    for (y = 0; y < S; y++) for (x = 0; x < S; x++) {
      var f = (Math.hypot(x + 0.5 - S / 2, y + 0.5 - S / 2) - size / 2) / halo;
      if (f > -0.2 && f < 1 && (1 - f) * 16 > BAYER[y & 3][x & 3] + 0.5) A.put(x, y, glow);
    }
    return (SUN_ART[size] = { c: A.c, S: S });
  }

  /* ── THE GROUND ──────────────────────────────────────────────────────────
     Terraria's cross-section, one texel per art pixel: grass, tall grass,
     flowers and bushes on the lip; a deep band of rich soil — dark topsoil
     threaded with grass roots, then dirt with clay, clumps, pebbles, ore, the
     odd worm, old bones and a buried chest; a seam of stone with the wordmark's
     colours in it; and bedrock at the foot, dark and quiet, where the words
     sit. Each 8x8 block carries a shade of its own, the way Terraria's tiles
     vary. `keep` is a column the bushes stay clear of: the sunflower's. */
  function groundTexture(cols, rows, soilRows, stoneRows, keep, noChest) {   // noChest: leave the buried chest out (the Instagram grid draws its own)
    var c = document.createElement('canvas');
    c.width = cols; c.height = rows;
    var g = c.getContext('2d'), img = g.createImageData(cols, rows), d = img.data;
    var mat = new Uint8Array(cols * rows);         // 0 air, 1 grass, 2 topsoil, 3 dirt, 4 clay, 5 stone, 6 bedrock
    function put(x, y, col, m) {
      if (x < 0 || y < 0 || x >= cols || y >= rows) return;
      var i = (y * cols + x) * 4;
      d[i] = col[0]; d[i + 1] = col[1]; d[i + 2] = col[2]; d[i + 3] = 255;
      if (m !== undefined) mat[y * cols + x] = m;
    }
    function tone(col, x, y, amt) {
      var f = 1 + (hash(x >> 3, (y >> 3) + 777) - 0.5) * amt;
      return [Math.min(255, col[0] * f), Math.min(255, col[1] * f), Math.min(255, col[2] * f)];
    }
    function at(x, y) { return x < 0 || y < 0 || x >= cols || y >= rows ? 0 : mat[y * cols + x]; }
    function stamp(rows2, x0, y0, pal) {           // a little picture, where it lands in the soil
      rows2.forEach(function (row, j) {
        for (var i = 0; i < row.length; i++) if (pal[row[i]]) put(x0 + i, y0 + j, pal[row[i]]);
      });
    }
    var x, y, k, r;
    for (x = 0; x < cols; x++) {
      var soilEnd = LEAD + soilRows + Math.round((noise1(x, 13, 3) - 0.5) * 10);
      var stoneEnd = soilEnd + stoneRows + Math.round((noise1(x, 9, 5) - 0.5) * 8);
      var topEnd = LEAD + Math.round(soilRows * 0.24 + (noise1(x, 11, 6) - 0.5) * 8);
      var drip = hash(x, 21) < 0.4 ? 1 + Math.floor(hash(x, 22) * 3) : 0;
      for (y = LEAD; y < rows; y++) {
        k = y - LEAD; r = hash(x, y);
        if (k === 0) put(x, y, r < 0.5 ? P.grassTip : P.grassLight, 1);
        else if (k === 1) put(x, y, r < 0.3 ? P.grassLight : P.grass, 1);
        else if (k === 2) put(x, y, r < 0.4 ? P.grass : P.grassDark, 1);
        else if (k < 3 + drip) put(x, y, P.grassDark, 1);
        else if (k === 3 + drip) put(x, y, r < 0.5 ? P.grassDeep : P.topDark, 1);
        else if (y < soilEnd) {
          if (y < topEnd) put(x, y, tone(r < 0.28 ? P.topDark : r < 0.5 ? P.topLight : P.top, x, y, 0.1), 2);
          else if (noise2(x, y, 10, 31) > 0.76) put(x, y, tone(r < 0.22 ? P.clayDark : r < 0.36 ? P.clayLight : P.clay, x, y, 0.08), 4);
          else {
            var clump = noise2(x, y, 5, 41) > 0.74;
            put(x, y, tone(clump ? (r < 0.5 ? P.dirtDark : P.topLight)
                         : r < 0.16 ? P.dirtDark : r < 0.27 ? P.dirtLight : r < 0.3 ? P.dirtHi : P.dirt, x, y, 0.1), 3);
          }
        } else if (y < stoneEnd) put(x, y, tone(r < 0.2 ? P.stoneDark : r < 0.32 ? P.stoneLight : P.stone, x, y, 0.08), 5);
        else put(x, y, r < 0.16 ? P.bedDark : r < 0.22 ? P.bedLight : P.bed, 6);
      }
    }
    for (y = LEAD; y < rows; y++) for (x = 0; x < cols; x++) {
      var m = at(x, y), f;
      if (m === 2 && hash(x * 3 + 7, y * 5 + 1) < 0.004)            // a grass root, down from the lip
        for (f = 0; f < 3 + Math.floor(hash(x, y + 5) * 5); f++) if (at(x, y + f) === 2) put(x, y + f, P.fibre);
      if ((m === 2 || m === 3) && hash(x * 5 + 1, y * 7 + 3) < 0.0035) {  // a pebble
        put(x, y, P.pebbleLight); put(x + 1, y, P.pebble); put(x, y + 1, P.pebble); put(x + 1, y + 1, P.pebbleDark);
        if (hash(x, y + 3) < 0.5) { put(x + 2, y, P.pebble); put(x + 2, y + 1, P.pebbleDark); }
      }
      if (m === 3 && hash(x * 13 + 5, y * 11 + 9) < 0.0005)            // a worm
        [0, -1, -1, 0, 0].forEach(function (o, i) { put(x + i, y + o, i === 0 || i === 4 ? P.wormDark : P.worm); });
      if ((m === 3 || m === 4) && y > LEAD + soilRows * 0.5 && hash(x * 17 + 3, y * 19 + 1) < 0.0009) {
        var cu = hash(x, y + 7) < 0.6;                                 // a seam of copper, or of iron
        [[0, 0], [1, 0], [0, 1], [1, 1], [2, 1], [1, 2]].forEach(function (p, i) {
          put(x + p[0], y + p[1], i === 0 ? (cu ? P.copperLight : P.ironLight) : (cu ? P.copper : P.iron));
        });
      }
      if (m === 5 && hash(x * 3 + 1, y * 5 + 2) < 0.012) {            // a gem
        var gem = GEMS[Math.floor(hash(x, y + 99) * GEMS.length)];
        put(x, y, shade(gem, 0.3)); put(x + 1, y, gem); put(x, y + 1, gem); put(x + 1, y + 1, shade(gem, -0.2));
      }
      if (m === 5 && (x & 7) === 0 && (y & 7) === 0 && hash(x, y + 91) < 0.4)   // a crack in a stone block
        for (f = 0; f < 3; f++) if (at(x + 2 + f, y + 2 + f) === 5) put(x + 2 + f, y + 2 + f, P.crack);
      if (m === 6 && hash(x * 7 + 3, y * 3 + 8) < 0.0015) put(x, y, P.bedGlint);
    }
    /* what the ground has kept: old bones, a skull, a chest and its coins */
    var BONE = { b: P.bone, s: P.boneShade, e: P.boneEdge };
    stamp(['.ee....ee.', 'ebbeeeebbe', 'essbbbbsse', '.ee....ee.'], Math.round(cols * 0.16), LEAD + Math.round(soilRows * 0.62), BONE);
    stamp(['.ee.', 'ebbe', '.eb.', '.eb.', '.eb.', '.eb.', 'ebbe', '.ee.'], Math.round(cols * 0.47), LEAD + Math.round(soilRows * 0.78), BONE);
    stamp(['..eeee..', '.ebbbbe.', 'ebbbbbbe', 'eb.bb.be', 'ebbbbbbe', '.ebssbe.', '..eeee..'], Math.round(cols * 0.33), LEAD + Math.round(soilRows * 0.7), BONE);
    var CH = { o: P.chestDark, w: P.chest, g: P.gold, l: P.goldLight, d: P.goldDark };
    var cx0 = Math.round(cols * 0.74), cy0 = LEAD + Math.round(soilRows * 0.8);
    if (!noChest) {
      stamp(['.oooooooooooo.', 'owwwwwgwwwwwwo', 'owwwwwgwwwwwwo', 'oggggggggggggo', 'owwwwldwwwwwwo', 'owwwwwdwwwwwwo', 'owwwwwgwwwwwwo', 'oooooooooooooo'], cx0, cy0, CH);
      [[-4, 6], [-2, 7], [16, 7], [18, 6], [15, 5], [-3, 3]].forEach(function (p, i) {
        put(cx0 + p[0], cy0 + p[1], i & 1 ? P.gold : P.goldLight); put(cx0 + p[0] + 1, cy0 + p[1], P.goldDark);
      });
    }
    /* on the lip: blades, tall grass, a flower here and there, and bushes */
    for (x = 0; x < cols; x++) {
      var hb = hash(x, 7);
      var h = hb < 0.35 ? 0 : hb < 0.62 ? 1 + Math.floor(hash(x, 8) * 2) : hb < 0.9 ? 3 + Math.floor(hash(x, 9) * 3) : 6 + Math.floor(hash(x, 10) * 4);
      var lean = h > 3 ? (hash(x, 12) < 0.5 ? -1 : 1) : 0;
      for (var b = 1; b <= h; b++) put(x + (b > h - 2 && b > 2 ? lean : 0), LEAD - b, b === h ? P.grassTip : b > h * 0.5 ? P.grassLight : P.grass);
      if (h >= 4 && hash(x, 13) < 0.11) {
        var fc = TINY[Math.floor(hash(x, 14) * TINY.length)], fx = x + lean, fy = LEAD - h - 1;
        put(fx, fy - 1, fc); put(fx - 1, fy, fc); put(fx + 1, fy, fc); put(fx, fy + 1, fc); put(fx, fy, P.petalLight);
      }
    }
    for (x = 6; x < cols - 6; x += 22 + Math.floor(hash(x, 71) * 34)) {
      if (Math.abs(x - keep) < 16 || hash(x, 72) < 0.3) continue;
      var bw = 10 + Math.floor(hash(x, 73) * 9), bh = 6 + Math.floor(hash(x, 74) * 6), lobes = [];
      for (var q = 0; q < 3; q++) lobes.push([x + bw * (0.25 + 0.25 * q), LEAD - bh * 0.45, bh * (0.5 + hash(x, 75 + q) * 0.3)]);
      for (y = LEAD - bh - 2; y < LEAD; y++) for (var bx = x - 2; bx < x + bw + 2; bx++) {
        var inside = function (xx, yy) { return lobes.some(function (l) { return Math.hypot(xx - l[0], (yy - l[1]) * 1.2) <= l[2]; }); };
        if (!inside(bx + 0.5, y + 0.5)) continue;
        var edge = !inside(bx + 0.5, y - 0.5) || !inside(bx - 0.5, y + 0.5) || !inside(bx + 1.5, y + 0.5);
        var col = edge && y < LEAD - 2 ? P.bushLight : hash(bx, y) < 0.18 ? P.bushDark : y > LEAD - 3 ? P.bushDark : P.bush;
        if (!edge && hash(bx * 3, y * 7) < 0.04) col = hash(x, 76) < 0.5 ? P.berry : TINY[Math.floor(hash(bx, y) * TINY.length)];
        put(bx, y, col);
      }
    }
    g.putImageData(img, 0, 0);
    return c;
  }

  /* ── THE SUNFLOWER ───────────────────────────────────────────────────────
     A Terraria sunflower at the art pixel: a three-pixel stem, four leaves that
     climb it one a step, and a head of sixteen pointed petals round a disc of
     seeds. Its parts are painted once per size; growing is showing more of them. */
  /* ── A LEAF ──────────────────────────────────────────────────────────────
     Out from where it joins the stem at `deg` above the horizontal, arching
     over as it goes; widest in the middle, a vein down the centre, a darker
     edge. DRAWN AT THE ANGLE IT IS HOLDING, one picture per angle: a pixel leaf
     turned by the canvas is mush, and a leaf here turns — it opens upright at
     the top of the stem and swings down to where it ends up as the stem grows
     past it. `dir` is +1 for the right of the stem, -1 for the left, and `open`
     is how far out it has come. The pixel it joins the stem at is (bx, by). */
  function leafArt(len, wid, deg, dir, open) {
    len = Math.max(2, Math.round(len * open)); wid = Math.max(1, wid * open);
    var a = deg * Math.PI / 180, arch = 0.34;
    var dxu = Math.cos(a) * dir, dyu = -Math.sin(a);              // along the leaf, screen y down
    var pxu = Math.sin(a) * dir, pyu = Math.cos(a);               // across it, on the underside
    var pts = [], i, lo = [1e9, 1e9], hi = [-1e9, -1e9];
    for (i = 0; i <= len; i++) {
      var u = i / len, along = u * len, off = arch * len * u * u;
      var x = dxu * along + pxu * off, y = dyu * along + pyu * off;
      var hw = wid * Math.pow(Math.sin(Math.PI * Math.min(0.98, (i + 0.6) / (len + 1))), 0.75);
      pts.push([x, y, hw, u]);
      lo[0] = Math.min(lo[0], x - hw - 1); lo[1] = Math.min(lo[1], y - hw - 1);
      hi[0] = Math.max(hi[0], x + hw + 1); hi[1] = Math.max(hi[1], y + hw + 1);
    }
    var bx = Math.ceil(-lo[0]), by = Math.ceil(-lo[1]);
    var A = art(bx + Math.ceil(hi[0]) + 1, by + Math.ceil(hi[1]) + 1);
    pts.forEach(function (pt) {
      for (var s2 = -pt[2]; s2 <= pt[2]; s2 += 0.34) {
        var edge = Math.abs(s2) > pt[2] - 0.7, vein = Math.abs(s2) < 0.34 && pt[3] < 0.85;
        A.put(Math.round(bx + pt[0] + pxu * s2), Math.round(by + pt[1] + pyu * s2),
              edge ? P.leafDark : vein ? P.vein : s2 < 0 ? P.leafLight : P.leaf);
      }
    });
    return { c: A.c, bx: bx, by: by };
  }
  function bloomArt(Rb, Rd, s) {
    var S = 2 * Rb + 5, A = art(S, S), cx = S / 2, cy = S / 2;
    var R = Rd + (Rb - Rd) * s, rd = Rd * (0.8 + 0.2 * s), PET = 16;
    function kind(x, y) {
      var dx = x + 0.5 - cx, dy = y + 0.5 - cy, r = Math.hypot(dx, dy);
      if (r <= rd) return 1;
      var f = Math.abs(Math.cos(PET / 2 * Math.atan2(dy, dx)));
      if (r > rd + (R - rd) * (0.42 + 0.58 * Math.pow(f, 0.6))) return 0;
      if (f < 0.16 && r > rd + 1.4) return 0;
      return 2;
    }
    for (var y = 0; y < S; y++) for (var x = 0; x < S; x++) {
      var k = kind(x, y);
      if (!k) continue;
      var dx = x + 0.5 - cx, dy = y + 0.5 - cy, r = Math.hypot(dx, dy), col;
      if (k === 1) {
        var ring = r > rd - 1;
        var spiral = Math.floor(Math.atan2(dy, dx) * 4 / Math.PI + r * 1.2) & 1;
        col = ring ? P.discRim : dx < -0.5 && dy < -0.5 && r < rd * 0.75 && spiral ? P.discLight : spiral ? P.disc : P.discDark;
      } else {
        var rim = kind(x - 1, y) === 0 || kind(x + 1, y) === 0 || kind(x, y - 1) === 0 || kind(x, y + 1) === 0;
        col = rim ? P.petalEdge : r < rd + 1.6 ? P.petalInner : r > rd + (R - rd) * 0.66 ? P.petalLight : dy > R * 0.3 ? P.petalShade : P.petal;
      }
      A.put(x, y, col);
    }
    return A.c;
  }
  function budArt(Rb, Rd) {
    var S = 2 * Rb + 5, A = art(S, S), cx = S / 2, cy = S / 2 + Rd * 0.3, rx = Rd * 0.85, ry = Rd * 1.25;
    function inside(x, y) { var dx = (x + 0.5 - cx) / rx, dy = (y + 0.5 - cy) / ry; return dx * dx + dy * dy <= 1; }
    for (var y = 0; y < S; y++) for (var x = 0; x < S; x++) {
      if (!inside(x, y)) continue;
      var rim = !inside(x - 1, y) || !inside(x + 1, y) || !inside(x, y - 1) || !inside(x, y + 1);
      A.put(x, y, rim ? P.leafDark : y + 0.5 < cy - ry * 0.55 ? P.petalLight : x + 0.5 < cx ? P.leafLight : P.leaf);
    }
    return A.c;
  }
  /* HOW A LEAF SITS. It opens at the top of the stem all but upright, and it
     holds the stem's end until the stem grows on — and over that growth it
     swings down to where it stays. */
  var LEAF_NEW = 74;                                      // degrees above the horizontal: just off vertical
  function leafRest(k) { return 12 + (k % 3) * 7; }       // where it settles, a little different each time
  function flowerArt(cellAp) {
    var Ht = Math.round(5 * cellAp), Rb = Math.max(11, Math.round(0.47 * cellAp));  // the head, half again as big
    var Rd = Math.max(5, Math.round(Rb * 0.42)), Hs = Ht - Rb;
    /* A slimmer stalk and smaller leaves than the head: three pixels of stalk,
       and leaves measured off the STEM (not the tile) so a stem this tall is
       neither bare nor swamped. */
    var SW = 3, stem = art(SW, Hs);
    for (var y = 0; y < Hs; y++) { stem.put(0, y, P.stemDark); stem.put(1, y, P.stem); stem.put(2, y, P.stemLight); }
    var len = Math.max(8, Math.round(Hs * 0.12)), wid = Math.max(2, Math.round(len * 0.27));
    return {
      Hs: Hs, Rb: Rb, S: 2 * Rb + 5, stem: stem.c, sw: SW, len: len, wid: wid, cut: {},
      /* six, one to a link: each joins the stem AT the height the stem has
         reached when its link is found, so the stem ends at that leaf until the
         next step raises a bare tip past it */
      leaves: [0, 1, 2, 3, 4, 5].map(function (k) {
        return { y: Math.round(Hs * GROW[k + 1]) - 1, dir: k & 1 ? -1 : 1, rest: leafRest(k) };
      }),
      blooms: [budArt(Rb, Rd), bloomArt(Rb, Rd, 0.5), bloomArt(Rb, Rd, 0.78), bloomArt(Rb, Rd, 1)]
    };
  }

  /* What the character holds: a golden key, and a striped sunflower seed. */
  var KEY_ART = null, SEED_ART = null;
  function keyArt() {
    if (KEY_ART) return KEY_ART;
    var A = art(19, 9);
    for (var y = 0; y < 9; y++) for (var x = 0; x < 9; x++) {
      var r = Math.hypot(x + 0.5 - 4.5, y + 0.5 - 4.5);
      if (r <= 4.4 && r >= 1.9) A.put(x, y, y < 4 ? P.keyLight : r > 3.5 ? P.keyDark : P.key);
    }
    for (x = 9; x < 19; x++) { A.put(x, 4, P.keyLight); A.put(x, 5, P.key); }
    A.put(13, 6, P.key); A.put(14, 6, P.key); A.put(13, 7, P.keyDark); A.put(14, 7, P.keyDark);
    A.put(16, 6, P.key); A.put(17, 6, P.key); A.put(18, 6, P.key); A.put(16, 7, P.key); A.put(18, 7, P.key);
    A.put(16, 8, P.keyDark); A.put(18, 8, P.keyDark);
    return (KEY_ART = A.c);
  }
  function seedArt() {
    if (SEED_ART) return SEED_ART;
    var A = art(4, 6), rows = ['.ee.', 'ebse', 'ebse', 'ebse', 'ebbe', '.ee.'];
    rows.forEach(function (row, y) {
      for (var x = 0; x < 4; x++) {
        var ch = row[x];
        if (ch !== '.') A.put(x, y, ch === 'e' ? P.seedEdge : ch === 's' ? P.seedStripe : P.seed);
      }
    });
    return (SEED_ART = A.c);
  }

  /* ── THE EGG ─────────────────────────────────────────────────────────────
     Your Arc, as an egg: panelled copper, a small screen, and a keyhole. It
     stands on its base, point up, and is turned 45 degrees about its long axis
     toward you — so its screen and keyhole have come round to the right, where a
     key can go in straight from your side. Turning a solid of revolution about
     its axis leaves its outline and its light alone and moves only what is on
     its surface, so the panels, the screen and the keyhole are laid out on the
     surface in the egg's own frame and carried round by TURN: they slide right
     and foreshorten as they go. Two paintings — at rest, and lit for the moment
     it opens. H is its height in art pixels. */
  var EGG = null;
  function eggArt(H) {
    if (EGG && EGG.H === H) return EGG;
    var W = Math.round(H * 0.8) | 1, a = W / 2, CW = W + 2, cx = CW / 2, CH = H + 1;
    var TURN = Math.PI / 4;
    var C = {
      dark: hex('#4A2616'), shadow: hex('#7A3F22'), mid: hex('#B0602F'), light: hex('#D98A4F'), hi: hex('#F3C08A'),
      seam: hex('#5A2E19'), rivet: hex('#F6C99A'), bezel: hex('#3B2A22'), glass: hex('#13201A'),
      text: hex('#5FD35A'), lit: hex('#8CF27A'), hole: hex('#1A0E08'), brass: hex('#F0B27A')
    };
    function half(Y) { return a * Math.sqrt(Math.max(0, 1 - Y * Y)) * (0.82 + 0.18 * Y); }   // widest below the middle
    function Yof(y) { return (y + 0.5) / H * 2 - 1; }
    var sw = Math.max(7, Math.round(W * 0.32)), sh = Math.max(5, Math.round(H * 0.15)), syc = H * 0.38;
    var ky = Math.round(H * 0.72), rings = [Math.round(H * 0.275), Math.round(H * 0.63)];
    var meridians = [-0.56, 0.56, -1.66, 1.66];               // panel seams, in the egg's own frame (radians round from its front)
    function paint(lit) {
      var A = art(CW, CH), x, y, Lx = -0.55, Ly = -0.5, Lz = 0.67;
      for (y = 0; y < H; y++) {
        var Y = Yof(y), w = half(Y);
        for (x = 0; x < CW; x++) {
          var dx = x + 0.5 - cx;
          if (Math.abs(dx) > w) continue;
          var nx = dx / Math.max(w, 0.5), nz = Math.sqrt(Math.max(0, 1 - nx * nx));
          var n0 = nx * 0.9, n1 = Y * 0.55, d = (n0 * Lx + n1 * Ly + nz * Lz) / Math.hypot(n0, n1, nz);
          var band = d > 0.86 ? 4 : d > 0.62 ? 3 : d > 0.34 ? 2 : d > 0.08 ? 1 : 0;
          var col = [C.dark, C.shadow, C.mid, C.light, C.hi][band];
          var phi = Math.asin(Math.max(-1, Math.min(1, nx))), u = (phi - TURN) * w;
          var seam = rings.indexOf(y) >= 0;
          meridians.forEach(function (m) {
            var pm = m + TURN;
            if (Math.abs(pm) < Math.PI / 2 && Math.abs(phi - pm) * w < 0.55) seam = seam ? 'rivet' : true;
          });
          if (seam) col = seam === 'rivet' ? C.rivet : C.seam;
          var v = y + 0.5 - syc;                               // the screen
          if (Math.abs(u) <= sw / 2 && Math.abs(v) <= sh / 2) {
            var edge = Math.abs(u) > sw / 2 - 1 || Math.abs(v) > sh / 2 - 1;
            col = edge ? C.bezel : lit ? C.lit : C.glass;
            if (!edge && !lit && Math.abs(v) < 0.5 && Math.floor(u + 100) % 3 !== 0 && Math.abs(u) < sw / 2 - 2) col = C.text;
          }
          var vk = y - ky, rk = Math.hypot(u, vk);             // the keyhole
          if (rk <= 1.2 || (vk > 0 && vk <= 3 && Math.abs(u) < 0.6)) col = C.hole;
          else if (rk <= 2.3 && vk <= 0) col = C.brass;
          A.put(x, y, Math.abs(dx) > w - 1 ? C.dark : col);
        }
      }
      var bw = Math.round(W * 0.46);                           // the base it sits on
      for (y = H - 2; y < CH; y++) {
        var inset = y === CH - 1 ? 1 : 0;
        for (x = Math.round(cx - bw / 2) + inset; x < Math.round(cx + bw / 2) - inset; x++)
          A.put(x, y, y === H - 2 ? C.light : y === H - 1 ? C.mid : C.shadow);
      }
      return A.c;
    }
    return (EGG = { H: H, w: CW, h: CH, kx: Math.round(cx + Math.sin(TURN) * half(Yof(ky)) - 0.5), ky: ky,
                    topX: Math.floor(cx), topY: 0, c: paint(false), lit: paint(true) });
  }

  /* ── THE EXPLAINER'S PANEL ───────────────────────────────────────────────
     A display sunk into the bedrock: a slab with a bevel, an inner line, a
     one-pixel dither in the fill, and a chamfer at each corner. The arrows are
     boxes of the same make with a triangle in them, dark when there is nothing
     that way. Everything in art pixels, like the rest of the picture. */
  function panelBox(A, x0, y0, w, h, sunk) {
    for (var y = 0; y < h; y++) for (var x = 0; x < w; x++) {
      if ((x < 1 || x > w - 2) && (y < 1 || y > h - 2)) continue;              // chamfered corners
      var edge = x === 0 || y === 0 || x === w - 1 || y === h - 1;
      var bevel = x === 1 || y === 1 || x === w - 2 || y === h - 2;
      var col = edge ? P.panelEdge
              : bevel ? ((x === 1 || y === 1) !== !sunk ? P.panelHi : P.panelLo)
              : ((x + y) & 1) ? P.panelFill : P.panelFill2;
      A.put(x0 + x, y0 + y, col);
    }
  }
  function panelArt(w, h, pad, box, prevOn, nextOn) {
    var A = art(w, h), x, y;
    panelBox(A, 0, 0, w, h, false);
    for (x = 3; x < w - 3; x++) { A.put(x, 3, P.panelLine); A.put(x, h - 4, P.panelLine); }   // the inner line
    for (y = 3; y < h - 3; y++) { A.put(3, y, P.panelLine); A.put(w - 4, y, P.panelLine); }
    /* The apex goes the way the step does, so the wide end of the triangle is
       the far side: +1 draws back, -1 draws on. */
    [[pad, prevOn, 1], [w - pad - box, nextOn, -1]].forEach(function (a) {
      panelBox(A, a[0], Math.round((h - box) / 2), box, box, true);
      var col = a[1] ? P.panelArrow : P.panelArrowOff, cy = Math.round(h / 2), half = Math.floor(box / 4);
      for (var k = 0; k <= half; k++) for (var j = -k; j <= k; j++)          // a triangle, pointing its way
        A.put(a[0] + Math.round(box / 2) + a[2] * (k - Math.round(half / 2)), cy + j, col);
    });
    return A.c;
  }

  /* ── THE POLES ───────────────────────────────────────────────────────────
     One straight bamboo pole under each link: five art pixels across, round in
     the light — dark on the left, a highlight to the right of the middle — with
     a node every so often, a collar above and below it and the cane a shade
     drier at the joint. `len` is its height above the grass, in art pixels; it
     goes three more into the ground so it stands in it rather than on it. The
     top of the pole is (ox, oy) in the picture. */
  function poleArt(len) {
    var W = 5, A = art(W, len + 4), ox = 2, oy = 0, y, x;
    var joint = Math.max(7, Math.round(len / 4)), col = [P.caneDark, P.cane, P.caneLight, P.cane, P.caneDark];
    for (y = 0; y < len + 4; y++) {
      var node = y > 2 && y < len - 1 && y % joint === 0, near = y > 2 && y < len - 1 && (y + 1) % joint === 0;
      for (x = 0; x < W; x++) A.put(x, oy + y, node ? P.caneNode : near ? P.caneDry : col[x]);
    }
    return { c: A.c, ox: ox, oy: oy, w: W };
  }

  /* ── THE DELTAS ──────────────────────────────────────────────────────────
     A delta is a slab in its author's outfit colour, their face at the left of
     it — their own sprite, brought down to six by six — and a line of what they
     wrote. */
  function miniFace(Tama, spec) {
    var mood = spec.personality ? spec.personality.rest : 'neutral';
    var L = Tama.renderLayers(spec, { emotion: mood, facing: 'down', frame: 0 }), pal = Tama.palette(spec);
    var top = N, lo = N, hi = -1, x, y;
    for (y = 0; y < N; y++) for (x = 0; x < N; x++) if (L.M[y * N + x]) { if (y < top) top = y; }
    for (y = top; y < Math.min(N, top + 16); y++) for (x = 0; x < N; x++) if (L.M[y * N + x]) { if (x < lo) lo = x; if (x > hi) hi = x; }
    var side = Math.max(16, hi - lo + 1), x0 = Math.round((lo + hi + 1) / 2 - side / 2), cell = side / 6, out = [];
    for (var j = 0; j < 6; j++) for (var i = 0; i < 6; i++) {
      var r = 0, g = 0, b = 0, n = 0, tot = 0;
      for (y = Math.floor(top + j * cell); y < Math.floor(top + (j + 1) * cell); y++)
        for (x = Math.floor(x0 + i * cell); x < Math.floor(x0 + (i + 1) * cell); x++) {
          tot++;
          if (x < 0 || y < 0 || x >= N || y >= N) continue;
          var c = pal[L.M[y * N + x]];
          if (!c) continue;
          r += c[0]; g += c[1]; b += c[2]; n++;
        }
      out.push(n && n / Math.max(1, tot) > 0.3 ? [r / n, g / n, b / n] : null);
    }
    return out;
  }
  function deltaArt(col, face, w, h) {
    var A = art(w, h), edge = shade(col, -0.55), dark = shade(col, -0.3), light = shade(col, 0.28), line = shade(col, 0.5);
    for (var y = 0; y < h; y++) for (var x = 0; x < w; x++) {
      if ((x === 0 || x === w - 1) && (y === 0 || y === h - 1)) continue;
      var rim = x === 0 || y === 0 || x === w - 1 || y === h - 1;
      A.put(x, y, rim ? edge : y === 1 ? light : y === h - 2 ? dark : col);
    }
    var fy = Math.floor((h - 6) / 2);                          // the author's face, on a pale card
    for (var j = -1; j < 7; j++) for (var i = -1; i < 7; i++) if (fy + j >= 1 && fy + j < h - 1) A.put(2 + i, fy + j, P.paper);
    face.forEach(function (c, k) { if (c) A.put(2 + (k % 6), fy + Math.floor(k / 6), c); });
    var ly = Math.floor(h / 2) - 1;
    for (var lx = 11; lx < w - 3; lx++) A.put(lx, ly, line);
    for (lx = 11; lx < w - 3 - Math.floor(w / 5); lx++) A.put(lx, ly + 2, line);
    return A.c;
  }

  /* ── THE ROOTS ───────────────────────────────────────────────────────────
     Grown, not drawn: each root wanders, leans toward the ground's pull (or
     toward what it is reaching for), thins as it goes, and throws off branches
     that do the same — each branch starting as its parent passes the place it
     leaves from. A root is a list of pixels revealed a pixel at a time. */
  function hairs(pts, salt) {
    var out = [];
    for (var i = 2; i < pts.length; i++) {
      if (hash(salt, i) < 0.1) out.push({ at: i, p: [pts[i][0] + (hash(i, salt) < 0.5 ? -1 : 1), pts[i][1] + (hash(salt + 1, i) < 0.5 ? 0 : 1)] });
    }
    return out;
  }
  /*  grow(list, o) — o: { x, y, ang, len, thick, at, dur, salt, depth, target, pull, wob, lim,
                            fine, retract, cord }
      target: [x, y] to steer for (and stop at), or null to lean down; wob, how
      much it wanders (1 by default: a root reaching for something wanders less,
      or it loops on itself short of what it reaches for).
      THE SLIME MOLD'S RULES, for the network that finds your chain (Tero et
      al.'s adaptive network; a foraging mycelium does the same with cords):
      fine fronts go out exploring (`fine`); the one that reaches food thickens
      into a cord as the flow through it grows (`cord`: from then, thick along
      its length); every other front, and every side branch off the one that
      found it, is pulled back (`retract`: [at, dur, keep], reeled in tip first
      to `keep` of its length). Not all the way, for most: a mycelium keeps a
      fine web of the ground it has explored, so the soil fills with it a link
      at a time. A branch keeps some of itself only if the place it leaves its
      parent from is kept. */
  function grow(list, o) {
    var pts = [], fx = o.x, fy = o.y, ang = o.ang, lx = 1e9, ly = 1e9, s, kids = [], wob = o.wob === undefined ? 1 : o.wob;
    for (s = 0; s < o.len; s++) {
      var want = o.target ? Math.atan2(o.target[1] - fy, o.target[0] - fx) : Math.PI / 2;
      var dAng = Math.atan2(Math.sin(want - ang), Math.cos(want - ang));
      /* homing: close to what it reaches for it turns in hard, or it circles it */
      var near = o.target ? Math.hypot(o.target[0] - fx, o.target[1] - fy) : 1e9;
      ang += dAng * ((o.pull || 0.06) + (near < 14 ? (1 - near / 14) * 0.6 : 0))
           + ((noise1(s, 5, o.salt) - 0.5) * 0.5 + (hash(o.salt, s) - 0.5) * 0.25) * wob * (near < 14 ? near / 14 : 1);
      fx += Math.cos(ang); fy += Math.sin(ang);
      if (fy > o.lim[1]) { fy = o.lim[1]; ang = -Math.abs(ang) * 0.3; }
      if (fy < o.lim[0]) { fy = o.lim[0]; ang = Math.abs(ang) * 0.3 + 0.3; }
      var ix = Math.round(fx), iy = Math.round(fy);
      if (ix !== lx || iy !== ly) { pts.push([ix, iy]); lx = ix; ly = iy; }
      if (o.target && Math.hypot(o.target[0] - fx, o.target[1] - fy) < 2.2) break;
      if (o.depth < 3 && s > 4 && s < o.len - 4 && hash(o.salt * 7 + 1, s) < (o.fine ? (o.depth < 2 ? 0.1 : 0.07) : o.depth ? 0.045 : 0.08)) {
        var side = hash(o.salt, s + 99) < 0.5 ? -1 : 1;
        kids.push({ at: pts.length, x: fx, y: fy, ang: ang + side * (0.5 + hash(o.salt, s + 7) * 0.6),
                    len: Math.round((o.len - s) * (0.3 + hash(o.salt, s + 3) * 0.35)) + 3 });
      }
    }
    var me = { pts: pts, at: o.at, dur: o.dur, thick: o.thick, fine: !!o.fine, cord: o.cord,
               retract: o.cord === undefined ? o.retract : null, hairs: o.fine && o.cord === undefined ? [] : hairs(pts, o.salt) };
    list.push(me);
    var keptN = me.retract ? Math.floor(me.retract[2] * pts.length) : pts.length;
    kids.forEach(function (k, i) {
      var ret = o.retract ? [o.retract[0], o.retract[1], k.at < keptN - 1 ? 0.2 + hash(o.salt * 3 + i, 61) * 0.5 : 0] : undefined;
      grow(list, { x: k.x, y: k.y, ang: k.ang, len: k.len, thick: false, salt: o.salt * 13 + i + 1, depth: o.depth + 1,
                   at: o.at + o.dur * k.at / Math.max(1, pts.length), dur: o.dur * k.len / Math.max(1, o.len) * 1.2,
                   target: null, pull: 0.05, wob: o.wob, lim: o.lim, fine: o.fine, retract: ret });
    });
    return me;
  }

  /* ── DRAWING ─────────────────────────────────────────────────────────── */
  var UNIT = null;
  /* The grid's own outline, power 4, on the unit square. */
  function squircle() {
    if (UNIT) return UNIT;
    UNIT = new Path2D();
    for (var i = 0; i <= 96; i++) {
      var t = i / 96 * Math.PI * 2, co = Math.cos(t), si = Math.sin(t);
      var x = Math.sign(co) * Math.sqrt(Math.abs(co)), y = Math.sign(si) * Math.sqrt(Math.abs(si));
      if (i) UNIT.lineTo(x, y); else UNIT.moveTo(x, y);
    }
    UNIT.closePath();
    return UNIT;
  }
  /* One tile, always at its cell's size, and SMOOTHED — see ONE PIXEL SIZE.
     `lift` is a faint shadow a new tile carries and a seat taken over from the
     renderer does not. */
  function drawTile(g, img, b, a, dpr, lift) {
    if (!img || !b || a <= 0.002) return;
    var hw = b[2] / 2, hh = b[3] / 2;
    g.save();
    g.imageSmoothingEnabled = true;
    g.imageSmoothingQuality = 'high';
    g.globalAlpha = a;
    g.translate((b[0] + hw) * dpr, (b[1] + hh) * dpr);
    g.scale(hw * dpr, hh * dpr);
    if (lift) {
      g.shadowColor = 'rgba(20,24,30,' + (0.14 * lift) + ')';
      g.shadowBlur = 7 * dpr; g.shadowOffsetY = 2 * dpr;
      g.fillStyle = '#E4E0D7';
      g.fill(squircle());
      g.shadowColor = 'transparent';
    }
    g.clip(squircle());
    g.drawImage(img, -1, -1, 2, 2);
    g.restore();
  }

  /* ── THE PAGE'S OWN LAYER ────────────────────────────────────────────── */
  var CSS = [
    '.how-canvas { position: fixed; inset: 0; width: 100%; height: 100%; z-index: 5; pointer-events: none; }',
    /* THE WORDS ARE A PICTURE, and this is the same words for a reader that
       cannot see it: off screen, spoken when they change. */
    '.how-cap { position: fixed; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden;',
    '  clip-path: inset(50%); white-space: nowrap; border: 0; }',
    /* The steps are the drawn arrows: the button is the hit area over one. */
    '.how-step { position: fixed; z-index: 6; margin: 0; padding: 0; background: none; border: 0;',
    '  cursor: pointer; pointer-events: auto; -webkit-tap-highlight-color: transparent; }',
    '.how-step[hidden] { display: none; }',
    '.how-step:disabled { cursor: default; }',
    '.how-step:focus-visible { outline: 2px solid rgba(var(--opal), .95); outline-offset: 2px; }',
    'body.how-on #nav, body.how-on #pages { opacity: 0 !important; pointer-events: none !important; transition: opacity .3s ease; }',
    /* a link's button, laid over its board, with a ? in the corner */
    '.how-board { position: fixed; z-index: 6; margin: 0; padding: 0; background: none; border: 0; border-radius: 24%;',
    '  cursor: pointer; pointer-events: auto; -webkit-tap-highlight-color: transparent; }',
    '.how-board[hidden] { display: none; }',
    '.how-board:focus-visible { outline: 2px solid rgba(var(--opal), .95); outline-offset: 3px; }',
    '.how-board-q { position: absolute; top: -7%; right: -7%; display: flex; align-items: center; justify-content: center;',
    '  width: 1.6em; height: 1.6em; font: 600 clamp(10px, calc(var(--how-cell, 80px) * .14), 15px)/1 var(--mono);',
    '  color: #F3EFE6; background: #29272E; border: 1px solid rgba(243,239,230,.55); border-radius: 3px; }',
    '.how-board:hover .how-board-q, .how-board:focus-visible .how-board-q { color: #29272E; background: #FFE068; border-color: #29272E; }',
    /* its card */
    '.how-modal { position: fixed; inset: 0; z-index: 30; display: flex; align-items: center; justify-content: center;',
    '  padding: 16px; background: rgba(14,13,18,.55); }',
    '.how-modal[hidden] { display: none; }',
    '.how-card { box-sizing: border-box; width: min(35rem, 100%); max-height: calc(100vh - 32px); max-height: calc(100dvh - 32px); overflow: auto;',
    '  padding: 22px 22px 18px; color: #F3EFE6; background: #29272E; border: 2px solid #4B4557; border-radius: 4px;',
    '  box-shadow: 0 0 0 2px #1F1D23, 0 18px 50px rgba(0,0,0,.45); font: 400 13.5px/1.6 var(--mono); text-align: left; }',
    '.how-card:focus { outline: none; }',
    '.how-card-kicker { margin: 0 0 4px; font-size: 11px; letter-spacing: .12em; text-transform: uppercase; color: rgba(243,239,230,.62); }',
    '.how-card h2 { margin: 0 0 10px; font: 600 21px/1.2 var(--mono); letter-spacing: .01em; color: #FFFFFF; }',
    '.how-card p { margin: 0 0 14px; }',
    '.how-card .under { border-top: 1px solid rgba(243,239,230,.16); padding-top: 14px; color: rgba(243,239,230,.7); }',
    '.how-card-close { font: inherit; font-size: 13px; letter-spacing: .06em; color: rgba(243,239,230,.9); background: none; border: 0;',
    '  border-bottom: 1px solid rgba(243,239,230,.35); padding: 0 0 2px; cursor: pointer; }',
    '.how-card-close:hover, .how-card-close:focus-visible { outline: none; border-color: rgba(var(--opal), .95); color: #FFFFFF; }',
    '@media (prefers-reduced-motion: no-preference) {',
    '  .how-modal:not([hidden]) { animation: how-fade .16s ease-out; }',
    '  .how-modal:not([hidden]) .how-card { animation: how-rise .2s ease-out; }',
    '}',
    '@keyframes how-fade { from { opacity: 0; } }',
    '@keyframes how-rise { from { transform: translateY(8px); } }'
  ].join('\n');
  var styled = false;
  function style() {
    if (styled) return;
    styled = true;
    var s = document.createElement('style');
    s.textContent = CSS;
    document.head.appendChild(s);
  }

  function add(parent, tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    parent.appendChild(e);
    return e;
  }

  /* ── THE CEREMONY ────────────────────────────────────────────────────── */
  var active = null;

  /*  o = {
        rows, cols,
        cell(r, c)       -> [x, y, w, h] in CSS px: the tile's box at a lattice
                            position, fractional and off-grid positions included
        seats            [{ id, r, c, canvas, spec }] — every character, where it stands
        chosen           the pressed seat's id
        spec             the pressed character's Tama spec, for their colours
        newCanvas()      a blank tile canvas at the page's slot size
        paint(c, R)      a device tile with LCD picture R, into canvas c
        render(state)    the pressed character's LCD picture
        mood             { rest, pet, poke } — the character's own emotions
        still            reduced motion
        sun()            -> [cx, cy, diameter] in CSS px: the page's mark, which
                            the sun stands behind; null while there is none
        hide(bool)       hide or show the grid's canvas
        onClose()        called once the board is back
      } */
  function play(o) {
    if (active) return active;
    if (o.cols !== 8 || o.rows < 4) throw new Error('ceremony: laid out for 8 columns (you, your head, six links)');
    var Tama = root.Tama;
    if (!Tama || !Tama.renderLayers || !Tama.paintColour) throw new Error('ceremony: tama.js is not loaded');
    style();
    var ROWS = o.rows, LINKS = o.cols - 2, FLOOR = ROWS - 1, RAISED = FLOOR - 1;
    /* WHERE THINGS STAND, in lattice columns. You come down at your own tile
       and walk left to the egg, which stands off the left edge of column 0. The
       flower is planted at the far right, and the chain comes up out of the
       ground from there back towards you — the first link nearest the flower,
       each next one a column further left. */
    var HEAD_C = o.cols - 1;
    function LINK_C(i) { return o.cols - 2 - i; }
    var still = !!o.still;
    var chosen = null;
    o.seats.forEach(function (s) { if (s.id === o.chosen) chosen = s; });
    if (!chosen) throw new Error('ceremony: the chosen seat is not on the board');

    /* ── the layer ── */
    var canvas = document.createElement('canvas');
    canvas.className = 'how-canvas';
    canvas.setAttribute('aria-hidden', 'true');
    canvas.style.opacity = '0';
    /* The words are drawn in the panel; this is them again, off screen, for a
       reader that cannot see it — and the two steps, which are hit areas laid
       over the arrows the panel draws. */
    var cap = document.createElement('div');
    cap.className = 'how-cap';
    cap.setAttribute('role', 'region');
    cap.setAttribute('aria-label', 'How WallFlowers works');
    var lineEl = add(cap, 'p', null, '');
    lineEl.setAttribute('aria-live', 'polite');
    var prevBtn = document.createElement('button'); prevBtn.type = 'button'; prevBtn.className = 'how-step'; prevBtn.hidden = true;
    prevBtn.setAttribute('aria-label', 'Back a step');
    var nextBtn = document.createElement('button'); nextBtn.type = 'button'; nextBtn.className = 'how-step'; nextBtn.hidden = true;
    nextBtn.setAttribute('aria-label', 'Forward a step');
    /* NOT INSIDE cap: it is clipped off screen for a screen reader, and a
       clip-path takes fixed children with it — the arrows would draw and
       answer to nothing. */
    var stepsEl = document.createElement('div');
    stepsEl.appendChild(prevBtn); stepsEl.appendChild(nextBtn);
    /* a button over each link, once it is up, and the card it opens */
    var boardsEl = document.createElement('div');
    var boardBtns = CHAIN.map(function (key, i) {
      var b = document.createElement('button');
      b.type = 'button'; b.className = 'how-board'; b.hidden = true; b.title = 'What is this?';
      b.setAttribute('aria-haspopup', 'dialog');
      b.setAttribute('aria-label', LINK[key].says + ': what is it?');
      add(b, 'span', 'how-board-q', '?').setAttribute('aria-hidden', 'true');
      b.addEventListener('click', function () { openCard(i); });
      boardsEl.appendChild(b);
      return { el: b, at: '' };
    });
    var modal = document.createElement('div'); modal.className = 'how-modal'; modal.hidden = true;
    var cardEl = add(modal, 'div', 'how-card');
    cardEl.setAttribute('role', 'dialog'); cardEl.setAttribute('aria-modal', 'true');
    cardEl.setAttribute('aria-labelledby', 'how-card-title'); cardEl.tabIndex = -1;
    var card = { open: false, i: -1, gen: 0 };
    document.body.appendChild(canvas);
    document.body.appendChild(boardsEl);
    document.body.appendChild(cap);
    document.body.appendChild(stepsEl);
    document.body.appendChild(modal);
    document.body.classList.add('how-on');
    var g = canvas.getContext('2d');

    /* ── THE PANEL'S PLAN: how big the display is and how each beat's words
       break in it, in art pixels. The height is the LONGEST beat's, so the
       panel is the same size for all eleven and the ground under it never
       moves. Worked out once per size of window. ── */
    var plan = { key: '' };
    function planPanel(ap, apDev, cw) {
      var a = o.cell(0, 0), b = o.cell(0, o.cols - 1);
      if (!a || !b) return plan;
      var mid = (a[0] + b[0] + b[2]) / 2, board = b[0] + b[2] - a[0];
      var avail = Math.min(root.innerWidth - 24, 1180);
      var key = [Math.round(mid), Math.round(board), Math.round(avail), apDev, Math.round(cw)].join();
      if (key === plan.key) return plan;
      /* AS BIG AS THE WINDOW ALLOWS AT TWO LINES, AND ONE SIZE FOR EVERY STEP.
         The panel is as wide as it needs to be to hold the longest of the words
         on two lines — wider than the board if there is room for it — and two
         lines tall whatever this step says, so it never changes shape under the
         reader. */
      var cellAp = cw / ap, s, pad, box, textW2, w, need;
      for (s = 4; s >= 1; s--) {
        pad = 3 + s; box = 9 * s + 2;
        need = BEATS.reduce(function (m, text) { return Math.max(m, twoWidth(text, s)); }, 0);
        w = Math.ceil(need) + 2 * (pad + box + 3 * s);
        if (w * ap <= avail || s === 1) break;
      }
      w = Math.max(Math.round(Math.min(board, avail) / ap), Math.min(Math.round(avail / ap), w));
      textW2 = w - 2 * (pad + box + 3 * s);
      var lines = BEATS.map(function (text) { return wrapText(text, textW2, s); });
      var most = lines.reduce(function (m, l) { return Math.max(m, l.length); }, 2);
      var left = Math.max(12, Math.min(root.innerWidth - 12 - w * ap, mid - w * ap / 2));
      var lineH = 10 * s, h = Math.max(box + 2 * pad, most * lineH + 2 * pad + 2 * s);
      boardsEl.style.setProperty('--how-cell', Math.round(a[2]) + 'px');
      plan = { key: key, left: left, w: w, h: h, s: s, pad: pad, box: box, tw: textW2, lineH: lineH, lines: lines,
               margin: Math.max(6, Math.round(cellAp * 0.22)) };
      return plan;
    }
    /* What the panel asks of the window: its own height and a margin each side. */
    function bandOf(ap, apDev, cw) {
      var pp = planPanel(ap, apDev, cw);
      return pp.h ? Math.ceil((pp.h + 2 * pp.margin) * ap) : 0;
    }

    /* ── WHERE EVERYTHING IS: the lattice, the camera, the art pixel ──
       The camera pans the scene up a row or two when the window has too little
       room under the board for deep soil and the words (a landscape screen);
       it moves in whole rows, so the tiles still stand on the lattice. */
    var dpr = 1, W = 0, H = 0;
    function lattice() {
      var a = o.cell(0, 0), b = o.cell(1, 1), f = o.cell(FLOOR, 0);
      if (!a || !b || !f) return null;
      var cw = b[0] - a[0], ch = b[1] - a[1];
      var apDev = Math.max(1, Math.round(a[2] * dpr * 7 / 256)), ap = apDev / dpr;
      var y0 = f[1] + f[3];                          // the grass line, in the scene
      var vh = root.innerHeight, bandH = bandOf(ap, apDev, cw);
      var need = 2.1 * ch + 0.3 * ch + bandH, room = vh - y0;
      var pan = Math.max(0, Math.min(2, Math.ceil((need - room) / ch)));
      return { cw: cw, ch: ch, ap: ap, apDev: apDev, y0: y0, pan: pan, bandH: bandH, vh: vh, vw: root.innerWidth };
    }
    function camY(L, tt) { return -L.pan * L.ch * (still ? (tt >= T.clear ? 1 : 0) : inOutCubic(seg(tt, T.clear, 700))); }
    function colX(c) {                                  // a column's centre in CSS px; fractional columns too
      var a = o.cell(FLOOR, 0), b = o.cell(FLOOR, 1);
      return a[0] + a[2] / 2 + c * (b[0] - a[0]);
    }
    function viewCell(r, c, cam) { var b = o.cell(r, c); return b ? [b[0], b[1] + cam, b[2], b[3]] : null; }
    /* How far below its place the ground is, as it comes up from the foot of
       the window. In the scene: add the camera to see it. */
    function groundDrop(L, tt) {
      if (still) return 0;                           // no rise: it fades in where it lies
      var travel = (L.vh - camY(L, tt)) - (L.y0 - LEAD * L.ap);
      return (1 - outCubic(seg(tt, T.ground, T.groundDur))) * Math.max(0, travel);
    }

    /* How a character's sprite sits in its 32x32: its lowest and highest rows. */
    function extent(spec, mood) {
      var Lr = Tama.renderLayers(spec, { emotion: mood, facing: 'down', frame: 0 }), top = 0, foot = N - 1, y, x;
      out: for (y = N - 1; y >= 0; y--) for (x = 0; x < N; x++) if (Lr.M[y * N + x]) { foot = y; break out; }
      up: for (y = 0; y < N; y++) for (x = 0; x < N; x++) if (Lr.M[y * N + x]) { top = y; break up; }
      return { foot: foot, top: top };
    }
    /* The egg is five thirds as tall as the cast, measured the way they are
       drawn: the average visible height of everyone on the board. */
    var castH = 0, castN = 0;
    o.seats.concat([{ spec: o.spec }]).forEach(function (s) {
      if (!s.spec) return;
      var e = extent(s.spec, s.spec.personality.rest);
      castH += e.foot - e.top + 1; castN++;
    });
    var EGG_H = Math.round((castN ? castH / castN : 26) * 5 / 3);
    /* Where the egg stands and where you stand to reach it, in art pixels. You
       stand at the first column and the egg stands left of you, its keyhole a
       key's reach from your hand once you step in — up to nearly half of it
       past the edge of a narrow window, and only then do you stand further right. */
    function stand(ap) {
      var E = eggArt(EGG_H), restPref = Math.round((colX(0) - 16 * ap) / ap);
      var eggLeft = restPref + 3 - 14 - KEY_STEP + KEY_DEPTH - E.kx, shiftR = Math.max(0, -Math.round(0.45 * E.w) - eggLeft);
      return { E: E, restTx: restPref + shiftR, etx: eggLeft + shiftR, ety: LEAD - E.h };
    }

    /* ── THE TIMELINE, in ms ── */
    dpr = Math.min(2, root.devicePixelRatio || 1);
    var L0 = lattice();
    var T = { in: 120, hide: 130, clear: 150 };
    var far = 0;
    o.seats.forEach(function (s) { s.d = Math.hypot(s.r - chosen.r, s.c - chosen.c); if (s.d > far) far = s.d; });
    T.fadeStep = still ? 0 : 35; T.fadeDur = still ? 240 : 320;
    T.ground = T.clear + 60; T.groundDur = still ? 240 : 560; T.groundEnd = T.ground + T.groundDur;
    T.out = T.clear + (still ? 200 : 140);

    /* YOUR FALL: out through the foot of your screen, behind your tile, onto the
       grass — or onto the ground on its way up, and up with it. */
    var chosenBox = o.cell(chosen.r, chosen.c);
    var lcdPx = chosenBox[2] * 7 / 256;
    var footRow = extent(o.spec, o.mood.rest).foot;
    var startTop = chosenBox[1] + chosenBox[3] * 16 / 256;       // the LCD's top edge, in the scene
    var startX = chosenBox[0] + chosenBox[2] * 16 / 256;
    var grav = L0.ch * 3.2e-5;
    function fallFeet(tt) { var tau = Math.max(0, tt - T.out); return startTop + (footRow + 1) * L0.ap + 0.5 * grav * tau * tau; }
    function surfaceAt(tt) { return L0.y0 + groundDrop(L0, tt); }
    if (still) T.landed = T.out;
    else {
      T.landed = T.out + 4000;
      for (var tau = 0; tau < 4000; tau += 6) { if (fallFeet(T.out + tau) >= surfaceAt(T.out + tau)) { T.landed = T.out + tau; break; } }
    }
    T.walk = Math.max(T.landed, T.groundEnd) + 160;
    T.walkDur = still ? 0 : chosen.c * 170;
    T.walkEnd = T.walk + T.walkDur;
    var B = [0], A = [];
    A[0] = T.walkEnd + 300;
    function hold(i) { return Math.max(900, 1100 + 40 * BEATS[i].length - (A[i] - B[i])); }
    B[1] = A[0] + hold(0);
    T.face = B[1] + 100;                                              // you turn to the egg
    T.keyUp = B[1] + 300;                                             // your key, in your hand
    T.insert = B[1] + 650; T.insertDur = still ? 0 : 520;             // stepping in, the key all the way into the keyhole
    T.keyTurn = T.insert + T.insertDur + 150; T.turnDur = still ? 0 : 300;
    T.open = T.keyTurn + T.turnDur + 120;                             // it opens: the screen lights, out comes a seed
    T.seedFly = still ? 0 : 520; T.keySeed = T.open + T.seedFly;      // into your hand
    T.faceBack = T.keySeed + 350;                                     // and you turn to where it will go
    A[1] = T.faceBack + 500;
    B[2] = A[1] + hold(1);
    /* YOU CARRY IT ACROSS. The egg is at the left and the flower goes in at the
       right, so the seed is walked the width of the board — at the pace you
       walked to the egg — and planted where you end up, a step short of it. */
    var st0 = stand(L0.ap), stepCols = colX(1) - colX(0);
    var restX0 = st0.restTx * L0.ap, plantX0 = colX(HEAD_C) - 32 * L0.ap;
    T.toPlant = B[2] + 200;
    T.toPlantDur = still ? 0 : Math.round(Math.abs(plantX0 - restX0) / stepCols * 170);
    T.throwAt = T.toPlant + T.toPlantDur + 200; T.throwDur = still ? 0 : 380; T.hit = T.throwAt + T.throwDur;
    T.burrowDur = still ? 0 : 420; T.buried = T.hit + T.burrowDur; T.rootsDur = still ? 0 : 900;
    T.shoot = T.buried + T.rootsDur + 100; T.shootDur = still ? 0 : 500;   // the roots first, then the stem
    T.crack = T.shoot + T.shootDur; T.crackDur = still ? 0 : 480;
    T.burst = T.crack + T.crackDur;
    T.grow = [{ at: T.burst, dur: still ? 0 : 600 }];                 // a bare shoot
    A[2] = T.burst + 900;
    /* YOUR CHAIN, A LINK A STEP. In each: the network sends fronts out
       exploring from where it has got to; one finds the ground under the next
       board and thickens into a cord while the rest are pulled back, part of
       the way; a single root goes up to the board's foot and on up as its
       stalk; the board comes up on it; and the head grows a leaf. See THE
       SLIME MOLD'S RULES at grow(). */
    T.stage = []; T.tile = []; T.tileDur = still ? 0 : 520;
    T.explore = still ? 0 : 1200; T.pruneDur = still ? 0 : 500; T.riseDur = still ? 0 : 350;
    for (var i0 = 0; i0 < LINKS; i0++) {
      B[CHAIN_BEAT + i0] = A[CHAIN_BEAT + i0 - 1] + hold(CHAIN_BEAT + i0 - 1);
      var s0 = B[CHAIN_BEAT + i0] + 150;
      T.stage.push(s0);
      T.tile.push(s0 + 1850);
      T.grow.push({ at: s0 + 1500, dur: still ? 0 : 800 });           // leaf i0 + 1
      A[CHAIN_BEAT + i0] = s0 + 1850 + 520 + 350;
    }
    B[WRITE_BEAT] = A[WRITE_BEAT - 1] + hold(WRITE_BEAT - 1);

    /* THE RUNS: you, and four of the cast back in from beyond the right edge,
       under the links and up at them until every link holds its quota — seven
       to twelve deltas. Planned here, before anything plays, as segments in
       lattice columns: each runner takes the link that is quickest to reach
       and free, a link is not bumped again until its last delta has landed, and
       when the quotas are met each runner walks to where they will stand. */
    var others = o.seats.filter(function (s) { return s !== chosen && s.spec; })
      .sort(function (a, b) { return hash(a.id * 7 + 3, 11) - hash(b.id * 7 + 3, 11); }).slice(0, FRIENDS);
    var enterCol = (L0.vw - colX(0)) / stepCols + 0.8;
    var youCol = (plantX0 + 16 * L0.ap - colX(0)) / stepCols;      // where the planting left you
    var runners = [{ key: 'me', spec: o.spec, mood: o.mood, x: youCol, t: B[WRITE_BEAT] + 500, rest: youCol, restFace: 'left' }];
    others.forEach(function (s, j) {
      runners.push({ key: 'f' + j, spec: s.spec, mood: s.spec.personality, x: enterCol, t: B[WRITE_BEAT] + 300 + j * 450,
                     rest: LINK_C(0) - 0.6 - j * 1.2, restFace: 'down' });
    });
    runners.forEach(function (r, n) { r.idx = n; r.enter = r.t; r.segs = []; r.ex = extent(r.spec, r.mood.rest); });
    /* HOW HIGH A STACK MAY EVER GO. The clouds are the ceiling: a stack stops a
       good pad short of the lowest of them, so however long this is left
       running the sky stays the sky. Measured at the size the scene settles at,
       which is the size it was opened at. */
    var ROOM = (function () {
      var cellAp = L0.cw / L0.ap, floor = 0;
      cloudPlan(cellAp).forEach(function (c) { floor = Math.max(floor, c.y + c.h); });
      var top = o.cell(RAISED, LINK_C(0))[1] - L0.pan * L0.ch;      // a link's top, where it comes to rest
      var pitch = (SLAB_H + SLAB_GAP) * L0.ap, pad = Math.max(12, 0.35 * L0.ch);
      return { top: top, cloud: floor * L0.ap, pitch: pitch, pad: pad,
               cap: Math.max(3, Math.floor((top - floor * L0.ap - pad) / pitch)) };
    })();
    /* And no stack is a tower even where the sky is deep: sixteen is as many as
       a board wears before it stops reading as a stack of things someone
       wrote. Portrait has room for twice that. */
    var CAP = Math.min(16, ROOM.cap);
    var quota = [], free = [], deltas = [], step = 0;
    /* The story puts four to seven on each board — enough to read as writing,
       short enough to watch — and the rest arrives later, while the cast are
       still out there. See AFTER THE STORY. */
    for (var qi = 0; qi < LINKS; qi++) {
      quota.push(Math.min(CAP, 4 + Math.floor(hash(qi * 13 + 5, 3) * 4)));
      free.push(0);
    }
    function remaining() { return quota.reduce(function (a, b) { return a + b; }, 0); }
    function walkSeg(r, x1) {
      var dur = Math.abs(r.x - x1) / RUN;
      if (dur > 0) r.segs.push({ t0: r.t, t1: r.t + dur, kind: 'walk', x0: r.x, x1: x1 });
      r.t += dur; r.x = x1;
    }
    while (remaining() > 0 && step < 400) {
      var rn = runners.reduce(function (m, r) { return !m || r.t < m.t ? r : m; }, null), best = -1, bestScore = 1e12;
      for (var li = 0; li < LINKS; li++) {
        if (quota[li] <= 0) continue;
        var arrive = rn.t + Math.abs(rn.x - LINK_C(li)) / RUN, wait = Math.max(0, free[li] - arrive);
        var score = arrive + wait * 1.6 + hash(step * 31 + rn.idx * 7, li) * 380 - quota[li] * 25;
        if (score < bestScore) { bestScore = score; best = li; }
      }
      walkSeg(rn, LINK_C(best));
      if (free[best] > rn.t) { rn.segs.push({ t0: rn.t, t1: free[best], kind: 'wait', x0: rn.x, x1: rn.x }); rn.t = free[best]; }
      rn.segs.push({ t0: rn.t, t1: rn.t + JUMP, kind: 'jump', x0: rn.x, x1: rn.x, link: best });
      deltas.push({ link: best, author: rn.key, at: rn.t + JUMP / 2 });
      free[best] = rn.t + JUMP + 120;                                 // the delta lands before this link is bumped again
      rn.t += JUMP; quota[best]--; step++;
    }
    runners.forEach(function (r) { walkSeg(r, r.rest); r.done = r.t; });
    deltas.sort(function (a, b) { return a.at - b.at; });
    var slot = {};
    deltas.forEach(function (d) { d.k = slot[d.link] = (slot[d.link] === undefined ? 0 : slot[d.link] + 1); });
    var byKey = {};
    runners.forEach(function (r) { byKey[r.key] = r; });
    A[WRITE_BEAT] = runners.reduce(function (m, r) { return Math.max(m, r.done); }, B[WRITE_BEAT] + 1200) + 500;
    B[LAST_BEAT] = A[WRITE_BEAT] + hold(WRITE_BEAT);
    T.grow.push({ at: B[LAST_BEAT] + 150, dur: still ? 0 : 700 });   // the last of the stem, and the bud
    T.bud = B[LAST_BEAT] + 850; T.openDur = still ? 0 : 520; T.bloom = T.bud + T.openDur;
    T.cheer = T.bloom + 150;
    A[LAST_BEAT] = T.cheer + 1200;
    T.end = A[LAST_BEAT];

    /* ── AFTER THE STORY, THEY STAY OUT ─────────────────────────────────────
       The words have run out but the world has not: the cast wander the grass,
       stand about, and knock on a board now and then, until every stack has as
       much on it as it can hold — about ten minutes in. Planned here with the
       same machinery as the runs, so it is still one pure function of t and a
       step back still plays it backwards. */
    var IDLE_FOR = 600000;                                   // ten minutes, near enough
    function wander(r, until) {
      var seed = Math.round(r.t / 97) + r.idx * 31;
      walkSeg(r, 0.6 + hash(seed, 41) * (o.cols - 2.2));
      var pause = 800 + hash(seed, 42) * 3200;
      if (r.t + pause > until) pause = Math.max(250, until - r.t);
      r.segs.push({ t0: r.t, t1: r.t + pause, kind: 'idle', x0: r.x, x1: r.x });
      r.t += pause;
    }
    if (!still) {
      var minted = [], li3, i3;
      for (i3 = 0; i3 < LINKS; i3++) minted.push(0);
      deltas.forEach(function (d) { minted[d.link]++; });
      var room = minted.map(function (m) { return Math.max(0, CAP - m); });
      var over = room.reduce(function (a, b) { return a + b; }, 0);
      var every = over > 0 ? (IDLE_FOR - 3000) / over : 0;
      runners.forEach(function (r) { r.t = Math.max(r.t, T.end + 1200); });
      for (var n3 = 0; n3 < over; n3++) {
        var when = T.end + 3000 + (n3 + 0.4 + (hash(n3, 61) - 0.5) * 0.8) * every;
        li3 = 0;                                             // whichever stack has the most room left
        for (i3 = 1; i3 < LINKS; i3++) if (room[i3] > room[li3] || (room[i3] === room[li3] && hash(n3, i3) > 0.5)) li3 = i3;
        var rn3 = runners.reduce(function (m, r) { return !m || r.t < m.t ? r : m; }, null);
        for (var w3 = 0; w3 < 8 && rn3.t < when - Math.abs(rn3.x - LINK_C(li3)) / RUN - 1800; w3++) wander(rn3, when);
        walkSeg(rn3, LINK_C(li3));
        if (rn3.t < when) { rn3.segs.push({ t0: rn3.t, t1: when, kind: 'idle', x0: rn3.x, x1: rn3.x }); rn3.t = when; }
        rn3.segs.push({ t0: rn3.t, t1: rn3.t + JUMP, kind: 'jump', x0: rn3.x, x1: rn3.x, link: li3 });
        deltas.push({ link: li3, author: rn3.key, at: rn3.t + JUMP / 2 });
        rn3.t += JUMP; room[li3]--;
      }
      runners.forEach(function (r) { wander(r, r.t + 4000); r.done = r.t; });
      deltas.sort(function (a, b) { return a.at - b.at; });
      slot = {};
      deltas.forEach(function (d) { d.k = slot[d.link] = (slot[d.link] === undefined ? 0 : slot[d.link] + 1); });
    }
    function G(at, dur) { return still ? (t >= at ? 1 : 0) : seg(t, at, dur); }    // growth: all or nothing when still
    function stemFrac() {
      var f = 0;
      T.grow.forEach(function (s, i) { if (t >= s.at) f = lerp(i ? GROW[i - 1] : 0, GROW[i], outCubic(G(s.at, s.dur))); });
      return f;
    }

    /* WHAT A BUMP DOES TO A BOARD. The thing on the screen is printed in the
       colour of whoever just wrote it — as though it had stepped out of the
       grey-green display into the colours everyone else is drawn in — and then
       goes back. NOTHING ELSE MOVES: the screen's own ground, its segment grid,
       the bezel and the shell stay the pixels they always were. It is a palette
       swap of the picture's ink, in whole steps, because a blend between two
       palettes is a gradient, and a gradient is not this picture. */
    var LIT_MS = 170, LIT_STEPS = [0, 0.55];        // full colour, then half way back
    var litPics = {}, LCD = Tama.LCD || { bg: [195, 204, 163], ink: [35, 42, 27], mid: [110, 122, 86] };
    var SKIN_T = Tama.SKIN_T || [0, 0.12, 0.24, 0.36, 0.48, 0.60];
    function lcdTone(level) {                        // what tama.js paints values 3..5 as
      return [0, 1, 2].map(function (j) { return Math.round(LCD.bg[j] + (LCD.ink[j] - LCD.bg[j]) * SKIN_T[level]); });
    }
    /* THE COLOURS A BOARD PRINTS IN are the author's own — what they are
       wearing, their hair, their shoes — so the thing on the screen joins the
       world the cast is drawn in rather than being tinted one hue. Anything
       theirs that is too close to another is dropped, and the wordmark's
       colours make up the difference. */
    var litSets = {};
    function litSet(key) {
      if (!litSets[key]) {
        var pal = Tama.palette ? Tama.palette(byKey[key].spec) : null, M = Tama.MAT || {}, out = [];
        function want(c) {
          if (!c) return;
          for (var i = 0; i < out.length; i++)
            if (Math.abs(out[i][0] - c[0]) + Math.abs(out[i][1] - c[1]) + Math.abs(out[i][2] - c[2]) < 70) return;
          out.push([c[0], c[1], c[2]]);
        }
        [M.OUTFIT, M.HAIR, M.OUTFIT2, M.HAT, M.SHOES, M.PATTERN, M.EYES].forEach(function (m) { if (pal && m !== undefined) want(pal[m]); });
        GEMS.forEach(want);
        litSets[key] = out.slice(0, 5);
      }
      return litSets[key];
    }
    function litPic(i, key, step) {
      var k = i + '|' + key + '|' + step;
      if (!litPics[k]) {
        if (Object.keys(litPics).length > 16) litPics = {};       // a cache, not a record
        var back = LIT_STEPS[step], set = litSet(key);
        /* Darkest tone to the strongest colour, and down the tones from there.
           THE GROUND AND ITS GHOST ARE NOT IN THIS LIST and are tested for
           first: the unlit pixels of a screen sit a whisker off the palest tone
           a picture can be drawn in, and catching them here turns the whole
           board that colour — which is the one thing this must never do. */
        var ghost = [0, 1, 2].map(function (j) { return Math.round(LCD.bg[j] + (LCD.ink[j] - LCD.bg[j]) * 0.07); });
        var keep = [LCD.bg, ghost];
        var from = [LCD.ink, LCD.mid, lcdTone(3), lcdTone(2)];
        var to = from.map(function (f, j) {
          var c = set[j % set.length];
          return c.map(function (v, q) { return Math.round(lerp(v, f[q], back)); });
        });
        var src = pic(CHAIN[i], PICTURES[CHAIN[i]]), c2 = o.newCanvas(), cg = c2.getContext('2d');
        cg.drawImage(src, 0, 0);
        var id = cg.getImageData(0, 0, c2.width, c2.height), d = id.data, n, j2;
        for (n = 0; n < d.length; n += 4) {
          var bestJ = -1, bestD = 11;                             // the tone it is, if it is one of them
          for (j2 = 0; j2 < keep.length; j2++)
            if (Math.max(Math.abs(d[n] - keep[j2][0]), Math.abs(d[n + 1] - keep[j2][1]), Math.abs(d[n + 2] - keep[j2][2])) < 14) { bestD = -1; break; }
          if (bestD < 0) continue;
          for (j2 = 0; j2 < from.length; j2++) {
            var dd = Math.max(Math.abs(d[n] - from[j2][0]), Math.abs(d[n + 1] - from[j2][1]), Math.abs(d[n + 2] - from[j2][2]));
            if (dd < bestD) { bestD = dd; bestJ = j2; }
          }
          if (bestJ >= 0) { d[n] = to[bestJ][0]; d[n + 1] = to[bestJ][1]; d[n + 2] = to[bestJ][2]; }
        }
        cg.putImageData(id, 0, 0);
        litPics[k] = c2;
      }
      return litPics[k];
    }

    /* ── the tile pictures, painted once each ── */
    var pics = {};
    function pic(key, make) {
      if (!pics[key]) { var c = o.newCanvas(); o.paint(c, make()); pics[key] = c; }
      return pics[key];
    }
    /* Your own tile, whose screen empties as you drop out of it. */
    var myTile = o.newCanvas(), myTileRows = -1;
    var R0 = o.render({ emotion: o.mood.poke, facing: 'down', frame: 0 });
    function myTileAt(rows) {
      if (rows !== myTileRows) {
        myTileRows = rows;
        var R = lcd();
        for (var y = rows; y < N; y++) for (var x = 0; x < N; x++) R[y * N + x] = R0[(y - rows) * N + x];
        o.paint(myTile, R);
      }
      return myTile;
    }
    /* …and everyone in their own colours, one canvas per pose. */
    var poses = {};
    function pose(who, spec, st) {
      var k = who + '|' + st.emotion + '|' + st.facing + '|' + st.frame + '|' + (st.blink ? 1 : 0);
      if (!poses[k]) {
        var A2 = art(N, N);
        Tama.paintColour(A2.g, Tama.renderLayers(spec, st), spec, { px: 1 });
        poses[k] = A2.c;
      }
      return poses[k];
    }
    /* A delta, by its author: their outfit colour and their face. */
    var slabs = {}, authorCols = {};
    function authorCol(key) {
      if (!authorCols[key]) {
        var r = byKey[key], pal = Tama.palette ? Tama.palette(r.spec) : null;
        authorCols[key] = pal && Tama.MAT && pal[Tama.MAT.OUTFIT] ? pal[Tama.MAT.OUTFIT] : GEMS[key.length % GEMS.length];
      }
      return authorCols[key];
    }
    function slab(key, w, h) {
      var k = key + '|' + w + '|' + h;
      if (!slabs[k]) slabs[k] = deltaArt(authorCol(key), miniFace(Tama, byKey[key].spec), w, h);
      return slabs[k];
    }

    /* ── THE WORLD: the sky, the ground's picture, the roots, the sunflower's
       parts. Built for the scene as it stands once the camera has settled, and
       again only when the window changes size. ── */
    var world = { key: '' };
    function build(L) {
      var cellAp = L.cw / L.ap;
      var topY = L.y0 - L.pan * L.ch - LEAD * L.ap;          // where the picture's first row sits, camera settled
      var cols = Math.ceil(L.vw / L.ap) + 2, rows = Math.ceil((L.vh - topY) / L.ap) + 2;
      var bandRows = Math.ceil(L.bandH / L.ap) + 3, stoneRows = Math.round(0.3 * L.ch / L.ap);
      /* The cobblestone seam sits three of its own thicknesses higher than the
         words need it to, so the bedrock under it is that much deeper. */
      var soilRows = Math.max(Math.round(0.9 * cellAp), rows - LEAD - bandRows - 4 * stoneRows);
      var key = [cols, rows, soilRows, stoneRows, L.apDev, Math.round(cellAp)].join();
      if (key === world.key) return world;
      var tx = function (x) { return Math.round(x / L.ap); };
      var cx = []; for (var c = 0; c < o.cols; c++) cx.push(tx(colX(c)));
      var sx = cx[HEAD_C], seedY = LEAD + Math.round(soilRows * 0.23);        // two thirds as deep as it was
      var lim = [LEAD + 5, LEAD + soilRows - 3], rootsList = [];
      /* the seed's own: a tap root and three laterals, branching */
      grow(rootsList, { x: sx, y: seedY + 5, ang: Math.PI / 2, len: Math.round(Math.min(soilRows * 0.62, cellAp * 1.7)), thick: true,
                        at: T.buried, dur: T.rootsDur, salt: 11, depth: 0, target: null, pull: 0.08, lim: lim });
      [[-1.0, 0.5], [1.05, 0.55], [-1.35, 0.4]].forEach(function (lat, i) {
        grow(rootsList, { x: sx + (lat[0] < 0 ? -1 : 1), y: seedY + 3, ang: Math.PI / 2 + lat[0], len: Math.round(cellAp * lat[1]),
                          thick: false, at: T.buried + 150 * i, dur: still ? 0 : 700, salt: 20 + i, depth: 1, target: null, pull: 0.04, lim: lim });
      });
      /* the network that finds your chain: from where it has got to, fronts go
         out exploring; the one that reaches the ground under the next board
         becomes a cord, the others are pulled back — a third of them all the
         way, the rest to between a third and two thirds of their length, and
         that stays — and one root, one to a board, goes up to its foot and on
         up out of the ground as its stalk. */
      var from = [sx + 2, seedY + 6];
      for (var i = 0; i < LINKS; i++) {
        var s0 = T.stage[i], ux = cx[LINK_C(i)];
        var food = [ux + Math.round((hash(i, 18) - 0.5) * cellAp * 0.25),
                    LEAD + Math.round(Math.min(soilRows * 0.42, cellAp * 1.25) + (hash(i, 17) - 0.5) * cellAp * 0.4)];
        var toward = Math.atan2(food[1] - from[1], food[0] - from[0]), dist = Math.hypot(food[0] - from[0], food[1] - from[1]);
        var prune = [s0 + T.explore + 50, T.pruneDur, 0];               // when; each front and branch has its own keep
        [-2.0, -1.45, -0.95, -0.5, 0.5, 0.95, 1.45, 2.0].forEach(function (off, j) {
          var keep = hash(i * 7 + j, 6) < 0.33 ? 0 : 0.33 + hash(i * 7 + j, 7) * 0.34;
          grow(rootsList, { x: from[0], y: from[1], ang: toward + off + (hash(i * 7 + j, 3) - 0.5) * 0.3,
                            len: Math.round(dist * (0.55 + hash(i * 7 + j, 4) * 0.5)), thick: false,
                            at: s0, dur: T.explore * (0.8 + hash(i * 7 + j, 5) * 0.2), salt: 200 + i * 10 + j, depth: 1,
                            target: null, pull: 0.02, wob: 1.1, lim: lim, fine: true, retract: [prune[0], prune[1], keep] });
        });
        var cord = grow(rootsList, { x: from[0], y: from[1], ang: toward, len: Math.round(dist * 2), thick: false,
                                     at: s0, dur: T.explore, salt: 300 + i, depth: 1, target: food, pull: 0.2, wob: 0.8,
                                     lim: lim, fine: true, retract: prune, cord: s0 + T.explore });
        var end = cord.pts.length ? cord.pts[cord.pts.length - 1] : from;
        grow(rootsList, { x: end[0], y: end[1], ang: -Math.PI / 2, len: Math.round(soilRows), thick: false,
                          at: s0 + 1500, dur: T.riseDur, salt: 400 + i, depth: 3, target: [ux, LEAD + 4], pull: 0.45, wob: 0.4,
                          lim: [LEAD + 3, lim[1]], cord: s0 + 1850 });
        from = end;
      }
      world = {
        key: key, cols: cols, rows: rows, soilRows: soilRows, cellAp: cellAp, cx: cx, sx: sx, seedY: seedY,
        tex: groundTexture(cols, rows, soilRows, stoneRows, sx), roots: rootsList,
        flower: flowerArt(cellAp), ug: art(cols, rows), sig: '', poles: {},
        sky: skyArt(Math.ceil(L.vw / L.ap) + 1, Math.ceil(L.vh / L.ap) + 1),
        clouds: cloudPlan(cellAp).map(function (c, n) {
          return { art: cloudArt(c.w, c.h, n + 1), w: c.w, x0: hash(n, 33) * (cols + c.w), y: c.y, v: c.v };
        })
      };
      return world;
    }

    /* ── the clock ── */
    var t = 0, last = 0, done = false, closing = false, seekTo = null, seekSpeed = 3;
    var shownLine = -1, swapAt = 0, wasLine = -1, hidden = false, opac = -1, capOn = false, beatShown = -1, now0 = 0;

    function size() {
      dpr = Math.min(2, root.devicePixelRatio || 1);
      var w = Math.round(root.innerWidth * dpr), h = Math.round(root.innerHeight * dpr);
      if (w !== W || h !== H) { W = canvas.width = w; H = canvas.height = h; }
    }
    function beatAt(tt) { var n = 0; for (var i = 0; i < B.length; i++) if (tt >= B[i]) n = i; return n; }
    /* The words change over, in the panel and for a reader who is listening.
       The fade is in wall time, not the scene's: stepping is a seek through t
       and the words should change at the pace they are read, not at its. */
    function caption(i) {
      if (i === shownLine) return;
      wasLine = shownLine;
      shownLine = i;
      swapAt = now0;
      lineEl.textContent = BEATS[i];
    }

    /* ── THE UNDERGROUND, composed onto its own canvas whenever it changes: the
       seed's tunnel, the seed, the roots, the pale stem, the cracks in the grass
       block and the hole it leaves. ── */
    function underground(Wd) {
      var u = Wd.ug, sx = Wd.sx, seedY = Wd.seedY;
      var burrow = G(T.hit, T.burrowDur), seedRow = t >= T.hit ? Math.round(lerp(LEAD, seedY, burrow)) : -1;
      var counts = Wd.roots.map(function (r) {
        var n = Math.floor(G(r.at, r.dur) * r.pts.length);
        return r.retract ? Math.floor(n * (1 - G(r.retract[0], r.retract[1]) * (1 - r.retract[2]))) : n;
      });
      var thickN = Wd.roots.map(function (r, i) { return r.cord !== undefined && t >= r.cord ? Math.floor(G(r.cord, 400) * counts[i]) : 0; });
      var shoot = t >= T.shoot ? Math.floor(G(T.shoot, T.shootDur) * (seedY - LEAD)) : -1;
      var crack = t >= T.crack && t < T.burst ? 1 + Math.min(2, Math.floor(G(T.crack, T.crackDur) * 3)) : 0;
      var burst = t >= T.burst ? 1 : 0;
      var sig = [seedRow, counts.join('.'), thickN.join('.'), shoot, crack, burst].join('|');
      if (sig === Wd.sig) return;
      Wd.sig = sig;
      var ug = u.g;
      ug.clearRect(0, 0, Wd.cols, Wd.rows);
      ug.drawImage(Wd.tex, 0, 0);
      if (seedRow >= 0) {
        for (var y = LEAD + 4; y < seedRow; y++) u.put(sx, y, P.topDark);             // the tunnel it dug
        Wd.roots.forEach(function (r, i) {
          var n = counts[i], tn = thickN[i];
          for (var k = 0; k < n; k++) {
            var p = r.pts[k], fat = k < tn || (r.thick && k < r.pts.length * 0.55);
            u.put(p[0], p[1], r.fine && !fat ? P.hypha : k < n - 1 ? P.root : P.rootMid);
            if (fat) u.put(p[0], p[1] + 1, P.rootDark);
          }
          r.hairs.forEach(function (h) { if (h.at < n) u.put(h.p[0], h.p[1], P.hair); });
        });
        ug.drawImage(seedArt(), sx - 2, seedRow);
      }
      if (burst) {                                              // the block is gone: a hole, and the grass on it with it
        for (var hy = LEAD; hy < LEAD + 8; hy++) for (var hx = sx - 4; hx < sx + 4; hx++) u.put(hx, hy, (hx + hy) & 1 ? P.wall : P.wallDark);
        ug.clearRect(sx - 4, 0, 8, LEAD);
      }
      if (shoot > 0) {                                          // the stem, pale under the ground
        for (var s = 0; s < shoot; s++) {
          var yy = seedY - 1 - s;
          u.put(sx - 1, yy, P.paleDark); u.put(sx, yy, P.pale); u.put(sx + 1, yy, P.paleLight);
        }
      }
      if (crack) {                                              // the grass block, splitting where the stem pushes on it
        var CR = [[3, 7], [4, 6], [4, 5], [3, 4], [2, 3], [5, 4], [6, 3], [1, 2], [5, 6], [2, 1], [6, 2], [7, 1], [0, 1], [3, 2], [4, 1], [1, 5], [6, 6]];
        var nCr = [0, 4, 9, 17][crack];
        for (var c = 0; c < nCr; c++) u.put(sx - 4 + CR[c][0], LEAD + CR[c][1], P.crackDark);
      }
    }

    /* Where a runner is at time t: in lattice columns, how high off the grass
       (CSS px), which way they face, and whether this is the top of a jump. */
    function runAt(r, apexH) {
      if (t < r.enter) return null;
      var s = null;
      for (var i = 0; i < r.segs.length; i++) if (t < r.segs[i].t1) { s = r.segs[i]; break; }
      if (!s) return { x: r.rest, up: 0, facing: r.restFace, frame: 0, top: false, rest: true };
      var u = s.t1 > s.t0 ? clamp01((t - s.t0) / (s.t1 - s.t0)) : 1;
      if (t < s.t0) u = 0;
      var dir = 'down';
      for (var j = i; j >= 0; j--) if (r.segs[j].kind === 'walk') { dir = r.segs[j].x1 < r.segs[j].x0 ? 'left' : 'right'; break; }
      if (s.kind === 'walk') return { x: lerp(s.x0, s.x1, u), up: 0, facing: dir, frame: Math.floor(t / 90) % 2, top: false };
      if (s.kind === 'wait') return { x: s.x0, up: 0, facing: 'up', frame: 0, top: false };
      if (s.kind === 'idle') return { x: s.x0, up: 0, facing: 'down', frame: 0, top: false };
      return { x: s.x0, up: apexH * (1 - Math.pow(2 * u - 1, 2)), facing: dir, frame: 1, top: u > 0.35 && u < 0.65 };
    }

    /* ── one frame at time t ── */
    function draw() {
      size();
      g.clearRect(0, 0, W, H);
      var a = seg(t, 0, T.in);
      if (a !== opac) { opac = a; canvas.style.opacity = String(a); }
      var hide = t >= T.hide;
      if (hide !== hidden) { hidden = hide; o.hide(hide); }
      var L = lattice();
      if (!L) return;
      var cam = camY(L, t), i, x;
      var Wd = build(L);
      var ap = L.ap, apDev = L.apDev;
      var surface = L.y0 + cam;                                  // the grass line, on screen
      var drop = groundDrop(L, t);
      var originY = surface - LEAD * ap + drop;
      var oyDev = Math.round(originY * dpr);                     // the ground picture's first row, device px
      function dx(tx) { return tx * apDev; }
      function dy(ty) { return oyDev + ty * apDev; }
      function tyOf(y) { return Math.round((y - originY) / ap); }   // CSS y on screen -> the picture's row
      function sprite(c, tx, ty, sw, sh) {
        g.drawImage(c, dx(tx), dy(ty), (sw || c.width) * apDev, (sh || c.height) * apDev);
      }
      function rect(tx, ty, w, h, col) { g.fillStyle = css(col); g.fillRect(dx(tx), dy(ty), w * apDev, h * apDev); }
      g.imageSmoothingEnabled = false;

      /* THE SKY: fixed to the window, not the scene; clouds drifting, birds going over. */
      var skyA = seg(t, T.clear, 420);
      if (skyA > 0) {
        g.save();
        g.globalAlpha = skyA;
        g.drawImage(Wd.sky, 0, 0, Wd.sky.width * apDev, Wd.sky.height * apDev);
        var face = o.sun ? o.sun() : null;             // the page's mark: [cx, cy, diameter], CSS px
        if (face) {
          var sun = sunArt(Math.max(4, Math.round(face[2] / ap)));
          g.drawImage(sun.c, Math.round(face[0] / ap - sun.S / 2) * apDev, Math.round(face[1] / ap - sun.S / 2) * apDev, sun.S * apDev, sun.S * apDev);
        }
        var span = Wd.cols + 80;
        Wd.clouds.forEach(function (cl) {
          var cxp = ((cl.x0 - (still ? 0 : t) * cl.v) % (span + cl.w) + span + cl.w) % (span + cl.w) - cl.w;
          g.drawImage(cl.art, Math.round(cxp) * apDev, cl.y * apDev, cl.art.width * apDev, cl.art.height * apDev);
        });
        if (!still) {
          var flock = (t % 16000) / 16000;
          for (var bi = 0; bi < 5; bi++) {
            var bxp = Math.round(-30 + flock * (Wd.cols + 60) - bi * 7 - (bi & 1) * 3);
            var byp = Math.round(Wd.cellAp * 0.9 + bi * 2 + (bi & 1) * 4 + Math.sin(t / 520 + bi) * 2);
            BIRD[Math.floor(t / 150 + bi) % 2].forEach(function (rw, j) {
              for (var q = 0; q < 5; q++) if (rw[q] === '#') { g.fillStyle = css(P.bird); g.fillRect((bxp + q) * apDev, (byp + j) * apDev, apDev, apDev); }
            });
          }
        }
        g.restore();
      }

      /* Each link's height: its rise out of the ground on its pole, and the
         bumps from underneath. WHETHER IT IS UP IS ITS OWN FLAG, not the sign
         of `rows`: outBack overshoots the top of the rise, so for the frames it
         is above its seat `rows` is slightly negative — and while that doubled
         as "not out of the ground yet" the board, its pole and its button all
         blinked out for those frames, a flash a sixth of a second long. */
      var bumpH = 0.1 * L.ch, lift = [];
      for (i = 0; i < LINKS; i++) {
        var up = t >= T.tile[i], knock = 0, lit = -1, who = null, since = 1e9;
        if (!still) deltas.forEach(function (d) {
          if (d.link !== i || t < d.at) return;
          knock = Math.max(knock, bump(seg(t, d.at, 220)));
          var age = t - d.at;                                    // the newest strike, while its colour holds
          if (age < LIT_MS * LIT_STEPS.length && age < since) { since = age; lit = Math.floor(age / LIT_MS); who = d.author; }
        });
        lift.push({ up: up, rows: up ? (1 - outBack(G(T.tile[i], T.tileDur))) * 2.1 : 0,
                    knock: knock * bumpH, lit: lit, who: who });
      }
      /* Where a link is: its seat, and the knock of being hit from below. A
         swell would mean scaling the picture, and a scaled pixel is a smudge. */
      function linkBox(i2) {
        var b = viewCell(RAISED + lift[i2].rows, LINK_C(i2), cam);
        return b ? [b[0], b[1] - lift[i2].knock, b[2], b[3]] : null;
      }
      /* Each link's button, over its board once the board is up. A button
         that goes while it has the focus hands it to the steps. */
      for (i = 0; i < LINKS; i++) {
        var bb = boardBtns[i], isUp = !closing && lift[i].up && t >= T.tile[i] + T.tileDur * 0.6;
        if (bb.el.hidden === isUp) {
          if (!isUp && document.activeElement === bb.el) (prevBtn.disabled ? nextBtn : prevBtn).focus({ preventScroll: true });
          bb.el.hidden = !isUp;
        }
        if (!isUp) continue;
        var bbox = linkBox(i), at2 = bbox.map(Math.round).join();
        if (at2 === bb.at) continue;
        bb.at = at2;
        bb.el.style.left = Math.round(bbox[0]) + 'px'; bb.el.style.top = Math.round(bbox[1]) + 'px';
        bb.el.style.width = Math.round(bbox[2]) + 'px'; bb.el.style.height = Math.round(bbox[3]) + 'px';
      }

      /* THE OTHERS LEAVE, nearest first, dropping a little as they go. */
      o.seats.forEach(function (s) {
        if (s === chosen) return;
        var k = seg(t, T.clear + T.fadeStep * s.d, T.fadeDur);
        if (k >= 1) return;
        drawTile(g, s.canvas, viewCell(s.r + (still ? 0 : 0.12 * k * k), s.c, cam), 1 - k * k, dpr, 0);
      });

      /* THE LINKS, as bars between the tiles. */
      var lw = Math.max(2, L.ch * 0.05) * dpr;
      for (i = 1; i < LINKS; i++) {
        var p = outCubic(G(T.tile[i] + 300, 180));
        if (p <= 0) continue;
        var A2 = viewCell(RAISED, LINK_C(i - 1), cam), B2 = viewCell(RAISED, LINK_C(i), cam);
        var ax = A2[0] + A2[2] / 2, ay = A2[1] + A2[3] / 2, bx = B2[0] + B2[2] / 2;
        g.save(); g.strokeStyle = 'rgba(20,24,30,.42)'; g.lineWidth = lw; g.lineCap = 'round';
        g.beginPath(); g.moveTo(ax * dpr, ay * dpr); g.lineTo(lerp(ax, bx, p) * dpr, ay * dpr); g.stroke();
        g.restore();
      }

      /* THE GROUND, and under it the roots. */
      if (t >= T.ground) {
        underground(Wd);
        g.save();
        g.globalAlpha = still ? seg(t, T.ground, T.groundDur) : 1;
        sprite(Wd.ug.c, 0, 0);
        g.restore();
      }

      /* THE SUNFLOWER: up through the grass block, a leaf a step. */
      var F = Wd.flower, h = t >= T.burst ? Math.round(stemFrac() * F.Hs) : 0;
      if (h > 0) {
        var stx = Wd.sx - 2;
        g.save();
        g.globalAlpha = still ? seg(t, T.burst, 400) : 1;
        g.drawImage(F.stem, 0, F.Hs - h, F.sw, h, dx(stx), dy(LEAD - h), F.sw * apDev, h * apDev);
        /* EVERY STEP ENDS AT A LEAF. Leaf k opens at the top of the stem as its
           link is found, all but upright; the step after raises a bare tip past
           it and swings it down to where it rests. */
        F.leaves.forEach(function (lf, k) {
          var mine = T.grow[k + 1], after = T.grow[k + 2];
          var open = G(mine.at + mine.dur, still ? 0 : 240);
          if (open <= 0) return;
          var swing = after ? inOutCubic(G(after.at, after.dur)) : 1;
          var deg = Math.round(lerp(LEAF_NEW, lf.rest, swing) / 3) * 3;
          var key = [deg, lf.dir, Math.round(open * 4)].join();
          var a2 = F.cut[key] || (F.cut[key] = leafArt(F.len, F.wid, deg, lf.dir, Math.max(0.25, Math.round(open * 4) / 4)));
          sprite(a2.c, stx + (lf.dir > 0 ? F.sw - 1 : 0) - a2.bx, LEAD - lf.y - a2.by);
        });
        if (t >= T.bud) {
          var open = G(T.bud, T.openDur), stage = open < 0.25 ? 0 : open < 0.5 ? 1 : open < 0.75 ? 2 : 3;
          sprite(F.blooms[stage], Wd.sx - Math.floor(F.S / 2), LEAD - F.Hs - Math.floor(F.S / 2) + 1);
        }
        g.restore();
      }

      /* THE EGG, on its base at the left edge of the scene, its keyhole turned
         toward you. */
      var S = stand(ap), E = S.E;
      var egg = { hole: [S.etx + E.kx, S.ety + E.ky], top: [S.etx + E.topX, S.ety + E.topY] };
      if (t >= T.ground) {
        g.save();
        g.globalAlpha = still ? seg(t, T.ground, T.groundDur) : 1;
        sprite(t >= T.open && t < T.open + 600 ? E.lit : E.c, S.etx, S.ety);
        g.restore();
      }

      /* THE POLES: one straight bamboo pole under each link — see poleArt. As
         long as the link is high, so it grows as the link rises and gives a
         little when it is bumped. Behind the people, who pass in front of it. */
      for (i = 0; i < LINKS; i++) {
        if (!lift[i].up) continue;
        var lb = linkBox(i), bottom = tyOf(lb[1] + lb[3]) - 1;
        if (bottom >= LEAD) continue;                           // still under the grass
        var plen = LEAD - bottom;
        var pl = Wd.poles[plen] || (Wd.poles[plen] = poleArt(plen));
        sprite(pl.c, Wd.cx[LINK_C(i)] - pl.ox, bottom - pl.oy);
      }

      /* EVERYONE ON THE GRASS: you — down out of your tile, onto the grass, to
         the egg, and out to write with the others — and your friends. */
      var rb = viewCell(RAISED, LINK_C(0), cam), clearance = surface - (rb[1] + rb[3]);
      var blinkOn = function (salt) { return !still && ((t + salt * 977) % 3200) < 130; };
      var me = null;
      if (t >= T.out) {
        var feet, mx, facing = 'down', emotion = o.mood.rest, frame = 0, alpha = 1, up = 0;
        var restX = S.restTx * ap, seatX = colX(chosen.c) - 16 * ap, plantX = colX(HEAD_C) - 32 * ap;
        var myRun = byKey.me, runNow = !still && t >= myRun.enter && t < myRun.done ? runAt(myRun, Math.max(L.ch * 0.2, clearance - (myRun.ex.foot - myRun.ex.top + 1) * ap)) : null;
        if (still) {
          feet = L.y0; mx = t >= T.toPlant ? plantX : restX; alpha = seg(t, T.out, 300);
        } else if (t < T.landed) {
          feet = fallFeet(t); mx = startX; emotion = o.mood.poke;
        } else if (runNow) {
          feet = L.y0; mx = colX(runNow.x) - 16 * ap; facing = runNow.facing; frame = runNow.frame; up = runNow.up;
          if (runNow.top) emotion = o.mood.poke;
        } else {
          feet = surfaceAt(t);
          mx = lerp(seatX, restX, inOutCubic(seg(t, T.walk, T.walkDur)));
          if (t >= T.walk && t < T.walkEnd) { facing = 'left'; frame = Math.floor(t / 100) % 2; }
          else if (t >= T.walkEnd) facing = 'right';
          if (t >= T.face && t < T.faceBack) { facing = 'left'; frame = 0; }
          /* a step in to the egg with the key, and back out once the seed is caught */
          mx -= KEY_STEP * ap * inOutCubic(G(T.insert, T.insertDur)) * (1 - inOutCubic(G(T.keySeed, 250)));
          if (t >= T.keyTurn && t < T.keySeed + 300) emotion = o.mood.poke;
          /* and the walk across, seed in hand, to where it goes in */
          if (t >= T.toPlant) {
            mx = lerp(mx, plantX, inOutCubic(seg(t, T.toPlant, T.toPlantDur)));
            if (t < T.toPlant + T.toPlantDur) { facing = 'right'; frame = Math.floor(t / 100) % 2; }
            else facing = t < T.hit ? 'right' : 'left';        // planted, and then watching the chain come up
          }
        }
        if (t >= T.bloom) emotion = o.mood.pet;
        var hop = 0;
        if (!still && t >= T.cheer) hop = Math.max(bump(seg(t, T.cheer, 380)), bump(seg(t, T.cheer + 450, 380))) * 0.3 * L.ch;
        var top = feet + cam - hop - up - (footRow + 1) * ap;
        me = { tx: Math.round(mx / ap), ty: tyOf(top) };
        g.save();
        g.globalAlpha = alpha;
        sprite(pose('me', o.spec, { emotion: emotion, facing: facing, frame: frame, blink: facing === 'down' || facing === 'right' ? t >= T.landed && blinkOn(0) : false }), me.tx, me.ty);
        g.restore();
      }
      runners.forEach(function (r, j) {
        if (r.key === 'me') return;
        var chh = (r.ex.foot - r.ex.top + 1) * ap, at = runAt(r, Math.max(L.ch * 0.2, clearance - chh));   // head to the link's underside
        if (!at) return;
        if (still) at = { x: r.rest, up: 0, facing: r.restFace, frame: 0, top: false };
        var hopF = !still && t >= T.cheer ? Math.max(bump(seg(t, T.cheer + 120 * j, 380)), bump(seg(t, T.cheer + 450 + 120 * j, 380))) * 0.3 * L.ch : 0;
        var emo = at.top ? r.mood.poke : t >= T.bloom ? r.mood.pet : r.mood.rest;
        var ftop = surface - at.up - hopF - (r.ex.foot + 1) * ap;
        g.save();
        g.globalAlpha = still ? seg(t, r.enter, 300) : 1;
        sprite(pose(r.key, r.spec, { emotion: emo, facing: at.facing, frame: at.frame, blink: at.facing === 'down' && blinkOn(j + 1) }),
               Math.round((colX(at.x) - 16 * ap) / ap), tyOf(ftop));
        g.restore();
      });

      /* DEBRIS, as the grass block breaks; DUST, where things land. */
      if (!still) {
        var u40 = Wd.cellAp / 40;
        var bt = t - T.burst;
        if (bt >= 0 && bt < 820) {
          for (var k = 0; k < 12; k++) {
            var vx = (hash(k, 1) - 0.5) * 0.09 * u40, vy = -(0.1 + hash(k, 2) * 0.1) * u40;
            var ptx = Wd.sx - 4 + (k % 4) * 2 + vx * bt, pty = LEAD + Math.floor(k / 4) * 2 + vy * bt + 0.00032 * u40 * bt * bt;
            if (pty > LEAD + 2) continue;
            rect(Math.round(ptx), Math.round(pty), 2, 2, k % 4 === 0 ? P.grassLight : k % 4 === 1 ? P.grass : k % 4 === 2 ? P.dirt : P.topDark);
          }
        }
        var puff = function (x0, t0, n, salt) {
          var pt = t - t0;
          if (pt < 0 || pt > 420) return;
          for (var q = 0; q < n; q++) {
            var pvx = (hash(q, salt) - 0.5) * 0.06 * u40, pvy = -(0.03 + hash(salt, q) * 0.05) * u40;
            var qx = x0 + (q - n / 2) * 1.5 + pvx * pt, qy = LEAD - 1 + pvy * pt + 0.0003 * u40 * pt * pt;
            if (qy > LEAD) continue;
            g.globalAlpha = 1 - pt / 420;
            rect(Math.round(qx), Math.round(qy), 1, 1, q & 1 ? P.grassLight : P.dirtLight);
          }
          g.globalAlpha = 1;
        };
        if (me) puff(me.tx + 16, T.landed, 6, 3);
        puff(Wd.sx, T.hit, 6, 5);
        for (i = 0; i < LINKS; i++) puff(Wd.cx[LINK_C(i)], T.tile[i], 6, 9 + i);
      }

      /* THE TILES: your own, emptying as you leave it; then your chain, on its
         stalks — the only tiles left. */
      if (t < T.clear) drawTile(g, chosen.canvas, viewCell(chosen.r, chosen.c, cam), 1, dpr, 0);
      else {
        var gone = seg(t, T.landed + 60, 260);
        if (gone < 1) {
          var rowsOut = still ? (t >= T.out ? N : 0)
                      : t >= T.landed ? N : Math.min(N, Math.max(0, Math.floor((fallFeet(t) - (startTop + (footRow + 1) * ap)) / lcdPx)));
          drawTile(g, myTileAt(rowsOut), viewCell(chosen.r, chosen.c, cam), 1 - gone, dpr, 0);
        }
      }
      g.save();
      g.beginPath(); g.rect(0, 0, W, Math.round(surface * dpr)); g.clip();       // they come up out of the ground
      for (i = 0; i < LINKS; i++) {
        if (!lift[i].up) continue;
        var al = still ? seg(t, T.tile[i], 300) : 1;
        var face = lift[i].lit >= 0 ? litPic(i, lift[i].who, lift[i].lit) : pic(CHAIN[i], PICTURES[CHAIN[i]]);
        drawTile(g, face, linkBox(i), al, dpr, 1);
      }
      g.restore();

      /* THE DELTAS: out of the top of a link at each bump, up, and down onto
         that link's stack — its own chain, in the order it was written, newest
         on top, each with its author's face. */
      var sw = Math.max(18, Math.round(Wd.cellAp * 0.62)), sh = SLAB_H, gap = SLAB_GAP;
      deltas.forEach(function (d) {
        if (t < d.at) return;
        var lb2 = linkBox(d.link), lx = Wd.cx[LINK_C(d.link)] - Math.floor(sw / 2);
        var base = tyOf(lb2[1]) - 1;                            // the row just above the link's top
        var slotY = base - (d.k + 1) * (sh + gap) + gap;
        var u = G(d.at, 440), riseTo = slotY - Math.round(Wd.cellAp * 0.3), yy;
        if (u < 0.5) yy = lerp(base - sh, riseTo, outCubic(u / 0.5));
        else yy = lerp(riseTo, slotY, inQuad((u - 0.5) / 0.5));
        g.save();
        g.globalAlpha = seg(t, d.at, 60);
        sprite(slab(d.author, sw, sh), lx, Math.round(yy));
        g.restore();
        var st = t - d.at;                                      // a sparkle as it is minted
        if (!still && st >= 0 && st < 300) {
          for (var q = 0; q < 4; q++) {
            var ang = q * Math.PI / 2 + 0.4, rr = 2 + st / 50;
            rect(Wd.cx[LINK_C(d.link)] + Math.round(Math.cos(ang) * (sw / 2 + rr)), base - 2 + Math.round(Math.sin(ang) * rr), 1, 1, st < 150 ? P.white : P.sparkle);
          }
        }
      });

      /* WHAT YOU HOLD: your key, all the way into the egg's keyhole and turned
         there; the seed the egg gives you; and, next step, that seed thrown. */
      if (me && t >= T.keyUp && t < T.hit) {
        var lhx = me.tx + 3, lhy = me.ty + 18;                  // your hand, facing the egg
        var rhx = me.tx + 29, rhy = me.ty + 18;                 // your hand, facing the flower's spot
        var sparkle = function (px2, py2, t0) {
          var st2 = t - t0;
          if (st2 < 0 || st2 > 420) return;
          for (var q2 = 0; q2 < 4; q2++) {
            var ang2 = q2 * Math.PI / 2 + 0.4, rr2 = 3 + st2 / 60;
            var qx = px2 + Math.round(Math.cos(ang2) * rr2), qy = py2 + Math.round(Math.sin(ang2) * rr2);
            rect(qx, qy, 1, 1, st2 < 200 ? P.white : P.sparkle);
            if (st2 < 200) { rect(qx - 1, qy, 1, 1, P.white); rect(qx + 1, qy, 1, 1, P.white); rect(qx, qy - 1, 1, 1, P.white); rect(qx, qy + 1, 1, 1, P.white); }
          }
        };
        if (t < T.open + 320) {                                 // the key: held out, slid all the way in, turned
          var ins = inOutCubic(G(T.insert, T.insertDur)), turn = inOutCubic(G(T.keyTurn, T.turnDur));
          var tipX = lhx - 14, tipY = lerp(lhy, egg.hole[1] + 0.5, ins);  // it stays in your hand: you step in with it
          var ka = keyArt();
          g.save();
          g.globalAlpha = seg(t, T.keyUp, 150) * (1 - seg(t, T.open + 150, 170));
          g.beginPath(); g.rect(dx(egg.hole[0]), 0, W, H); g.clip();   // what has gone into the keyhole is inside the egg
          g.translate(Math.round(tipX * apDev), Math.round(oyDev + tipY * apDev));
          g.scale(-1, 1 - 0.78 * turn);                        // tip first; and a quarter turn about its own shaft: the bow goes edge-on
          g.drawImage(ka, -18.5 * apDev, -5 * apDev, ka.width * apDev, ka.height * apDev);
          g.restore();
        }
        if (t >= T.open) {                                      // the seed: out of the top of the egg, into your hand
          var sa = seedArt(), sxT, syT, fly = G(T.open, T.seedFly);
          if (t < T.keySeed && !still) {
            sxT = lerp(egg.top[0] - 2, lhx - 2, fly);
            syT = lerp(egg.top[1] - 6, lhy - 3, fly) - bump(fly) * Wd.cellAp * 0.4;
          } else if (t < T.faceBack) { sxT = lhx - 2; syT = lhy - 3; }
          else if (t < T.throwAt || still) { sxT = rhx - 2; syT = rhy - 3; }
          else {
            var th = G(T.throwAt, T.throwDur);
            sxT = lerp(rhx - 2, Wd.sx - 2, th);
            syT = lerp(rhy - 3, LEAD - 6, th) - bump(th) * Wd.cellAp * 0.45;
          }
          g.save();
          g.globalAlpha = seg(t, T.open, 100);
          sprite(sa, Math.round(sxT), Math.round(syT));
          g.restore();
        }
        if (!still) { sparkle(egg.hole[0] + 2, egg.hole[1], T.keyTurn + T.turnDur); sparkle(lhx, lhy - 2, T.keySeed); }
      }

      /* THE WORDS, in their display, sunk in the bedrock once the ground is up.
         Fixed to the window like the sky, not to the scene. */
      var n = beatAt(t);
      caption(n);
      if (n !== beatShown) {
        beatShown = n;
        prevBtn.disabled = n === 0;
        nextBtn.disabled = n === B.length - 1;
      }
      var on = !closing && t >= T.groundEnd - 80;
      if (on !== capOn) { capOn = on; prevBtn.hidden = nextBtn.hidden = !on; }
      var pp = planPanel(ap, apDev, L.cw);
      if (on && pp.w) {
        var px0 = Math.round(pp.left / ap), py0 = Math.round((L.vh - L.bandH) / ap) + pp.margin;
        var chrome = pp.chrome === (n === 0 ? 'p' : '') + (n === B.length - 1 ? 'n' : '') ? pp.art
                   : (pp.chrome = (n === 0 ? 'p' : '') + (n === B.length - 1 ? 'n' : ''),
                      pp.art = panelArt(pp.w, pp.h, pp.pad, pp.box, n > 0, n < B.length - 1));
        g.save();
        g.globalAlpha = Math.min(1, seg(t, T.groundEnd - 80, 260));
        g.drawImage(chrome, px0 * apDev, py0 * apDev, pp.w * apDev, pp.h * apDev);
        /* the words themselves, each beat's lines drawn once and kept */
        var fade = still ? 1 : clamp01((now0 - swapAt) / 190);
        [[wasLine, 1 - fade], [shownLine, fade]].forEach(function (w) {
          if (w[0] < 0 || w[1] <= 0.01) return;
          var art2 = lineArt(pp, w[0]);
          g.globalAlpha = Math.min(1, seg(t, T.groundEnd - 80, 260)) * w[1];
          g.drawImage(art2, (px0 + Math.round((pp.w - art2.width) / 2)) * apDev,
                      (py0 + Math.round((pp.h - art2.height) / 2)) * apDev, art2.width * apDev, art2.height * apDev);
        });
        g.restore();
        /* the steps: a hit area over each drawn arrow */
        var bx = Math.round(pp.box * ap), by = Math.round((py0 + (pp.h - pp.box) / 2) * ap);
        seatStep(prevBtn, Math.round((px0 + pp.pad) * ap), by, bx);
        seatStep(nextBtn, Math.round((px0 + pp.w - pp.pad - pp.box) * ap), by, bx);
      }
    }
    /* Where a step's button sits, over the arrow the panel drew. (`step` is
       taken: the run planner counts with it.) */
    function seatStep(btn, x, y, side) {
      var key = x + ',' + y + ',' + side;
      if (btn.dataset.at === key) return;
      btn.dataset.at = key;
      btn.style.left = x + 'px'; btn.style.top = y + 'px';
      btn.style.width = side + 'px'; btn.style.height = side + 'px';
    }
    /* A beat's words, drawn once at this size and kept. */
    var lines = {};
    function lineArt(pp, i) {
      var k = pp.key + '|' + i;
      if (lines[k]) return lines[k];
      var rows = pp.lines[i], A = art(pp.tw, rows.length * pp.lineH);
      rows.forEach(function (line, j) {
        putText(A, line, Math.round((pp.tw - textW(line, pp.s)) / 2), j * pp.lineH, pp.s, P.panelText, P.panelShade);
      });
      return (lines[k] = A.c);
    }

    /* ── A LINK'S CARD: one to a feature, in a line. The scene waits while it
       is open. ── */
    function openCard(i) {
      if (done || closing) return;
      var link = LINK[CHAIN[i]];
      card.open = true; card.i = i;
      cardEl.textContent = '';
      add(cardEl, 'h2', null, link.says).id = 'how-card-title';
      add(cardEl, 'p', null, link.card);
      add(cardEl, 'p', 'under', UNDER);
      var shut = add(cardEl, 'button', 'how-card-close', 'Close');
      shut.type = 'button';
      shut.addEventListener('click', closeCard);
      modal.hidden = false;
      cardEl.scrollTop = 0;
      cardEl.focus({ preventScroll: true });
    }
    function closeCard() {
      if (!card.open) return;
      card.open = false; card.gen++;
      modal.hidden = true;
      var from = boardBtns[card.i];
      (from && !from.el.hidden ? from.el : nextBtn).focus({ preventScroll: true });
    }
    modal.addEventListener('click', function (e) { if (e.target === modal) closeCard(); });
    /* Tab goes round the card and never out of it while it is open. */
    function trapTab(e) {
      var f = Array.prototype.filter.call(cardEl.querySelectorAll('a[href], button'), function (x) { return !x.disabled; });
      if (!f.length) return;
      var first = f[0], lastF = f[f.length - 1], at = document.activeElement;
      if (!cardEl.contains(at) || (e.shiftKey && (at === first || at === cardEl))) { e.preventDefault(); (e.shiftKey ? lastF : first).focus(); }
      else if (!e.shiftKey && at === lastF) { e.preventDefault(); first.focus(); }
    }

    function tick(now) {
      if (done) return;
      var dt = last ? Math.min(50, now - last) : 16;
      last = now; now0 = now;
      if (card.open) { draw(); return; }                        // the scene waits for the reader
      if (seekTo !== null) {
        var d = seekTo - t;
        if (still) t = seekTo;
        else t += Math.sign(d) * Math.min(Math.abs(d), dt * seekSpeed);
        if (t === seekTo) {
          seekTo = null;
          if (closing) { draw(); finish(); return; }
        }
      } else t += dt;
      draw();
    }

    /* A step plays the scene there at a rate fixed when it is asked for, so
       every step takes the same time however far it goes: `ms`. */
    function seek(target, ms) {
      seekTo = Math.max(0, target);
      seekSpeed = Math.max(3, Math.abs(seekTo - t) / (ms || 600));
    }
    function next() { if (closing) return; var n = beatAt(seekTo !== null ? seekTo : t); if (n < B.length - 1) seek(B[n + 1]); }
    /* Back is always the step before, never the start of this one: a reader
       who presses it and sees the same words has been told nothing. Step 0 has
       nothing before it, so it plays again from the top. */
    function prev() {
      if (closing) return;
      var n = beatAt(seekTo !== null ? seekTo : t);
      seek(n <= 1 ? T.clear : B[n - 1]);
    }
    /* CLOSING HANDS THE BOARD BACK; IT DOES NOT PERFORM (2026-09-24). This used
       to `seek(0, 1400)` — the ceremony playing itself backwards, every step it
       had just shown, before it let go. It reads as a bug rather than a
       flourish: a press on Close, on Escape or on Home is someone asking to
       leave, and replaying what they have just watched, in reverse, is not an
       answer to that. It looks like the thing has hung.

       It could also hang for real. The tick above returns early while a card is
       open — the scene waits for the reader — so a close that only SET a seek
       and left the rest to the tick never advanced while a card held the keys.
       Escape could not reach that state (it closes the card first), but the
       Home link in site.html calls this entry point directly, which made the
       one way out of the ceremony the one way to wedge it. Closing is now what
       the word says. */
    function close() {
      if (done || closing) return;
      closing = true;
      finish();
    }
    function finish() {
      done = true;
      document.removeEventListener('keydown', onKey, true);
      o.hide(false);
      card.open = false;
      canvas.remove(); cap.remove(); stepsEl.remove(); boardsEl.remove(); modal.remove();
      document.body.classList.remove('how-on');
      active = null;
      o.onClose();
    }
    function onKey(e) {
      if (card.open) {                                          // the card has the keys, and the steps wait
        if (e.key === 'Escape') { e.stopImmediatePropagation(); e.preventDefault(); closeCard(); }
        else if (e.key === 'Tab') trapTab(e);
        else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') e.stopImmediatePropagation();
        return;
      }
      if (e.key === 'Escape') { e.stopImmediatePropagation(); e.preventDefault(); close(); }
      else if (e.key === 'ArrowRight') { e.stopImmediatePropagation(); e.preventDefault(); next(); }
      else if (e.key === 'ArrowLeft') { e.stopImmediatePropagation(); e.preventDefault(); prev(); }
    }
    prevBtn.addEventListener('click', prev);
    nextBtn.addEventListener('click', next);
    document.addEventListener('keydown', onKey, true);
    requestAnimationFrame(function () { nextBtn.focus({ preventScroll: true }); });

    active = {
      tick: tick, close: close, next: next, prev: prev,
      timeline: T, beats: B, deltas: deltas, room: ROOM, at: function () { return t; }, beat: function () { return beatAt(t); },
      card: function () { return card.open ? card.i : -1; }
    };
    draw();
    return active;
  }

  /* Called every frame by site.html's drive(): runs a ceremony that is playing,
     and nothing at all when none is. */
  function tick(now) {
    if (active) active.tick(now);
  }

  var K = root.Kenjin = root.Kenjin || {};
  K.Ceremony = {
    play: play, tick: tick, PICTURES: PICTURES,
    /* Read-only, for whoever is watching, and the steps, for a test to press. */
    playing: function () { return !!active; },
    /* The same Close as the button: puts the ceremony away and hands the board back. */
    close: function () { if (active) active.close(); },
    timeline: function () { return active ? active.timeline : null; },
    beats: function () { return active ? active.beats : null; },
    deltas: function () { return active ? active.deltas : null; },
    at: function () { return active ? active.at() : null; },
    beat: function () { return active ? active.beat() : null; },
    card: function () { return active ? active.card() : null; },     // the open card's link, or -1
    room: function () { return active ? active.room : null; },       // what a stack may hold, and the clouds it stops under
    /* The scene's pictures, for anything else that draws it — branding's
       docs/instagram-grid.html composes the grid from these, not a copy. */
    art: { P: P, SKY: SKY, BAYER: BAYER, LEAD: LEAD, BIRD: BIRD, hash: hash, css: css, art: art, skyArt: skyArt, cloudArt: cloudArt,
           textW: textW, putText: putText, panelBox: panelBox,
           lcd: lcd, px: px, box: box, frame: frame, glyph: glyph, TONE: { OFF: OFF, INK: INK, MID: MID, PALE: PALE, TONE: TONE, DEEP: DEEP },
           sunArt: sunArt, groundTexture: groundTexture, bloomArt: bloomArt }
  };
})(typeof self !== 'undefined' ? self : this);
