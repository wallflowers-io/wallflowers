#!/usr/bin/env python3
"""A development server for app/web's tools (the graph explorer, graph/serve.sh).

`python3 -m http.server` with ONE change: every answer says `Cache-Control:
no-cache`. The plain server sends Last-Modified and nothing else, so a browser
caches heuristically and a page can keep running yesterday's wasm against a core
rebuilt a minute ago, with mismatches that look like bugs (thedoor, 22 Sep). no-cache still lets the browser keep a copy; it just asks first, and an
unchanged file costs a 304.

    python3 docs-serve.py <port> [dir]  serves ./docs (or <dir>), 127.0.0.1 only
"""
import functools, http.server, os, sys


class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8100
    here = os.path.dirname(os.path.abspath(__file__))
    root = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "docs")
    handler = functools.partial(NoCache, directory=root)
    http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
