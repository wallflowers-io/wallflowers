#!/usr/bin/env python3
"""object-store.py PORT DIR — object storage on this machine, answering as R2 does, for a local
relay's presigns (RELAY_R2_ENDPOINT=http://127.0.0.1:PORT). A PUT with If-None-Match: * creates
and is refused 412 over an object already there; a PUT without it (a hosted object's overwrite,
media.hosted) replaces it; a GET returns it or 404. Signatures are not checked: the relay's are
tested in the relay. For scripts' own runs only."""
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT, ROOT = int(sys.argv[1]), sys.argv[2]


def path_of(target: str) -> str:
    key = target.split("?", 1)[0].strip("/").replace("/", "_")
    return os.path.join(ROOT, key)


class Store(BaseHTTPRequestHandler):
    def do_PUT(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        p = path_of(self.path)
        if self.headers.get("If-None-Match") == "*" and os.path.exists(p):
            return self.answer(412)
        with open(p + ".part", "wb") as f:
            f.write(body)
        os.replace(p + ".part", p)
        self.answer(200)

    def do_GET(self):
        p = path_of(self.path)
        if not os.path.exists(p):
            return self.answer(404)
        with open(p, "rb") as f:
            self.answer(200, f.read())

    def answer(self, status, body=b""):
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


os.makedirs(ROOT, exist_ok=True)
ThreadingHTTPServer(("127.0.0.1", PORT), Store).serve_forever()
