# app/web — the web client

WallFlowers on the web is a browser talking to the Door, a hosted node. The browser
holds no key: the passkey's PRF is sealed to the Door's session process, the Door runs
the core, and the page holds a session cookie. The Door's API is the Community API
(docs.wallflowers.io).

| | |
|---|---|
| `door/` | the sign-in window and its scripts, served by the Door from its binary |
| `webapp/` | the webapp, deployed by the `app` route (`webapp/README.md`) |
| `resources/` | the editor, markdown and PDF resources the Door serves |
| `shared/` | `wallflowers-ops.js`, generated from the ICD (`core/coordination/gen-web-ops.py`); the head recovery contract |
| `graph/` | a diagnostic graph explorer (`graph/README.md`); its data routes were removed under SEC-5 |
| `smoke/` | `e2e.py`: the Arc account protocol end to end, against a running Arc |
| `build-wasm.sh` | builds core-wasm into `docs/core/` for the graph explorer and the core-wasm smoke tests |

The keyholder, the embed (`pacific.js`), the browser account client and their test
pages were retired on 28 Oct 2026: they held device keys in the browser.
