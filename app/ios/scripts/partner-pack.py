#!/usr/bin/env python3
"""partner-pack — the DEBUG seed pack, built from the partners' own material.

Reads the four case studies in the private business repo and writes
`ui/SeedMedia/partners/` (gitignored, never bundled): `pack.json` and its JPEGs.
`PartnerSeeder.swift` mints from it through the real pipe.

    partner-pack.py [--src DIR] [--out DIR]

RIGHTS. Every partner's README says their pictures and words are not ours to
redistribute. So this file holds no partner content — only where to look and
how to read it — and its output never enters git or an app bundle.

Text is the partners' own: captions, edition names, persona keywords, alt text.
The one invention is TIME (Ralph, 9 Nov): a past night is re-dated into the
coming weeks so the feed shows it. The pack carries day offsets, not dates, so
it never goes stale.
"""

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = Path.home() / "wallflowers/business/collaborations"
OUT = HERE.parent / "ui/SeedMedia/partners"

# Longest edge for the pack's JPEGs. Sizing for the wire is the mint's (#56, #111);
# this only keeps the pack small.
EDGE = 1600
ICON_EDGE = 512

# STOMA's pages that are nights, not documentation.
STOMA_EVENTS = {"facc-anual-event-2024", "on-immigration", "feast-xalapa",
                "las-3000", "story-hole", "homeless-2018", "mural-fest-kosovo"}
MANOLIN_POSTS = 6
MONTHS = r"(January|February|March|April|May|June|July|August|September|October|November|December)"


def jpeg(src: Path, dst: Path, edge: int = EDGE, square: bool = False) -> str:
    """`sips` to a JPEG no longer than `edge`; `square` centre-crops first (icons)."""
    dst.parent.mkdir(parents=True, exist_ok=True)
    args = ["sips", "-s", "format", "jpeg", "-s", "formatOptions", "85"]
    if square:
        w, h = dims(src)
        s = min(w, h)
        args += ["-c", str(s), str(s)]
    args += ["-Z", str(edge), str(src), "--out", str(dst)]
    subprocess.run(args, check=True, capture_output=True)
    return dst.name


def dims(p: Path) -> tuple[int, int]:
    out = subprocess.run(["sips", "-g", "pixelWidth", "-g", "pixelHeight", str(p)],
                         check=True, capture_output=True, text=True).stdout
    w = int(re.search(r"pixelWidth: (\d+)", out).group(1))
    h = int(re.search(r"pixelHeight: (\d+)", out).group(1))
    return w, h


def need(p: Path) -> Path:
    if not p.exists():
        sys.exit(f"partner-pack: missing {p}")
    return p


def stoma(src: Path, out: Path) -> dict:
    d = src / "stoma"
    index = json.loads(need(d / "assets/imagery/index.json").read_text())
    pages: dict[str, list[dict]] = {}
    for img in index["images"]:
        pages.setdefault(img["page"], []).append(img)

    posts, events = [], []
    for page, imgs in pages.items():
        # "Title, Place Year — what it was"
        head, _, what = imgs[0]["caption"].partition(" — ")
        title, _, place = head.partition(", ")
        place = re.sub(rf"\b{MONTHS}\b|\b(19|20)\d\d\b", "", place).strip(" ,")
        files = [jpeg(d / "assets/imagery" / imgs[0]["file"], out / "stoma" / imgs[0]["file"])]
        if page in STOMA_EVENTS:
            # The night is re-dated, so its old year leaves the title with it.
            # Banner only: a draft has no photo slot, and no screen shows them (#109).
            events.append({"title": re.sub(r"\s+(19|20)\d\d$", "", title),
                           "descriptor": what, "venue": place,
                           "banner": f"stoma/{files[0]}"})
        else:
            # No mark (stoma/docs/brand.md), and not ours to make: the icon is the
            # same photograph, squared.
            icon = jpeg(d / "assets/imagery" / imgs[0]["file"],
                        out / "stoma" / f"icon-{imgs[0]['file']}", ICON_EDGE, square=True)
            posts.append({"title": title, "body": imgs[0]["caption"],
                          "banner": f"stoma/{files[0]}", "icon": f"stoma/{icon}"})
    first = index["images"][0]["file"]
    return {"name": "STOMA", "shape": "community", "cover": f"stoma/{first}",
            "posts": posts, "events": events}


