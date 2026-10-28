/* pick.mjs — /map: the open map, and choosing places on it.
 *
 * THE MAP IS OPENSTREETMAP, drawn by MapLibre GL JS from one PMTiles archive
 * (assets/map/, vendored and pinned in assets/map/VENDOR.json). The archive is
 * read from this origin a few kilobytes at a time: no token, no tile service,
 * nobody else told where you are looking. OpenStreetMap's licence asks for the
 * credit in the corner; it stays.
 *
 * IN CEREMONY.JS'S COLOURS: its paper for land, its sky for water, its greens
 * for parks. No lettering is drawn on the map — that needs a font served in
 * 256-glyph files, and none is vendored yet — so a place says its name when
 * you point at it, and wears it once chosen.
 *
 * A PLACE IS ITS GEOHASH, nine characters: under five metres, what the iPhone
 * app mints a venue at. Tap a dot to choose it; tap its label to let it go.
 * Two or more, and the key sends you to walk between them (roam.js).
 *
 * AND IT IS THE SAME GEOHASH WHATEVER ZOOM IT WAS TAPPED AT. A tile rounds
 * where a place is to its own grid, and a shallower tile's grid is coarser:
 * the same place came back up to half a metre away at zoom 13, which is enough
 * to change the ninth character, and with it the land. So a place is only ever
 * taken from the archive's deepest tiles: tapped further out, the map goes in
 * to it first and takes it there, by the id the tiles give it.
 *
 * SEVERAL LOOKS, AND A FLICK BETWEEN THEM. The land is walked in shapes
 * (town.js) in any of skins.js's looks, or in blocks as it first was (roam.js).
 * The row of names along the foot of the walk, or a number key, changes it:
 * between two of town.js's looks nothing is rebuilt; to or from blocks the walk
 * is taken up again where you stood, with the signs you had raised.
 *
 * THE CHOICE LIVES IN THE ADDRESS, after the #: a link to it opens the same
 * places and so the same land (and, with ?walk, opens in it). That is all that holds it. It is not WallFlowers
 * state — no Delta carries a chosen place — and saying so is not a detail: a
 * chosen place that belonged to you would be a Place with base.setLocation.
 */
import { Map as GLMap, Marker, AttributionControl, addProtocol } from '../map/maplibre-gl.mjs';
import { layers, namedFlavor } from '../map/basemaps.mjs';

const K = globalThis.Kenjin, Geo = K.Geo, A = K.Ceremony.art, P = A.P, pm = globalThis.pmtiles;
const $ = id => document.getElementById(id);
const css = c => `rgb(${c.map(Math.round).join(',')})`;
const MOST = 8;                                 // places to a land: the block grows with them, and stops

const box = $('map'), url = new URL(box.dataset.tiles, location.href).href;
const protocol = new pm.Protocol(), archive = new pm.PMTiles(url);
protocol.add(archive); addProtocol('pmtiles', protocol.tile);
const head = await archive.getHeader(), edge = [[head.minLon, head.minLat], [head.maxLon, head.maxLat]];

const flavor = { ...namedFlavor('light'), background: css(P.paper), earth: css(P.paper), water: css(A.SKY[1]),
                 park_a: css(P.paleLight), park_b: css(P.pale), wood_a: css(P.paleLight), wood_b: css(P.pale), buildings: css(P.boneShade) };
const map = new GLMap({
  container: box, attributionControl: false, bounds: edge, maxBounds: edge, maxZoom: head.maxZoom + 3, dragRotate: false, pitchWithRotate: false, touchPitch: false,
  style: {
    version: 8,
    sources: { osm: { type: 'vector', url: 'pmtiles://' + url, attribution: '<a href="https://www.openstreetmap.org/copyright">© OpenStreetMap</a>' } },
    layers: [...layers('osm', flavor), {
      id: 'places', type: 'circle', source: 'osm', 'source-layer': 'pois', filter: ['has', 'name'],
      paint: { 'circle-radius': ['interpolate', ['linear'], ['zoom'], 12, 2, 16, 5], 'circle-color': css(P.key), 'circle-stroke-color': css(P.panelEdge), 'circle-stroke-width': 1 }
    }]
  }
});
map.addControl(new AttributionControl({ compact: true }), 'bottom-left');
map.touchZoomRotate.disableRotation();

