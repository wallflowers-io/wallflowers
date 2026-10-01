#!/usr/bin/env python3
"""The keyholder's origin, with just enough auth surface to be exercised.

WHY THIS EXISTS AT ALL. `python3 -m http.server` answers every POST with 501, so
`POST /auth/signout` could not be reached, and the one thing sign-out has to
prove -- that the session is gone afterwards -- could not be observed. A static
file can answer "who is signed in"; it cannot answer "stop being signed in".

WHAT IT WILL NOT DO ANY MORE. An earlier version answered `GET /signin` by
flipping a boolean and redirecting back, so clicking Sign in put you straight in
as Ada. That was worse than having no sign-in at all: it made a door that has
never been walked look like a door that works, and the three real ones -- Apple,
AI Passport, a Kenjin passkey -- were never touched. A harness may decline to
implement something. It must not pretend the thing it declined is done.

SO WHERE IS SIGN-IN. Not reachable from here, and not because of missing effort:

  · Apple and Passport refuse http and refuse localhost return URLs, so
    /auth/{provider}/start cannot come back to localhost:8103 at all.
  · The passkey RP ID is `kenjin.cc`, the registrable domain -- deliberately, so
    a credential made in dev is the same credential in production. An origin of
    localhost:8103 cannot use one, and minting one under RP ID `localhost` would
    strand it the moment it mattered.

Which is exactly why site/auth/dev/ runs local development behind a real
HTTPS hostname through cloudflared instead of on a port. To drive the doors,
serve the keyholder from that hostname -- production puts it on kenjin.cc, the
same origin as the auth service, which is what makes the cookie and the RP ID
line up in the first place. Set AUTH_UPSTREAM to proxy /auth to a service that
is already running; leave it unset and every endpoint that needs one says so.

WHAT REMAINS HONEST HERE. A session fixture, which is a session you already had
rather than one you just obtained -- the difference between finding a cookie in
the jar and being issued one -- and a sign-out that really removes it.

ONE PROCESS IS ONE DEVICE. run.sh starts this twice, on 8103 and 8104, and the
two are separate origins with separate IndexedDB and separate sessions. That is
not a limitation of the harness; it is what two devices are.
"""
import json
import os
import re
import sys
import urllib.error
import urllib.request
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer

# A real auth service to hand /auth/ to, if one is running. Unset by default,
# because the default has to be the state that tells the truth about itself.
UPSTREAM = os.environ.get("AUTH_UPSTREAM", "").rstrip("/")

# THE SESSION FIXTURE IS GONE, along with the session. This used to read
# auth/session at startup and answer GET /auth/session from it, with a
# `signed_in` flag that POST /auth/signout could clear. The service has no such
# route any more and neither does the keyholder: what signs a browser in is a
# seed in ITS OWN IndexedDB, which no harness can fake and none should try to.
# Point AUTH_UPSTREAM at a running service and the real doors work.

_HOP = {"transfer-encoding", "content-length", "connection", "keep-alive"}

_SECURE = re.compile(r";\s*Secure", re.I)


def _drop_secure(value: str) -> str:
    """Strip Secure from a Set-Cookie on its way to http://localhost.

    THE AUTH SERVICE IS RIGHT TO SET IT and is not changed. Its own dev proxy
    refuses to do this and says why: local development runs behind real HTTPS
    through cloudflared, so faking the flag there would only hide a class of bug
    that production would then hit for real.

    Here there is no HTTPS and cannot be one worth having -- a self-signed cert
    on localhost is a trust prompt or a keychain change, neither of which
    belongs in a smoke run. So the choice is between stripping this one
    attribute and not exercising the sign-in path in a browser at all, and the
    path is what was never being run. It is stripped ONLY here, only on the way
    to a loopback address, and the service continues to emit it.
    """
    return _SECURE.sub("", value)

MISSING = (
    "<!doctype html><meta charset=utf-8><title>No sign-in here</title>"
    "<style>body{font:14px/1.6 ui-sans-serif,-apple-system,sans-serif;color:#111;"
    "background:#fbfbfa;margin:0;display:flex;min-height:100vh;align-items:center;"
    "justify-content:center}div{max-width:30rem;padding:2rem}h1{font-size:11px;"
    "letter-spacing:.34em;text-transform:uppercase;color:#8a8a8a;font-weight:500}"
    "code{background:#f0efed;padding:1px 4px;border-radius:2px}p{color:#555}</style>"
    "<div><h1>Pacific &middot; harness</h1>"
    "<p><b>%s</b> needs the auth service, and this is <code>serve.py</code>.</p>"
    "<p>The three doors cannot complete from <code>localhost</code> whatever is "
    "running: Apple and Passport refuse http and localhost return URLs, and the "
    "passkey RP ID is <code>kenjin.cc</code>. Local development puts the whole "
    "origin behind a real HTTPS hostname for that reason &mdash; see "
    "<code>site/auth/dev/</code>.</p>"
    "<p>To proxy <code>/auth</code> to a service that is already running, start "
    "this with <code>AUTH_UPSTREAM=https://&hellip;</code>. To get the session "
    "fixture back after signing out, restart the harness.</p></div>"
)


