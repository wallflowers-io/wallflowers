#!/usr/bin/env python3
"""backup-verify.py — K-48's restore check, on the Mac (srr/scmp.md § 11).

    backup-verify.py sets <box>                      the box's ticks, oldest first; * unfinished
    backup-verify.py check <box> <stamp|latest> -i <identity> [-o <dir>]
                                                     fetch a manifest and every object it names,
                                                     from whichever tick, decrypt them with the
                                                     identity, and hold each file to it: sha256,
                                                     size, a database's integrity_check and row
                                                     counts, a pg_dump's PGDMP header, the roles'
                                                     cluster dump header; -o keeps
                                                     the plaintext (0700)
    backup-verify.py pair <door-box> <stamp|latest> <arc-box>
                                                     the arc manifest a seal manifest restores
                                                     with: the newest one before it (SECURITY)

The sets come from R2 by curl's SigV4, with BACKUP_R2_ENDPOINT, BACKUP_R2_BUCKET,
BACKUP_R2_ACCESS_KEY_ID and BACKUP_R2_SECRET_ACCESS_KEY from the environment or from
BACKUP_ENV (a file of KEY=value lines); or, for a test, from the directory BACKUP_DEST_DIR.
Needs age on PATH. Exits nonzero on the first thing that does not hold.
"""
import hashlib
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

NAMES = ("BACKUP_R2_ENDPOINT", "BACKUP_R2_BUCKET", "BACKUP_R2_ACCESS_KEY_ID", "BACKUP_R2_SECRET_ACCESS_KEY")
STAMP = re.compile(r"^\d{8}T\d{6}Z$")
KEY = re.compile(r"^(?P<box>[^/]+)/(?P<stamp>\d{8}T\d{6}Z)/(?P<name>[^/]+)\.age$")


def fail(why: str) -> None:
    print(f"backup-verify: {why}", file=sys.stderr)
    sys.exit(1)


def config() -> dict:
    cfg = {k: os.environ[k] for k in NAMES if os.environ.get(k)}
    env = os.environ.get("BACKUP_ENV")
    if env:
        for line in Path(env).read_text().splitlines():
            k, _, v = line.partition("=")
            if k in NAMES and k not in cfg:
                cfg[k] = v.strip().strip('"').strip("'")
    missing = [k for k in NAMES if not cfg.get(k)]
    if missing:
        fail("no " + ", ".join(missing) + " (the environment, or BACKUP_ENV)")
    return cfg


class Store:
    """Where the sets are: a directory (BACKUP_DEST_DIR), or R2."""

    def __init__(self) -> None:
        self.dir = os.environ.get("BACKUP_DEST_DIR")
        self.cfg = None if self.dir else config()

    def _curl(self, url: str, out: str | None = None) -> bytes:
        c = self.cfg
        args = ["curl", "-fsS", "--retry", "3", "-K", "-", "--aws-sigv4", "aws:amz:auto:s3", url]
        if out:
            args += ["-o", out]
        r = subprocess.run(args, input=f'user = "{c["BACKUP_R2_ACCESS_KEY_ID"]}:{c["BACKUP_R2_SECRET_ACCESS_KEY"]}"\n'.encode(),
                           capture_output=True)
        if r.returncode:
            fail(f"R2 answered {r.returncode} for {url.split('?')[0]}: {r.stderr.decode()[-300:]}")
        return r.stdout

    def keys(self, prefix: str) -> list[str]:
        if self.dir:
            root = Path(self.dir)
            return sorted(str(p.relative_to(root)) for p in (root / prefix).rglob("*") if p.is_file()) if (root / prefix).exists() else []
        base = f'{self.cfg["BACKUP_R2_ENDPOINT"]}/{self.cfg["BACKUP_R2_BUCKET"]}'
        out, token = [], None
        while True:
            q = f"?list-type=2&prefix={prefix}" + (f"&continuation-token={token}" if token else "")
            tree = ET.fromstring(self._curl(base + q))
            ns = {"s": tree.tag.split("}")[0].strip("{")} if tree.tag.startswith("{") else {}
            find = (lambda e, t: e.findall(f"s:{t}", ns)) if ns else (lambda e, t: e.findall(t))
            out += [k.text for c in find(tree, "Contents") for k in find(c, "Key")]
            more = [e.text for e in find(tree, "IsTruncated")]
            nxt = [e.text for e in find(tree, "NextContinuationToken")]
            if more and more[0] == "true" and nxt:
                from urllib.parse import quote
                token = quote(nxt[0], safe="")
            else:
                return sorted(out)

    def fetch(self, key: str, to: str) -> None:
        if self.dir:
            src = Path(self.dir) / key
            if not src.is_file():
                fail(f"no {key}")
            Path(to).write_bytes(src.read_bytes())
        else:
            self._curl(f'{self.cfg["BACKUP_R2_ENDPOINT"]}/{self.cfg["BACKUP_R2_BUCKET"]}/{key}', out=to)


def sets(store: Store, box: str) -> dict[str, bool]:
    """Each tick of the box, and whether its manifest is there (the tick finished)."""
    found: dict[str, bool] = {}
    for k in store.keys(f"{box}/"):
        parts = k.split("/")
        if len(parts) == 3 and STAMP.match(parts[1]):
            found[parts[1]] = found.get(parts[1], False) or parts[2] == "manifest.json.age"
    return dict(sorted(found.items()))