/* ── choosing ── */
const chosen = new Map();                       // geohash -> { site, marker }
function write() {
  history.replaceState(null, '', location.pathname + location.search + '#' + [...chosen.values()].map(c => [c.site.id, c.site.kind || '', encodeURIComponent(c.site.name || '')].join('~')).join('|'));
  $('go').dataset.ready = chosen.size >= 2 ? 'yes' : 'no';
}
function take(site) {
  if (chosen.has(site.id) || chosen.size >= MOST) return;
  const tag = document.createElement('button');
  tag.type = 'button'; tag.className = 'name chosen'; tag.textContent = site.name || site.id;
  tag.addEventListener('click', e => { e.stopPropagation(); chosen.get(site.id).marker.remove(); chosen.delete(site.id); write(); });
  chosen.set(site.id, { site, marker: new Marker({ element: tag, anchor: 'bottom', offset: [0, -6] }).setLngLat([site.lng, site.lat]).addTo(map) });
  write();
}
function under(point) {                         // the place nearest a point of the page, within a finger's width
  const hits = map.queryRenderedFeatures([[point.x - 12, point.y - 12], [point.x + 12, point.y + 12]], { layers: ['places'] });
  let best = null, far = 1e9;
  for (const f of hits) {
    const at = map.project(f.geometry.coordinates), d = Math.hypot(at.x - point.x, at.y - point.y);
    if (d < far) { far = d; best = f; }
  }
  if (!best) return null;
  return placeOf(best);
}
function placeOf(f) {
  const [lng, lat] = f.geometry.coordinates;
  return { id: Geo.encode(lat, lng, 9), lat, lng, name: f.properties['name:en'] || f.properties.name, kind: f.properties.kind, tile: f.id };
}
map.on('click', e => {
  const s = under(e.point);
  if (!s) return;
  if (map.getZoom() >= head.maxZoom) return take(s);
  map.easeTo({ center: [s.lng, s.lat], zoom: head.maxZoom + 0.4, duration: 450 });
  map.once('idle', () => {
    const f = map.queryRenderedFeatures({ layers: ['places'] }).find(q => q.id === s.tile);
    if (f) take(placeOf(f));
  });
});
map.on('mousemove', e => {
  const s = under(e.point), tip = $('tip');
  map.getCanvas().style.cursor = s ? 'pointer' : '';
  tip.hidden = !s;
  if (s) { tip.textContent = s.name; tip.style.left = e.point.x + 'px'; tip.style.top = e.point.y - 14 + 'px'; }
});
for (const part of location.hash.slice(1).split(/\||%7C/i)) {
  const [id, kind, name] = part.split('~'), at = id && Geo.centre(id);
  if (at) take({ id, lat: at[0], lng: at[1], kind, name: decodeURIComponent(name || '') });
}
write();

/* ── walking ── */
let game = null;
const cast = K.Hud.cast();
function names(list) {                          // the name of where you are heading, over it, or at the edge of the page that way
  const host = $('names'), w = innerWidth, h = innerHeight;
  while (host.children.length < list.length) host.appendChild(Object.assign(document.createElement('span'), { className: 'name' }));
  while (host.children.length > list.length) host.lastChild.remove();
  const r = $('radar').getBoundingClientRect();
  list.forEach((s, i) => {
    const tag = host.children[i], x = Math.max(70, Math.min(w - 70, s.at[0]));
    let y = Math.max(132, Math.min(h - 40, s.at[1]));                                      // clear of the key and the sun,
    if (x > r.left - 90 && x < r.right + 90 && y > r.top - 8 && y < r.bottom + 40) y = r.top < h / 2 ? r.bottom + 40 : r.top - 8;   // and of the radar
    if (tag.textContent !== s.name) tag.textContent = s.name;
    tag.style.transform = `translate(-50%, -100%) translate(${x}px, ${y}px)`;
  });
}
K.Hud.key($('go').firstElementChild, 'right', false);
K.Hud.key($('back').firstElementChild, 'left', false);
/* ?walk in the address is the land itself: a link to it opens there, not on the map. ?look is what it is wearing. */
const LOOKS = [...K.Skins.LIST.map(s => ({ id: s.id, name: s.name })), { id: 'blocks', name: 'Blocks' }];
let look = new URLSearchParams(location.search).get('look');
if (!LOOKS.some(l => l.id === look)) look = LOOKS[0].id;
function address(walking) {
  const u = new URL(location.href);
  if (walking) { u.searchParams.set('walk', ''); u.searchParams.set('look', look); } else u.searchParams.delete('walk');
  history.replaceState(null, '', u.href.replace('walk=', 'walk'));
}
async function go(from) {                       // from: where you stood and what you had reached, when only the look is changing
  if (chosen.size < 2 || game) return;
  const old = $('world'), fresh = old.cloneNode(false);          // a canvas keeps its first context: each walk gets its own
  old.replaceWith(fresh); $('names').replaceChildren();
  document.body.dataset.mode = 'walk'; address(true);
  const o = { canvas: fresh, mark: $('mark'), names, radar: K.Hud.radar, say: K.Hud.say, skin: look, at: from && from.at, seen: from && from.seen };
  game = K.Roam.now = (look === 'blocks' ? K.Roam : K.Town).start(await cast, [...chosen.values()].map(c => c.site), o);
  K.Hud.bind(game);
}
function wear(id) {
  const was = look, now = LOOKS.find(l => l.id === id);
  if (!now || id === was) return;
  look = id;
  for (const b of $('looks').children) b.classList.toggle('chosen', b.dataset.look === look);
  if (!game) return;
  address(true); K.Hud.say(now.name);
  if (id !== 'blocks' && was !== 'blocks') { game.skin(id); return; }        // only numbers change
  const s = game.state();
  game.stop(); game = K.Roam.now = null;
  go({ at: [s.at[0], s.at[2]], seen: s.spots.map(p => p.seen) });
}
LOOKS.forEach((l, i) => {
  const b = Object.assign(document.createElement('button'), { type: 'button', className: 'name look' });
  b.dataset.look = l.id; b.classList.toggle('chosen', l.id === look);
  b.append(Object.assign(document.createElement('b'), { textContent: (i + 1) % 10 }), Object.assign(document.createElement('span'), { textContent: ' ' + l.name }));
  b.addEventListener('click', () => wear(l.id));
  $('looks').append(b);
});
addEventListener('keydown', e => {
  const n = /^Digit(\d)$/.exec(e.code);
  if (n && game && !e.metaKey && !e.ctrlKey && !e.altKey) wear((LOOKS[(+n[1] + 9) % 10] || {}).id);
});
$('go').addEventListener('click', () => go());
$('back').addEventListener('click', () => {
  if (!game) return;
  game.stop(); game = K.Roam.now = null;
  document.body.dataset.mode = 'map'; address(false);
  map.resize();
});
if (new URLSearchParams(location.search).has('walk')) go();
K.Pick = { map, chosen, under, take, wear, looks: LOOKS };
