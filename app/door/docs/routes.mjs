/* WHAT EACH SITE ROUTE IS FOR: what a person needs besides what the routers state (
   app/door/routes.json gives the method, path, session and refusals; build.mjs reads it and
   checks this list against it). One entry for every route of audience "site", no more. */

export const ROUTES = [
  /* ── signing in ── */
  {
    method: 'GET', path: '/v2/signin.js', origin: 'door', group: 'Signing in', summary: 'The sign-in script',
    doc: 'WallFlowers\' sign-in script. Load it once per page, pinned by its integrity hash; it defines <code>WallFlowers.signIn</code>, <code>finish</code> and <code>current</code> (see <a href="signin.html">Sign in</a>). Its hash changes only with a Door release.',
    request: '<script src="{{DOOR}}/v2/signin.js"\n        integrity="{{SRI}}"\n        crossorigin="anonymous"></script>',
    response: 'text/javascript'
  },
  {
    method: 'POST', path: '/v2/token', origin: 'door', group: 'Signing in', summary: 'Trade the sign-in code for a session (the script does this)',
    doc: 'The sign-in script calls this in <code>WallFlowers.finish</code>: it trades the code WallFlowers\' window returned for the session token, with the page\'s PKCE verifier and a DPoP proof. You never call it yourself.',
    response: '{ "access_token": "…", "token_type": "DPoP", "expires_in": {{TOKEN_SECS}}, "scope": "<community id>" }   (read by the script, never by your code)'
  },
  {
    method: 'POST', path: '/v2/signout', origin: 'door', group: 'Signing in', summary: 'End the session',
    doc: 'Ends the session here and at WallFlowers. <code>session.signOut()</code> calls it and forgets the key and token in your origin\'s storage; call that, not this.',
    request: 'await session.signOut();'
  },
  /* ── reading ── */
  {
    method: 'GET', path: '/v2/me', origin: 'door', group: 'Reading', summary: 'Who is signed in',
    doc: 'The signed-in member. <code>pk</code> identifies them and is never a name; <code>display_name</code> is their name if they have set one. <code>noncompliant</code> lists records in reach that could not be read, with the reason: show them as unavailable. <code>card_note</code> is present when their picture was left out of their card, and says why.',
    request: 'const me = await (await session.fetch(\'/v2/me\')).json();',
    response: '{ "pk": "ed25519:9442…3a2e", "display_name": "Hana", "objects": 7,\n  "noncompliant": [ { "object": "…", "kind": "forum", "why": "…" } ],\n  "card_note": "…", "resume": { "objects": …, "spine": … } }'
  },
  {
    method: 'GET', path: '/v2/graph', origin: 'door', group: 'Reading', summary: 'Everything the session can read',
    doc: 'One call returns every object the session can read: the community, its rooms, its page, its treasury, and anything this session made. Draw from it, and read it again when <a href="#get-v2-events">the stream</a> says something changed. See <a href="views.html">What you read</a> for each kind\'s <code>view</code>.',
    request: 'const g = await (await session.fetch(\'/v2/graph\')).json();',
    response: '{ "me": { "pk": "ed25519:…", "display_name": "Hana" },\n  "objects": [\n    { "id": "71c1…6a6c", "kind": "group", "name": "Mill Road Allotments",\n      "owner": "2d63…edfa", "members": ["2d63…", "0224…"],\n      "peer": null, "role": null, "folds": true, "why": null,\n      "view": { … } },\n    … ],\n  "spine": [] }'
  },
  {
    method: 'GET', path: '/v2/members', origin: 'door', group: 'Reading', summary: 'The community\'s members, their about, answers and intentions',
    doc: 'Your community\'s members as people are shown them: each one\'s name and picture from their card (<code>null</code> where they have none: show "New member", never the key), their role (<code>owner</code>, <code>admin</code> or <code>member</code>), their own <code>about</code> ({bio, links}), their <code>answers</code> to the community\'s questions (by question id), and their <code>intentions</code> (the choice they joined with: <code>resources</code>, <code>skills</code> or <code>financial</code>). The owner comes first, then the admins, then everyone else by name. <code>questions</code> are the owner\'s and admins\' questions, oldest first, each with its form and the tally per option. <code>can_define</code> says whether the signed-in member may put a question. It is read from the same graph as <code>/v2/graph</code>, so it shows nothing a member could not read there, and everything in it is readable by every member of the community: say so wherever you show or edit it. Write with <code>/v2/apply</code> on the community: <code>base.publishAbout</code> {bio, links} replaces the member\'s whole about; <code>base.answerQuestion</code> {target_author, target_gen, text, choices} their whole answer; <code>base.defineQuestion</code> and <code>base.retireQuestion</code> are the owner\'s and admins\'. Read it again after a write, or when <a href="#get-v2-events">the stream</a> says something changed.',
    request: 'const m = await (await session.fetch(\'/v2/members\')).json();',
    response: '{ "site": "71c1…6a6c", "me": "0224…", "can_define": false,\n  "members": [\n    { "key": "2d63…edfa", "name": "Hana", "icon": { "mime": "image/webp", "data": "…" }, "role": "owner",\n      "about": { "bio": "I grow beans", "links": ["https://…"] },\n      "answers": { "2d63…:7": { "text": null, "choices": [0, 2] } },\n      "intentions": ["skills"] },\n    … ],\n  "questions": [\n    { "id": "2d63…:7", "text": "What do you offer?", "hint": null, "options": ["Tools", "Seeds", "Time"],\n      "multi": true, "max": 2, "free": false, "textMax": null, "retired": false, "tally": [3, 0, 5] } ] }'
  },
  {
    method: 'GET', path: '/v2/members/:site', origin: 'door', group: 'Reading', summary: 'A named community\'s members',
    doc: 'The same answer as <a href="#get-v2-members"><code>/v2/members</code></a>, for the community named by its id. A site\'s session may name only its own community; any other, or one the session does not reach, is <code>404 "no such Site in reach"</code>. WallFlowers\' own app reads it this way.',
    request: 'const m = await (await session.fetch(`/v2/members/${site}`)).json();',
    response: '(as /v2/members)'
  },
  {
    method: 'GET', path: '/v2/events', origin: 'door', group: 'Reading', summary: 'A stream that says when something changed',
    doc: 'A server-sent event stream: <code>event: changed</code> whenever something the session can read changes. It says only that, so read <code>/v2/graph</code> again, about 300 ms after the last one in a burst. Use <code>session.events(onChanged, onEnded)</code>, which carries the session (EventSource cannot); it opens the stream again by itself after a gap (2 s, doubling to a minute) and then says <code>changed</code> once with the version <code>\'\'</code>, meaning "caught up after a gap". A 401 ends it and calls <code>onEnded</code>. Several streams on one session all hear the changes; one per page is plenty.',
    request: 'const stop = session.events(onChanged, onEnded);   // stop() closes it',
    response: 'event: changed\ndata: 1790791234567\n\n'
  },
  {
    method: 'GET', path: '/v2/icd', origin: 'door', group: 'The model', summary: 'The data model',
    doc: 'The data model the Door runs: every kind, every op, its arguments, and who may write it (the field <code>ego</code>: owner or member). No session needed. <a href="model.html">The model</a> is this document, drawn.',
    request: 'const icd = await (await fetch(\'{{DOOR}}/v2/icd\')).json();',
    response: '{ "version": "{{ICD}}", "kinds": { "forum": { "ops": { "forum.post": { "args": { "text": { "type": "string", "required": true }, … }, "ego": "member" } } } }, "facets": { … } }'
  },
  {
    method: 'GET', path: '/v2/icd/:hash', origin: 'door', group: 'The model', summary: 'The data model by its hash, cached for good',
    doc: 'The same document by its content hash: it never changes, so it is served to be cached for good. The Door\'s own page names the current hash.'
  },
  {
    method: 'GET', path: '/v2/kinds', origin: 'door', group: 'The model', summary: 'Which kinds this session may create',
    doc: 'Each kind, and whether this session may create one (<code>mintable</code>), with the reason when not.',
    response: '[ { "kind": "forum", "mintable": true }, { "kind": "group", "mintable": true }, … ]'
  },
  {
    method: 'GET', path: '/v2/draft/:kind', origin: 'door', group: 'The model', summary: 'Which fields create an object of a kind',
    doc: 'The draft fields <code>POST /v2/mint</code> takes for a kind, and which argument of the kind\'s profile op each fills. A room takes only <code>name</code>.',
    request: 'const d = await (await session.fetch(\'/v2/draft/forum\')).json();',
    response: '{ "kind": "forum", "mintable": true, "name_only": true, "fields": [ … ] }'
  },
  /* ── writing ── */
  {
    method: 'POST', path: '/v2/join', origin: 'door', group: 'Writing', summary: 'Join the community with a claim',
    doc: 'A claim from your community\'s kiosk or QR code (<code>v1.…</code>) joins the signed-in member to the community and to its rooms: those marked for the claim\'s choice or, if no room is marked for it, every unmarked room. WallFlowers sends a scanned claim to your registered home with the claim in the address\'s fragment, <code>#join=…</code>, which no request carries: read it there, sign in if the person is not signed in yet (one WallFlowers page, in your community\'s look: a new member names themselves and makes a passkey there, and comes straight back), then send it here. The claim is a single-use way in, so: take it off the address in your page\'s first script, an inline script in <code>&lt;head&gt;</code> before any other (<code>history.replaceState(history.state, \'\', location.pathname + location.search)</code>, keeping <code>history.state</code>, which frameworks use); keep it in <code>sessionStorage</code> while the person signs in, never in a URL of yours and never in a log; send it only in this request\'s body. With a strict Content-Security-Policy, allow that inline script by its <code>sha256</code>. Only a claim for your own community is accepted (<code>403 "a claim for another Site"</code>). <code>unjoined</code> lists rooms not added yet: send the same claim once more a few seconds later; nothing is spent twice. A refusal is WallFlowers\' sentence: show it.',
    request: 'await session.fetch(\'/v2/join\', { method: \'POST\', body: JSON.stringify({ claim }) });',
    response: '{ "admitted": true, "site": "<community id>", "c": "resources", "a": null, "fallback": null,\n  "rooms": [ … ], "unjoined": [], "home": "https://your.site/…" }'
  },
  {
    method: 'POST', path: '/v2/apply', origin: 'door', group: 'Writing', summary: 'Make one change',
    doc: 'One op on one object, as the member. The op and its arguments are the model\'s (<a href="model.html">The model</a>); arguments are text or integers, and <code>gen</code> is added by WallFlowers, never sent. Keys are 64 hex characters. Read the graph again after it answers.',
    request: 'await session.fetch(\'/v2/apply\', { method: \'POST\', body: JSON.stringify({\n  object: roomId, op: \'forum.post\', args: { text: \'Hello\', ts: Date.now() } }) });',
    response: '{ "delta": "…" }',
    errors: [['400 "… is outside this site\'s scope"', 'The object is not in your session\'s reach. Don\'t offer it.'], ['400 · 403', 'Refused by the model: not the member\'s to write, or an argument the op does not take. Show the sentence.']]
  },
  {
    method: 'POST', path: '/v2/batch', origin: 'door', group: 'Writing', summary: 'Make several changes in order',
    doc: 'Between 1 and {{STEPS}} steps, applied in order. A later step names an object an earlier step created as <code>{"$step": i}</code>. If a step is refused the batch stops there, and what was made stands: retry only the steps that were not made.',
    request: '{ "steps": [\n  { "do": "mint",  "kind": "forum", "draft": { "name": "Issue 73" } },\n  { "do": "apply", "object": "<community>", "op": "base.setPart",\n    "args": { "part": { "$step": 0 }, "role": "room", "at": 1790000000000 } },\n  { "do": "apply", "object": { "$step": 0 }, "op": "base.setParent",\n    "args": { "parent": "<community>", "role": "room", "at": 1790000000000 } } ] }',
    response: '200 { "made": [ … ] }\n422 { "made": [ … ], "refused": { "step": 1, "why": "…" } }',
    errors: [['422', 'A step was refused: <code>refused.step</code> and <code>refused.why</code>; the earlier ones in <code>made</code> stand.'], ['400 "a batch is 1 to 16 steps, not …"', 'Split it.']],
    limits: 'Minting several objects in one batch is much faster than one awaited mint after another (see <a href="#post-v2-mint">mint</a>).'
  },
  {
    method: 'POST', path: '/v2/mint', origin: 'door', group: 'Writing', summary: 'Create one object',
    doc: 'Creates an object of a kind from its draft fields (<code>GET /v2/draft/:kind</code> lists them; an unknown field is refused). The session owns what it creates and reaches it until the session ends; to make it part of the community, link it in the same <a href="#post-v2-batch">batch</a>.<div class="note warn"><p><strong>Creating many objects:</strong> each mint costs more as the account holds more objects: about 20 ms for a new account, about 250 ms by its tenth, 500 ms and more by its fiftieth. Make many objects in one <code>POST /v2/batch</code>, or send the mints together without awaiting each; never one awaited mint after another. Posting a message (<code>POST /v2/apply</code>) stays fast.</p></div>',
    request: '{ "kind": "forum", "draft": { "name": "Issue 73" } }',
    response: '{ "object_id": "…" }'
  },
  {
    method: 'POST', path: '/v2/site/address', origin: 'door', group: 'Writing', summary: 'Claim the community\'s address on wallflowers.io',
    doc: 'Claims <code>wallflowers.io/&lt;slug&gt;</code> for the community\'s page (its Host). The owner only, and most communities get theirs at sign-up. A taken or reserved address is refused with WallFlowers\' reason.',
    request: '{ "slug": "mill-road", "host": "<the community\'s page id>" }',
    response: '{ "slug": "mill-road", "host": "…" }'
  },
  {
    method: 'GET', path: '/v2/resources.js', origin: 'door', group: 'Writing', summary: 'The Resources editor',
    doc: 'WallFlowers\' editor for a community\'s resources: write, preview and publish a post, with pictures and a PDF. Load it pinned by its integrity hash, after the sign-in script; it defines <code>WallFlowersResources.mount</code> (see <a href="resources.html">The Resources editor</a>). Its hash changes only with a Door release.',
    request: '<script src="{{DOOR}}/v2/resources.js"\n        integrity="{{RESOURCES_SRI}}"\n        crossorigin="anonymous"></script>',
    response: 'text/javascript'
  },
  /* ── the public page (the Arc, no session) ── */
  {
    method: 'GET', path: '/v1/face/:slug', origin: 'arc', group: 'The public page (no session)', summary: 'The community\'s public page',
    doc: 'The community\'s public page, as served at <code>wallflowers.io/&lt;slug&gt;</code>: HTML, for anyone.',
    request: 'GET {{ARC}}/v1/face/mill-road',
    errors: [['404 "no such page"', 'No published page at that address.']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/items', origin: 'arc', group: 'The public page (no session)', summary: 'Its events and resources, as data',
    doc: 'What the public page draws, as data, readable from any origin: the community\'s published items by key, <code>event:&lt;id&gt;</code> (<code>title</code>, <code>startMs</code>, <code>endMs</code>, <code>venue</code>, <code>descriptor</code>) and <code>post:&lt;id&gt;</code>, each with its op\'s arguments by the model\'s names and <code>at</code>. This is how a site shows events and resources, which a member\'s session does not reach. Cached briefly: read it on your server and keep it a minute. Read it when the page loads, and again when something changes (<code>session.events</code>, on a signed-in page); never poll it faster than once every 30 s, and never in a loop. {{PUBLISHED_ROUTE}}',
    request: 'const { items } = await (await fetch(\'{{ARC}}/v1/face/mill-road/items\')).json();',
    response: '{ "items": { "event:3c1f…": { "title": "Open day", "startMs": 1790607600000, "endMs": 1790693940000, "venue": "The allotments, Mill Road", "at": … },\n             "post:9a0e…": { "title": "Growing guide", "form": "pdf", "link": "https://…", "at": … } } }',
    errors: [['404 "no such page"', 'No published page at that address.']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/m/:slot', origin: 'arc', group: 'The public page (no session)', summary: 'One of its pictures',
    doc: 'One of the community\'s pictures: <code>mark</code>, <code>cover</code>, <code>logo</code> or <code>wallpaper</code> (PNG, JPEG or WebP). Cached briefly: a new picture replaces a slot in place.',
    request: '<img src="{{ARC}}/v1/face/mill-road/m/mark" alt="">',
    errors: [['404 "no such page"', 'No such community, or no picture in that slot.']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/post/:id', origin: 'arc', group: 'The public page (no session)', summary: 'A resource\'s public page',
    doc: 'One published resource, as served at <code>wallflowers.io/&lt;slug&gt;/post/&lt;id&gt;</code>: its title, excerpt, whole body, pictures and PDF, in the community\'s look. HTML, for anyone. <code>id</code> is the <code>&lt;id&gt;</code> of its <code>post:&lt;id&gt;</code> item.',
    request: '<a href="https://wallflowers.io/mill-road/post/9a0e…">Growing guide</a>',
    errors: [['404 "no such page"', 'No such community, or no such published resource.']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/post/:id/document', origin: 'arc', group: 'The public page (no session)', summary: 'A resource\'s PDF',
    doc: 'The resource\'s PDF, as a file to save (<code>Content-Disposition: attachment</code>), when the public page holds it: only a PDF of 16 KB or less is (<a href="resources.html#in-this-release">The Resources editor</a>). A larger one answers <code>404</code> here; members read it in WallFlowers.',
    response: 'application/pdf',
    errors: [['404 "no such page"', 'No such resource, or its PDF isn\'t held on the public page (over 16 KB).']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/brand', origin: 'arc', group: 'The public page (no session)', summary: 'Its name and look, for a sign-in window',
    doc: 'The community\'s name, its look as CSS custom properties, and whether it has a mark: what WallFlowers\' sign-in window wears for it. Public; a site may use it to match its own look.',
    response: '{ "name": "Mill Road Allotments", "style": "--bg:#243819;--ink:#F3F1E4;…", "mark": true }',
    errors: [['404 "no such page"', 'No published page at that address.']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/door', origin: 'arc', group: 'The public page (no session)', summary: 'Its join page',
    doc: 'The community\'s join page, in its look: where a visitor asks to join. HTML. (The path says <code>door</code>; it is not the Door, WallFlowers\' API.)',
    errors: [['404 "no such page"', 'No published page at that address.']]
  },
  {
    method: 'GET', path: '/v1/face/:slug/escape', origin: 'arc', group: 'The public page (no session)', summary: 'Its way out of an app\'s own browser',
    doc: 'A page with the community\'s name and mark that asks the visitor to open the link in their real browser. In-app browsers (Instagram and similar) cannot make a passkey, so send them here. HTML.',
    errors: [['404 "no such page"', 'No published page at that address.']]
  }
];

/* What each audience is, for the routes a site does not call (listed from the inventory). */
export const AUDIENCES = {
  window: 'WallFlowers\' sign-in window: the passkey ceremony, run on app.wallflowers.io. Your page sends people there with <code>WallFlowers.signIn</code>.',
  webapp: 'WallFlowers\' own app. A site\'s session is refused on these (adding people, a person\'s account).',
  internal: 'The Door\'s own processes. Not reachable as an API.',
  asset: 'The sign-in window\'s own files.',
  arc: 'The Arc\'s services behind WallFlowers\' apps (relay, membership, console). Not an API for sites.'
};
