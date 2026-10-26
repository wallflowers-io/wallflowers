#!/usr/bin/env python3
"""Vendor the public face's third-party parts into app/web/webapp/assets/face/vendor/.

    app/web/vendor-face.py

The Door's copy. www keeps its own (the website's scripts/vendor-face.py, its
site/assets/face/vendor/), for the faces the Arc draws at wallflowers.io/<slug>.
After a re-run here, arc/lib/face-render's copy of face-assets.js and its pin in
vendor/SOURCES move in the same commit: vendored_copies_match_the_webapp says so.

A face is a public page, and every file it loads from someone else's server
tells that server who looked at which face: Google Fonts logs the visitor's
address, and an emoji CDN would too. The footer says "No tracking", so the face
carries its own. This script fetches each part ONCE, at a pinned version, and
what it writes is committed. Re-run it to change the set or bump a version;
never edit its output by hand.

    fonts     Fontsource's latin woff2s of Google Fonts families      SIL OFL 1.1
    stickers  Microsoft's Fluent Emoji, 3D, shrunk to 192px WebP      MIT
    icons     Simple Icons' paths for the Social widget's platforms   CC0 1.0
    coloris   the colour picker — a classic script, no build step     MIT
    qrcode    Kazuhiko Arase's QR generator — a classic script        MIT

vendor/face-assets.js (WallFlowers.FaceAssets = {fonts, stickers, icons}) and
vendor/fonts.css are written from the SAME tables below, so the list face.js
offers and the files on disk cannot disagree. Each licence lands in
vendor/LICENSES/.

Stickers are shrunk with cwebp when it is on PATH (brew install webp); without
it they are kept as the 256px PNGs they arrive as, and face-assets.js names
whichever was written.
"""
import json
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "app" / "web" / "webapp" / "assets" / "face" / "vendor"

FONTSOURCE = "5.3.0"
FLUENT = "1ffb34c752ecf5d402f04cfb4b392c77f57c54bc"     # microsoft/fluentui-emoji main, 24 Aug 2026
SIMPLE_ICONS = "16.31.0"
COLORIS = "0.25.0"
QRCODE = "1.4.4"          # 2.x is still a classic script, but 1.4.4 is the one read

# (id, family, group, files). A file is "wght" for a variable font's whole
# weight axis, or a single static weight. A static face is declared across every
# weight it is the only file for, so the browser uses it as drawn instead of
# smearing a fake bold over a display face that has no bold.
FONTS = [
    ("inter", "Inter", "Sans", ["wght"]),
    ("dm-sans", "DM Sans", "Sans", ["wght"]),
    ("manrope", "Manrope", "Sans", ["wght"]),
    ("figtree", "Figtree", "Sans", ["wght"]),
    ("outfit", "Outfit", "Sans", ["wght"]),
    ("instrument-sans", "Instrument Sans", "Sans", ["wght"]),
    ("space-grotesk", "Space Grotesk", "Sans", ["wght"]),
    ("bricolage-grotesque", "Bricolage Grotesque", "Sans", ["wght"]),
    ("syne", "Syne", "Sans", ["wght"]),
    ("rubik", "Rubik", "Sans", ["wght"]),
    ("fraunces", "Fraunces", "Serif", ["wght"]),
    ("playfair-display", "Playfair Display", "Serif", ["wght"]),
    ("lora", "Lora", "Serif", ["wght"]),
    ("newsreader", "Newsreader", "Serif", ["wght"]),
    ("instrument-serif", "Instrument Serif", "Serif", [400]),
    ("young-serif", "Young Serif", "Serif", [400]),
    ("dm-serif-display", "DM Serif Display", "Serif", [400]),
    ("bagel-fat-one", "Bagel Fat One", "Display", [400]),
    ("unbounded", "Unbounded", "Display", ["wght"]),
    ("shrikhand", "Shrikhand", "Display", [400]),
    ("righteous", "Righteous", "Display", [400]),
    ("belanosima", "Belanosima", "Display", [600]),
    ("abril-fatface", "Abril Fatface", "Display", [400]),
    ("rubik-mono-one", "Rubik Mono One", "Display", [400]),
    ("caveat", "Caveat", "Handwritten", ["wght"]),
    ("pacifico", "Pacifico", "Handwritten", [400]),
    ("space-mono", "Space Mono", "Mono", [400, 700]),
    ("jetbrains-mono", "JetBrains Mono", "Mono", ["wght"]),
]
FALLBACK = {
    "Sans": '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
    "Serif": 'Georgia, "Times New Roman", serif',
    "Display": '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
    "Handwritten": "cursive",
    "Mono": "ui-monospace, SFMono-Regular, Menlo, monospace",
}

