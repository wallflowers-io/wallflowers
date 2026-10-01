# webapp — WallFlowers on the web

One page, one engine. Wide, it is four columns — sites, the site's panel, the pane, people
— laid out as /demo's Discord-shaped interior was. Narrow, the same engine is rail and pane,
with the site's panel and people as drawers. That narrow form is the mobile webapp.

```
app/door/run.sh         http://127.0.0.1:8233 — the door, and its relay
./serve.sh              http://127.0.0.1:8231
```

`/#signin` opens the door directly. `/#site=<object id>` opens the door and lands on that
site — where the embed's anchor points.

Test mobile in the iOS simulator's Safari, desktop in Chrome.

## Live, and more than one door

The door streams `/v1/events`: one event whenever what the graph would show changed — a
write here, or what its account's background sync (every 3 s) brought in from the relay.
The page refetches `/v1/graph` on each.

Each door node is a DEVICE of every person who signs in there (resumption.md A4): words it
has never seen restore the identity and `resume` it, taking a pool leaf in every object the
person is in. Two nodes, one relay:

```
app/door/run.sh                                                        # node A, owns the relay
DOOR_PORT=8234 DOOR_ROOT=/tmp/door-b DOOR_RELAY=ws://127.0.0.1:8788/v1/relay app/door/run.sh
```

`?door=http://127.0.0.1:8234` points the page at node B. Both nodes set a `door` cookie on
127.0.0.1, so one browser holds a session on one node at a time — use two browsers.

## Embedding

```html
<script src="https://…/webapp/embed.js" data-site="<group object id>"></script>
```

pacific.js's embedding, and nothing else of it: one `<div>` on `<body>`, a closed shadow
root, no layout taken, the host's CSS kept out (`embedded.html` is a hostile host to check
that). The mark is an anchor to `webapp/#site=<id>`, in a new tab — the door's cookie is
refused inside a third-party frame, so the interior does not open over the host.

## Check it on a simulator, not just headless

```
xcrun simctl boot "iPhone 17 Pro" && open -a Simulator
xcrun simctl openurl booted "http://127.0.0.1:8231/"
```

Headless has **no browser chrome**, so it cannot show what iOS Safari's floating toolbar
covers. The face, door and interior use **`svh`**, the viewport that is always visible.

## Where each object is drawn

`/v1/graph` arranged, never extended:

| | |
|---|---|
| the self record | spine index 0 — you, in the people panel |
| a site | a `group` no other object hosts — the rail |
| a channel | a `forum` a group is made of (`base.setPart`, role `room`) — the site's panel |
| comments | a `forum` any other object hosts — under that object |
| a connection | the door's `role: channel`; its DM is `role: chat` |
| anything else | a card in the feed, opening to its own view |

An object that will not fold is at the top of the feed, in the core's words.

The composer writes `forum.post` through `POST /v1/apply` — `Node::apply`, the one write
path. A refusal is shown under it, in the core's words.

## The browser holds a session and nothing else

Ruled 23 September 2026: the door (`app/door`) holds the seed and runs the native `Node`.
The words cross once, in `POST /v1/session {words}`; the door answers with an HttpOnly
cookie this page cannot read. A live cookie on load goes straight inside.

## What is real

- **The words door.** The door derives the key from the words. Known words open that
  persona's account process; unknown words restore one.
- **The interior.** `/v1/graph`: each object folded by `Node::object_view` in the door.
  A read syncs first.

## Not built yet

1. **The passkey.** The ceremony moves to the door; until then the button is disabled.
2. **Sign up.** Still the website's.
3. **Time.** `forum.post` carries only `text` in the ICD, so a message has no time to show.
4. **The head.** Resume runs without one (`resume(None)`): no auth service here stores it.

## Files

| | |
|---|---|
| `index.html` | the three screens, all CSS, `data-state` as the whole state machine |
| `webapp.js` | the door session, the model, the drawing |
| `embed.js` | the anchor a site embeds |
| `embedded.html` | a hostile host page for it |
| `serve.sh` | a static server on 8231 |
