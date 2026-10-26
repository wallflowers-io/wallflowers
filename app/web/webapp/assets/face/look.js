/* look.js — a face's LOOK, rendered: colours, type, wallpaper and blocks as CSS
 * custom properties on any element that holds it.
 *
 *     WallFlowers.Look.apply(node, look)   look is a face document's `look`
 *
 * Moved out of face.js (2026-09-25) so the face editor and the webapp
 * (product/app/web/webapp, which themes itself from the Face of the community
 * being viewed) render a look with one function, not two. face.js keeps what is
 * the editor's alone — the swatches, the presets, the pickers.
 *
 * Needs vendor/face-assets.js (the fonts beyond the three built in) loaded
 * first; vendor/fonts.css carries their @font-face rules.
 *
 * ?v= IS LOAD-BEARING: assets/ is served immutable for a year.
 */
(function (global) {
  'use strict';
  var W = global.WallFlowers = global.WallFlowers || {};
  var ASSETS = W.FaceAssets || { fonts: [] };

  /* ---- colour ---------------------------------------------------------------- */
  var PAPER = '#FAF8F3', INK = '#14181E', WHITE = '#FFFFFF';
  function rgb(hex) {
    var m = /^#?([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(hex || '');
    return m ? [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)] : [0, 0, 0];
  }
  function rgba(hex, a) { var c = rgb(hex); return 'rgba(' + c[0] + ',' + c[1] + ',' + c[2] + ',' + a + ')'; }
  function luminance(hex) {
    var l = rgb(hex).map(function (v) {
      var c = v / 255;
      return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
    });
    return 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
  }
  // WCAG's ratio: 4.5 is the floor for text, 3 for large type
  function contrast(a, b) {
    var x = luminance(a), y = luminance(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  }
  // ink or white on a colour, whichever reads better
  function onColour(hex) { return contrast(hex, INK) >= contrast(hex, WHITE) ? INK : WHITE; }
  function isHex(v) { return typeof v === 'string' && /^#[\da-f]{6}$/i.test(v); }
  // EVERY COLOUR A #rrggbb, OR ITS DEFAULT (O-62, Software Security): each is written into
  // CSS as it stands, so a string that is not one draws the default, never itself. The
  // defaults are face-render's (look.rs), so the Arc and a page draw a look alike.
  function hexOr(v, d) { return isHex(v) ? v : d; }
  function colours(c) {
    c = c || {};
    var text = hexOr(c.text, INK), buttons = hexOr(c.buttons, '#546CAC');
    return { background: hexOr(c.background, PAPER), text: text, title: hexOr(c.title, text), cards: hexOr(c.cards, WHITE),
             buttons: buttons, buttonText: hexOr(c.buttonText, onColour(buttons)) };
  }

  /* ---- type -------------------------------------------------------------------
     Three built in; the rest from vendor/face-assets.js, which also writes the
     @font-face rules (vendor/fonts.css). A font is fetched only once it is shown. */
  var SANS = '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", sans-serif';
  var SERIF = '"Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif';
  var MARKER = '"Wallflowers", ' + SANS;      // the marker draws ASCII only; the rest falls through
  var FONTS = [
    { id: 'marker', name: 'Marker', group: 'Built in', stack: MARKER },
    { id: 'system-sans', name: 'System sans', group: 'Built in', stack: SANS },
    { id: 'system-serif', name: 'System serif', group: 'Built in', stack: SERIF }
  ].concat(ASSETS.fonts);
  function fontOf(id) { return FONTS.filter(function (f) { return f.id === id; })[0] || FONTS[1]; }

  /* ---- wallpaper and blocks ----------------------------------------------------- */
  var PATTERNS = {
    dots:   { name: 'Dots', w: 18, h: 18, alpha: 0.22, svg: function (c) { return '<circle cx="9" cy="9" r="1.7" fill="' + c + '"/>'; } },
    grid:   { name: 'Grid', w: 24, h: 24, alpha: 0.16, svg: function (c) { return '<path d="M24 .5H.5V24" fill="none" stroke="' + c + '"/>'; } },
    lines:  { name: 'Lines', w: 14, h: 14, alpha: 0.14, svg: function (c) { return '<path d="M-2 2 2-2M0 14 14 0M12 16l4-4" stroke="' + c + '" stroke-width="1.3"/>'; } },
    waves:  { name: 'Waves', w: 40, h: 14, alpha: 0.2, svg: function (c) { return '<path d="M0 7q10-7 20 0t20 0" fill="none" stroke="' + c + '" stroke-width="1.4"/>'; } },
    checks: { name: 'Checks', w: 28, h: 28, alpha: 0.07, svg: function (c) { return '<path d="M0 0h14v14H0zM14 14h14v14H14z" fill="' + c + '"/>'; } },
    plus:   { name: 'Plus', w: 26, h: 26, alpha: 0.22, svg: function (c) { return '<path d="M13 8v10M8 13h10" stroke="' + c + '" stroke-width="1.6" stroke-linecap="round"/>'; } }
  };
  var CORNERS = [['Square', 4], ['Soft', 12], ['Round', 20], ['Pill', 999]];

  function patternUrl(id, colour) {
    var p = PATTERNS[id] || PATTERNS.dots;
    var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + p.w + '" height="' + p.h + '">' + p.svg(colour) + '</svg>';
    return 'url("data:image/svg+xml,' + encodeURIComponent(svg) + '")';
  }
  // A WALLPAPER IMAGE IN ONE OF TWO SHAPES (O-62, Software Security; the webapp's IMAGE):
  // a plain path on this origin, or an image's own data. It is written into url("…") as
  // it stands, so anything else, a quote that closes it included, draws no image.
  var IMAGE = /^(?!\/\/)[A-Za-z0-9._~\/-]+$|^data:image\/(png|jpeg|gif|webp);base64,[A-Za-z0-9+\/=]+$/;
  function wallpaperCss(lk) {
    var wp = lk.wallpaper || {}, c = colours(lk.colours), bg = c.background, c2 = hexOr(wp.colour2, bg), c3 = hexOr(wp.colour3, c2);
    if (wp.style === 'gradient') {
      return wp.angle === 'radial' ? 'radial-gradient(130% 80% at 50% 0%, ' + c2 + ', ' + bg + ')'
        : 'linear-gradient(' + (wp.angle === 'diagonal' ? '155deg' : '180deg') + ', ' + bg + ', ' + c2 + ')';
    }
    if (wp.style === 'blur') {
      return 'radial-gradient(60% 42% at 12% 10%, ' + c2 + ', transparent 72%), '
        + 'radial-gradient(55% 40% at 92% 44%, ' + c3 + ', transparent 72%), '
        + 'radial-gradient(70% 46% at 30% 96%, ' + c2 + ', transparent 72%), ' + bg;
    }
    if (wp.style === 'pattern') {
      return patternUrl(wp.pattern, rgba(c.text, (PATTERNS[wp.pattern] || PATTERNS.dots).alpha)) + ', ' + bg;
    }
    if (wp.style === 'image' && typeof wp.image === 'string' && IMAGE.test(wp.image)) {
      var veil = rgba(bg, (wp.soften || 0) / 100);
      return 'linear-gradient(' + veil + ', ' + veil + '), url("' + wp.image + '") center / cover no-repeat, ' + bg;
    }
    return bg;
  }

  // How cards and buttons are filled, edged and lifted, from the colours given.
  function blockVars(c, b) {
    c = colours(c);
    var edge = b.shadow === 'hard' ? '2px solid ' + c.text : '';
    var v = { '--shadow': { none: 'none', soft: '0 1px 2px rgba(20,24,30,.06), 0 6px 18px rgba(20,24,30,.08)',
                            strong: '0 16px 30px -12px rgba(0,0,0,.5)', hard: '4px 4px 0 ' + c.text }[b.shadow] || 'none' };
    if (b.style === 'glass') {
      v['--btn-bg'] = rgba(c.buttons, 0.26); v['--btn-fg'] = c.buttonText; v['--btn-border'] = edge || '1px solid ' + rgba(WHITE, 0.45);
      v['--card-bg'] = rgba(c.cards, 0.18); v['--card-border'] = edge || '1px solid ' + rgba(WHITE, 0.3);
      v['--blur'] = 'blur(18px) saturate(1.4)';
    } else if (b.style === 'outline') {
      v['--btn-bg'] = 'transparent'; v['--btn-fg'] = c.buttons; v['--btn-border'] = '2px solid ' + c.buttons;
      v['--card-bg'] = 'transparent'; v['--card-border'] = edge || '1.5px solid ' + rgba(c.text, 0.45);
      v['--blur'] = 'none';
    } else {
      v['--btn-bg'] = c.buttons; v['--btn-fg'] = c.buttonText; v['--btn-border'] = edge || '0 solid transparent';
      v['--card-bg'] = c.cards; v['--card-border'] = edge || '0 solid transparent';
      v['--blur'] = 'none';
    }
    return v;
  }

  // The look, as custom properties on any element that holds a face (face.css reads them).
  function apply(node, lk) {
    var c = colours(lk.colours), r = (CORNERS[lk.blocks.corners] || CORNERS[3])[1];
    var v = blockVars(c, lk.blocks);
    v['--bg'] = c.background; v['--fg'] = c.text; v['--title'] = c.title; v['--surface'] = c.cards;
    v['--accent'] = c.buttons; v['--on-accent'] = c.buttonText;
    v['--muted'] = rgba(c.text, 0.64); v['--line'] = rgba(c.text, 0.14);
    v['--wallpaper'] = wallpaperCss(lk);
    v['--font'] = fontOf(lk.text.font).stack; v['--name-font'] = fontOf(lk.text.title || lk.text.font).stack;
    v['--btn-radius'] = r + 'px'; v['--card-radius'] = Math.min(r, 24) + 'px'; v['--door-radius'] = r + 'px';
    Object.keys(v).forEach(function (k) { node.style.setProperty(k, v[k]); });
    node.classList.toggle('wf-dark', luminance(c.background) < 0.3);
    return v;
  }

  W.Look = {
    PAPER: PAPER, INK: INK, WHITE: WHITE, SANS: SANS, SERIF: SERIF, MARKER: MARKER,
    FONTS: FONTS, PATTERNS: PATTERNS, CORNERS: CORNERS,
    rgb: rgb, rgba: rgba, luminance: luminance, contrast: contrast, onColour: onColour, isHex: isHex,
    fontOf: fontOf, patternUrl: patternUrl, wallpaperCss: wallpaperCss, blockVars: blockVars, apply: apply
  };
})(window);
