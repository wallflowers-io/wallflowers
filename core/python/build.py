#!/usr/bin/env python3
"""Build the sdist and the wheel with the standard library only.

Why this exists instead of `python -m build`: the workspace's Python is
provisioned and not `pip install`-ed into (the doctrine, working notes), and
it has no `build`, `setuptools`, `wheel` or `twine`. A wheel is a zip with a
`dist-info` directory and an sdist is a tarball with a `PKG-INFO`, both of which
`zipfile`, `tarfile` and `hashlib` can produce exactly.

`pyproject.toml` is still the source of truth for the metadata and still
declares a setuptools backend, so anyone who *does* have pip builds the same
artefacts the normal way. This script reads that file rather than restating it.

    python3 build.py            # -> dist/
"""
from __future__ import annotations

import base64
import hashlib
import io
import shutil
import sys
import tarfile
import zipfile
from pathlib import Path

if sys.version_info >= (3, 11):
    import tomllib
else:  # pragma: no cover
    try:
        import tomli as tomllib
    except ModuleNotFoundError:
        sys.exit("needs Python 3.11+ (tomllib) to read pyproject.toml")

HERE = Path(__file__).resolve().parent
DIST = HERE / "dist"

#: A fixed timestamp, so two builds of one tree are byte-identical.
EPOCH = (1980, 1, 1, 0, 0, 0)


def metadata(project: dict, readme: str) -> str:
    """METADATA / PKG-INFO, core metadata 2.1."""
    lines = [
        "Metadata-Version: 2.1",
        f"Name: {project['name']}",
        f"Version: {project['version']}",
        f"Summary: {project['description']}",
    ]
    for author in project.get("authors", []):
        lines.append(f"Author: {author['name']}")
    if "license" in project:
        lines.append(f"License: {project['license']['text']}")
    for label, url in project.get("urls", {}).items():
        lines.append(f"Project-URL: {label}, {url}")
    if project.get("keywords"):
        lines.append("Keywords: " + ",".join(project["keywords"]))
    for classifier in project.get("classifiers", []):
        lines.append(f"Classifier: {classifier}")
    if "requires-python" in project:
        lines.append(f"Requires-Python: {project['requires-python']}")
    lines.append("Description-Content-Type: text/markdown")
    return "\n".join(lines) + "\n\n" + readme


#: What goes inside the importable package — the wheel's whole payload, and part
#: of the sdist's. `py.typed` is listed explicitly because it is neither a `.py`
#: file nor extensionless, and a glob that misses it ships a package whose
#: `Typing :: Typed` classifier is a lie: type checkers ignore the annotations
#: of a package with no marker.
PACKAGE_PATTERNS = ("wallflowers/*.py", "wallflowers/py.typed")


def package_files() -> list[Path]:
    """The importable package's files, sorted."""
    out: list[Path] = []
    for pattern in PACKAGE_PATTERNS:
        out.extend(sorted(HERE.glob(pattern)))
    return [p for p in out if p.is_file()]


def sources() -> list[Path]:
    """Every file that ships in the sdist, sorted. No caches, no build output."""
    out = package_files()
    out.extend(sorted(HERE.glob("tests/*.py")))
    out.extend([HERE / "pyproject.toml", HERE / "README.md",
                HERE / "LICENSE", HERE / "build.py"])
    return [p for p in out if p.is_file()]


def build_wheel(project: dict, meta: str) -> Path:
    name, version = project["name"], project["version"]
    info = f"{name}-{version}.dist-info"
    target = DIST / f"{name}-{version}-py3-none-any.whl"
    record: list[str] = []

    def add(zf: zipfile.ZipFile, arcname: str, payload: bytes) -> None:
        entry = zipfile.ZipInfo(arcname, date_time=EPOCH)
        entry.external_attr = 0o644 << 16
        entry.compress_type = zipfile.ZIP_DEFLATED
        zf.writestr(entry, payload)
        digest = base64.urlsafe_b64encode(hashlib.sha256(payload).digest())
        record.append(f"{arcname},sha256={digest.decode().rstrip('=')},{len(payload)}")

    with zipfile.ZipFile(target, "w") as zf:
        for path in package_files():
            add(zf, str(path.relative_to(HERE)), path.read_bytes())
        add(zf, f"{info}/METADATA", meta.encode())
        add(zf, f"{info}/WHEEL",
            b"Wheel-Version: 1.0\nGenerator: wallflowers-build\n"
            b"Root-Is-Purelib: true\nTag: py3-none-any\n")
        add(zf, f"{info}/licenses/LICENSE", (HERE / "LICENSE").read_bytes())
        record.append(f"{info}/RECORD,,")
        entry = zipfile.ZipInfo(f"{info}/RECORD", date_time=EPOCH)
        entry.external_attr = 0o644 << 16
        zf.writestr(entry, "\n".join(record) + "\n")
    return target


def build_sdist(project: dict, meta: str) -> Path:
    name, version = project["name"], project["version"]
    root = f"{name}-{version}"
    target = DIST / f"{root}.tar.gz"

    def entry(path: str, payload: bytes) -> tuple[tarfile.TarInfo, io.BytesIO]:
        info = tarfile.TarInfo(f"{root}/{path}")
        info.size = len(payload)
        info.mtime = 315532800  # 1 Jan 1980, matching EPOCH
        info.mode = 0o644
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        return info, io.BytesIO(payload)

    with tarfile.open(target, "w:gz") as tf:
        tf.addfile(*entry("PKG-INFO", meta.encode()))
        for path in sources():
            tf.addfile(*entry(str(path.relative_to(HERE)), path.read_bytes()))
    return target


def main() -> None:
    project = tomllib.loads((HERE / "pyproject.toml").read_text())["project"]
    readme = (HERE / "README.md").read_text()
    meta = metadata(project, readme)
    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir()
    for built in (build_sdist(project, meta), build_wheel(project, meta)):
        print(f"  {built.relative_to(HERE)}  {built.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