# ── WHAT MAY BE FETCHED, BY NAME ─────────────────────────────────────────────
#
# THE KEYHOLDER IS AN ORIGIN, AND AN ORIGIN IS A PUBLIC SURFACE. It was written
# when this only ever ran on localhost, so it served its whole directory —
# which was harmless right up until the moment it was put behind a tunnel at
# `keys.wallflowers.io` on 16 Sep 2026 and the directory became the open
# internet. `serve.py` itself, `world.test.cjs`, `world.site-group.test.cjs` and
# `_link-frame.html` were all fetchable. None of them holds a secret — they are
# the reason this is a list and not a scramble — but a dev server publishing its
# own source and its test fixtures is a directory listing of how the thing is
# built, handed to anyone who asks.
#
# SO: DENY BY DEFAULT, the same rule `keyholder.js::permitted` keeps one layer
# up. A file is served because it is named here, and a new browser asset has to
# be added deliberately. That is the point — the failure mode of a list is a
# 404 on something you forgot, which you see immediately; the failure mode of a
# directory is a file you never meant to publish, which you do not see at all.
PUBLIC = frozenset({
    "/", "/index.html",          # the door, and the frame that renders nothing
    "/keyholder.js",             # the port protocol, the device key, sync
    "/pacific-account.js",       # seed, identity, passkey, wrap, challenge
    "/world.js",                 # fold -> world; the interior borrows it
    "/wallflowers-ops.js",       # the op table GENERATED from the ICD; ops.capabilities reads it
    "/_authn-shim.js",           # the WebAuthn shim the door loads
    "/_link-frame.html",         # device linking, opened as a frame
    "/core/core_wasm_bg.wasm",   # the core
    "/favicon.ico",
})


class Handler(SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    # ---- helpers ----------------------------------------------------------
    def _json(self, code, body):
        raw = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        # No caching, or a 200 session survives the 401 that replaced it.
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(raw)

    def _missing(self, what):
        raw = (MISSING % what).encode()
        self.send_response(503)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(raw)

    def _proxy(self, body=None):
        """Hand it to the real service verbatim, cookies and all.

        The browser only ever sees this origin, so a Set-Cookie from upstream
        lands on localhost:8103 -- which is enough for session and sign-out. It
        is NOT enough for the doors: a provider redirect comes back to whatever
        redirect_uri the service was configured with, which is not here.
        """
        headers = {k: v for k, v in self.headers.items()
                   if k.lower() not in ("host", "accept-encoding")}
        headers["X-Forwarded-Proto"] = "https"
        req = urllib.request.Request(UPSTREAM + self.path, data=body,
                                     method=self.command, headers=headers)
        try:
            r = urllib.request.urlopen(req, timeout=30)
            status, hdrs, payload = r.status, r.headers, r.read()
        except urllib.error.HTTPError as e:
            status, hdrs, payload = e.code, e.headers, e.read()
        except urllib.error.URLError as e:
            return self._json(502, {"error": "upstream_unreachable",
                                    "upstream": UPSTREAM, "detail": str(e.reason)})
        self.send_response(status)
        for k, v in hdrs.items():
            if k.lower() in _HOP:
                continue
            if k.lower() == "set-cookie":
                v = _drop_secure(v)
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _is_auth(self):
        p = self.path.split("?")[0]
        return p.startswith("/auth/") or p == "/signin" or p.startswith("/signin/")

    # ---- routes -----------------------------------------------------------
    def do_GET(self):
        p = self.path.split("?")[0]

        if UPSTREAM and self._is_auth():
            return self._proxy()

        # No fixture, no boolean flip. There is no way into an account from this
        # process, because there is no way to earn one here.
        if self._is_auth():
            return self._missing(p)

        if p not in PUBLIC:
            # 404 and nothing else — not 403. A refusal that distinguishes
            # "exists but you may not have it" from "does not exist" tells a
            # stranger which files are here, which is most of what they wanted.
            self.send_error(404, "Not Found")
            return

        return SimpleHTTPRequestHandler.do_GET(self)

    def _with_body(self):
        p = self.path.split("?")[0]
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else None

        if UPSTREAM and self._is_auth():
            return self._proxy(body)

        if self._is_auth():
            return self._missing(p)
        return self._json(404, {"error": "not_found"})

    # PUT is how an account stores its two artefacts -- the wrap and the
    # history. Without it SimpleHTTPRequestHandler answers 501 with an HTML
    # page, and registering an account fails at the last step with a JSON parse
    # error that says nothing about the cause.
    do_POST = _with_body
    do_PUT = _with_body
    do_DELETE = _with_body

    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8103
    # FIXTURE went with the session fixture (see the note at the top) and the
    # last reference to it did not -- so this line raised NameError at startup
    # and the keyholder had stopped coming up at all. There are two states now
    # and neither of them is a fixture.
    where = ("proxying /auth to " + UPSTREAM) if UPSTREAM \
            else "no AUTH_UPSTREAM -- signed out, and the doors cannot open"
    sys.stderr.write("keyholder :%d — %s\n" % (port, where))
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
