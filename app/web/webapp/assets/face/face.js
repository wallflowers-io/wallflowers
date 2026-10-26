/* face.js — THE PUBLIC FACE, step 2 of /signup.
 *
 * A site's face is the link-in-bio page an EXTERNAL USER lands on: someone who
 * follows the link after the site is published. Anyone signed in skips it and
 * lands in the webapp, which is a separate module — so nothing here is a webapp
 * feature. The site's creator lays the face out here the way a phone's home
 * screen is laid out:
 *
 *   - a widget reaches the face ONE way: dragged there, from the Widgets list
 *     or from the configure window an entry there opens;
 *   - a placed widget is dragged to move and tapped to configure;
 *   - STICKERS are dragged on anywhere, then moved, turned and sized in place;
 *   - Show / Hide switches what the header says;
 *   - the DOOR is always there, bottom centre, outside the scroll: the way in.
 *
 * And dresses it the way link-in-bio pages taught people to, keeping the good
 * bits: a THEME to start from (or Shuffle), then each part on its own — header,
 * wallpaper, cards and buttons, type, colours, with a warning when a pairing is
 * hard to read — and what a visitor meets first: the door, a content notice,
 * how the link looks when shared, whether search engines list it. Dropped: the
 * pixels, analytics and UTM tags, "connect your Instagram", content scraped
 * from elsewhere, and anything locked behind a plan.
 *
 * Two columns: the phone on the right, and on the left one panel — a RAIL of
 * sections, the basics first (theme, header, widgets, door), then what makes the
 * face its own, then what a visitor meets before it, with Shuffle at its foot;
 * and beside it the DETAIL of the section the rail has lit, with the previous
 * and next at its foot. Tapping a thing on the phone opens its section.
 *
 * A widget may take its own colours over the look's — each one it does not set
 * follows the look, so a new theme still reaches it.
 *
 * Fonts, stickers and platform icons are SELF-HOSTED, in vendor/ beside this
 * file, fetched once by scripts/vendor-face.py: a face that loaded them from a
 * CDN would tell the CDN who looked at it.
 *
 * Its widgets have no content yet and are not meant to: events and publications
 * come from the site's own MLS activity once it is live, and the Arc serves them
 * via the Host System. So the phone shows shapes, never samples.
 *
 * THE FIELDS A WIDGET CAN SHOW ARE READ FROM THE ICD — assets/icd/, a copy of
 * product/core/coordination/delta-graph.icd.json vendored by scripts/vendor.py
 * (`just vendor`; gitignored, like assets/pacific/) — never transcribed. An op
 * renamed or removed there is said so here, loudly. The vendor step checks the
 * ops named below against the document BEFORE it copies, so a deploy cannot take
 * an ICD this file cannot read.
 *
 * The widgets are held as a Lexical-shaped document (root.children of typed
 * nodes), the shape Ghost's renderer reads.
 *
 * NOTHING HERE IS SAVED — see FACE_SAVED in signup.html. Undo and redo live in
 * this page's memory and go with it.
 *
 *   WallFlowers.Face.mount(el)        build the editor into el
 *   WallFlowers.Face.site(s)          the site from step 1: {name, purpose, slug, kind}
 *   WallFlowers.Face.summary()        {count, stickers, names, look, door} for the Review step
 *   WallFlowers.Face.document()       the face as data
 *   WallFlowers.Face.deltas()         the face as the Deltas that would carry it (none is sent)
 *   WallFlowers.Face.load(doc, media) an existing Site's face into the editor: document()'s
 *                                     inverse, its pictures beside it by the Host's slot
 *   WallFlowers.Face.media()          the pictures, as data URLs by the Host's slot
 *
 * The webapp mounts this same editor to change a Site's face (Ralph, 28 Sep: "a direct
 * import from the Face editor on wallflowers.io/signup"), and writes what it makes itself.
 */
