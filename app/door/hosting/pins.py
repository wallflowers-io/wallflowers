#!/usr/bin/env python3
"""pins.py — the Door's pinned files kept across deploys (SECURITY's ruling (a), 27 Sep).

A site that mounts the webapp pins webapp.<hex>.js and the livery by name and SRI, so a
pin must outlive the deploy that made it. deploy-door.sh runs this on the host, as root:

    pins.py ship <live> <new> <manifest> <commit> <pins.list>
        this deploy's pins into the manifest, then the retained ones copied from the live
        webapp into the new one, then the rest pruned: a pin stays while it belongs to one
        of the last KEEP_DEPLOYS deploys or is younger than KEEP_DAYS, whichever keeps more
    pins.py prune <live> <prev> <manifest> <commit|name>...
        at once, from the live webapp and the previous one (a security fix): every pin of a
        deploy whose commit is the argument or starts with it (7 characters at least), or
        the pinned file so named; never one the live index.html loads
    pins.py carry <live> <prev> <manifest>
        after a rollback: every retained pin the previous webapp holds, into the live one
    pins.py list <manifest>
        path|sri, one per retained file, for verify

The manifest is a line per (pinned path, deploy): path, commit, date (UTC), SRI, tab
separated. A file stays while any line names it. PINS_NOW (unix seconds) fixes the clock
for a test.
"""
from __future__ import annotations

import base64
import datetime as dt
import hashlib
import os
import re
import shutil
import sys

KEEP_DEPLOYS = 3
KEEP_DAYS = 14
FMT = "%Y-%m-%dT%H:%M:%SZ"


def now() -> dt.datetime:
    t = os.environ.get("PINS_NOW")
    return dt.datetime.fromtimestamp(int(t), dt.timezone.utc) if t else dt.datetime.now(dt.timezone.utc)


def when(s: str) -> dt.datetime:
    return dt.datetime.strptime(s, FMT).replace(tzinfo=dt.timezone.utc)


def read(manifest: str) -> list[list[str]]:
    if not os.path.exists(manifest):
        return []
    return [l.rstrip("\n").split("\t") for l in open(manifest) if l.strip()]


def write(manifest: str, rows: list[list[str]]) -> None:
    tmp = manifest + ".new"
    with open(tmp, "w") as f:
        f.writelines("\t".join(r) + "\n" for r in rows)
    os.chmod(tmp, 0o644)
    os.replace(tmp, manifest)


def copy_in(src_root: str, dst_root: str, path: str) -> bool:
    """`path` from one webapp into another, unless it is there: a pinned name is its bytes."""
    src, dst = os.path.join(src_root, path), os.path.join(dst_root, path)
    if os.path.exists(dst):
        return True
    if not os.path.isfile(src):
        return False
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(src, dst)
    return True


def retained(rows: list[list[str]]) -> list[list[str]]:
    """The rows a pin stays by: the last KEEP_DEPLOYS deploys, or younger than KEEP_DAYS."""
    newest: dict[str, dt.datetime] = {}
    for path, commit, date, _ in rows:
        newest[commit] = max(newest.get(commit, when(date)), when(date))
    recent = set(sorted(newest, key=newest.get, reverse=True)[:KEEP_DEPLOYS])
    cut = now() - dt.timedelta(days=KEEP_DAYS)
    return [r for r in rows if r[1] in recent or when(r[2]) >= cut]


def loaded(live: str) -> set[str]:
    """What the live index.html loads, which no prune may take away."""
    page = os.path.join(live, "index.html")
    s = open(page).read() if os.path.exists(page) else ""
    return set(re.findall(r'(?:src|href)="([^"?#]+)"', s))


PINNED = re.compile(r"\.[0-9a-f]{16}\.(?:js|css)$")


