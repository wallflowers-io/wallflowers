#!/usr/bin/env python3
"""The webapp, and the website's /assets beside it — one origin, as in production.

    python3 serve.py [port]        default 8231, 127.0.0.1 only

The boards, their names and the scene are the website's (assets/tiles/tama/tama.js,
tiles/tile.js, tiles/boards.js, how/ceremony.js), loaded, never copied. The path to
them is derived from this file: webapp -> web -> app -> the product checkout -> the
workspace, then business/website/site.

No-cache on every answer, for the reason docs-serve.py gives: a heuristically cached
script runs yesterday's code against today's door.
"""
import functools, http.server, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SITE = os.path.abspath(os.path.join(HERE, "..", "..", "..", "..", "business", "website", "site"))


class Handler(http.server.SimpleHTTPRequestHandler):
    def translate_path(self, path):
        clean = path.split("?", 1)[0].split("#", 1)[0]
        if clean.startswith("/assets/"):
            return os.path.join(SITE, clean.lstrip("/"))
        return super().translate_path(path)

    def end_headers(self):
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()


if __name__ == "__main__":
    if not os.path.isdir(os.path.join(SITE, "assets")):
        sys.exit(f"no website assets at {SITE}/assets — is business/website checked out beside the product?")
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8231
    http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(Handler, directory=HERE)).serve_forever()
