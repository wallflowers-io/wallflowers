# WallFlowers Community API: for AI coding agents

You are building a web app that signs a person in with WallFlowers and reads and writes their community.
Read the linked pages before writing code; they are generated from the running system and are the truth.

- Docs: https://docs.wallflowers.io/  (quickstart: https://docs.wallflowers.io/quickstart.html, routes: https://docs.wallflowers.io/routes.html, model: https://docs.wallflowers.io/model.html)
- API origin: https://app.wallflowers.io  (every call is `session.fetch('/v2/...')`, JSON in and out)
- Sign-in script: `<script src="https://app.wallflowers.io/v2/signin.js" integrity="sha384-tlelF9i6GEcF6TboulAVcsSZoIVw6WnnBFG/cvUqcASGz3MuwBrF8jtAh5f5RfW/" crossorigin="anonymous"></script>`
- Machine-readable routes: https://docs.wallflowers.io/routes.json · the data model: https://app.wallflowers.io/v2/icd (no session needed; from a browser, only on an origin WallFlowers allows)
- Baseline these docs describe: model 2.3.1 (sha256 a71fb32573731ca2de99e637834c279fd44d661a947b9542c2d3b088dd10cb13), the Door at e1061bb2; machine-readable: https://docs.wallflowers.io/baseline.json
- A working starter to begin from (two files, no build): https://docs.wallflowers.io/starter/index.html and https://docs.wallflowers.io/starter/app.js

## Do

1. On the person's own machine, use the shared client `hackathon-local`: nothing to register. Callbacks, matched exactly: `http://localhost:<port>/callback` on 3000–3009; `http://localhost:<port>/` on 3000–3009, 5173–5175, 8000, 8080 (the same on 127.0.0.1). The person must own or administer a community first (made at https://wallflowers.io/signup); at sign-in they pick it, and the session reaches only it. A site on the web gets registered (https://docs.wallflowers.io/register.html).
2. Load the sign-in script once per page with the integrity attribute above. Never copy it, bundle it, or load it unpinned.
3. On the callback page call `WallFlowers.finish({client, callback})`; on every other page `WallFlowers.current({client})`. `null` means signed out: show a button that calls `WallFlowers.signIn({client, callback})`.
4. Read everything with `GET /v2/graph` and draw from it. Write one change with `POST /v2/apply {object, op, args}`, several in order with `POST /v2/batch`.
5. Open one change stream per page (`session.events`), and on `changed` re-read `/v2/graph` about 300 ms after the last one. Re-read after your own writes too.
6. Treat a `401`, or a `400` saying `not signed in`, as signed out: show your Sign in button again (it calls `WallFlowers.signIn`). If you send the person to sign in yourself, at most once per page view, and never in a loop. Show any other refusal's sentence to the person; do not parse it.
7. Sign out every session you open: `session.signOut()` (`POST /v2/signout`) when done, and in scripts and tests at exit. An abandoned session is held until it has been idle for 15 minutes, and once WallFlowers holds 112 sessions, everyone's sign-in is refused (`503 the Door is at its session limit`).
8. Name people from `profiles` on the community's or the room's view. Never show a key as a name; say "New member".
9. Take arguments from the model: `GET /v2/icd` (or https://docs.wallflowers.io/model.html). Arguments are text or integers. Never send `gen`: the model lists it as required, and WallFlowers fills it in.
10. The community is `session.site`; its rooms are its view's `parts` with role `room`. A room's messages are its view's `messages` (`author`, `text`, `ts`, `gen`, …: https://docs.wallflowers.io/views.html); a message has no length of its own beyond the request body (https://docs.wallflowers.io/limits.html). Events and resources your site's session created are in its reach, in `/v2/graph`; others are not. The community's published ones are at `https://arc.wallflowers.io/v1/face/<address>/items` (no session); a community is published as it is made at signup (one made before 1 October, 14:00 KST, may not be yet, and answers 404 there). `<address>` (`:slug` on the routes page) is the community's address, as in wallflowers.io/<address>. A member's session can't read it: put it in your app's configuration, as signup showed it or as it was chosen at Register. Read it on load and when something changes, never faster than once every 30 s, never in a loop.
11. Creating many objects: one `POST /v2/batch` (up to 16 steps), or send them concurrently. Never one awaited mint after another: each mint gets slower as the account holds more.
12. Serve your page from the exact origin and callback you use, e.g. `python3 -m http.server 5173 --bind 127.0.0.1` (the bind keeps it to this machine), and open exactly `http://localhost:5173/`.

## Don't

- Don't store or send passwords, passkeys or keys: WallFlowers' window does the passkey; your page never sees it.
- Don't poll faster than every few seconds, and don't open a stream per component.
- Don't put third-party scripts on pages where a member is signed in: anything on the page can use the session.
- Don't guess routes. If it is not on https://docs.wallflowers.io/routes.html, it is not for sites.
