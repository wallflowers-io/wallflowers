/* palette.js — a Site's colours in the webapp: a quieter palette of many shades DERIVED
   from its Face's look, never the look laid on as it is (Ralph, 28 Sep: "Instead of using
   the theme directly, sites should use a subtler, multi-shade palette derived from the Face
   theme").

     WallFlowers.Palette.from(look)   { vars: {'--s0': '#…', …}, dark }

   The ground takes the background's hue at a whisper of its chroma, in five steps of
   lightness: the pane (s0), the rails (s1), a hover (s2), a selection (s3) and the hairline.
   The text is the look's text, held to a readable contrast on the pane, in three strengths.
   The accent, the look's buttons, comes as six shades from a tint (a1) through itself (a4)
   to a deep (a6). The title is the look's title, held readable. Worked in OKLab, so a step
   looks like the same step in every hue.

   A look with no colours is the Face editor's own first theme, Paper. Pure: no DOM, so a
   test can hand it any look (palette.test.mjs). */
(function (root) {
  'use strict';

  var PAPER = { background: '#FAF8F3', cards: '#FFFFFF', buttons: '#546CAC', buttonText: '#FFFFFF',
                text: '#14181E', title: '#14181E' };

  function clamp(x, a, b) { return Math.min(b, Math.max(a, x)); }
  function rgbOf(h) {
    var m = /^#?([\da-f]{6})$/i.exec(h || '');
    if (!m) return null;
    var n = parseInt(m[1], 16);
    return [(n >> 16) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  }
  function lin(c) { return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); }
  function gam(c) { return c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055; }

  /* sRGB <-> OKLab (Björn Ottosson's), and OKLCH over it. */
  function lab(rgb) {
    var r = lin(rgb[0]), g = lin(rgb[1]), b = lin(rgb[2]);
    var l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    var m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    var s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    return [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s];
  }
  function rgb(o) {
    var l = Math.pow(o[0] + 0.3963377774 * o[1] + 0.2158037573 * o[2], 3);
    var m = Math.pow(o[0] - 0.1055613458 * o[1] - 0.0638541728 * o[2], 3);
    var s = Math.pow(o[0] - 0.0894841775 * o[1] - 1.2914855480 * o[2], 3);
    return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s];
  }
  function lch(hex) { var o = lab(rgbOf(hex)); return { L: o[0], C: Math.hypot(o[1], o[2]), h: Math.atan2(o[2], o[1]) }; }
  /* Into sRGB: a colour outside it keeps its lightness and hue and gives up chroma. */
  function hex(c) {
    var C = c.C;
    for (var i = 0; i < 40; i++) {
      var v = rgb([c.L, C * Math.cos(c.h), C * Math.sin(c.h)]);
      if (C < 1e-4 || v.every(function (x) { return x >= -1e-4 && x <= 1 + 1e-4; })) {
        return '#' + v.map(function (x) {
          var n = Math.round(clamp(gam(clamp(x, 0, 1)), 0, 1) * 255);
          return (n < 16 ? '0' : '') + n.toString(16);
        }).join('').toUpperCase();
      }
      C *= 0.92;
    }
    return hex({ L: c.L, C: 0, h: c.h });
  }
  function mix(a, b, t) {
    var A = [a.L, a.C * Math.cos(a.h), a.C * Math.sin(a.h)], B = [b.L, b.C * Math.cos(b.h), b.C * Math.sin(b.h)];
    var o = A.map(function (x, i) { return x + (B[i] - x) * t; });
    return { L: o[0], C: Math.hypot(o[1], o[2]), h: Math.atan2(o[2], o[1]) };
  }
  function step(c, L, C) { return { L: clamp(L, 0, 1), C: C == null ? c.C : C, h: c.h }; }

  /* WCAG's contrast, which the Face editor warns by (look.js). */
  function luminance(h) { var c = rgbOf(h).map(lin); return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]; }
  function contrast(a, b) { var x = luminance(a), y = luminance(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); }
  /* A colour moved away from the ground, lightness only, until it reads on it. */
  function readable(c, ground, min, dark) {
    var o = { L: c.L, C: c.C, h: c.h };
    for (var i = 0; i < 60 && contrast(hex(o), ground) < min; i++) o.L = clamp(o.L + (dark ? 0.015 : -0.015), 0, 1);
    return o;
  }
  function valid(h) { return typeof h === 'string' && /^#[\da-f]{6}$/i.test(h); }

  function from(look) {
    var given = (look && look.colours) || {}, c = {};
    Object.keys(PAPER).forEach(function (k) { c[k] = valid(given[k]) ? given[k] : PAPER[k]; });
    if (!valid(given.title) && valid(given.text)) c.title = given.text;
    var bg = lch(c.background), fg = lch(c.text), ti = lch(c.title);
    /* THE ACCENT is the Face's first real colour: its buttons, unless they are white, black
       or grey (Tulip's are white on pink), then its background, its wallpaper's second
       colour, its title. A Face with no colour anywhere keeps its buttons, quietly. */
    var wp = look && look.wallpaper && valid(look.wallpaper.colour2) ? look.wallpaper.colour2 : null;
    var ac = [c.buttons, c.background, wp, c.title].filter(Boolean).map(lch)
      .filter(function (x) { return x.C >= 0.05; })[0] || lch(c.buttons);
    var dark = bg.L < 0.6;

    /* THE GROUND: the background's own hue, or the accent's where the background is grey,
       held to a whisper of chroma so a loud Face still makes a quiet room. */
    var tinted = bg.C > 0.004, hue = { h: (tinted ? bg : ac).h };
    var C = tinted ? Math.min(bg.C, dark ? 0.05 : 0.03) : 0.008;
    var s;
    if (dark) {
      var d = clamp(bg.L, 0.2, 0.34);
      s = { s0: step(hue, d, C), s1: step(hue, d - 0.03, C), s2: step(hue, d + 0.05, C),
            s3: step(hue, d + 0.09, C * 1.1), hair: step(hue, d + 0.08, C * 0.8) };
    } else {
      var l = clamp(bg.L, 0.9, 0.975);
      s = { s0: step(hue, l + (1 - l) * 0.6, C * 0.45), s1: step(hue, l, C), s2: step(hue, l - 0.035, C),
            s3: step(hue, l - 0.07, C * 1.1), hair: step(hue, l - 0.075, C * 0.8) };
    }
    var ground = hex(s.s0);

    /* THE WORDS, three strengths of the look's own text colour. */
    var ink = readable(fg, ground, 9, dark);
    var i2 = readable(mix(ink, s.s0, 0.3), ground, 5.5, dark);
    var i3 = readable(mix(ink, s.s0, 0.5), ground, 3.4, dark);

    /* THE ACCENT, six shades of the look's buttons: tints on the ground, itself, deeps. */
    var accent = readable(ac, ground, 3, dark);
    var a = [mix(accent, s.s0, 0.9), mix(accent, s.s0, 0.78), mix(accent, s.s0, 0.55),
             accent, mix(accent, ink, 0.3), mix(accent, ink, 0.55)];
    var A = hex(accent), white = '#FFFFFF', black = '#14181E';
    var onAccent = contrast(c.buttonText, A) >= 3 ? c.buttonText : contrast(white, A) >= contrast(black, A) ? white : black;

    var vars = {
      '--s0': ground, '--s1': hex(s.s1), '--s2': hex(s.s2), '--s3': hex(s.s3), '--hair': hex(s.hair),
      '--ink': hex(ink), '--i2': hex(i2), '--i3': hex(i3), '--title': hex(readable(ti, ground, 4.5, dark)),
      '--accent': A, '--on-accent': onAccent, '--bar': 'color-mix(in srgb,' + hex(s.s1) + ' 86%,transparent)'
    };
    a.forEach(function (x, i) { vars['--a' + (i + 1)] = hex(x); });
    return { vars: vars, dark: dark };
  }

  var api = { from: from, contrast: contrast, PAPER: PAPER };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else { root.WallFlowers = root.WallFlowers || {}; root.WallFlowers.Palette = api; }
})(typeof self !== 'undefined' ? self : this);