def manolin(src: Path, out: Path) -> dict:
    d = src / "manolin"
    eds = json.loads(need(d / "editions.json").read_text())["products"]
    eng = [e for e in eds if e["name"].startswith("[PDF ENG]")]
    eng.sort(key=lambda e: -e["idx"])
    icon = jpeg(need(d / "brand/mark-152.png"), out / "manolin/icon.jpg", ICON_EDGE, square=True)
    cover = jpeg(need(d / "brand/reader-photo-1200x630.png"), out / "manolin/cover.jpg")
    posts = []
    for e in eng[:MANOLIN_POSTS]:
        cov = d / "covers" / f"{e['idx']}.jpg"
        if not cov.exists():
            continue
        posts.append({"title": e["name"].removeprefix("[PDF ENG]").strip(), "body": "",
                      "banner": f"manolin/{jpeg(cov, out / 'manolin' / cov.name)}",
                      "icon": f"manolin/{icon}"})
    return {"name": "Manolin", "shape": "organisation", "cover": f"manolin/{cover}",
            "posts": posts, "events": []}


def cyp3(src: Path, out: Path) -> dict:
    d = src / "cyp3/assets"
    md = need(d / "personas/personas.md").read_text()
    icon = jpeg(need(d / "brand/logo-render-2026-09-17.jpeg"), out / "cyp3/icon.jpg",
                ICON_EDGE, square=True)
    cover = jpeg(need(d / "brand/installation-sketch.png"), out / "cyp3/cover.jpg")
    posts = []
    plates = sorted((d / "personas").glob("identity-*.png"))
    sections = re.split(r"^## \d · ", md, flags=re.M)[1:len(plates) + 1]
    for plate, sec in zip(plates, sections):
        name = sec.splitlines()[0].split()[-1]
        keyword = re.search(r"\*\*Keywords\*\* — ([^·\n]+)", sec)
        origin = re.search(r"\*\*Origin\*\* — (.+)", sec)
        posts.append({"title": f"{name} — {keyword.group(1).strip()}" if keyword else name,
                      "body": origin.group(1).strip() if origin else "",
                      "banner": f"cyp3/{jpeg(plate, out / 'cyp3' / (plate.stem + '.jpg'))}",
                      "icon": f"cyp3/{icon}"})
    return {"name": "CYP3", "shape": "organisation", "cover": f"cyp3/{cover}",
            "posts": posts, "events": []}


def prodigal(src: Path, out: Path) -> dict:
    d = src / "thedoor"
    prov = json.loads(need(d / "brand/provenance.json").read_text())
    # The fan page's own alt text names each scan; the brand copy is the same image.
    page = need(d / "site/fanpage.html").read_text()
    alt = {m.group(1): m.group(2)
           for m in re.finditer(r'src="media/([^"]+)"[^>]*alt="([^"]+)"', page)}
    icon = jpeg(need(d / "brand/door-glyph-flower.jpg"), out / "prodigal/icon.jpg",
                ICON_EDGE, square=True)
    cover = jpeg(need(d / "brand/wordmark-on-fisheye.jpg"), out / "prodigal/cover.jpg")
    posts = []
    for p in prov:
        title = alt.get(p["source"])
        # Two brand files can be one fan-page image (the fisheye and its wordmark).
        if not title or any(q["title"] == title for q in posts):
            continue
        posts.append({"title": title, "body": "",
                      "banner": f"prodigal/{jpeg(d / 'brand' / p['file'], out / 'prodigal' / p['file'])}",
                      "icon": f"prodigal/{icon}"})
    return {"name": "Prodigal", "shape": "organisation", "cover": f"prodigal/{cover}",
            "posts": posts, "events": []}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", type=Path, default=SRC)
    ap.add_argument("--out", type=Path, default=OUT)
    a = ap.parse_args()
    if a.out.exists():
        shutil.rmtree(a.out)
    a.out.mkdir(parents=True)

    partners = [stoma(a.src, a.out), manolin(a.src, a.out), cyp3(a.src, a.out),
                prodigal(a.src, a.out)]

    # TIME, the one invention: one night every five days from three days out, at 19:00;
    # FACC keeps its three days. Offsets, so the seeder dates them from its own today.
    slot = 0
    for p in partners:
        for e in p["events"]:
            e["dayOffset"] = 3 + 5 * slot
            e["hour"] = 19
            e["hours"] = 72 if "FACC" in e["title"] else 4
            slot += 1

    (a.out / "pack.json").write_text(json.dumps({"partners": partners}, ensure_ascii=False, indent=1))
    for p in partners:
        print(f"{p['name']:9} {len(p['posts'])} posts, {len(p['events'])} events")
    print(f"→ {a.out}")


if __name__ == "__main__":
    main()