# (id, name, group, Fluent Emoji folder). Skin-toned emoji take the Default tone.
STICKERS = [
    ("down-arrow", "Down arrow", "This way", "Down arrow"),
    ("right-arrow", "Right arrow", "This way", "Right arrow"),
    ("curving-arrow", "Arrow curving down", "This way", "Right arrow curving down"),
    ("point-down", "Pointing down", "This way", "Backhand index pointing down"),
    ("point-right", "Pointing right", "This way", "Backhand index pointing right"),
    ("point-at-you", "Pointing at you", "This way", "Index pointing at the viewer"),
    ("eyes", "Eyes", "This way", "Eyes"),
    ("tulip", "Tulip", "Flowers", "Tulip"),
    ("sunflower", "Sunflower", "Flowers", "Sunflower"),
    ("blossom", "Blossom", "Flowers", "Blossom"),
    ("cherry-blossom", "Cherry blossom", "Flowers", "Cherry blossom"),
    ("hibiscus", "Hibiscus", "Flowers", "Hibiscus"),
    ("rose", "Rose", "Flowers", "Rose"),
    ("bouquet", "Bouquet", "Flowers", "Bouquet"),
    ("lotus", "Lotus", "Flowers", "Lotus"),
    ("rosette", "Rosette", "Flowers", "Rosette"),
    ("seedling", "Seedling", "Flowers", "Seedling"),
    ("clover", "Four leaf clover", "Flowers", "Four leaf clover"),
    ("wilted-flower", "Wilted flower", "Flowers", "Wilted flower"),
    ("honeybee", "Honeybee", "Flowers", "Honeybee"),
    ("butterfly", "Butterfly", "Flowers", "Butterfly"),
    ("snail", "Snail", "Flowers", "Snail"),
    ("party-popper", "Party popper", "Celebrate", "Party popper"),
    ("confetti-ball", "Confetti ball", "Celebrate", "Confetti ball"),
    ("balloon", "Balloon", "Celebrate", "Balloon"),
    ("sparkles", "Sparkles", "Celebrate", "Sparkles"),
    ("glowing-star", "Glowing star", "Celebrate", "Glowing star"),
    ("gift", "Gift", "Celebrate", "Wrapped gift"),
    ("birthday-cake", "Birthday cake", "Celebrate", "Birthday cake"),
    ("hundred", "Hundred points", "Celebrate", "Hundred points"),
    ("fire", "Fire", "Celebrate", "Fire"),
    ("rainbow", "Rainbow", "Celebrate", "Rainbow"),
    ("handshake", "Handshake", "Together", "Handshake"),
    ("raising-hands", "Raising hands", "Together", "Raising hands"),
    ("waving-hand", "Waving hand", "Together", "Waving hand"),
    ("red-heart", "Red heart", "Together", "Red heart"),
    ("sparkling-heart", "Sparkling heart", "Together", "Sparkling heart"),
    ("speech-balloon", "Speech balloon", "Together", "Speech balloon"),
    ("megaphone", "Megaphone", "Together", "Megaphone"),
    ("love-letter", "Love letter", "Together", "Love letter"),
    ("pin", "Pin", "Out and about", "Round pushpin"),
    ("house", "House with garden", "Out and about", "House with garden"),
    ("tickets", "Tickets", "Out and about", "Admission tickets"),
    ("microphone", "Microphone", "Out and about", "Microphone"),
    ("musical-notes", "Musical notes", "Out and about", "Musical notes"),
    ("hot-drink", "Hot drink", "Out and about", "Hot beverage"),
    ("calendar", "Calendar", "Out and about", "Spiral calendar"),
    ("books", "Books", "Out and about", "Books"),
    ("newspaper", "Newspaper", "Out and about", "Rolled-up newspaper"),
    ("bicycle", "Bicycle", "Out and about", "Bicycle"),
    ("globe", "Globe", "Out and about", "Globe showing europe-africa"),
    ("sun", "Sun", "Out and about", "Sun with face"),
    ("moon", "Moon", "Out and about", "Crescent moon"),
]

# Simple Icons slugs. LinkedIn and Eventbrite are no longer in the set (their
# owners asked for removal), so face.js draws those two as letters.
ICONS = ["instagram", "tiktok", "x", "youtube", "facebook", "threads", "mastodon", "bluesky",
         "spotify", "soundcloud", "bandcamp", "twitch", "discord", "whatsapp", "telegram",
         "signal", "substack", "patreon", "github", "pinterest", "snapchat", "meetup"]


