#!/usr/bin/env python3
"""dev-serve.py — play/ on loopback, the two things a static host must do for it.

    scripts/dev-serve.py [port]        default 8931

1. /play answers play.html (try_files $uri $uri.html).
2. Range requests are answered. A .pmtiles archive is read a few kilobytes at
   a time, and python's own http.server sends the whole file whatever is asked.

Loopback only: for looking at a page.
"""
import os
import re
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

SITE = Path(__file__).resolve().parent.parent
RANGE = re.compile(r"bytes=(\d*)-(\d*)$")


class Handler(SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    extensions_map = {**SimpleHTTPRequestHandler.extensions_map, ".mjs": "text/javascript", ".pmtiles": "application/octet-stream"}

    def __init__(self, *a, **k):
        super().__init__(*a, directory=str(SITE), **k)

    def translate_path(self, path):
        p = super().translate_path(path)
        return p + ".html" if not os.path.exists(p) and os.path.isfile(p + ".html") else p

    def send_head(self):
        m = RANGE.match(self.headers.get("Range", ""))
        path = self.translate_path(self.path)
        if not m or not os.path.isfile(path):
            return super().send_head()
        size = os.path.getsize(path)
        if m.group(1):
            start, end = int(m.group(1)), min(int(m.group(2)) if m.group(2) else size - 1, size - 1)
        else:                                   # the last n bytes
            start, end = max(0, size - int(m.group(2) or 0)), size - 1
        if start > end:
            self.send_error(416)
            return None
        f = open(path, "rb")
        f.seek(start)
        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()
        self.left = end - start + 1
        return f

    def copyfile(self, source, out):
        left = getattr(self, "left", None)
        if left is None:
            return super().copyfile(source, out)
        self.left = None
        while left > 0:
            chunk = source.read(min(65536, left))
            if not chunk:
                break
            out.write(chunk)
            left -= len(chunk)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8931
    print(f"http://127.0.0.1:{port}/  ({SITE})")
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