(function (global) {
  'use strict';
  var W = global.WallFlowers = global.WallFlowers || {};

  // v=2: kinds/facets, not components.messages. A page that has the model elsewhere names it:
  // the webapp, on the Door, sets WallFlowers.FaceIcd to the Door's own /v2/icd.
  var ICD_URL = W.FaceIcd || 'assets/icd/delta-graph.icd.json?v=2';
  // vendor/ sits beside this file, wherever the page that loads it lives.
  var VENDOR = ((document.currentScript && document.currentScript.src) || '').replace(/[^\/]*$/, '') + 'vendor/';
  var ASSETS = W.FaceAssets;
  if (!ASSETS) {
    ASSETS = { fonts: [], stickers: [], icons: {} };
    if (global.console) console.error('face: vendor/face-assets.js is not loaded — no extra fonts, stickers or platform icons.');
  }

  /* ---- colour ----------------------------------------------------------------
     WallFlowers' own: paper #FAF8F3 and ink #14181E (site.html), the marker
     colours (branding/wallflowers/parametric/wordmark.py). Lilac is left out —
     the branding README keeps it for the flower alone. They are the picker's
     swatches; any colour can be typed or picked. */
  /* The look's own machinery — colour, type, wallpaper, blocks — is look.js
     (WallFlowers.Look), shared with the webapp; these are its names here. */
  var L = W.Look;
  if (!L) throw new Error('face: assets/face/look.js must load before face.js');
  var PAPER = L.PAPER, INK = L.INK, WHITE = L.WHITE;
  var COLOURS = [
    ['Blue', '#546CAC'], ['Raspberry', '#CD5B7A'], ['Orange', '#DC7629'], ['Gold', '#CBB25A'],
    ['Periwinkle', '#8BA1C8'], ['Pink', '#D793B5'], ['Sky', '#A6C8E0'], ['Yellow', '#EADD3E'],
    ['Ink', INK], ['Paper', PAPER], ['White', WHITE], ['Black', '#000000']
  ];
  var rgb = L.rgb, rgba = L.rgba, luminance = L.luminance, contrast = L.contrast, onColour = L.onColour, isHex = L.isHex;

  /* ---- type -------------------------------------------------------------------
     Three built in; the rest from vendor/face-assets.js, which also writes the
     @font-face rules (vendor/fonts.css). A font is fetched only once it is shown. */
  var SANS = L.SANS, SERIF = L.SERIF, MARKER = L.MARKER, FONTS = L.FONTS;
  var FONT_GROUPS = ['Built in', 'Sans', 'Serif', 'Display', 'Handwritten', 'Mono'];
  var fontOf = L.fontOf;

  /* ---- the look ----------------------------------------------------------------
     Everything but the header: wallpaper, cards and buttons, type, colours. A
     theme is one whole look; changing any part of it makes the look Custom. */
  var WALLPAPERS = [['fill', 'Fill'], ['gradient', 'Gradient'], ['blur', 'Blur'], ['pattern', 'Pattern'], ['image', 'Image']];
  var PATTERNS = L.PATTERNS;
  var DIRECTIONS = { down: 'Down', diagonal: 'Diagonal', radial: 'Radial' };
  var STYLES = [['solid', 'Solid'], ['glass', 'Glass'], ['outline', 'Outline']];
  var CORNERS = L.CORNERS;
  var SHADOWS = { none: 'None', soft: 'Soft', strong: 'Strong', hard: 'Hard' };
  var COLOUR_ROWS = [['background', 'Background'], ['cards', 'Cards'], ['buttons', 'Buttons'],
                     ['buttonText', 'Button text'], ['text', 'Text'], ['title', 'Title']];

  function look(o) {
    return {
      wallpaper: { style: o.wallpaper || 'fill', colour2: o.colour2 || o.bg, colour3: o.colour3 || o.colour2 || o.bg,
                   angle: o.angle || 'down', pattern: o.pattern || 'dots', image: '', soften: 30 },
      blocks: { style: o.style || 'solid', corners: o.corners == null ? 3 : o.corners, shadow: o.shadow || 'soft' },
      text: { font: o.font || 'system-sans', title: o.title || '' },
      colours: { background: o.bg, cards: o.cards || WHITE, buttons: o.buttons, buttonText: o.buttonText || onColour(o.buttons),
                 text: o.text || INK, title: o.titleColour || o.text || INK }
    };
  }
  var PRESETS = [
    ['paper', 'Paper', { bg: PAPER, buttons: '#546CAC', title: 'marker' }],
    ['ink', 'Ink', { bg: '#000000', cards: INK, buttons: PAPER, text: '#E6EDE9', titleColour: PAPER, title: 'marker', shadow: 'none' }],
    ['marker', 'Marker', { bg: '#F4EFB4', cards: '#FFFCE8', buttons: INK, font: 'space-grotesk', title: 'marker', corners: 1, shadow: 'hard' }],
    ['allotment', 'Allotment', { wallpaper: 'pattern', pattern: 'grid', bg: '#E3EFD3', buttons: WHITE, font: 'dm-sans', title: 'bagel-fat-one', shadow: 'hard' }],
    ['tulip', 'Tulip', { wallpaper: 'gradient', bg: '#CD5B7A', colour2: '#DC7629', angle: 'diagonal', buttons: WHITE, buttonText: WHITE,
                         text: WHITE, font: 'figtree', title: 'fraunces', style: 'glass', corners: 2, shadow: 'none' }],
    ['dusk', 'Dusk', { wallpaper: 'blur', bg: '#1C1A3A', colour2: '#D793B5', colour3: '#546CAC', buttons: WHITE, buttonText: WHITE,
                       text: WHITE, font: 'outfit', title: 'syne', style: 'glass', shadow: 'none' }],
    ['broadsheet', 'Broadsheet', { bg: WHITE, buttons: INK, font: 'newsreader', title: 'instrument-serif', style: 'outline', corners: 0, shadow: 'none' }],
    ['terminal', 'Terminal', { bg: '#0B0F0C', cards: '#0B0F0C', buttons: '#7CFFB2', text: '#B8F5CF', titleColour: '#7CFFB2',
                               font: 'jetbrains-mono', title: 'jetbrains-mono', style: 'outline', corners: 0, shadow: 'none' }],
    ['sunflower', 'Sunflower', { wallpaper: 'pattern', pattern: 'dots', bg: '#EADD3E', cards: '#FFF9D6', buttons: INK, font: 'rubik', title: 'shrikhand' }],
    ['periwinkle', 'Periwinkle', { wallpaper: 'gradient', bg: '#8BA1C8', colour2: '#CFE3F0', buttons: WHITE, buttonText: INK,
                                   text: WHITE, font: 'manrope', title: 'unbounded', style: 'glass', corners: 2 }],
    ['blossom', 'Blossom', { bg: '#F8E6EE', buttons: '#CD5B7A', text: '#3A1F2B', titleColour: '#B3405F', font: 'lora', title: 'young-serif', corners: 2 }],
    ['club', 'Club night', { wallpaper: 'pattern', pattern: 'lines', bg: INK, cards: '#1F242C', buttons: '#DC7629', text: '#F2F2F2',
                             titleColour: '#DC7629', font: 'space-grotesk', title: 'rubik-mono-one', corners: 0, shadow: 'strong' }],
    ['riso', 'Riso', { bg: '#F3EDE0', cards: '#F3EDE0', buttons: '#546CAC', text: '#A83A5A', font: 'space-mono', title: 'bricolage-grotesque', corners: 1, shadow: 'hard' }],
    ['zine', 'Zine', { wallpaper: 'pattern', pattern: 'waves', bg: WHITE, buttons: INK, font: 'space-mono', title: 'caveat', corners: 1, shadow: 'hard' }],
    ['sky', 'Sky', { wallpaper: 'blur', bg: '#DDEBF5', colour2: '#A6C8E0', colour3: '#EADD3E', buttons: WHITE, buttonText: INK,
                     font: 'figtree', title: 'pacifico', style: 'glass' }]
  ].map(function (p) { return { id: p[0], name: p[1], look: look(p[2]) }; });

  var patternUrl = L.patternUrl, wallpaperCss = L.wallpaperCss, blockVars = L.blockVars;

  // The look, as custom properties on any element that holds a face (face.css reads them).
  function applyLook(node, lk) { L.apply(node, lk || state.look); }
  /* A widget's own colours, laid over the look's AFTER it: `bg` fills the card (or
     the button, for a Link), `fg` is its text (or a Social row's icons). One not
     set is the look's, so changing the theme still reaches it. The style, corners
     and shadow stay the look's; a hard edge keeps the look's ink. */
  function ownColour(w, k) {
    var c = state.look.colours;
    if (w.colours && w.colours[k]) return w.colours[k];
    if (k === 'bg') return w.type === 'wf-link' ? c.buttons : c.cards;
    return w.type === 'wf-link' ? c.buttonText : c.text;
  }
  function paint(n, w) {
    var o = w.colours;
    if (!o || (!o.bg && !o.fg)) return n;
    var c = {};
    Object.keys(state.look.colours).forEach(function (k) { c[k] = state.look.colours[k]; });
    if (o.bg) { c.cards = o.bg; c.buttons = o.bg; }
    if (o.fg) c.buttonText = o.fg;
    var v = blockVars(c, state.look.blocks);
    ['--btn-bg', '--btn-fg', '--btn-border', '--card-bg', '--card-border'].forEach(function (k) { n.style.setProperty(k, v[k]); });
    if (o.fg) {
      n.style.setProperty('--fg', o.fg); n.style.setProperty('--btn-fg', o.fg);
      n.style.setProperty('--muted', rgba(o.fg, 0.64)); n.style.setProperty('--title', o.fg);
    }
    return n;
  }
  function presetNow() {
    var now = JSON.stringify(state.look);
    return PRESETS.filter(function (p) { return JSON.stringify(p.look) === now; })[0] || null;
  }

  /* ---- what can be shown -----------------------------------------------------
     A feed is a list of the site's own objects of one kind. Which OPS describe
     each is the only choice made here; their fields come from the ICD. */
  var FEEDS = {
    // A room is a part of the Site (base.setPart, role room, since the composition
    // migration): which object, and when, are the Arc's to address, not a visitor's to read.
    rooms:        { label: 'Rooms', one: 'room', ops: ['base.setPart'],
                    hide: ['part', 'at'] },
    publications: { label: 'Publications', one: 'publication', ops: ['post.setProfile'] },
    listings:     { label: 'Listings', one: 'listing', ops: ['thing.setProfile', 'thing.setPosture'],
                    // a listing's words and its price are set by two different ops, and a
                    // marketplace tile that cannot show a price is not a marketplace
                    hide: ['posture'] },
    events:       { label: 'Events', one: 'event', ops: ['event.setProfile'] },
    funding:      { label: 'The fund', one: 'entry', ops: ['treasury.recordDeposit', 'ratify.propose'],
                    // rule and gen are the ratify machinery; payload is the proposal's body,
                    // carried but not summarised on a tile this size
                    hide: ['rule', 'gen'] }
  };
  // The member count is the roster's, counted by the site's device.
  var MEMBER_OPS = ['base.memberJoined', 'base.memberLeft'];
  /* What the header may show, each switched on or off. */
  var SHOW = [
    { id: 'members', label: 'Member count' },
    { id: 'address', label: 'Address' },
    { id: 'purpose', label: 'What it’s for' },
    { id: 'share', label: 'Share button' }
  ];
  /* Location is the Group's geohash set — a PROPOSED ICD change: the Group
     carries no location today (base.setLocation is on Place and Event). The
     owner's device cuts every cell to this many characters before handing it to
     the Host System, so the Arc never holds anything finer. */
  var LEVELS = { 3: 'Region', 4: 'City', 5: 'District', 6: 'Neighbourhood', 7: 'Street', 8: 'Building', 9: 'Exact' };
  var DEFAULT_PRECISION = 5;   // thing.rs publishes a listing's area at 3–5; a face starts at the same 5
  // A cell's larger side, from the geohash definition: n characters are 5n bits,
  // latitude floor(5n/2) of them, longitude ceil(5n/2). At the equator.
  function cellSize(n) {
    var km = Math.max(180 / Math.pow(2, Math.floor(5 * n / 2)), 360 / Math.pow(2, Math.ceil(5 * n / 2))) * 111.32;
    return km >= 1 ? (km >= 10 ? Math.round(km) : km.toFixed(1)) + ' km' : Math.round(km * 1000) + ' m';
  }

  /* Where else a site is. A handle is typed, not a URL: the prefix is shown
     beside it and joined on. An empty prefix takes the whole address. */
  var PLATFORMS = [
    ['instagram', 'Instagram', 'instagram.com/'], ['tiktok', 'TikTok', 'tiktok.com/@'], ['youtube', 'YouTube', 'youtube.com/@'],
    ['x', 'X', 'x.com/'], ['threads', 'Threads', 'threads.net/@'], ['bluesky', 'Bluesky', 'bsky.app/profile/'],
    ['mastodon', 'Mastodon', ''], ['facebook', 'Facebook', 'facebook.com/'], ['linkedin', 'LinkedIn', 'linkedin.com/in/', 'in'],
    ['whatsapp', 'WhatsApp', 'wa.me/'], ['telegram', 'Telegram', 't.me/'], ['signal', 'Signal', ''],
    ['discord', 'Discord', 'discord.gg/'], ['twitch', 'Twitch', 'twitch.tv/'], ['spotify', 'Spotify', ''],
    ['soundcloud', 'SoundCloud', 'soundcloud.com/'], ['bandcamp', 'Bandcamp', ''], ['substack', 'Substack', ''],
    ['patreon', 'Patreon', 'patreon.com/'], ['meetup', 'Meetup', 'meetup.com/'], ['github', 'GitHub', 'github.com/'],
    ['pinterest', 'Pinterest', 'pinterest.com/'], ['snapchat', 'Snapchat', 'snapchat.com/add/'],
    ['website', 'Website', ''], ['email', 'Email', '']
  ].map(function (p) { return { id: p[0], name: p[1], prefix: p[2], letters: p[3] || p[1].charAt(0) }; });
  function platform(id) { return PLATFORMS.filter(function (p) { return p.id === id; })[0] || PLATFORMS[PLATFORMS.length - 2]; }
  function placeholderOf(p) {
    return p.id === 'email' ? 'hello@example.org' : p.id === 'mastodon' ? '@you@example.social' : p.prefix ? 'yourname' : 'https://…';
  }
  function linkUrl(l) {
    var h = (l.handle || '').trim(), p = platform(l.platform), m;
    if (!h) return '';
    if (l.platform === 'email') return 'mailto:' + h;
    if (/^https?:\/\//i.test(h)) return h;
    if (l.platform === 'mastodon' && (m = /^@?([^@\s]+)@([^@\s]+)$/.exec(h))) return 'https://' + m[2] + '/@' + m[1];
    return 'https://' + (p.prefix || '') + h.replace(/^@/, '');
  }
  var OWN_ICONS = {
    website: '<circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M3 12h18M12 3c3 3.2 3 14.8 0 18M12 3c-3 3.2-3 14.8 0 18" fill="none" stroke="currentColor" stroke-width="1.8"/>',
    email: '<rect x="3" y="5" width="18" height="14" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="m4 7 8 6 8-6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>'
  };
  function iconSvg(id) {
    var inner = ASSETS.icons[id] ? '<path fill="currentColor" d="' + ASSETS.icons[id] + '"/>' : OWN_ICONS[id];
    return inner ? '<svg viewBox="0 0 24 24" aria-hidden="true">' + inner + '</svg>' : '';
  }
  function glyph(cls, id) {
    var g = el('span', cls), svg = iconSvg(id);
    if (svg) g.innerHTML = svg; else g.textContent = platform(id).letters;
    return g;
  }

  /* ---- the door ----------------------------------------------------------------
     The way in, and the one thing every face has: always shown, bottom centre,
     outside the scroll. Who may come through it is the site's to say; what it
     is called follows unless the creator names it. */
  var ACCESS = [
    { id: 'invite', label: 'By invitation', door: 'I have an invitation' },
    { id: 'request', label: 'Ask to join', door: 'Knock' },
    { id: 'open', label: 'Open to anyone', door: 'Join' }
  ];
  function accessOf(id) { return ACCESS.filter(function (a) { return a.id === id; })[0] || ACCESS[1]; }
  var DOOR_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 21V4.5A1.5 1.5 0 0 1 7.5 3h9A1.5 1.5 0 0 1 18 4.5V21"/><path d="M3.5 21h17"/><circle cx="14.6" cy="12.2" r="1" fill="currentColor" stroke="none"/></svg>';

  /* ---- before a visitor comes in -------------------------------------------------
     A notice shown over the face until they choose to go on, and how the link
     reads where it is shared. Search listing is the page's robots meta. */
  var NOTICES = [
    { id: 'adult', label: 'Over 18s only', title: 'Over 18s only', text: 'You need to be 18 or over to see this.' },
    { id: 'sensitive', label: 'Sensitive content', title: 'Sensitive content', text: 'This may include things some people find upsetting.' },
    { id: 'custom', label: 'In your own words', title: 'Before you go in', text: '' }
  ];
  function noticeOf(id) { return NOTICES.filter(function (n) { return n.id === id; })[0] || NOTICES[1]; }

  /* ---- stickers -------------------------------------------------------------------
     Doodles are drawn here, in one of the face's own colours; the rest are
     Fluent Emoji (vendor/). A sticker sits in the face's own coordinates —
     x across and y down, both as fractions of the face's WIDTH — so it stays
     where it was put on any phone. */
  var DOODLES = [
    ['doodle-loop', 'Loop arrow', '<path d="M8 30c18-16 46-14 52 4 5 15-10 25-20 17-9-7 1-23 18-22 19 1 30 18 30 42"/><path d="m75 62 13 12 9-15"/>'],
    ['doodle-down', 'Arrow down', '<path d="M52 8c-9 26 7 50-2 80"/><path d="m33 70 17 18 18-16"/>'],
    ['doodle-swoop', 'Swoop arrow', '<path d="M8 66c20-32 56-38 80-12"/><path d="m70 38 19 16-17 16"/>'],
    ['doodle-circle', 'Circle', '<path d="M80 26C66 10 28 12 14 34 2 54 22 84 54 84c30 0 44-22 36-44-6-16-26-24-48-20"/>'],
    ['doodle-underline', 'Underline', '<path d="M6 58c16-8 30 6 46-2s28-8 42 0"/>'],
    ['doodle-sparkle', 'Sparkle', '<path d="M50 6c4 28 16 40 44 44-28 4-40 16-44 44-4-28-16-40-44-44 28-4 40-16 44-44z" fill="currentColor" stroke="none"/>'],
    ['doodle-burst', 'Burst', '<path d="M50 8v18m0 48v18M8 50h18m48 0h18M20 20l13 13m34 34 13 13m0-60L67 33M33 67 20 80"/>'],
    ['doodle-heart', 'Heart', '<path d="M50 84C22 64 8 48 14 30c5-15 26-18 36 0 10-18 31-15 36 0 6 18-8 34-36 54z"/>'],
    ['doodle-zigzag', 'Zigzag', '<path d="m6 58 14-18 14 18 14-18 14 18 14-18 14 18"/>'],
    ['doodle-smile', 'Smile', '<circle cx="50" cy="50" r="40"/><path d="M34 58c8 10 24 10 32 0M38 38v4m24-4v4"/>']
  ];
  var STICKERS = DOODLES.map(function (d) { return { id: d[0], name: d[1], group: 'Doodles', svg: d[2] }; }).concat(ASSETS.stickers);
  var STICKER_GROUPS = ['Doodles', 'This way', 'Flowers', 'Celebrate', 'Together', 'Out and about'];
  var INKS = { buttons: 'var(--accent)', text: 'var(--fg)', title: 'var(--title)' };
  function stickerOf(id) { return STICKERS.filter(function (s) { return s.id === id; })[0] || null; }
  function doodleSvg(def) {
    return '<svg viewBox="0 0 100 100" fill="none" stroke="currentColor" stroke-width="6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + def.svg + '</svg>';
  }

  /* ---- the widgets -----------------------------------------------------------
     Sizes as a phone's widgets have them: small sits two to a row, medium and
     large take the row. Each widget offers the sizes that suit it, first the
     one it arrives in. */
  var SIZE_NAMES = { s: 'Small', m: 'Medium', l: 'Large' };
  var WIDGETS = {
    link:     { name: 'Link', blurb: 'A button to anywhere', sizes: ['m', 's'],
                make: function () { return { type: 'wf-link', title: '', url: '' }; } },
    social:   { name: 'Social links', blurb: 'Your other platforms, as icons', sizes: ['m'],
                make: function () { return { type: 'wf-social', links: [] }; } },
    next:     { name: 'Next event', blurb: 'The soonest of your events', sizes: ['m', 's'],
                make: function () { return { type: 'wf-feed', source: 'events', mode: 'next', heading: 'Next event', fields: ['venue'], empty: 'hide', emptyNote: '' }; } },
    rooms:    { name: 'Rooms', blurb: 'Your rooms, and who is talking in them', sizes: ['m', 's', 'l'],
                make: function () { return { type: 'wf-feed', source: 'rooms', mode: 'list', heading: 'Rooms', fields: [], empty: 'hide', emptyNote: '' }; } },
    events:   { name: 'Upcoming events', blurb: 'Your next few events', sizes: ['m', 's', 'l'],
                make: function () { return { type: 'wf-feed', source: 'events', mode: 'list', heading: 'Coming up', fields: ['venue'], empty: 'hide', emptyNote: '' }; } },
    posts:    { name: 'Latest publications', blurb: 'Your newest posts', sizes: ['m', 's', 'l'],
                make: function () { return { type: 'wf-feed', source: 'publications', mode: 'list', heading: 'Latest', fields: ['link'], empty: 'hide', emptyNote: '' }; } },
    listings: { name: 'For sale', blurb: 'What you have to sell, and what it costs', sizes: ['m', 's', 'l'],
                make: function () { return { type: 'wf-feed', source: 'listings', mode: 'list', heading: 'For sale', fields: ['price'], empty: 'hide', emptyNote: '' }; } },
    fund:     { name: 'The fund', blurb: 'What has been given, and what is proposed', sizes: ['m', 's', 'l'],
                make: function () { return { type: 'wf-feed', source: 'funding', mode: 'list', heading: 'The fund', fields: ['source'], empty: 'hide', emptyNote: '' }; } },
    location: { name: 'Location', blurb: 'Where you are, as precisely as you choose', sizes: ['s', 'm'],
                make: function () { return { type: 'wf-location', heading: 'Where we are', precision: DEFAULT_PRECISION, label: '' }; } },
    text:     { name: 'Text', blurb: 'Words of your own', sizes: ['m', 's', 'l'],
                make: function () { return { type: 'wf-text', text: '' }; } }
  };
  var TRAY = ['rooms', 'posts', 'listings', 'events', 'fund', 'next', 'link', 'social', 'location', 'text'];
  var FEED_KEYS = { rooms: 'rooms', publications: 'posts', listings: 'listings', events: 'events', funding: 'fund' };
  function keyOf(w) {
    if (w.type === 'wf-feed') return w.mode === 'next' ? 'next' : FEED_KEYS[w.source];
    return { 'wf-link': 'link', 'wf-social': 'social', 'wf-location': 'location', 'wf-text': 'text' }[w.type];
  }
  function create(key, size) {
    var w = WIDGETS[key].make();
    w.size = size || WIDGETS[key].sizes[0];
    return w;
  }
  function copy(w) { return JSON.parse(JSON.stringify(w)); }
  function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }
  function turn(deg) { deg = ((deg + 180) % 360 + 360) % 360 - 180; return Math.round(deg); }

  /* ---- the ICD ---------------------------------------------------------------- */
  /* THE DOCUMENT CHANGED SHAPE (25 Sep 2026), and the old reader did not notice.
     It used to be AsyncAPI-shaped: every op once under `components.messages`, with
     `channels.<kind>.messages` saying which kind could carry it, and each op's
     fields under `payload.properties` with a `payload.required` list beside them.
     All of that is gone. An op is declared once where it belongs — `kinds.<k>.ops`
     for a kind's own, `facets.<f>.ops` for the ones several kinds share, with
     `facets.<f>.on` listing which — and its fields are `args`, each carrying its
     own `required`.

     THE OLD READER FAILED SILENTLY, which is the part worth keeping in mind. It
     did not throw: `doc.components` and `doc.channels` are simply absent now, so
     every lookup returned undefined, `fieldsOf` produced an empty list, and the
     editor would have offered no fields at all while reporting no fault. A reader
     that cannot tell "no fields" from "no document" is the same defect as a
     keychain that cannot tell "no key" from "no answer".

     So: flatten both tables once, into the only two questions this editor asks —
     what are this op's fields, and may this kind carry it — and refuse loudly if
     the document has neither table or is missing an op a widget needs. */
  var icd = null, icdOn = null, icdTrouble = '';
  function indexIcd(doc) {
    var ops = {}, on = {};
    function add(name, spec, kinds) {
      ops[name] = spec;
      kinds.forEach(function (k) { (on[k] = on[k] || {})[name] = true; });
    }
    Object.keys(doc.kinds || {}).forEach(function (k) {
      var o = (doc.kinds[k] || {}).ops || {};
      Object.keys(o).forEach(function (name) { add(name, o[name], [k]); });
    });
    Object.keys(doc.facets || {}).forEach(function (f) {
      var spec = doc.facets[f] || {}, o = spec.ops || {};
      Object.keys(o).forEach(function (name) { add(name, o[name], spec.on || []); });
    });
    return { ops: ops, on: on };
  }
  function loadIcd() {
    return fetch(ICD_URL).then(function (r) {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    }).then(function (doc) {
      if (!doc.kinds && !doc.facets) {
        throw new Error('no kinds and no facets — this is not the ICD this editor reads');
      }
      var ix = indexIcd(doc);
      icd = ix.ops;
      icdOn = ix.on;
      var need = Object.keys(FEEDS).reduce(function (a, k) { return a.concat(FEEDS[k].ops); }, []).concat(MEMBER_OPS);
      var gone = need.filter(function (op) { return !icd[op]; });
      if (gone.length) throw new Error('not in the ICD: ' + gone.join(', '));
    }).catch(function (e) {
      icdTrouble = 'Could not read the ICD (' + (e && e.message || e) + ') — the fields a widget can show cannot be listed.';
      if (global.console) console.error('face: ' + icdTrouble);
    });
  }
  function fieldsOf(feed) {
    var hide = feed.hide || [], seen = {}, out = [];
    feed.ops.forEach(function (op) {
      var args = (icd && icd[op] && icd[op].args) || {};
      Object.keys(args).forEach(function (k) {
        if (hide.indexOf(k) >= 0 || seen[k]) return;
        seen[k] = true;
        out.push({ key: k, op: op, required: !!args[k].required });
      });
    });
    return out;
  }
  // startMs → "Start". For the settings only; the ICD's own key is the control's title.
  function fieldLabel(key) {
    var bare = key.replace(/(Ms|Cents)$/, '');
    return bare.charAt(0).toUpperCase() + bare.slice(1).replace(/([A-Z])/g, ' $1').toLowerCase();
  }

  /* ---- state ------------------------------------------------------------------ */
  var state = {
    site: { name: '', purpose: '', slug: '', kind: '' },
    look: copy(PRESETS[0].look),
    header: { layout: 'classic', title: 'name', logo: '', banner: '' },
    show: { members: true, address: true, purpose: true, share: true },
    door: { access: 'request', label: '', questions: [] },
    notice: { on: false, kind: 'sensitive', text: '' },
    sharing: { listed: true },
    avatar: '',
    // one of each live thing, and no content: the face fills itself once the site is in use
    widgets: [create('next', 'm'), create('location', 's'), create('text', 's'), create('posts', 'm'), create('social', 'm')],
    stickers: [],
    // Where the editor is: the section open in the panel (null: the menu), and what
    // is picked within it — a placed 'widget', a 'draft' being configured, a 'sticker'.
    section: 'theme',
    pick: null,
    selected: -1,
    draft: null,
    visited: { theme: true },   // the sections opened so far, for the ticks on the basics
    fontFor: 'text',
    custom: null          // the last look that was not a theme, for the Custom tile
  };
  var root, panel, stage, phone, backdrop, screen, canvas, profile, grid, layer, door, sheet, notice, inspector, tools;

  /* ---- building blocks -------------------------------------------------------- */
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function button(cls, text) { var b = el('button', cls, text); b.type = 'button'; return b; }
  function initials(name) {
    return (name || '').trim().split(/\s+/).slice(0, 2).map(function (w) { return (w[0] || '').toUpperCase(); }).join('') || '·';
  }

  /* ---- a widget, as it looks before the site has anything to show --------------
     The same shapes the published face starts from; the Arc's feed fills them. */
  function skeleton(rows, compact) {
    var box = el('div', 'wf-items');
    for (var i = 0; i < rows; i++) {
      var r = el('div', 'wf-skel-row' + (compact ? ' wf-compact' : ''));
      r.appendChild(el('span', 'wf-skel wf-skel-lead'));
      r.appendChild(el('span', 'wf-skel wf-skel-main'));
      if (!compact) r.appendChild(el('span', 'wf-skel wf-skel-meta'));
      box.appendChild(r);
    }
    return box;
  }
  function widgetEl(w) { return paint(shapeOf(w), w); }
  function shapeOf(w) {
    var size = w.size || 'm', n, box;
    if (w.type === 'wf-link') return el('a', 'wf-w wf-link' + (w.url ? '' : ' wf-unset'), w.title || 'Link');
    if (w.type === 'wf-text') {
      n = el('div', 'wf-w wf-textcard' + (w.text ? '' : ' wf-unset'));
      n.appendChild(el('p', '', w.text || 'Your words'));
      return n;
    }
    if (w.type === 'wf-social') {
      n = el('div', 'wf-w wf-social');
      var links = (w.links || []).filter(function (l) { return linkUrl(l); });
      if (!links.length) for (var i = 0; i < 4; i++) n.appendChild(el('span', 'wf-glyph wf-skel'));
      links.forEach(function (l) { n.appendChild(glyph('wf-glyph', l.platform)); });
      return n;
    }
    n = el('div', 'wf-w wf-' + w.type.slice(3));
    if (w.heading) n.appendChild(el('div', 'wf-head', w.heading));
    if (w.type === 'wf-feed') {
      n.appendChild(skeleton(w.mode === 'next' ? 1 : (size === 'l' ? 4 : 2), size === 's'));
    } else if (w.type === 'wf-location') {
      box = el('div', 'wf-place');
      box.appendChild(el('span', 'wf-pin'));
      var words = el('div', 'wf-place-words');
      words.appendChild(w.label ? el('span', 'wf-place-label', w.label) : el('span', 'wf-skel wf-skel-main'));
      words.appendChild(el('span', 'wf-place-scale', 'Within about ' + cellSize(w.precision || DEFAULT_PRECISION)));
      box.appendChild(words);
      n.appendChild(box);
    }
    return n;
  }
  function doorEl() {
    var d = button('wf-btn wf-door');
    var icon = el('span', 'wf-door-icon');
    icon.innerHTML = DOOR_SVG;
    d.appendChild(icon);
    d.appendChild(el('span', 'wf-door-label', state.door.label.trim() || accessOf(state.door.access).door));
    return d;
  }
  function stickerEl(s) {
    var def = stickerOf(s.id), n = el('div', 'wf-sticker');
    poseSticker(n, s);
    if (def && def.svg) {
      n.innerHTML = doodleSvg(def);
      n.style.color = INKS[s.colour] || INKS.buttons;
    } else if (def) {
      var img = el('img');
      img.src = VENDOR + def.file; img.alt = ''; img.draggable = false;
      n.appendChild(img);
    }
    return n;
  }
  function poseSticker(n, s) {
    n.style.left = (s.x * 100) + '%';
    n.style.top = 'calc(' + s.y + ' * 100cqw)';
    n.style.width = (s.size * 100) + '%';
    n.style.transform = 'translate(-50%, -50%) rotate(' + s.rot + 'deg)' + (s.flip ? ' scaleX(-1)' : '');
  }
  function noticeEl() {
    var n = noticeOf(state.notice.kind), box = el('div', 'wf-notice'), card = el('div', 'wf-notice-card');
    card.appendChild(el('span', 'wf-notice-icon', '!'));
    card.appendChild(el('p', 'wf-notice-title', n.title));
    var words = n.id === 'custom' ? state.notice.text.trim() : n.text;
    if (words) card.appendChild(el('p', 'wf-notice-text', words));
    card.appendChild(button('wf-btn wf-notice-go', 'Continue'));
    card.appendChild(button('wf-btn wf-notice-back', 'Go back'));
    box.appendChild(card);
    return box;
  }

  /* ---- the header ------------------------------------------------------------------ */
  function avatarEl(cls) {
    if (state.avatar) {
      var img = el('img', cls); img.src = state.avatar; img.alt = '';
      return img;
    }
    return el('div', cls + ' wf-initials', initials(state.site.name));
  }
  function drawProfile() {
    var h = state.header;
    profile.className = 'wf-profile wf-layout-' + h.layout;
    profile.innerHTML = '';
    if (h.layout === 'hero') {
      var hero = el('div', 'wf-hero' + (state.avatar ? '' : ' wf-hero-empty'));
      hero.appendChild(avatarEl('wf-hero-img'));
      profile.appendChild(hero);
    } else {
      if (h.layout === 'banner') {
        var ban = el('div', 'wf-banner');
        if (h.banner) ban.style.backgroundImage = 'url("' + h.banner + '")';
        profile.appendChild(ban);
      }
      profile.appendChild(avatarEl('wf-avatar'));
    }
    if (h.title === 'logo' && h.logo) {
      var logo = el('img', 'wf-logo'); logo.src = h.logo; logo.alt = state.site.name || 'Your site';
      profile.appendChild(logo);
    } else {
      profile.appendChild(el('h2', 'wf-name', state.site.name || 'Your site'));
    }
    if (state.show.address && state.site.slug) profile.appendChild(el('p', 'wf-handle', 'wallflowers.io/' + state.site.slug));
    if (state.show.members) {
      var count = el('p', 'wf-members');
      count.appendChild(el('b', 'wf-skel-num', '—'));
      count.appendChild(document.createTextNode(' members'));
      profile.appendChild(count);
    }
    if (state.show.purpose && state.site.purpose) profile.appendChild(el('p', 'wf-bio', state.site.purpose));
    if (state.show.share) {
      var share = button('wf-btn wf-sharebtn');
      share.setAttribute('aria-label', 'Share');
      share.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 15V3.5M8 7.5l4-4 4 4"/><path d="M6.5 11H6a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-.5"/></svg>';
      profile.appendChild(share);
    }
  }

  /* ---- the phone ---------------------------------------------------------------- */
  function drawPhone() {
    applyLook(phone);
    drawProfile();

    grid.innerHTML = '';
    state.widgets.forEach(function (w, i) {
      var item = el('div', 'face-item wf-size-' + (w.size || 'm') + (state.pick === 'widget' && i === state.selected ? ' on' : ''));
      item.draggable = true;
      item.tabIndex = 0;
      item.setAttribute('role', 'button');
      item.setAttribute('aria-label', WIDGETS[keyOf(w)].name + ', ' + SIZE_NAMES[w.size || 'm'].toLowerCase() + '. Drag to move; Delete removes it.');
      item.appendChild(widgetEl(w));
      var x = button('face-x', '−');
      x.setAttribute('aria-label', 'Remove ' + WIDGETS[keyOf(w)].name);
      x.addEventListener('click', function (e) { e.stopPropagation(); remove(i); });
      item.appendChild(x);
      item.addEventListener('click', function (e) { e.stopPropagation(); focusOn('widget', i); });
      item.addEventListener('keydown', function (e) { keyOnItem(e, i); });
      item.addEventListener('dragstart', function (e) { startMove(e, i, item); });
      item.addEventListener('dragend', endDrag);
      grid.appendChild(item);
    });
    if (!state.widgets.length) grid.appendChild(el('div', 'face-empty', 'Drag widgets here'));

    drawStickers();

    var d = doorEl();
    d.classList.toggle('on', state.section === 'door');
    d.setAttribute('aria-label', 'The door: ' + d.textContent + '. Configure it.');
    d.addEventListener('click', function (e) { e.stopPropagation(); openSection('door'); });
    door.replaceWith(d);
    door = d;

    // What a visitor meets on knocking, shown while the Door is open and there is something to ask.
    var sh = state.section === 'door' && state.door.access === 'request' && state.door.questions.length ? sheetEl() : el('span');
    sheet.replaceWith(sh);
    sheet = sh;

    // The notice covers the face while its section is open, so it can be seen as a visitor meets it.
    var n = state.notice.on && state.section === 'notice' ? noticeEl() : el('span');
    notice.replaceWith(n);
    notice = n;

    changed();
  }

  function drawStickers() {
    layer.innerHTML = '';
    state.stickers.forEach(function (s, i) {
      var def = stickerOf(s.id), n = stickerEl(s), on = state.pick === 'sticker' && state.selected === i;
      n.classList.add('face-sticker');
      if (on) n.classList.add('on');
      n.tabIndex = 0;
      n.setAttribute('role', 'button');
      n.setAttribute('aria-label', (def ? def.name : 'Sticker') + '. Drag to move; arrows nudge, [ and ] turn, minus and plus size, Delete removes.');
      var x = button('face-x', '−');
      x.setAttribute('aria-label', 'Remove ' + (def ? def.name : 'sticker'));
      x.addEventListener('click', function (e) { e.stopPropagation(); removeSticker(i); });
      n.appendChild(x);
      if (on) {
        var knob = el('span', 'face-turn');
        knob.title = 'Drag to turn and size — Shift snaps';
        knob.addEventListener('pointerdown', function (e) { turnSticker(e, i, n); });
        n.appendChild(knob);
      }
      n.addEventListener('pointerdown', function (e) {
        if (e.target.closest('.face-x, .face-turn')) return;
        grabSticker(e, i);
      });
      n.addEventListener('click', function (e) { e.stopPropagation(); });
      n.addEventListener('keydown', function (e) { keyOnSticker(e, i); });
      layer.appendChild(n);
    });
  }

  // A section of the panel, opened: from the menu, the foot of another, or the phone.
  function openSection(id) {
    state.section = id;
    state.pick = null; state.selected = -1; state.draft = null;
    if (id) state.visited[id] = true;
    drawPhone(); drawPanel();
  }
  // Something picked, which opens its section: a placed 'widget' (tapped again, it is put
  // down), a 'draft' from the Widgets list, a placed 'sticker'. Nothing: put it down.
  function focusOn(what, i) {
    if (what === 'widget' && state.pick === 'widget' && state.selected === i) what = null;
    if (what) state.section = what === 'sticker' ? 'stickers' : 'widgets';
    state.pick = what || null;
    state.selected = what === 'widget' || what === 'sticker' ? i : -1;
    if (what !== 'draft') state.draft = null;
    if (state.section) state.visited[state.section] = true;
    drawPhone();
    drawPanel();
  }
  function remove(i) {
    state.widgets.splice(i, 1);
    if (state.pick === 'widget') { state.pick = null; state.selected = -1; }
    drawPhone();
    drawPanel();
  }
  function move(from, to) {
    var w = state.widgets.splice(from, 1)[0];
    state.widgets.splice(to, 0, w);
    state.section = 'widgets';
    state.pick = 'widget';
    state.selected = to;
  }
  function place(w, at) {
    state.widgets.splice(at, 0, w);
    state.section = 'widgets';
    state.pick = 'widget';
    state.selected = at;
    state.draft = null;
    drawPhone(); drawPanel();
  }
  // Dragging is not the only way to MOVE: a focused widget moves with Alt+arrows and goes with Delete.
  function keyOnItem(e, i) {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); focusOn('widget', i); return; }
    if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); remove(i); return; }
    if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      var to = Math.max(0, Math.min(state.widgets.length - 1, i + (e.key === 'ArrowUp' ? -1 : 1)));
      if (to === i) return;
      move(i, to);
      drawPhone(); drawPanel();
      grid.children[to].focus();
    }
  }

  /* ---- stickers: put down, moved, turned -------------------------------------------- */
  function addSticker(id, clientX, clientY) {
    var def = stickerOf(id), box = canvas.getBoundingClientRect();
    if (!def) return;
    var s = { id: id, x: clamp((clientX - box.left) / box.width, 0.04, 0.96), y: Math.max(0.04, (clientY - box.top) / box.width),
              size: def.svg ? 0.3 : 0.24, rot: Math.round(Math.random() * 24 - 12), flip: false };
    if (def.svg) s.colour = 'buttons';
    state.stickers.push(s);
    focusOn('sticker', state.stickers.length - 1);
  }
  function removeSticker(i) {
    state.stickers.splice(i, 1);
    if (state.pick === 'sticker') { state.pick = null; state.selected = -1; }
    drawPhone(); drawPanel();
  }
  function grabSticker(e, i) {
    if (e.button !== 0) return;
    e.preventDefault();          // no native image drag, no text selection
    if (!(state.pick === 'sticker' && state.selected === i)) focusOn('sticker', i);
    var s = state.stickers[i], n = layer.children[i], box = canvas.getBoundingClientRect();
    var x0 = e.clientX, y0 = e.clientY, sx = s.x, sy = s.y, moved = false;
    function go(ev) {
      if (!moved && Math.abs(ev.clientX - x0) + Math.abs(ev.clientY - y0) < 3) return;
      moved = true;
      s.x = clamp(sx + (ev.clientX - x0) / box.width, 0, 1);
      s.y = Math.max(0, sy + (ev.clientY - y0) / box.width);
      poseSticker(n, s);
    }
    function stop() {
      global.removeEventListener('pointermove', go);
      global.removeEventListener('pointerup', stop);
      if (moved) { drawPhone(); drawPanel(); }
      if (layer.children[i]) layer.children[i].focus({ preventScroll: true });   // so the keys work on it next
    }
    global.addEventListener('pointermove', go);
    global.addEventListener('pointerup', stop);
  }
  // The knob at the corner: its angle from the centre turns the sticker, its distance sizes it.
  function turnSticker(e, i, n) {
    e.preventDefault(); e.stopPropagation();
    var s = state.stickers[i], r = n.getBoundingClientRect(), cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    var a0 = Math.atan2(e.clientY - cy, e.clientX - cx), d0 = Math.max(8, Math.hypot(e.clientX - cx, e.clientY - cy));
    var rot0 = s.rot, size0 = s.size;
    function go(ev) {
      var rot = rot0 + (Math.atan2(ev.clientY - cy, ev.clientX - cx) - a0) * 180 / Math.PI;
      s.rot = turn(ev.shiftKey ? Math.round(rot / 15) * 15 : rot);
      s.size = clamp(size0 * Math.hypot(ev.clientX - cx, ev.clientY - cy) / d0, 0.06, 0.95);
      poseSticker(n, s);
    }
    function stop() {
      global.removeEventListener('pointermove', go);
      global.removeEventListener('pointerup', stop);
      drawPhone(); drawPanel();
    }
    global.addEventListener('pointermove', go);
    global.addEventListener('pointerup', stop);
  }
  function keyOnSticker(e, i) {
    var s = state.stickers[i], step = e.shiftKey ? 0.05 : 0.01, k = e.key;
    if (k === 'Delete' || k === 'Backspace') { e.preventDefault(); removeSticker(i); return; }
    if (k === 'Enter' || k === ' ') { e.preventDefault(); focusOn('sticker', i); return; }
    if (k === 'ArrowLeft') s.x = clamp(s.x - step, 0, 1);
    else if (k === 'ArrowRight') s.x = clamp(s.x + step, 0, 1);
    else if (k === 'ArrowUp') s.y = Math.max(0, s.y - step);
    else if (k === 'ArrowDown') s.y += step;
    else if (k === '[' || k === ']') s.rot = turn(s.rot + (k === '[' ? -5 : 5));
    else if (k === '-' || k === '=' || k === '+') s.size = clamp(s.size + (k === '-' ? -0.02 : 0.02), 0.06, 0.95);
    else return;
    e.preventDefault();
    drawPhone(); drawPanel();
    if (layer.children[i]) layer.children[i].focus({ preventScroll: true });
  }

  /* ---- drag and drop ------------------------------------------------------------
     A drag carries a NEW widget — from a tray tile, or the configured one in the
     configure window — or MOVES one already placed, or brings a STICKER, which
     lands exactly where it is let go. While a widget is in the air the moved one
     steps out of the grid, and a gap the size of what is carried shows where it
     will land. */
  var drag = null, gap = null;
  function startMove(e, i, item) {
    drag = { from: i, size: state.widgets[i].size || 'm' };
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', 'wf-move:' + i);
    setTimeout(function () { item.classList.add('lifted'); phone.classList.add('dragging'); }, 0);
  }
  function startNew(e, widget) {
    drag = { widget: widget, size: widget.size || 'm' };
    e.dataTransfer.effectAllowed = 'copy';
    e.dataTransfer.setData('text/plain', 'wf-new:' + widget.type);
    phone.classList.add('dragging');
  }
  function placed() {   // the grid's widgets, without the one in the air or the gap
    return [].slice.call(grid.children).filter(function (c) {
      return c.classList.contains('face-item') && !c.classList.contains('lifted');
    });
  }
  function landingIndex(x, y) {
    if (gap) {
      var g = gap.getBoundingClientRect();
      if (x >= g.left && x <= g.right && y >= g.top && y <= g.bottom) return Number(gap.dataset.at);
    }
    var items = placed();
    for (var i = 0; i < items.length; i++) {
      var r = items[i].getBoundingClientRect();
      if (y < r.top) return i;
      if (y <= r.bottom && x < r.left + r.width / 2) return i;
    }
    return items.length;
  }
  function showGap(at) {
    if (gap && Number(gap.dataset.at) === at) return;
    if (!gap) gap = el('div', 'face-gap');
    gap.className = 'face-gap wf-size-' + drag.size;
    gap.dataset.at = at;
    var items = placed();
    grid.insertBefore(gap, items[at] || null);
    var empty = grid.querySelector('.face-empty');
    if (empty) empty.remove();
  }
  function endDrag() {
    drag = null;
    if (gap) { gap.remove(); gap = null; }
    phone.classList.remove('dragging');
    [].forEach.call(grid.querySelectorAll('.lifted'), function (n) { n.classList.remove('lifted'); });
  }
  function wireDrop(target) {
    target.addEventListener('dragover', function (e) {
      if (!drag) return;
      e.preventDefault();
      if (drag.sticker) { e.dataTransfer.dropEffect = 'copy'; return; }
      e.dataTransfer.dropEffect = drag.widget ? 'copy' : 'move';
      showGap(landingIndex(e.clientX, e.clientY));
    });
    target.addEventListener('drop', function (e) {
      if (!drag) return;
      e.preventDefault();
      var at = gap ? Number(gap.dataset.at) : state.widgets.length;
      var d = drag;
      endDrag();
      if (d.sticker) { addSticker(d.sticker, e.clientX, e.clientY); return; }
      if (d.widget) { place(copy(d.widget), at); return; }
      move(d.from, at);   // `at` counts the grid without the widget in the air
      drawPhone(); drawPanel();
    });
  }

  /* ---- undo ------------------------------------------------------------------------
     Everything the face is, as one snapshot, taken once edits pause — so a word
     typed is one step, not one per letter. ⌘Z / Ctrl+Z, and Shift for redo,
     anywhere in the editor but a text field, which keeps its own. */
  var past = [], future = [], lastSnap = '', snapTimer = null, restoring = false;
  function snapshot() {
    return JSON.stringify({ look: state.look, header: state.header, show: state.show, door: state.door, notice: state.notice,
      sharing: state.sharing, avatar: state.avatar, widgets: state.widgets, stickers: state.stickers });
  }
  function changed() {
    if (!presetNow()) state.custom = copy(state.look);
    if (restoring || !lastSnap || snapshot() === lastSnap) return;    // a redraw that changed nothing
    clearTimeout(snapTimer);
    snapTimer = setTimeout(commit, 400);
    if (tools) tools.undo.disabled = false;
  }
  function commit() {
    clearTimeout(snapTimer);
    var s = snapshot();
    if (s !== lastSnap) {
      past.push(lastSnap);
      if (past.length > 100) past.shift();
      lastSnap = s;
      future = [];
    }
    toolState();
  }
  function restore(json) {
    var s = JSON.parse(json);
    Object.keys(s).forEach(function (k) { state[k] = s[k]; });
    if ((state.pick === 'widget' && !state.widgets[state.selected]) || (state.pick === 'sticker' && !state.stickers[state.selected])) {
      state.pick = null; state.selected = -1;
    }
    restoring = true;
    drawPhone(); drawPanel();
    restoring = false;
    toolState();
  }
  function undo() {
    commit();
    if (!past.length) return;
    future.push(lastSnap);
    lastSnap = past.pop();
    restore(lastSnap);
  }
  function redo() {
    commit();
    if (!future.length) return;
    past.push(lastSnap);
    lastSnap = future.pop();
    restore(lastSnap);
  }
  function toolState() {
    if (!tools) return;
    tools.undo.disabled = !past.length;
    tools.redo.disabled = !future.length;
  }

  /* ---- controls ------------------------------------------------------------------
     Drawn rather than left to the browser, so a switch looks like a switch on
     every platform: each keeps its native role (switch, radio, checkbox) so a
     screen reader and the keyboard treat it as the real thing. */
  function switchEl(label, on, flip, hint) {
    var b = button('fswitch');
    b.setAttribute('role', 'switch');
    b.setAttribute('aria-checked', String(!!on));
    var words = el('span', 'fswitch-words');
    words.appendChild(el('span', 'fswitch-label', label));
    if (hint) words.appendChild(el('span', 'fswitch-hint', hint));
    b.appendChild(words);
    b.appendChild(el('span', 'fswitch-track'));
    b.addEventListener('click', flip);
    return b;
  }
  function radios(options, value, pick) {    // options: [[value, label, hint?], …]
    var g = el('div', 'fradios');
    g.setAttribute('role', 'radiogroup');
    options.forEach(function (o) {
      var b = button('fradio');
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-checked', String(o[0] === value));
      b.appendChild(el('span', 'fradio-dot'));
      var words = el('span', 'fradio-words');
      words.appendChild(el('span', '', o[1]));
      if (o[2]) words.appendChild(el('span', 'fradio-hint', o[2]));
      b.appendChild(words);
      b.addEventListener('click', function () { pick(o[0]); });
      g.appendChild(b);
    });
    return g;
  }
  function checkbox(label, on, locked, title, flip) {
    var b = button('fcheckbox');
    b.setAttribute('role', 'checkbox');
    b.setAttribute('aria-checked', String(!!on));
    if (locked) { b.setAttribute('aria-disabled', 'true'); b.classList.add('locked'); }
    if (title) b.title = title;
    b.appendChild(el('span', 'fcheckbox-box'));
    b.appendChild(el('span', '', label));
    if (!locked) b.addEventListener('click', flip);
    return b;
  }
  function seg(options, value, pick, names) {
    var s = el('div', 'fseg');
    s.setAttribute('role', 'radiogroup');
    options.forEach(function (o) {
      var b = button('', names ? names[o] : o);
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-checked', String(o === value));
      b.addEventListener('click', function () { pick(o); });
      s.appendChild(b);
    });
    return s;
  }
  // Choices shown as pictures of themselves: a layout, a wallpaper, a button style.
  function pictures(options, value, pick, draw, cls) {    // options: [[value, label], …]; draw(value, pic)
    var g = el('div', 'fpics ' + (cls || ''));
    g.setAttribute('role', 'radiogroup');
    options.forEach(function (o) {
      var b = button('fpic');
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-checked', String(o[0] === value));
      var pic = el('span', 'fpic-pic');
      draw(o[0], pic);
      b.appendChild(pic);
      b.appendChild(el('span', 'fpic-name', o[1]));
      b.addEventListener('click', function () { pick(o[0]); });
      g.appendChild(b);
    });
    return g;
  }
  // Sizes as pictures of themselves: a square, a wide bar, a tall block.
  function sizes(options, value, pick) {
    var g = el('div', 'fsizes');
    g.setAttribute('role', 'radiogroup');
    // shown small to large, whichever a widget arrives in
    ['s', 'm', 'l'].filter(function (o) { return options.indexOf(o) >= 0; }).forEach(function (o) {
      var b = button('fsize fsize-' + o);
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-checked', String(o === value));
      b.appendChild(el('span', 'fsize-shape'));
      b.appendChild(el('span', 'fsize-name', SIZE_NAMES[o]));
      b.addEventListener('click', function () { pick(o); });
      g.appendChild(b);
    });
    return g;
  }
  function range(min, max, value, onInput, label) {
    var r = el('input', 'frange');
    r.type = 'range'; r.min = min; r.max = max; r.value = value;
    if (label) r.setAttribute('aria-label', label);
    r.addEventListener('input', function () { onInput(Number(r.value)); });
    return r;
  }
  function field(label, control) {
    var f = el('div', 'ffield');
    f.appendChild(el('span', 'flabel', label));
    f.appendChild(control);
    return f;
  }
  // A text field. With `max`, it counts what is left, as a post does.
  function input(value, placeholder, onInput, multiline, max) {
    var i = el(multiline ? 'textarea' : 'input', 'finput');
    if (!multiline) i.type = 'text';
    i.value = value || '';
    i.placeholder = placeholder || '';
    if (multiline) {
      i.rows = 3;
      i.addEventListener('input', function () { fit(i); });
      setTimeout(function () { fit(i); }, 0);     // once it is in the page, and has a width to wrap in
    }
    if (!max) {
      i.addEventListener('input', function () { onInput(i.value); drawPhone(); refreshPreview(); });
      return i;
    }
    i.maxLength = max;
    var box = el('div', 'fcounted'), n = el('span', 'fcount');
    var count = function () { n.textContent = i.value.length + '/' + max; };
    i.addEventListener('input', function () { onInput(i.value); count(); drawPhone(); refreshPreview(); });
    count();
    box.appendChild(i);
    box.appendChild(n);
    return box;
  }
  // A text box as tall as its words, up to a point, then it scrolls.
  function fit(t) {
    t.style.height = 'auto';
    var h = t.scrollHeight + 2;
    t.style.height = Math.min(h, 240) + 'px';
    t.style.overflowY = h > 240 ? 'auto' : 'hidden';
  }
  // A colour, as a dot and its hex: the dot opens the picker (Coloris, vendor/), the hex can be typed.
  function colourPill(obj, key, label, after) {
    var box = el('label', 'fcolour');
    var dot = el('span', 'fcolour-dot');
    dot.style.background = obj[key];
    var inp = el('input', 'fcolour-input');
    inp.type = 'text'; inp.value = obj[key]; inp.spellcheck = false; inp.maxLength = 7;
    inp.setAttribute('aria-label', label);
    inp.addEventListener('input', function () {
      var v = inp.value.trim();
      if (v.charAt(0) !== '#') v = '#' + v;
      if (!isHex(v)) return;
      obj[key] = v.toUpperCase();
      dot.style.background = obj[key];
      if (after) after();          // first: it may be what puts the colour where the phone reads it
      drawPhone();
    });
    // settled — the picker closed, or the field was left: put the hex right and redraw what depends on it
    inp.addEventListener('change', function () { inp.value = obj[key]; setTimeout(drawPanel, 0); });
    box.appendChild(dot);
    box.appendChild(inp);
    return box;
  }
  // An image, picked on this device, shrunk here, and kept here (see FACE_SAVED).
  function imagePicker(current, shape, label, set, convert) {
    var row = el('div', 'fmarkrow');
    var pick = el('label', 'fmark fmark-' + shape + (current ? ' has' : ''));
    var file = el('input');
    file.type = 'file'; file.accept = 'image/png,image/jpeg,image/webp';
    file.setAttribute('aria-label', label);
    file.addEventListener('change', function () {
      var f = file.files && file.files[0];
      if (f) convert(f).then(set);
    });
    pick.appendChild(file);
    if (current) { var img = el('img'); img.src = current; img.alt = ''; pick.appendChild(img); }
    else pick.appendChild(el('span', '', 'Add'));
    row.appendChild(pick);
    if (current) {
      var rm = button('fx', 'Remove');
      rm.addEventListener('click', function () { set(''); });
      row.appendChild(rm);
    }
    return row;
  }
  function redraw() { drawPhone(); drawPanel(); }

  /* ---- the panel ----------------------------------------------------------------------
     Everything that is not the phone: the RAIL of sections, always there, the one
     open lit; and the DETAIL of that section beside it, with the sections either
     side at its foot. A widget being configured opens inside Widgets' detail, with
     its own way back. A redraw of the same view keeps its scroll. */
  var preview = null, og = null, panelView = '', pendingFocus = null;
  var BACK_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 5-7 7 7 7"/></svg>';
  var ON_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 5 7 7-7 7"/></svg>';
  function drawPanel() {
    var s = sectionOf(state.section) || SECTIONS[0];
    var w = state.pick === 'draft' ? state.draft && state.draft.widget : state.pick === 'widget' ? state.widgets[state.selected] : null;
    var sub = s.id === 'widgets' && !!w;
    var view = [s.id, state.pick, state.selected, state.draft ? state.draft.key : ''].join(':');
    var old = panel.querySelector('.fpanel-body'), keep = old && view === panelView ? old.scrollTop : 0;
    var oldRail = panel.querySelector('.fnav-list'), railKeep = oldRail ? oldRail.scrollTop : 0;
    panel.innerHTML = '';
    preview = null; og = null;
    var nav = el('nav', 'fnav');
    nav.setAttribute('aria-label', 'Sections of your face');
    drawRail(nav, s);
    var detail = el('section', 'fdetail');
    var top = el('div', 'fpanel-top'), body = el('div', 'fpanel-body'), foot = el('div', 'fpanel-foot');
    detail.appendChild(top);
    detail.appendChild(body);
    detail.appendChild(foot);
    panel.appendChild(nav);
    panel.appendChild(detail);
    inspector = body;
    if (sub) {
      var done = button('fdone', 'Done');
      done.addEventListener('click', function () { focusOn(null); });
      topBar(top, WIDGETS[keyOf(w)].name, function () { focusOn(null); }, done);
      drawWidget(w, state.pick === 'draft');
    } else {
      topBar(top, s.name, null, null);
      s.draw();
      drawFoot(foot, s);
    }
    body.scrollTop = keep;
    panel.querySelector('.fnav-list').scrollTop = railKeep;
    panelView = view;
    if (pendingFocus) {
      var f = panel.querySelector(pendingFocus);
      if (f) f.focus();
      pendingFocus = null;
    }
  }
  // The rail: every section in the menu's order, the open one lit; the basics
  // numbered, ticked once opened. Shuffle sits at its foot, whatever is open.
  function drawRail(nav, open) {
    var list = el('div', 'fnav-list');
    GROUPS.forEach(function (g) {
      list.appendChild(el('span', 'fnav-group', g[1]));
      SECTIONS.filter(function (x) { return x.group === g[0]; }).forEach(function (x, i) {
        var on = x === open, v = x.value(), b = button('fnav-item' + (on ? ' on' : ''));
        if (on) b.setAttribute('aria-current', 'page');
        b.title = x.name + (typeof v === 'string' ? ': ' + v : '');
        b.appendChild(iconEl(x.id));
        b.appendChild(el('span', 'fnav-name', x.name));
        if (g[0] === 'basics') {
          var seen = !!state.visited[x.id], step = el('span', 'fnav-step' + (seen ? ' done' : ''), seen ? '' : String(i + 1));
          step.setAttribute('aria-hidden', 'true');
          b.appendChild(step);
        }
        b.addEventListener('click', function () { openSection(x.id); });
        list.appendChild(b);
      });
    });
    nav.appendChild(list);
    nav.appendChild(shuffleButton());
  }
  // The detail's title, centred: where you are is the rail's to say, so it says only what.
  function topBar(top, title, back, extra) {
    var left = el('div', 'fpanel-side'), right = el('div', 'fpanel-side');
    if (back) {
      var b = button('fback');
      b.setAttribute('aria-label', 'Back');
      b.innerHTML = BACK_SVG;
      b.addEventListener('click', back);
      left.appendChild(b);
    }
    if (extra) right.appendChild(extra);
    top.appendChild(left);
    top.appendChild(el('h3', 'ftitle', title));
    top.appendChild(right);
  }
  // The foot: the section before and the one after, in the rail's order — so the
  // basics lead straight on into the rest.
  function drawFoot(foot, s) {
    var i = SECTIONS.indexOf(s), prev = SECTIONS[i - 1], next = SECTIONS[i + 1];
    if (prev) {
      var p = button('fprev');
      p.innerHTML = BACK_SVG;
      p.appendChild(el('span', '', prev.name));
      p.addEventListener('click', function () { openSection(prev.id); });
      foot.appendChild(p);
    } else {
      foot.appendChild(el('span'));
    }
    if (!next) return;
    var n = button('fnext');
    n.appendChild(el('span', '', next.name));
    n.insertAdjacentHTML('beforeend', ON_SVG);
    n.addEventListener('click', function () { openSection(next.id); });
    foot.appendChild(n);
  }

  // The configure window's own copy of the widget, as the face will show it —
  // and the thing that is dragged onto the phone.
  function drawPreview(w) {
    var box = el('div', 'fpreview');
    box.draggable = true;
    box.setAttribute('aria-label', 'Drag onto the phone to add');
    var face = el('div', 'wf-face fpreview-face');
    applyLook(face);
    var g = el('div', 'wf-grid');
    var item = el('div', 'fpreview-item wf-size-' + (w.size || 'm'));
    item.appendChild(widgetEl(w));
    g.appendChild(item);
    face.appendChild(g);
    box.appendChild(face);
    box.appendChild(el('span', 'fpreview-hint', 'Drag onto the phone'));
    box.addEventListener('dragstart', function (e) { startNew(e, copy(w)); });
    box.addEventListener('dragend', endDrag);
    inspector.appendChild(box);
    preview = { box: box, item: item, widget: w };
  }
  function refreshPreview() {
    if (og) og();
    if (!preview) return;
    preview.item.className = 'fpreview-item wf-size-' + (preview.widget.size || 'm');
    preview.item.innerHTML = '';
    preview.item.appendChild(widgetEl(preview.widget));
  }

  function drawWidget(w, isDraft) {
    var spec = WIDGETS[keyOf(w)];
    if (isDraft) drawPreview(w);
    var again = function () { if (isDraft) drawPanel(); else redraw(); };

    if (spec.sizes.length > 1) {
      inspector.appendChild(field('Size', sizes(spec.sizes, w.size, function (s) { w.size = s; again(); })));
    }
    if (w.type === 'wf-link') {
      inspector.appendChild(field('Title', input(w.title, 'Our website', function (v) { w.title = v; })));
      inspector.appendChild(field('Address', input(w.url, 'https://…', function (v) { w.url = v; })));
    } else if (w.type === 'wf-text') {
      inspector.appendChild(field('Text', input(w.text, 'Your words', function (v) { w.text = v; }, true)));
    } else if (w.type === 'wf-social') {
      drawSocial(w, again);
    } else if (w.type === 'wf-feed') {
      drawFeed(w, again);
    } else if (w.type === 'wf-location') {
      inspector.appendChild(field('Heading', input(w.heading, 'None', function (v) { w.heading = v; })));
      var levels = el('div', 'flevels');
      levels.setAttribute('role', 'radiogroup');
      [3, 4, 5, 6, 7, 8, 9].forEach(function (n) {
        var b = button('');
        b.setAttribute('role', 'radio');
        b.setAttribute('aria-checked', String(n === (w.precision || DEFAULT_PRECISION)));
        b.title = n + ' geohash characters';
        b.appendChild(el('b', '', LEVELS[n]));
        b.appendChild(el('span', '', cellSize(n)));
        b.addEventListener('click', function () { w.precision = n; again(); });
        levels.appendChild(b);
      });
      inspector.appendChild(field('How precisely', levels));
      inspector.appendChild(field('Described as', input(w.label, 'Mill Road, Cambridge', function (v) { w.label = v; })));
    }

    inspector.appendChild(field('Colours', widgetColours(w, again)));

    if (!isDraft) {
      var rm = button('fremove', 'Remove widget');
      rm.addEventListener('click', function () { remove(state.selected); });
      inspector.appendChild(rm);
    }
  }

  // A widget's colours: the look's until set, and back to the look's with Reset.
  function widgetColours(w, again) {
    var box = el('div', 'fcolours'), chip = null;
    var rows = w.type === 'wf-social' ? [['fg', 'Icons']] : [['bg', w.type === 'wf-link' ? 'Button' : 'Card'], ['fg', 'Text']];
    function check() {
      if (!chip) return;
      chip.hidden = contrast(ownColour(w, 'fg'), ownColour(w, 'bg')) >= 4.5 || !w.colours;
    }
    rows.forEach(function (r) {
      var k = r[0], own = !!(w.colours && w.colours[k]), line = el('div', 'fcolour-row'), name = el('span', 'fcolour-name', r[1]);
      if (k === 'fg' && rows.length > 1) {
        chip = button('fcontrast', 'Hard to read · Fix');
        chip.title = 'Swap for the ink or white that reads best';
        chip.addEventListener('click', function () {
          w.colours = w.colours || {};
          w.colours.fg = onColour(ownColour(w, 'bg'));
          again();
        });
        name.appendChild(chip);
      }
      line.appendChild(name);
      var set = el('div', 'fcolour-set'), holder = { v: ownColour(w, k) };
      var pill = colourPill(holder, 'v', r[1], function () {
        w.colours = w.colours || {};
        w.colours[k] = holder.v;
        refreshPreview();
        check();
      });
      if (!own) pill.classList.add('inherited');
      set.appendChild(pill);
      if (own) {
        var reset = button('fcolour-reset', 'Reset');
        reset.title = 'Back to the theme’s';
        reset.addEventListener('click', function () {
          delete w.colours[k];
          if (!w.colours.bg && !w.colours.fg) delete w.colours;
          again();
        });
        set.appendChild(reset);
      } else {
        set.appendChild(el('span', 'fcolour-from', 'Theme'));
      }
      line.appendChild(set);
      box.appendChild(line);
    });
    check();
    return box;
  }

  function drawFeed(w, again) {
    var feed = FEEDS[w.source];
    inspector.appendChild(field('Heading', input(w.heading, feed.label, function (v) { w.heading = v; })));
    if (icdTrouble) {
      inspector.appendChild(el('p', 'ftrouble', icdTrouble));
    } else if (!icd) {
      inspector.appendChild(el('p', 'fnote', 'Reading the ICD…'));
    } else {
      var box = el('div', 'fchecks');
      fieldsOf(feed).forEach(function (f) {
        var on = f.required || w.fields.indexOf(f.key) >= 0;
        box.appendChild(checkbox(fieldLabel(f.key), on, f.required, f.op + ' · ' + f.key, function () {
          w.fields = on ? w.fields.filter(function (k) { return k !== f.key; }) : w.fields.concat([f.key]);
          again();
        }));
      });
      inspector.appendChild(field('Each ' + feed.one + ' shows', box));
    }
    inspector.appendChild(field('When there are none', radios([['hide', 'Hide the widget'], ['note', 'Say so']], w.empty,
      function (v) { w.empty = v; again(); })));
    if (w.empty === 'note') {
      inspector.appendChild(field('Saying', input(w.emptyNote, 'Nothing yet', function (v) { w.emptyNote = v; })));
    }
  }

  // Pick the platforms, then type only the handle: the address is joined on.
  function drawSocial(w, again) {
    var picks = el('div', 'fplatforms');
    PLATFORMS.forEach(function (p) {
      var on = w.links.some(function (l) { return l.platform === p.id; });
      var b = button('fplatform');
      b.setAttribute('aria-pressed', String(on));
      b.setAttribute('aria-label', p.name);
      b.title = p.name;
      b.appendChild(glyph('fplatform-glyph', p.id));
      b.addEventListener('click', function () {
        if (on) w.links = w.links.filter(function (l) { return l.platform !== p.id; });
        else { w.links.push({ platform: p.id, handle: '' }); pendingFocus = '[data-platform="' + p.id + '"]'; }
        again();
      });
      picks.appendChild(b);
    });
    inspector.appendChild(field('Platforms', picks));
    if (!w.links.length) return;
    var rows = el('div', 'fhandles');
    w.links.forEach(function (l) {
      var p = platform(l.platform), row = el('label', 'fhandle');
      row.appendChild(glyph('fhandle-glyph', p.id));
      if (p.prefix) row.appendChild(el('span', 'fhandle-prefix', p.prefix));
      var i = el('input', 'fhandle-input');
      i.type = 'text'; i.value = l.handle || ''; i.placeholder = placeholderOf(p); i.spellcheck = false;
      i.setAttribute('aria-label', p.name);
      i.setAttribute('data-platform', p.id);
      i.addEventListener('input', function () { l.handle = i.value; drawPhone(); refreshPreview(); });
      row.appendChild(i);
      rows.appendChild(row);
    });
    inspector.appendChild(field('Where to find you', rows));
  }

  function drawDoor() {
    inspector.appendChild(field('Who can come in', radios(ACCESS.map(function (a) { return [a.id, a.label, '“' + a.door + '”']; }),
      state.door.access, function (v) { state.door.access = v; redraw(); })));
    inspector.appendChild(field('Called', input(state.door.label, accessOf(state.door.access).door, function (v) { state.door.label = v; })));
    if (state.door.access === 'request') drawQuestions(state.door);
  }

  /* ---- asking to join: what the site asks before it lets someone in ----------------------
     As Discord's apply-to-join has it: a few questions, each a short answer, a
     paragraph, a choice, or something to agree to, and each required or not. The
     answers travel with the request to whoever may let the person in. */
  var QUESTION_KINDS = [['short', 'Short answer'], ['long', 'Paragraph'], ['choice', 'Multiple choice'], ['agree', 'Agreement']];
  var MAX_QUESTIONS = 5, MAX_OPTIONS = 6;
  var ASKING = { short: 'How did you hear about us?', long: 'Tell us a little about yourself',
                 choice: 'Which would you like?', agree: 'I will follow the site’s rules' };
  function kindName(k) { return QUESTION_KINDS.filter(function (x) { return x[0] === k; })[0][1]; }
  function drawQuestions(d) {
    var list = el('div', 'fquestions');
    d.questions.forEach(function (q, i) {
      var card = el('div', 'fquestion'), top = el('div', 'fquestion-top');
      top.appendChild(el('span', 'fquestion-kind', (i + 1) + ' · ' + kindName(q.kind)));
      var req = button('fquestion-req' + (q.required ? ' on' : ''), 'Required');
      req.setAttribute('role', 'switch');
      req.setAttribute('aria-checked', String(!!q.required));
      req.addEventListener('click', function () { q.required = !q.required; redraw(); });
      top.appendChild(req);
      [['up', -1, 'm6 15 6-6 6 6'], ['down', 1, 'm6 9 6 6 6-6']].forEach(function (m) {
        var to = i + m[1], b = button('fquestion-move');
        b.disabled = to < 0 || to >= d.questions.length;
        b.setAttribute('aria-label', 'Move ' + m[0]);
        b.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="' + m[2] + '"/></svg>';
        b.addEventListener('click', function () { d.questions.splice(to, 0, d.questions.splice(i, 1)[0]); redraw(); });
        top.appendChild(b);
      });
      var rm = button('fx', '×');
      rm.setAttribute('aria-label', 'Remove this question');
      rm.addEventListener('click', function () { d.questions.splice(i, 1); redraw(); });
      top.appendChild(rm);
      card.appendChild(top);
      card.appendChild(input(q.prompt, ASKING[q.kind], function (v) { q.prompt = v; }, q.kind !== 'short', q.kind === 'agree' ? 200 : 120));
      if (q.kind === 'choice') {
        var opts = el('div', 'foptions');
        q.options.forEach(function (o, j) {
          var row = el('div', 'foption');
          row.appendChild(el('span', 'foption-dot'));
          row.appendChild(input(o, 'Option ' + (j + 1), function (v) { q.options[j] = v; }));
          if (q.options.length > 2) {
            var x = button('fx', '×');
            x.setAttribute('aria-label', 'Remove option ' + (j + 1));
            x.addEventListener('click', function () { q.options.splice(j, 1); redraw(); });
            row.appendChild(x);
          }
          opts.appendChild(row);
        });
        if (q.options.length < MAX_OPTIONS) {
          var more = button('fadd', 'Add option');
          more.addEventListener('click', function () { q.options.push(''); pendingFocus = '.fquestion:nth-child(' + (i + 1) + ') .foption:last-of-type .finput'; redraw(); });
          opts.appendChild(more);
        }
        card.appendChild(opts);
      }
      list.appendChild(card);
    });
    if (d.questions.length < MAX_QUESTIONS) {
      var adds = el('div', 'fquestion-adds');
      QUESTION_KINDS.forEach(function (k) {
        var b = button('fadd', '+ ' + k[1]);
        b.addEventListener('click', function () {
          d.questions.push({ kind: k[0], prompt: '', required: true, options: k[0] === 'choice' ? ['', ''] : [] });
          pendingFocus = '.fquestion:nth-child(' + d.questions.length + ') .finput';
          redraw();
        });
        adds.appendChild(b);
      });
      list.appendChild(adds);
    }
    inspector.appendChild(field('Questions · ' + d.questions.length + ' of ' + MAX_QUESTIONS, list));
  }
  // The sheet a visitor sees on knocking: the questions, as they will answer them.
  function sheetEl() {
    var d = state.door, box = el('div', 'wf-sheet'), card = el('div', 'wf-sheet-card');
    card.appendChild(el('span', 'wf-sheet-grab'));
    var head = el('div', 'wf-sheet-head'), icon = el('span', 'wf-sheet-icon'), words = el('div');
    icon.innerHTML = DOOR_SVG;
    head.appendChild(icon);
    words.appendChild(el('p', 'wf-sheet-title', d.label.trim() || accessOf(d.access).door));
    words.appendChild(el('p', 'wf-sheet-sub', state.site.name || 'Your site'));
    head.appendChild(words);
    card.appendChild(head);
    d.questions.forEach(function (q) {
      var f = el('div', 'wf-q');
      if (q.kind === 'agree') {
        var agree = el('p', 'wf-q-agree', q.prompt.trim() || ASKING.agree);
        if (q.required) agree.appendChild(el('i', '', ' *'));
        f.appendChild(agree);
      } else {
        var label = el('p', 'wf-q-label', q.prompt.trim() || ASKING[q.kind]);
        if (q.required) label.appendChild(el('i', '', ' *'));
        f.appendChild(label);
        if (q.kind === 'choice') {
          q.options.forEach(function (o, j) { f.appendChild(el('span', 'wf-q-option', o.trim() || 'Option ' + (j + 1))); });
        } else {
          f.appendChild(el('span', 'wf-q-field' + (q.kind === 'long' ? ' wf-q-long' : '')));
        }
      }
      card.appendChild(f);
    });
    card.appendChild(button('wf-btn wf-sheet-go', 'Send request'));
    box.appendChild(card);
    return box;
  }

  // The sticker picked on the phone, above the sheet it came from.
  function stickerCard(s) {
    var def = stickerOf(s.id), card = el('div', 'fsticker'), top = el('div', 'fsticker-top');
    top.appendChild(el('b', 'fsticker-name', def ? def.name : 'Sticker'));
    var rm = button('fsticker-rm', 'Remove');
    rm.addEventListener('click', function () { removeSticker(state.selected); });
    top.appendChild(rm);
    var done = button('fdone', 'Done');
    done.addEventListener('click', function () { focusOn(null); });
    top.appendChild(done);
    card.appendChild(top);
    var two = el('div', 'fsticker-grid');
    two.appendChild(field('Size', range(6, 95, Math.round(s.size * 100), function (v) { s.size = v / 100; drawPhone(); }, 'Size')));
    two.appendChild(field('Turn', range(-180, 180, s.rot, function (v) { s.rot = v; drawPhone(); }, 'Turn')));
    card.appendChild(two);
    card.appendChild(switchEl('Mirror', !!s.flip, function () { s.flip = !s.flip; redraw(); }));
    if (def && def.svg) {
      card.appendChild(field('Colour', seg(['buttons', 'text', 'title'], s.colour || 'buttons',
        function (v) { s.colour = v; redraw(); }, { buttons: 'Buttons', text: 'Text', title: 'Title' })));
    }
    return card;
  }

  /* ---- the sections ----------------------------------------------------------------------
     Basics first, numbered; then what makes the face its own; then what a visitor
     meets before it. Each value is what the rail says of it when pointed at. */
  var GROUPS = [['basics', 'Basics'], ['style', 'Make it yours'], ['visitors', 'Before they arrive']];
  var LAYOUTS = { classic: 'Classic', hero: 'Hero', banner: 'Banner' };
  var SECTIONS = [
    { id: 'theme', group: 'basics', name: 'Theme', draw: drawThemes,
      value: function () { var p = presetNow(); return p ? p.name : 'Custom'; } },
    { id: 'header', group: 'basics', name: 'Header', draw: drawHeader,
      value: function () { return LAYOUTS[state.header.layout]; } },
    { id: 'widgets', group: 'basics', name: 'Widgets', draw: drawWidgets,
      value: function () { var n = state.widgets.length; return n + (n === 1 ? ' widget' : ' widgets'); } },
    { id: 'door', group: 'basics', name: 'Door', draw: drawDoor,
      value: function () { return state.door.label.trim() || accessOf(state.door.access).door; } },
    { id: 'wallpaper', group: 'style', name: 'Wallpaper', draw: drawWallpaper,
      value: function () { return WALLPAPERS.filter(function (o) { return o[0] === state.look.wallpaper.style; })[0][1]; } },
    { id: 'blocks', group: 'style', name: 'Cards & buttons', draw: drawBlocks,
      value: function () { return STYLES.filter(function (o) { return o[0] === state.look.blocks.style; })[0][1]; } },
    { id: 'text', group: 'style', name: 'Text', draw: drawText,
      value: function () { return fontOf(state.look.text.font).name; } },
    { id: 'colours', group: 'style', name: 'Colours', draw: drawColours,
      value: function () { return ['background', 'buttons', 'text'].map(function (k) { return state.look.colours[k]; }).join(' · '); } },
    { id: 'stickers', group: 'style', name: 'Stickers', draw: drawStickerSheet,
      value: function () { var n = state.stickers.length; return n ? n + ' on' : 'None yet'; } },
    { id: 'notice', group: 'visitors', name: 'Content notice', draw: drawNotice,
      value: function () { return state.notice.on ? noticeOf(state.notice.kind).label : 'Off'; } },
    { id: 'sharing', group: 'visitors', name: 'Search & sharing', draw: drawSharing,
      value: function () { return state.sharing.listed ? 'Listed' : 'Unlisted'; } }
  ];
  function sectionOf(id) { return SECTIONS.filter(function (s) { return s.id === id; })[0] || null; }
  var ICONS = {
    theme: '<rect x="3.5" y="5.5" width="8" height="13" rx="2.2"/><path d="M14.5 6.2 18.8 7.4a2 2 0 0 1 1.4 2.4l-2.5 9.3a2 2 0 0 1-2.4 1.4l-1.8-.5"/><path d="M6 15.5h3"/>',
    header: '<rect x="4" y="4" width="16" height="16" rx="4"/><circle cx="12" cy="10" r="2.6"/><path d="M7.5 17.2c1.2-2 2.7-2.9 4.5-2.9s3.3.9 4.5 2.9"/>',
    widgets: '<rect x="4" y="4" width="7" height="7" rx="2"/><rect x="13" y="4" width="7" height="7" rx="2"/><rect x="4" y="13" width="16" height="7" rx="2"/>',
    door: '<path d="M6 21V4.5A1.5 1.5 0 0 1 7.5 3h9A1.5 1.5 0 0 1 18 4.5V21M3.5 21h17"/><circle cx="14.6" cy="12.2" r=".8"/>',
    wallpaper: '<rect x="4" y="4" width="16" height="16" rx="4"/><path d="m4.5 16 4.5-4.5 4 4 2.5-2.5 4 4"/><circle cx="15" cy="9" r="1.4"/>',
    blocks: '<rect x="3.5" y="8" width="17" height="8" rx="4"/>',
    text: '<path d="M4 18 8.5 6h1L14 18M5.5 14h6.5M16 18v-6.5m0 2.2c.5-1.4 1.5-2.2 2.9-2.2.4 0 .8.1 1.1.2"/>',
    colours: '<circle cx="12" cy="12" r="8.5"/><circle cx="9" cy="9.5" r="1.2"/><circle cx="14.5" cy="8.5" r="1.2"/><circle cx="16" cy="13" r="1.2"/><path d="M11 20.4c-1.8-.9-1.1-3.4.8-3.4"/>',
    stickers: '<path d="M20 12.5V7a3 3 0 0 0-3-3H7a3 3 0 0 0-3 3v10a3 3 0 0 0 3 3h5.5"/><path d="M20 12.5 12.5 20c0-4.1 3.4-7.5 7.5-7.5z"/>',
    notice: '<path d="M12 3.5 21 19H3z"/><path d="M12 10v4M12 16.6v.4"/>',
    sharing: '<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/>'
  };
  function iconEl(name) {
    var s = el('span', 'fnav-icon');
    s.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + ICONS[name] + '</svg>';
    return s;
  }
  function drawThemes() {
    var list = el('div', 'fthemes');
    list.setAttribute('role', 'radiogroup');
    list.setAttribute('aria-label', 'Theme');
    var now = presetNow();
    (state.custom ? [{ id: 'custom', name: 'Custom', look: state.custom }] : []).concat(PRESETS).forEach(function (p) {
      var on = p.id === 'custom' ? !now : !!now && now.id === p.id;
      var b = button('ftheme');
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-checked', String(on));
      var card = el('span', 'ftheme-card');
      applyLook(card, p.look);
      card.appendChild(el('span', 'ftheme-aa', 'Aa'));
      card.appendChild(el('span', 'ftheme-btn'));
      card.appendChild(el('span', 'ftheme-btn'));
      b.appendChild(card);
      b.appendChild(el('span', 'ftheme-name', p.name));
      b.addEventListener('click', function () { state.look = copy(p.look); redraw(); });
      list.appendChild(b);
    });
    inspector.appendChild(list);
  }
  function shuffleButton() {
    var b = button('fpill fshuffle');
    b.title = 'A theme at random — undo brings the last one back';
    b.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="4"/><circle cx="9" cy="9" r="1" fill="currentColor"/><circle cx="15" cy="15" r="1" fill="currentColor"/><circle cx="15" cy="9" r="1" fill="currentColor"/><circle cx="9" cy="15" r="1" fill="currentColor"/></svg><span>Shuffle</span>';
    b.addEventListener('click', shuffleLook);
    return b;
  }
  // A theme at random, and half the time a different title face from the same shelf.
  function shuffleLook() {
    var now = presetNow(), pool = PRESETS.filter(function (p) { return p !== now; });
    var lk = copy(pool[Math.floor(Math.random() * pool.length)].look);
    if (Math.random() < 0.5) {
      var was = lk.text.title || lk.text.font, shelf = fontOf(was).group;
      var others = FONTS.filter(function (f) { return f.group === shelf && f.id !== was; });
      if (others.length) lk.text.title = others[Math.floor(Math.random() * others.length)].id;
    }
    state.look = lk;
    redraw();
  }

  function drawHeader() {
    var h = state.header;
    inspector.appendChild(field('Profile image', imagePicker(state.avatar, 'round', 'Profile image',
      function (url) { state.avatar = url; redraw(); }, toAvatar)));
    inspector.appendChild(field('Layout', pictures([['classic', 'Classic'], ['hero', 'Hero'], ['banner', 'Banner']], h.layout,
      function (v) { h.layout = v; redraw(); },
      function (v, pic) {
        pic.classList.add('fpic-layout', 'fpic-layout-' + v);
        ['a', 'b', 'c'].forEach(function (k) { pic.appendChild(el('i', 'fpl-' + k)); });
      })));
    if (h.layout === 'banner') {
      inspector.appendChild(field('Banner', imagePicker(h.banner, 'wide', 'Banner image',
        function (url) { h.banner = url; redraw(); }, toBanner)));
    }
    inspector.appendChild(field('Title', seg(['name', 'logo'], h.title, function (v) { h.title = v; redraw(); }, { name: 'Name', logo: 'Logo' })));
    if (h.title === 'logo') {
      inspector.appendChild(field('Logo', imagePicker(h.logo, 'logo', 'Logo', function (url) { h.logo = url; redraw(); }, toLogo)));
    }
    var shows = el('div', 'fshows');
    SHOW.forEach(function (x) {
      shows.appendChild(switchEl(x.label, state.show[x.id], function () { state.show[x.id] = !state.show[x.id]; redraw(); }));
    });
    inspector.appendChild(field('Show', shows));
  }

  function drawWallpaper() {
    var lk = state.look, wp = lk.wallpaper;
    inspector.appendChild(field('Style', pictures(WALLPAPERS, wp.style, function (v) { wp.style = v; redraw(); },
      function (v, pic) {
        var trial = copy(lk);
        trial.wallpaper.style = v;
        if (v === 'image' && !wp.image) pic.classList.add('fpic-image');
        else pic.style.background = wallpaperCss(trial);
      }, 'fpics-5')));
    inspector.appendChild(field(wp.style === 'fill' ? 'Colour' : 'Base colour', colourPill(lk.colours, 'background', 'Background colour')));
    if (wp.style === 'gradient') {
      inspector.appendChild(field('Blend to', colourPill(wp, 'colour2', 'Gradient colour')));
      inspector.appendChild(field('Direction', seg(Object.keys(DIRECTIONS), wp.angle, function (v) { wp.angle = v; redraw(); }, DIRECTIONS)));
    } else if (wp.style === 'blur') {
      inspector.appendChild(field('Glow', colourPill(wp, 'colour2', 'First glow')));
      inspector.appendChild(field('Second glow', colourPill(wp, 'colour3', 'Second glow')));
    } else if (wp.style === 'pattern') {
      inspector.appendChild(field('Pattern', pictures(Object.keys(PATTERNS).map(function (k) { return [k, PATTERNS[k].name]; }), wp.pattern,
        function (v) { wp.pattern = v; redraw(); },
        function (v, pic) { var trial = copy(lk); trial.wallpaper.style = 'pattern'; trial.wallpaper.pattern = v; pic.style.background = wallpaperCss(trial); },
        'fpics-3')));
    } else if (wp.style === 'image') {
      inspector.appendChild(field('Image', imagePicker(wp.image, 'wide', 'Wallpaper image', function (url) { wp.image = url; redraw(); }, toWallpaper)));
      if (wp.image) inspector.appendChild(field('Soften', range(0, 90, wp.soften, function (v) { wp.soften = v; drawPhone(); }, 'Soften')));
    }
  }

  function drawBlocks() {
    var lk = state.look, b = lk.blocks;
    inspector.appendChild(field('Style', pictures(STYLES, b.style, function (v) { b.style = v; redraw(); },
      function (v, pic) {
        var trial = copy(lk);
        trial.blocks.style = v;
        applyLook(pic, trial);
        pic.classList.add('fpic-block');
        pic.appendChild(el('i', 'ftheme-btn'));
      })));
    var corners = el('div', 'fcorners');
    corners.setAttribute('role', 'radiogroup');
    CORNERS.forEach(function (c, i) {
      var k = [0, 3.5, 7, 12][i], btn = button('fcorner');
      btn.setAttribute('role', 'radio');
      btn.setAttribute('aria-checked', String(i === b.corners));
      btn.setAttribute('aria-label', c[0]);
      btn.title = c[0];
      btn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 20V' + (6 + k) + (k ? 'Q6 6 ' + (6 + k) + ' 6' : '') + 'H20"/></svg>';
      btn.addEventListener('click', function () { b.corners = i; redraw(); });
      corners.appendChild(btn);
    });
    inspector.appendChild(field('Corners', corners));
    inspector.appendChild(field('Shadow', seg(Object.keys(SHADOWS), b.shadow, function (v) { b.shadow = v; redraw(); }, SHADOWS)));
  }

  function drawText() {
    var t = state.look.text, which = state.fontFor;
    inspector.appendChild(seg(['text', 'title'], which, function (v) { state.fontFor = v; drawPanel(); }, { text: 'Text', title: 'Title' }));
    var current = which === 'title' ? t.title : t.font, wrap = el('div', 'ffonts');
    var pick = function (id) {
      if (which === 'title') t.title = id; else t.font = id;
      redraw();
    };
    if (which === 'title') {
      var same = button('ffont ffont-same', 'Same as text');
      same.setAttribute('role', 'radio');
      same.setAttribute('aria-checked', String(!current));
      same.addEventListener('click', function () { pick(''); });
      wrap.appendChild(same);
    }
    FONT_GROUPS.forEach(function (g) {
      var fonts = FONTS.filter(function (f) { return f.group === g; });
      if (!fonts.length) return;
      wrap.appendChild(el('p', 'ffonts-group', g));
      var box = el('div', 'ffonts-grid');
      box.setAttribute('role', 'radiogroup');
      box.setAttribute('aria-label', g);
      fonts.forEach(function (f) {
        var b = button('ffont', f.name);
        b.style.fontFamily = f.stack;
        b.setAttribute('role', 'radio');
        b.setAttribute('aria-checked', String(f.id === current));
        b.addEventListener('click', function () { pick(f.id); });
        box.appendChild(b);
      });
      wrap.appendChild(box);
    });
    inspector.appendChild(wrap);
  }

  // Each pairing a visitor has to read, and the least contrast it may have.
  function readable(lk) {
    var c = lk.colours, style = lk.blocks.style, out = [
      { fg: 'text', bg: c.background, need: 4.5 },
      { fg: 'title', bg: c.background, need: 3 }
    ];
    if (style === 'solid') out.push({ fg: 'buttonText', bg: c.buttons, need: 4.5 });
    if (style === 'outline') out.push({ fg: 'buttons', bg: c.background, need: 3 });
    return out;
  }
  function drawColours() {
    var lk = state.look, c = lk.colours, list = el('div', 'fcolours'), chips = {};
    function check() {
      Object.keys(chips).forEach(function (k) { chips[k].hidden = true; });
      readable(lk).forEach(function (r) {
        var chip = chips[r.fg];
        if (contrast(c[r.fg], r.bg) >= r.need) return;
        chip.hidden = false;
        chip.onclick = function () { c[r.fg] = onColour(r.bg); redraw(); };
      });
    }
    COLOUR_ROWS.forEach(function (r) {
      var line = el('div', 'fcolour-row');
      var name = el('span', 'fcolour-name', r[1]);
      var chip = button('fcontrast', 'Hard to read · Fix');
      chip.hidden = true;
      chip.title = 'Swap for the ink or white that reads best';
      chips[r[0]] = chip;
      name.appendChild(chip);
      line.appendChild(name);
      line.appendChild(colourPill(c, r[0], r[1], check));
      list.appendChild(line);
    });
    inspector.appendChild(list);
    check();
  }

  function drawNotice() {
    var n = state.notice;
    inspector.appendChild(switchEl('Show a notice first', n.on, function () { n.on = !n.on; redraw(); }));
    if (!n.on) return;
    inspector.appendChild(field('Saying', radios(NOTICES.map(function (x) { return [x.id, x.label]; }), n.kind,
      function (v) { n.kind = v; redraw(); })));
    if (n.kind === 'custom') {
      inspector.appendChild(field('In your words', input(n.text, 'What a visitor should know first', function (v) { n.text = v; }, true, 140)));
    }
  }

  function drawSharing() {
    var s = state.sharing;
    inspector.appendChild(switchEl('Show in search results', s.listed, function () { s.listed = !s.listed; redraw(); }));
    // The card a message app draws from the link: the face's own look, its mark, its name and
    // what it is for — the site's own words from step 1, said once, not restated here.
    var card = el('div', 'fog'), pic = el('div', 'fog-pic'), words = el('div', 'fog-words');
    card.appendChild(pic);
    card.appendChild(words);
    inspector.appendChild(field('When it’s shared', card));
    og = function () {
      applyLook(pic);
      pic.innerHTML = '';
      pic.appendChild(avatarEl('fog-mark'));
      pic.appendChild(el('span', 'fog-name', state.site.name || 'Your site'));
      words.innerHTML = '';
      words.appendChild(el('b', '', state.site.name || 'Your site'));
      if (state.site.purpose) words.appendChild(el('span', 'fog-desc', state.site.purpose));
      words.appendChild(el('span', 'fog-domain', 'wallflowers.io'));
    };
    og();
  }

  // Shrunk and cut on this device, so an image weighs what the face needs and no more.
  function shrink(file, maxW, maxH, crop, type, quality) {
    return new Promise(function (resolve, reject) {
      var url = URL.createObjectURL(file), img = new Image();
      img.onload = function () {
        var iw = img.naturalWidth, ih = img.naturalHeight, sx = 0, sy = 0, sw = iw, sh = ih, cw, ch, k;
        if (crop) {
          k = Math.max(maxW / iw, maxH / ih);
          sw = maxW / k; sh = maxH / k; sx = (iw - sw) / 2; sy = (ih - sh) / 2; cw = maxW; ch = maxH;
        } else {
          k = Math.min(1, maxW / iw, maxH / ih);
          cw = Math.round(iw * k); ch = Math.round(ih * k);
        }
        var c = document.createElement('canvas');
        c.width = cw; c.height = ch;
        c.getContext('2d').drawImage(img, sx, sy, sw, sh, 0, 0, cw, ch);
        URL.revokeObjectURL(url);
        resolve(c.toDataURL(type, quality));
      };
      img.onerror = function (e) { URL.revokeObjectURL(url); reject(e); };
      img.src = url;
    });
  }
  function toAvatar(f) { return shrink(f, 256, 256, true, 'image/jpeg', 0.85); }
  function toBanner(f) { return shrink(f, 900, 360, true, 'image/jpeg', 0.82); }
  function toLogo(f) { return shrink(f, 640, 240, false, 'image/png'); }
  function toWallpaper(f) { return shrink(f, 860, 1600, false, 'image/jpeg', 0.8); }

  /* ---- the widgets, and the sticker sheet ------------------------------------------------ */
  // Tapped, an entry opens the configure window — it does not add the widget. Dragged,
  // it brings one as it arrives; the configure window's copy is dragged the same way.
  function drawWidgets() {
    var list = el('div', 'tray-list');
    TRAY.forEach(function (key) {
      var spec = WIDGETS[key], tile = el('div', 'tray-tile tray-' + key);
      tile.draggable = true;
      tile.tabIndex = 0;
      tile.setAttribute('role', 'button');
      tile.setAttribute('aria-label', spec.name + ': configure, then drag onto the phone');
      tile.appendChild(el('span', 'tray-icon'));
      var words = el('span', 'tray-words');
      words.appendChild(el('b', '', spec.name));
      words.appendChild(el('span', '', spec.blurb));
      tile.appendChild(words);
      tile.addEventListener('dragstart', function (e) { startNew(e, create(key)); });
      tile.addEventListener('dragend', endDrag);
      var configure = function () { state.draft = { key: key, widget: create(key) }; focusOn('draft'); };
      tile.addEventListener('click', configure);
      tile.addEventListener('keydown', function (e) { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); configure(); } });
      list.appendChild(tile);
    });
    inspector.appendChild(list);
  }
  // Dragged onto the phone, a sticker lands where it is let go. The keyboard's way
  // is Enter, which puts it in the middle of what the phone is showing.
  function drawStickerSheet() {
    var picked = state.pick === 'sticker' && state.stickers[state.selected];
    if (picked) {
      inspector.appendChild(stickerCard(picked));
    } else if (state.stickers.length) {
      var clear = button('tray-clear', 'Clear all');
      clear.addEventListener('click', function () { state.stickers = []; redraw(); });
      inspector.appendChild(clear);
    }
    STICKER_GROUPS.forEach(function (g) {
      var list = STICKERS.filter(function (s) { return s.group === g; });
      if (!list.length) return;
      inspector.appendChild(el('h3', 'ftitle', g));
      var box = el('div', 'tray-stickers');
      list.forEach(function (s) {
        var t = el('div', 'tray-sticker');
        t.draggable = true;
        t.tabIndex = 0;
        t.title = s.name;
        t.setAttribute('role', 'button');
        t.setAttribute('aria-label', s.name + ' sticker: drag onto the phone, or press Enter');
        if (s.svg) t.innerHTML = doodleSvg(s);
        else {
          var img = el('img');
          img.src = VENDOR + s.file; img.alt = ''; img.loading = 'lazy'; img.draggable = false;
          t.appendChild(img);
        }
        t.addEventListener('dragstart', function (e) {
          drag = { sticker: s.id };
          e.dataTransfer.effectAllowed = 'copy';
          e.dataTransfer.setData('text/plain', 'wf-sticker:' + s.id);
        });
        t.addEventListener('dragend', endDrag);
        t.addEventListener('keydown', function (e) {
          if (e.key !== 'Enter' && e.key !== ' ') return;
          e.preventDefault();
          var r = screen.getBoundingClientRect();
          addSticker(s.id, r.left + r.width / 2, r.top + r.height / 2);
        });
        box.appendChild(t);
      });
      inspector.appendChild(box);
    });
  }

  /* ---- the whole editor --------------------------------------------------------- */
  function mount(host) {
    root = host;
    root.innerHTML = '';
    root.classList.add('face-editor');
    panel = el('aside', 'face-panel');
    stage = el('div', 'face-stage');
    phone = el('div', 'face-phone');
    backdrop = el('div', 'wf-backdrop');
    screen = el('div', 'face-screen');
    var face = el('div', 'wf-face');
    canvas = el('div', 'wf-canvas');
    profile = el('header', 'wf-profile');
    grid = el('div', 'wf-grid');
    layer = el('div', 'wf-stickers');
    canvas.appendChild(profile);
    canvas.appendChild(grid);
    canvas.appendChild(layer);
    face.appendChild(canvas);
    screen.appendChild(face);
    phone.appendChild(backdrop);
    phone.appendChild(screen);
    door = el('span');          // replaced by the real door on the first draw
    phone.appendChild(door);
    sheet = el('span');
    phone.appendChild(sheet);
    notice = el('span');
    phone.appendChild(notice);
    stage.appendChild(phone);

    var bar = el('div', 'face-tools');
    tools = { undo: button('face-tool'), redo: button('face-tool') };
    tools.undo.setAttribute('aria-label', 'Undo');
    tools.undo.title = 'Undo';
    tools.undo.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/></svg>';
    tools.redo.setAttribute('aria-label', 'Redo');
    tools.redo.title = 'Redo';
    tools.redo.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 14 5-5-5-5"/><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"/></svg>';
    tools.undo.addEventListener('click', undo);
    tools.redo.addEventListener('click', redo);
    bar.appendChild(tools.undo);
    bar.appendChild(tools.redo);
    stage.appendChild(bar);

    root.appendChild(panel);
    root.appendChild(stage);
    wireDrop(screen);
    // a click on the phone off anything puts down what was picked; on the header, opens it
    screen.addEventListener('click', function () { if (state.pick) focusOn(null); });
    profile.addEventListener('click', function (e) { e.stopPropagation(); openSection('header'); });
    // Enter here edits the face; it must not reach the page's form, where it means Continue.
    root.addEventListener('keydown', function (e) { if (e.key === 'Enter') e.stopPropagation(); });
    document.addEventListener('keydown', function (e) {
      if (!root.offsetParent) return;                        // the face step is not showing
      var t = e.target;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      // Escape puts down what is picked (and leaves a widget's configure window).
      if (e.key === 'Escape' && !document.querySelector('.clr-picker.clr-open')) {
        if (state.pick) focusOn(null);
        return;
      }
      var k = (e.key || '').toLowerCase();
      if (!(e.metaKey || e.ctrlKey) || (k !== 'z' && k !== 'y')) return;
      e.preventDefault();
      if (k === 'y' || e.shiftKey) redo(); else undo();
    });
    if (global.Coloris) {
      global.Coloris({
        el: '.fcolour-input', theme: 'large', themeMode: 'light', format: 'hex', alpha: false, wrap: false,
        margin: 8, closeButton: true, closeLabel: 'Done', focusInput: false, selectInput: false,
        swatches: COLOURS.map(function (c) { return c[1]; })
      });
    } else if (global.console) {
      console.error('face: vendor/coloris is not loaded — colours can be typed as hex, not picked.');
    }
    drawPhone();
    drawPanel();
    lastSnap = snapshot();
    toolState();
    loadIcd().then(drawPanel);
  }

  /* ---- on the wire ------------------------------------------------------------------
     What this face would be as Deltas — condensed, so each fact is said ONCE, by the
     op that already owns it (core/docs/the-face-on-the-wire.md):

       group.setProfile   the name, what it is for, the mark, and where else to find
                          it (the Social widget's links are the card's `urls`)
       group.setMedia     each picture in its own slot — setCover, generalised
       base.setLocation   where it is, as an `area` of geohash cells (the Location
                          widget's precision and words are the location's, not the widget's)
       base.mintDoorbell  the public way in, when there is one: "Ask to join" is a
                          doorbell that waits for a member, "Open" one that does not,
                          "By invitation" none at all
       group.setFace      what is left, which is only presentation

     An op's args are flat — text or integer — so structure is JSON in one text arg,
     as `card` already is. Whether each op can be authored is read from the ICD: it
     must be declared on the channel of the kind it is authored to (`onChannel`).
     What the editor cannot know, it names in `missing`. NOTHING HERE IS SENT — see
     FACE_SAVED in signup.html. */
  var SHAPES = { Business: 'organisation', Community: 'community', Fund: 'organisation' };
  function base64Of(url) {
    var m = /^data:([^;]+);base64,(.*)$/.exec(url || '');
    return m ? { mime: m[1], data: m[2] } : null;
  }
  /* May this kind carry this op? Was channels.<kind>.messages[op]; now it is
     the kind's own ops plus every facet whose `on` lists it, flattened at load. */
  function onChannel(kind, op) {
    var c = icdOn && icdOn[kind];
    return !!(c && c[op]);
  }
  // The face with nothing in it that another op already says.
  function faceDoc() {
    var look = copy(state.look);
    delete look.wallpaper.image;                       // a picture: group.setMedia 'wallpaper'
    return {
      v: 1, look: look, header: { layout: state.header.layout, title: state.header.title }, show: state.show,
      widgets: { root: { type: 'root', version: 1, children: state.widgets.map(function (w) {
        var o = Object.assign({ version: 1 }, w);
        if (w.type === 'wf-social') delete o.links;                            // the card's urls
        if (w.type === 'wf-location') { delete o.precision; delete o.label; } // the location's
        return o;
      }) } },
      stickers: state.stickers.map(function (k) {       // a thousandth of the face's width is finer than a pixel
        var o = copy(k); o.x = +o.x.toFixed(3); o.y = +o.y.toFixed(3); o.size = +o.size.toFixed(3); return o;
      }),
      notice: state.notice,
      door: { doorbell: state.door.access === 'invite' ? '' : 'door', label: state.door.label.trim(), questions: state.door.questions },
      listed: state.sharing.listed
    };
  }
  function deltas() {
    var out = [], mark = base64Of(state.avatar);
    function add(op, args, carries, missing) {
      var d = { log: 'group', op: op, onChannel: onChannel('group', op), carries: carries, args: args };
      if (missing) d.missing = missing;
      out.push(d);
    }
    var urls = [];
    state.widgets.forEach(function (w) {
      if (w.type !== 'wf-social') return;
      w.links.forEach(function (l) { var u = linkUrl(l); if (u) urls.push({ label: l.platform, value: u }); });
    });
    add('group.setProfile', {
      displayName: state.site.name, shape: SHAPES[state.site.kind] || 'community',
      card: JSON.stringify({ note: state.site.purpose, photo: mark ? mark.data : '', photo_mime: mark ? mark.mime : '', urls: urls })
    }, 'the name, what it is for, the mark, where else to find it');
    [['cover', state.header.banner], ['logo', state.header.logo], ['wallpaper', state.look.wallpaper.image]].forEach(function (p) {
      var m = base64Of(p[1]);
      if (m) add('group.setMedia', { slot: p[0], media: m.data, mediaMime: m.mime, mediaKind: 'still', mediaVia: 'inline' }, 'the ' + p[0]);
    });
    var at = state.widgets.filter(function (w) { return w.type === 'wf-location'; })[0];
    if (at) {
      add('base.setLocation', { source: JSON.stringify({ shape: 'area', precision: at.precision, label: at.label, cells: [] }) },
        'where it is, no finer than chosen', 'cells: the editor has no position; the device fills them, cut to ' + at.precision + ' characters');
    }
    add('group.setFace', { face: JSON.stringify(faceDoc()) }, 'the look, the layout, the stickers, the notice, the door’s words and questions');
    if (state.door.access !== 'invite') {
      add('base.mintDoorbell', { id: 'door', mode: state.door.access === 'open' ? 'auto' : 'review', role: 'member', type: '', notAfter: 0 },
        state.door.access === 'open' ? 'the open door' : 'the door that waits for a member',
        'key: minted on the owner’s device; the face carries its public half');
    }
    out.forEach(function (d) { d.bytes = JSON.stringify(d.args).length; });
    return out;
  }

  /* A picture held in the editor is a data URL; the document names one only in words
     ('data URL, n chars'), so a picture comes back from its slot or not at all. */
  function dataUrl(u) { return typeof u === 'string' && /^data:image\//.test(u) ? u : ''; }
  function obj(o) { return o && typeof o === 'object' && !Array.isArray(o) ? o : {}; }

  W.Face = {
    mount: mount,
    deltas: deltas,
    /* AN EXISTING FACE, as document() made it, and the Site's pictures, which live as its
       Host's media (mark, cover, logo, wallpaper) and not in the face. Whatever the document
       leaves out keeps the editor's own; the history starts again from here, so an undo never
       walks back into what was open before. */
    load: function (doc, media) {
      doc = obj(doc); media = obj(media);
      var cur = JSON.parse(snapshot()), lk = obj(doc.look), h = obj(doc.header);
      var look = {};
      ['wallpaper', 'blocks', 'text', 'colours'].forEach(function (k) { look[k] = Object.assign({}, cur.look[k], obj(lk[k])); });
      look.wallpaper.image = dataUrl(media.wallpaper) || dataUrl(look.wallpaper.image);
      var widgets = doc.blocks && doc.blocks.root && Array.isArray(doc.blocks.root.children)
        ? doc.blocks.root.children.map(function (w) { var o = Object.assign({}, w); delete o.version; return o; })
          .filter(function (w) { return WIDGETS[keyOf(w)]; })
        : cur.widgets;
      var s = {
        look: look,
        header: Object.assign({}, cur.header, { layout: h.layout || cur.header.layout, title: h.title || cur.header.title,
                                                logo: dataUrl(media.logo), banner: dataUrl(media.cover) }),
        show: Object.assign({}, cur.show, obj(doc.show)),
        door: Object.assign({}, cur.door, obj(doc.door)),
        notice: Object.assign({}, cur.notice, obj(doc.notice)),
        sharing: Object.assign({}, cur.sharing, obj(doc.sharing)),
        avatar: dataUrl(media.mark),
        widgets: widgets,
        stickers: Array.isArray(doc.stickers) ? doc.stickers : cur.stickers
      };
      if (phone) restore(JSON.stringify(s));
      else Object.keys(s).forEach(function (k) { state[k] = s[k]; });
      past = []; future = []; lastSnap = snapshot();
      if (phone) toolState();
    },
    /* The pictures, by the Host's slot, as data URLs, a slot with none left out: the shape of
       the signup's draft `media` (BUILD's Pictures, 12ec0f7) and of what load() takes. */
    media: function () {
      var out = {}, all = { mark: state.avatar, cover: state.header.banner, logo: state.header.logo, wallpaper: state.look.wallpaper.image };
      Object.keys(all).forEach(function (k) { if (dataUrl(all[k])) out[k] = all[k]; });
      return out;
    },
    site: function (s) {
      state.site = { name: s.name || '', purpose: s.purpose || '', slug: s.slug || '', kind: s.kind || '' };
      if (phone) { drawPhone(); drawPanel(); }
    },
    summary: function () {
      var p = presetNow(), t = state.look.text;
      return {
        count: state.widgets.length,
        stickers: state.stickers.length,
        names: state.widgets.map(function (w) { return WIDGETS[keyOf(w)].name; }),
        look: (p ? p.name : 'Custom') + ' · ' + fontOf(t.title || t.font).name
          + (t.title && t.title !== t.font ? ' and ' + fontOf(t.font).name : ''),
        door: state.door.label.trim() || accessOf(state.door.access).door
      };
    },
    document: function () {
      var h = state.header, img = function (u) { return u ? 'data URL, ' + u.length + ' chars' : ''; };
      var lk = copy(state.look);
      lk.wallpaper.image = img(lk.wallpaper.image);
      return {
        look: lk,
        header: { layout: h.layout, title: h.title, logo: img(h.logo), banner: img(h.banner) },
        show: state.show,
        door: state.door,
        notice: state.notice,
        sharing: state.sharing,
        mark: img(state.avatar),
        stickers: state.stickers,
        blocks: { root: { type: 'root', version: 1, direction: 'ltr', format: '', indent: 0,
          children: state.widgets.map(function (w) { return Object.assign({ version: 1 }, w); }) } }
      };
    }
  };
})(window);
