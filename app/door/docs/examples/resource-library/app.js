/*
 * WallFlowers Community API example: a resource library.
 *
 * Lists a community's resources, and lets a signed-in member write one and publish it to
 * the library. Two files, no build: this one and index.html. Serve the folder and open it:
 *
 *   python3 -m http.server 5175 --bind 127.0.0.1      then open http://localhost:5175/
 *
 * Three parts of the Community API (https://docs.wallflowers.io/), one per section below:
 *   1. the public page's items ("Events and resources"): anyone can read them, no session;
 *   2. the sign-in script ("Sign in"): a member's session, for one community;
 *   3. the Resources editor ("The Resources editor"): the member writes and publishes.
 */

export const CONFIG = {
  /** WallFlowers' Door (sign-in, the editor, the API) and Arc (the public pages). */
  door: '{{DOOR}}',
  arc: '{{ARC}}',
  /** Your community's address, as in wallflowers.io/<address>. A session can't read it: set it here. */
  address: 'your-community-address',
  /** Each script's sha384, as the current Door release publishes it. */
  integrity: '{{SRI}}',
  resourcesIntegrity: '{{RESOURCES_SRI}}',
  /** Your client id, and its callback, character for character: this page. */
  client: '{{QS_CLIENT_JS}}',
  callback: location.origin + location.pathname,
};

const $ = (id) => document.getElementById(id);
let session = null;
let timer = 0;

/* ── 1. the library: the community's published resources ────────────────────── */

// The public page's items, read with a plain fetch: the same for everyone, signed in or not.
// Read it on load and when something changes; never in a loop (it is every community's page).
async function library() {
  // An unpublished community answers 404, which a browser may report as a failed fetch (no CORS header).
  const res = await fetch(`${CONFIG.arc}/v1/face/${encodeURIComponent(CONFIG.address)}/items`).catch(() => null);
  if (!res || res.status === 404) return say(`No published community at "${CONFIG.address}": set CONFIG.address in app.js.`);
  if (!res.ok) return say(`GET /v1/face/…/items: ${res.status}`);
  const { items } = await res.json();
  // Items are keyed event:<id> and post:<id>; a resource is a post. Newest first.
  const posts = Object.entries(items)
    .filter(([key]) => key.startsWith('post:'))
    .map(([key, item]) => ({ ...item, id: key.slice(5) }))
    .sort((a, b) => (b.at ?? 0) - (a.at ?? 0));
  $('library').replaceChildren(...posts.map((p) => {
    // Each resource has its own public page: title, body, pictures and PDF.
    const link = Object.assign(document.createElement('a'), {
      href: `${CONFIG.arc}/v1/face/${encodeURIComponent(CONFIG.address)}/post/${p.id}`,
      textContent: p.title, // text, never HTML
    });
    return item(link, p.form ?? 'article', p.excerpt);
  }));
  if (!posts.length) $('library').replaceChildren(item(null, 'Nothing published yet.'));
  return posts.map((p) => p.id);
}

// The public page takes a minute or two to show a new post: read it again every 30 s until it
// does, at most 4 times. Never faster, never forever.
function until(id, tries = 4) {
  setTimeout(async () => { if (!(await library())?.includes(id) && tries > 1) until(id, tries - 1); }, 30_000);
}

/* ── 2. sign in: a member's session ─────────────────────────────────────────── */

// A script from the Door, pinned by its hash: the browser refuses it if one byte differs.
function load(path, integrity) {
  return new Promise((resolve, reject) => {
    const s = Object.assign(document.createElement('script'), { src: CONFIG.door + path, integrity, crossOrigin: 'anonymous' });
    s.onload = resolve;
    s.onerror = () => reject(new Error(`${path} did not load. If this page's address (${location.origin}) isn't allowed: {{STARTER_ORIGIN_FIX}}`));
    document.head.append(s);
  });
}

async function main() {
  library();
  await load('/v2/signin.js', CONFIG.integrity);
  const { client, callback } = CONFIG;
  // Back from WallFlowers' sign-in window, the address carries ?state: finish() makes the session.
  session = new URLSearchParams(location.search).has('state')
    ? await WallFlowers.finish({ client, callback })
    : await WallFlowers.current({ client });
  if (!session) {
    $('signin').hidden = false;
    $('signin').onclick = () => WallFlowers.signIn({ client, callback });
    return;
  }
  $('signout').hidden = false;
  $('signout').onclick = async () => { await session.signOut(); location.reload(); };
  // One change stream: something changed, so read again once a burst has settled.
  session.events(() => { clearTimeout(timer); timer = setTimeout(refresh, 300); }, () => location.reload());
  await load('/v2/resources.js', CONFIG.resourcesIntegrity);
  $('member').hidden = false;
  $('new').onclick = () => edit(null);
  refresh();
}

/* ── 3. write and publish: the Resources editor ─────────────────────────────── */

// The posts this session made, drafts included. A session reaches its community and what it
// made, so this is the member's own work here; a post made elsewhere is edited in WallFlowers.
async function refresh() {
  library();
  const res = await session.fetch('/v2/graph');
  if (res.status === 401) return location.reload();
  if (!res.ok) return say(`GET /v2/graph: ${res.status}`);
  const { objects = [] } = await res.json();
  const posts = objects.filter((o) => o.kind === 'post' && o.folds !== false && !o.view?.retracted);
  $('mine').replaceChildren(...posts.map((o) => {
    // Published: the post names the community (and the community names it).
    const published = (o.view?.backlinks ?? []).some((b) => b.object === session.site);
    const open = Object.assign(document.createElement('button'), { type: 'button', textContent: 'Edit' });
    open.onclick = () => edit(o.id);
    const li = item(document.createTextNode(o.view?.title || o.name || 'Untitled'), published ? 'published' : 'draft');
    li.append(' ', open);
    return li;
  }));
  if (!posts.length) $('mine').replaceChildren(item(null, 'None yet.'));
}

// Save draft keeps the post to you; Publish puts it in the library above. The editor reads
// its ops from the model and writes them with your session: this page sends nothing itself.
function edit(post) {
  $('editor').hidden = false;
  WallFlowersResources.mount($('editor'), {
    fetch: session.fetch,
    site: session.site,
    post, // null: a new one
    onSaved: ({ post, published }) => { refresh(); if (published) until(post); },
    onClose: () => { $('editor').hidden = true; $('editor').replaceChildren(); },
  });
}

/* ── helpers ────────────────────────────────────────────────────────────────── */

function item(lead, tag, text) {
  const li = document.createElement('li');
  if (lead) li.append(lead, ' ');
  li.append(Object.assign(document.createElement('small'), { textContent: tag }));
  if (text) li.append(Object.assign(document.createElement('p'), { textContent: text }));
  return li;
}

function say(text) {
  $('status').textContent = text;
  $('status').hidden = !text;
}

main().catch((err) => say(err.message));
