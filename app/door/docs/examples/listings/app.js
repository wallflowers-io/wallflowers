/*
 * WallFlowers Community API example: a listings board.
 *
 * A community's Trade board: every member reads it, a member lists something of theirs, and
 * takes it down again; the owner or an admin can take anyone's down. Two files, this one and
 * index.html, no build. It is the starter (docs: Quickstart) plus the board (docs: "The
 * listings board"). Serve this folder and open the page:
 *
 *   python3 -m http.server 5174 --bind 127.0.0.1      then open http://localhost:5174/
 */

export const CONFIG = {
  /** WallFlowers' Door: where signin.js and the API are served. */
  door: '{{DOOR}}',
  /** signin.js's sha384, as WallFlowers publishes it for the current Door release. */
  integrity: '{{SRI}}',
  /** Your client id: on your own machine, the shared local client where the release has one;
      otherwise the one WallFlowers registered for you. */
  client: '{{QS_CLIENT_JS}}',
  /** The callback registered for the client, character for character: this page. */
  callback: location.origin + location.pathname,
};

const $ = (id) => document.getElementById(id);
const bare = (pk) => String(pk ?? '').replace(/^ed25519:/, '').toLowerCase();

/* ── sign in: the same as the starter ─────────────────────────────────────── */

function loadSignIn() {
  return new Promise((resolve, reject) => {
    if (window.WallFlowers) return resolve(window.WallFlowers);
    const script = Object.assign(document.createElement('script'), {
      src: `${CONFIG.door}/v2/signin.js`, integrity: CONFIG.integrity, crossOrigin: 'anonymous',
    });
    script.onload = () => resolve(window.WallFlowers);
    script.onerror = () => reject(new Error('signin.js did not load: {{STARTER_ORIGIN_FIX}} Otherwise check CONFIG.door and CONFIG.integrity.'));
    document.head.append(script);
  });
}

async function main() {
  const wallflowers = await loadSignIn();
  const { client, callback } = CONFIG;
  // Back from the Door's window (?code&state): finish() makes the session. Otherwise, any held one.
  const session = new URLSearchParams(location.search).has('state')
    ? await wallflowers.finish({ client, callback })
    : await wallflowers.current({ client });
  if (!session) {
    $('signin').hidden = false;
    $('signin').onclick = () => wallflowers.signIn({ client, callback });
    return;
  }
  run(session);
}

/* ── the board ────────────────────────────────────────────────────────────── */

function run(session) {
  let graph = null;
  $('signout').hidden = false;
  $('signout').onclick = async () => { stop(); await session.signOut(); location.reload(); };

  // Everything this session can read, in one answer: the community, and its listings in its view.
  async function refresh() {
    const res = await session.fetch('/v2/graph');
    if (res.status === 401) return say('Your session has ended. Reload to sign in again.');
    if (!res.ok) return say(`GET /v2/graph: ${res.status} ${await res.text()}`);
    graph = await res.json();
    draw();
  }

  // The change stream says only that something changed: read the graph again once it settles.
  let timer = 0;
  const stop = session.events(() => { clearTimeout(timer); timer = setTimeout(refresh, 300); }, () => say('Signed out.'));

  // One write through the API. A refusal is WallFlowers' own sentence: show it, don't parse it.
  async function write(path, body) {
    const res = await session.fetch(path, { method: 'POST', body: JSON.stringify(body) });
    if (!res.ok) {
      say(`Not done: ${await res.text()}`);
      return false;
    }
    say('');
    refresh(); // don't wait for the stream
    return true;
  }

  function draw() {
    const community = (graph.objects ?? []).find((o) => o.id === session.site);
    if (!community) return say('This account is not a member of this community yet.');
    const me = bare(graph.me?.pk);
    // The owner is the community's `owner`; admins are in its `roles`, as [key, role].
    const admin = bare(community.owner) === me || (community.view?.roles ?? []).some(([k, r]) => bare(k) === me && r === 'admin');
    // Names come from members' cards on the community's view (yours also from graph.me); a key is never a name.
    const nameOf = (key) => community.view?.profiles?.[bare(key)]?.name?.trim() || (bare(key) === me ? graph.me?.display_name : null) || 'A member';

    $('app').hidden = false;
    $('community').textContent = community.view?.display_name || community.name || 'Community';

    // Only live listings are on the board (not withdrawn, not taken down); the newest first.
    const listings = [...(community.view?.listings ?? [])].sort((a, b) => b.gen - a.gen);
    $('board').replaceChildren(...listings.map((l) => row(l, bare(l.author) === me, admin, nameOf)));
    $('empty').hidden = listings.length > 0;
  }

  // One listing: its photo, title, what and where, and who listed it (nameOf, from draw). Text, never HTML.
  function row(l, mine, admin, nameOf) {
    const item = document.createElement('li');
    if (l.photo && l.photoMime) item.append(Object.assign(document.createElement('img'), { src: `data:${l.photoMime};base64,${l.photo}`, alt: '' }));
    const text = document.createElement('div');
    text.append(Object.assign(document.createElement('b'), { textContent: l.title }));
    const line = [l.posture, l.price, l.area].filter(Boolean).join(' · ');
    text.append(Object.assign(document.createElement('small'), { textContent: `${line} — ${nameOf(l.author)}` }));
    if (l.descriptor) text.append(Object.assign(document.createElement('p'), { textContent: l.descriptor }));
    item.append(text);

    // Yours: take it down by listing it again with `withdrawn` 1, at a later `rev`.
    // Anyone's, if you are the owner or an admin: group.removeListing. It stays down.
    const down = mine
      ? () => write('/v2/apply', { object: session.site, op: 'group.publishListing', args: {
          thingId: l.thingId, posture: l.posture, title: l.title, reach: l.reach, rev: Math.max(Date.now(), l.rev + 1), withdrawn: 1 } })
      : admin && (() => write('/v2/apply', { object: session.site, op: 'group.removeListing', args: { author: l.author, thingId: l.thingId } }));
    if (down) {
      const button = Object.assign(document.createElement('button'), { type: 'button', textContent: 'Take down' });
      button.onclick = down;
      item.append(button);
    }
    return item;
  }

  // List something: a Thing of yours, its posture and price, and the listing on the community,
  // in one batch, so it is all or nothing. {"$step": 0} names the Thing the first step made.
  $('list').onsubmit = async (e) => {
    e.preventDefault();
    const title = $('title').value.trim(), price = $('price').value.trim(), descriptor = $('descriptor').value.trim();
    if (!title) return;
    const posture = $('posture').value;
    const listing = { thingId: { $step: 0 }, posture, title, reach: 'network', rev: Date.now() };
    const setPosture = { posture, reach: 'network' };
    if (price && (posture === 'selling' || posture === 'buying')) listing.price = setPosture.price = price; // price goes with buying or selling only
    if (descriptor) listing.descriptor = descriptor;
    $('add').disabled = true;
    const ok = await write('/v2/batch', { steps: [
      { do: 'mint', kind: 'thing', draft: { name: title } },
      { do: 'apply', object: { $step: 0 }, op: 'thing.setPosture', args: setPosture },
      { do: 'apply', object: session.site, op: 'group.publishListing', args: listing },
    ] });
    $('add').disabled = false;
    if (ok) $('list').reset();
  };

  refresh();
}

function say(text) {
  $('status').textContent = text;
  $('status').hidden = !text;
}

main().catch((err) => say(err.message));
