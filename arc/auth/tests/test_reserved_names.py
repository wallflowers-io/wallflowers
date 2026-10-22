"""Every name the site answers to is reserved at signup — derived, not listed.

A slug is served at wallflowers.io/<slug>, but only for a path the site itself
does not already answer: nginx tries its own files, pages and locations first and
asks the Arc last (site/default.conf.template, @face). So a group that claimed a
name the site answers would hold an address that can never show its face.

_RESERVED is kept by hand, on purpose — the service runs in its own image and
cannot see the site's files. This test is what stops the hand-kept list falling
behind: it reads what the site image actually serves, from the Dockerfile's COPY
lines and the nginx template's locations, and fails naming every slug-shaped name
the list does not reserve. It found `demo`, `demo-media` and `escape` the day it
was written (22 Sep 2026).
"""
from __future__ import annotations

import re
from pathlib import Path

from app.main import _RESERVED, _SLUG_OK

SITE = Path(__file__).resolve().parents[2] / "site"
WEBROOT = "/usr/share/nginx/html/"


def _served_by_the_image() -> set[str]:
    """Top-level names the image puts in the web root: pages without .html,
    directories and files as they are."""
    names: set[str] = set()
    for line in (SITE / "Dockerfile").read_text().splitlines():
        m = re.match(r"^COPY\s+(.+?)\s+(\S+)$", line.strip())
        if not m or not m.group(2).startswith(WEBROOT):
            continue
        dest = m.group(2)[len(WEBROOT):].strip("/")
        if dest:                               # COPY <dir> /usr/share/nginx/html/<dir>
            names.add(dest.split("/")[0])
        else:                                  # COPY a.html b.html /usr/share/nginx/html/
            names.update(Path(src).name for src in m.group(1).split())
    return {n[:-len(".html")] if n.endswith(".html") else n for n in names}


def _answered_by_nginx() -> set[str]:
    """First path segments of the template's literal locations, and the names in
    its regex locations' alternations (signup|signin|profile, lock)."""
    names: set[str] = set()
    for line in (SITE / "default.conf.template").read_text().splitlines():
        m = re.match(r"^\s*location\s+(?:=|\^~|~\*?)?\s*(\S+)\s*\{", line)
        if not m or m.group(1).startswith("@"):
            continue
        path = m.group(1)
        if path.startswith("^/"):
            body = path[2:]
            alt = re.match(r"^\(\?<\w+>([a-z|-]+)\)", body) or re.match(r"^\(([a-z|-]+)\)", body)
            if alt:
                names.update(alt.group(1).split("|"))
            else:
                head = re.match(r"^([a-z0-9-]+)", body)
                if head:
                    names.add(head.group(1))
        elif path.startswith("/"):
            seg = path.strip("/").split("/")[0]
            if seg:
                names.add(seg.lstrip("."))
    return names


def test_the_derivation_sees_what_the_site_serves():
    """Guard the reader, or the real test could pass by reading nothing."""
    served, answered = _served_by_the_image(), _answered_by_nginx()
    assert {"site", "collaborate", "demo", "escape", "assets", "brand"} <= served, served
    assert {"signup", "signin", "profile", "lock", "unlock", "auth"} <= answered, answered


def test_every_name_the_site_answers_is_reserved():
    names = _served_by_the_image() | _answered_by_nginx()
    shaped = {n for n in names if _SLUG_OK.match(n)}
    missing = sorted(shaped - _RESERVED)
    assert not missing, (
        f"the site answers {missing} but signup would hand them out as addresses — "
        "add them to _RESERVED in auth/app/main.py")