def get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "wallflowers-vendor-face"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def put(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def snake(name: str) -> str:
    return name.lower().replace(" ", "_")


def fonts() -> list:
    css = ["/* GENERATED by scripts/vendor-face.py from its FONTS table — do not edit.",
           " * Fontsource " + FONTSOURCE + ", latin subset. SIL Open Font License 1.1: vendor/LICENSES/fonts/. */", ""]
    listed = []
    for fid, family, group, files in FONTS:
        variable = files == ["wght"]
        pkg = ("@fontsource-variable/" if variable else "@fontsource/") + fid
        base = "https://cdn.jsdelivr.net/npm/" + pkg + "@" + FONTSOURCE
        for i, f in enumerate(files):
            name = fid + ("" if variable or len(files) == 1 else "-" + str(f)) + ".woff2"
            src = base + "/files/" + fid + "-latin-" + ("wght" if variable else str(f)) + "-normal.woff2"
            put(OUT / "fonts" / name, get(src))
            if len(files) == 1:
                weight = "100 1000"
            else:                                  # split the range between the weights there are
                lo = 100 if i == 0 else (files[i - 1] + f) // 2 + 1
                hi = 1000 if i == len(files) - 1 else (f + files[i + 1]) // 2
                weight = str(lo) + " " + str(hi)
            css += ["@font-face {",
                    '  font-family: "' + family + '"; font-style: normal; font-weight: ' + weight + ";",
                    "  font-display: swap; src: url(fonts/" + name + ') format("woff2");', "}"]
        put(OUT / "LICENSES" / "fonts" / (fid + ".txt"), get(base + "/LICENSE"))
        listed.append({"id": fid, "name": family, "group": group,
                       "stack": '"' + family + '", ' + FALLBACK[group]})
    put(OUT / "fonts.css", ("\n".join(css) + "\n").encode())
    print("fonts: %d families" % len(listed))
    return listed


def stickers() -> list:
    raw = "https://raw.githubusercontent.com/microsoft/fluentui-emoji/" + FLUENT + "/assets/"
    cwebp = shutil.which("cwebp")
    listed = []
    with tempfile.TemporaryDirectory() as tmp:
        for sid, name, group, folder in STICKERS:
            enc = urllib.request.quote(folder)
            try:
                png = get(raw + enc + "/3D/" + snake(folder) + "_3d.png")
            except Exception:
                png = get(raw + enc + "/Default/3D/" + snake(folder) + "_3d_default.png")
            if cwebp:
                src = Path(tmp) / (sid + ".png")
                src.write_bytes(png)
                dest = OUT / "stickers" / (sid + ".webp")
                dest.parent.mkdir(parents=True, exist_ok=True)
                subprocess.run([cwebp, "-quiet", "-q", "82", "-alpha_q", "90", "-resize", "192", "192",
                                str(src), "-o", str(dest)], check=True)
            else:
                dest = OUT / "stickers" / (sid + ".png")
                put(dest, png)
            listed.append({"id": sid, "name": name, "group": group, "file": "stickers/" + dest.name})
    put(OUT / "LICENSES" / "fluent-emoji.txt",
        get("https://raw.githubusercontent.com/microsoft/fluentui-emoji/" + FLUENT + "/LICENSE"))
    print("stickers: %d (%s)" % (len(listed), "webp" if cwebp else "png — no cwebp"))
    return listed


def icons() -> dict:
    base = "https://cdn.jsdelivr.net/npm/simple-icons@" + SIMPLE_ICONS
    paths = {}
    for slug in ICONS:
        svg = get(base + "/icons/" + slug + ".svg").decode()
        d = re.findall(r'<path d="([^"]+)"', svg)
        if len(d) != 1:
            sys.exit("vendor-face: simple-icons %s has %d paths, expected 1" % (slug, len(d)))
        paths[slug] = d[0]
    put(OUT / "LICENSES" / "simple-icons.md", get(base + "/LICENSE.md"))
    print("icons: %d" % len(paths))
    return paths


def scripts() -> None:
    gh = "https://cdn.jsdelivr.net/gh/mdbassit/Coloris@" + COLORIS
    put(OUT / "coloris" / "coloris.min.js", get(gh + "/dist/coloris.min.js"))
    put(OUT / "coloris" / "coloris.min.css", get(gh + "/dist/coloris.min.css"))
    put(OUT / "LICENSES" / "coloris.txt", get(gh + "/LICENSE"))
    qr = get("https://cdn.jsdelivr.net/npm/qrcode-generator@" + QRCODE + "/qrcode.js")
    put(OUT / "qrcode" / "qrcode.js", qr)
    # The package ships no LICENSE file; its header is the notice.
    head = qr.decode().split("\nvar qrcode")[0]
    if "MIT" not in head:
        sys.exit("vendor-face: qrcode.js no longer opens with its MIT notice")
    put(OUT / "LICENSES" / "qrcode-generator.txt", (head.strip() + "\n").encode())
    print("scripts: coloris %s, qrcode-generator %s" % (COLORIS, QRCODE))


def main() -> int:
    if OUT.exists():
        shutil.rmtree(OUT)
    assets = {"fonts": fonts(), "stickers": stickers(), "icons": icons()}
    scripts()
    js = ("/* GENERATED by scripts/vendor-face.py — do not edit. The fonts, stickers and\n"
          " * platform icons the face editor offers, and where each file is, relative to\n"
          " * assets/face/vendor/. Licences: vendor/LICENSES/. */\n"
          "(function (global) {\n"
          "  var W = global.WallFlowers = global.WallFlowers || {};\n"
          "  W.FaceAssets = " + json.dumps(assets, indent=1, ensure_ascii=True) + ";\n"
          "})(window);\n")
    put(OUT / "face-assets.js", js.encode())
    total = sum(f.stat().st_size for f in OUT.rglob("*") if f.is_file())
    print("vendor-face: %d KB -> %s" % (total // 1024, OUT.relative_to(ROOT)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