def finished(store: Store, box: str, stamp: str) -> str:
    s = sets(store, box)
    done = [t for t, ok in s.items() if ok]
    if stamp == "latest":
        if not done:
            fail(f"{box} has no manifest")
        return done[-1]
    if stamp not in s:
        fail(f"{box} has no tick {stamp}")
    if not s[stamp]:
        fail(f"{box}/{stamp} has no manifest: the tick did not finish")
    return stamp


def decrypt(src: str, dst: str, identity: str) -> None:
    r = subprocess.run(["age", "-d", "-i", identity, "-o", dst, src], capture_output=True)
    if r.returncode:
        fail(f"{Path(src).name} would not decrypt: {r.stderr.decode().strip()[-200:]}")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def check(store: Store, box: str, stamp: str, identity: str, keep: str | None) -> None:
    stamp = finished(store, box, stamp)
    os.umask(0o077)
    work = Path(keep) if keep else Path(tempfile.mkdtemp(prefix="backup-verify."))
    work.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        store.fetch(f"{box}/{stamp}/manifest.json.age", str(work / "manifest.json.age"))
        decrypt(str(work / "manifest.json.age"), str(work / "manifest.json"), identity)
        m = json.loads((work / "manifest.json").read_text())
        if (m.get("v"), m.get("box"), m.get("stamp")) != (2, box, stamp):
            fail(f"the manifest is v{m.get('v')}, {m.get('box')}/{m.get('stamp')}'s, not v2, {box}/{stamp}'s")
        names = [e["name"] for e in m["files"]]
        if len(set(names)) != len(names):
            fail(f"the manifest names a store twice: {sorted(names)}")
        # Each store's newest object at or before the manifest: this box's, its own name, no later tick.
        ticks = set()
        for e in m["files"]:
            k = KEY.match(e.get("key", ""))
            if not k or (k["box"], k["name"]) != (box, e["name"]) or k["stamp"] > stamp or e["name"].startswith("."):
                fail(f"the manifest names {e['name']!r} at {e.get('key')!r}")
            ticks.add(k["stamp"])
        stray = set(store.keys(f"{box}/{stamp}/")) - {e["key"] for e in m["files"]} - {f"{box}/{stamp}/manifest.json.age"}
        if stray:
            fail(f"objects the manifest does not name: {sorted(stray)}")
        for e in m["files"]:
            name = e["name"]
            store.fetch(e["key"], str(work / f"{name}.age"))
            decrypt(str(work / f"{name}.age"), str(work / name), identity)
            (work / f"{name}.age").unlink()
            p = work / name
            if p.stat().st_size != e["size"] or sha256(p) != e["sha256"]:
                fail(f"{name}: not the file the manifest names (size or sha256)")
            what = "sha256"
            if e.get("format") == "pg_dump -Fc":
                with open(p, "rb") as f:
                    if f.read(5) != b"PGDMP":
                        fail(f"{name}: not a pg_dump archive (no PGDMP header)")
                what = f"sha256, a pg_dump archive at WAL {e.get('lsn', '?')}"
            if e.get("format") == "pg_dumpall --globals-only":
                with open(p, "rb") as f:
                    if not f.read(64).startswith(b"--\n-- PostgreSQL database cluster dump"):
                        fail(f"{name}: not pg_dumpall's globals (no cluster dump header)")
                what = "sha256, the cluster's roles (pg_dumpall --globals-only)"
            if "rows" in e:
                c = sqlite3.connect(f"file:{p}?mode=ro", uri=True)
                ok = c.execute("PRAGMA integrity_check").fetchone()[0]
                if ok != "ok" or ok != e["integrity"]:
                    fail(f"{name}: integrity_check {ok!r}, the manifest {e['integrity']!r}")
                rows = {t: c.execute('SELECT count(*) FROM "%s"' % t.replace('"', '""')).fetchone()[0] for t in e["rows"]}
                c.close()
                if rows != e["rows"]:
                    fail(f"{name}: rows {rows}, the manifest {e['rows']}")
                what = f"sha256, integrity ok, {sum(rows.values())} rows in {len(rows)} tables"
            print(f"  ok  {name}  {e['size']} bytes  {what}  ({e['key'].split('/')[1]})")
        print(f"{box}/{stamp} ({m['role']}{', full' if m.get('full') else ''}): {len(m['files'])} files from "
              f"{len(ticks)} ticks hold to the manifest" + (f"; plaintext in {work}" if keep else ""))
    finally:
        if not keep:
            for p in work.iterdir():
                p.unlink()
            work.rmdir()


def pair(store: Store, door_box: str, stamp: str, arc_box: str) -> None:
    stamp = finished(store, door_box, stamp)
    before = [t for t, ok in sets(store, arc_box).items() if ok and t < stamp]
    if not before:
        fail(f"no {arc_box} manifest before {door_box}/{stamp}")
    print(f"{door_box}/{stamp} restores with {arc_box}/{before[-1]}")


def main(argv: list[str]) -> None:
    if len(argv) >= 2 and argv[0] == "sets":
        for t, ok in sets(Store(), argv[1]).items():
            print(t + ("" if ok else "  *"))
    elif len(argv) >= 3 and argv[0] == "check":
        ident = argv[argv.index("-i") + 1] if "-i" in argv else fail("check needs -i <identity>")
        keep = argv[argv.index("-o") + 1] if "-o" in argv else None
        check(Store(), argv[1], argv[2], ident, keep)
    elif len(argv) >= 4 and argv[0] == "pair":
        pair(Store(), argv[1], argv[2], argv[3])
    else:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main(sys.argv[1:])