def adopt(live: str, rows: list[list[str]]) -> list[list[str]]:
    """Pinned files the live webapp serves and no line names (a deploy before the
    manifest): its commit (`COMMIT` beside it), dated when the file was written."""
    named = {r[0] for r in rows}
    commit_file = os.path.join(os.path.dirname(os.path.abspath(live)), "COMMIT")
    commit = open(commit_file).read().strip() if os.path.exists(commit_file) else "unknown"
    out = []
    for root, _, files in os.walk(live):
        for f in files:
            full = os.path.join(root, f)
            p = os.path.relpath(full, live)
            if PINNED.search(f) and p not in named:
                body = open(full, "rb").read()
                date = dt.datetime.fromtimestamp(os.path.getmtime(full), dt.timezone.utc).strftime(FMT)
                out.append([p, commit, date, "sha384-" + base64.b64encode(hashlib.sha384(body).digest()).decode()])
    return out


def ship(live: str, new: str, manifest: str, commit: str, pins_list: str) -> None:
    rows = read(manifest)
    if os.path.isdir(live):
        rows += adopt(live, rows)
    stamp = now().strftime(FMT)
    mine = [l.split() for l in open(pins_list) if l.strip()]
    rows = [r for r in rows if r[1] != commit] + [[q, commit, stamp, i] for _, q, i in mine]
    keep = retained(rows)
    kept_paths = {r[0] for r in keep}
    carried, missing = 0, []
    for p in sorted(kept_paths - {q for _, q, _ in mine}):
        if copy_in(live, new, p):
            carried += 1
        else:
            missing.append(p)
    keep = [r for r in keep if r[0] not in missing]
    gone = sorted({r[0] for r in rows} - {r[0] for r in keep})
    write(manifest, keep)
    deploys = len({r[1] for r in keep})
    print(f"pins: {len(mine)} this deploy's, {carried} carried from earlier ones, {deploys} deploys kept")
    for p in gone:
        print(f"pins: pruned {p}")
    for p in missing:
        print(f"pins: {p} was retained but not in the live webapp; dropped from the manifest")


def prune(live: str, prev: str, manifest: str, names: list[str]) -> int:
    rows = read(manifest)
    hit = [r for r in rows if any(r[1] == n or (len(n) >= 7 and r[1].startswith(n)) or r[0] == n or os.path.basename(r[0]) == n for n in names)]
    if not hit:
        print("pins: nothing matches " + " ".join(names))
        return 1
    live_needs = loaded(live) & {r[0] for r in hit}
    if live_needs:
        print("pins: refused, the live index.html loads " + " ".join(sorted(live_needs)) + ": deploy the fix first")
        return 1
    left = [r for r in rows if r not in hit]
    still = {r[0] for r in left}
    for p in sorted({r[0] for r in hit} - still):
        for root in (live, prev):
            f = os.path.join(root, p)
            if os.path.isfile(f):
                os.remove(f)
        print(f"pins: removed {p} ({', '.join(sorted({r[1][:12] for r in hit if r[0] == p}))})")
    for p in sorted({r[0] for r in hit} & still):
        print(f"pins: kept {p}, another deploy still names it")
    write(manifest, left)
    return 0


def carry(live: str, prev: str, manifest: str) -> None:
    n = sum(copy_in(prev, live, p) for p in sorted({r[0] for r in read(manifest)}))
    print(f"pins: {n} retained pins in the live webapp")


def main(argv: list[str]) -> int:
    cmd, args = (argv[1], argv[2:]) if len(argv) > 1 else ("", [])
    if cmd == "ship" and len(args) == 5:
        ship(*args)
    elif cmd == "prune" and len(args) >= 4:
        return prune(args[0], args[1], args[2], args[3:])
    elif cmd == "carry" and len(args) == 3:
        carry(*args)
    elif cmd == "list" and len(args) == 1:
        seen = {}
        for path, _, _, sri in read(args[0]):
            seen[path] = sri
        print(" ".join(f"{p}|{i}" for p, i in sorted(seen.items())))
    else:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
