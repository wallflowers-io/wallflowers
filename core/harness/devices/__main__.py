"""The device agents, from the command line.

    python -m harness.devices ios       I1        two simulators, one MLS room
    python -m harness.devices all       everything
    python -m harness.devices i1        any one case by name

Every device is started here and reaped here: its own semaphore on a free port
and its own Harness-* simulator per phone. Nothing already running is touched:
not :8787, not the E2E-* simulators. Logs, transcripts and screenshots land in --out (a temp directory
by default) and the path is printed.

Exit status is 0 only if every check in every case is green.
"""
from __future__ import annotations

import argparse
import asyncio
import shutil
import sys
import tempfile
from pathlib import Path

from ..cases.framework import report
from ..live.relay import SEMAPHORE
from .cases import GROUPS, i1


def preflight(cases) -> list[str]:
    problems = []
    if not SEMAPHORE.exists():
        problems.append(f"no relay binary at {SEMAPHORE} (cargo build -p relay)")
    if i1 in cases:
        if not shutil.which("xcrun"):
            problems.append("no xcrun — the phone agent needs Xcode's simctl")
        else:
            from .ios import newest_app
            try:
                newest_app()
            except FileNotFoundError as ex:
                problems.append(str(ex))
    return problems


async def run(cases, out: Path):
    return [await c(out) for c in cases]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="harness.devices", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("what", nargs="?", default="all", choices=sorted(GROUPS))
    ap.add_argument("--out", type=Path, default=None,
                    help="where logs, transcripts and screenshots go (default: a new temp dir)")
    args = ap.parse_args(argv)

    cases = GROUPS[args.what]
    problems = preflight(cases)
    if problems:
        print("cannot run:\n" + "\n".join(f"  · {p}" for p in problems))
        return 2

    out = args.out or Path(tempfile.mkdtemp(prefix="harness-devices-"))
    out.mkdir(parents=True, exist_ok=True)
    print(f"artefacts: {out}\n")
    results = asyncio.run(run(cases, out))
    text, green = report(results)
    print(text)
    print("\nGREEN" if green else "\nRED")
    return 0 if green else 1


if __name__ == "__main__":
    sys.exit(main())
