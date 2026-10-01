"""The device agents, from the command line.

    python -m harness.devices web       W1        two browser devices over a relay
    python -m harness.devices embed     W2 W3     pacific.js in a page, and with a second device
    python -m harness.devices browser   W1 W2 W3  do the drivers work
    python -m harness.devices findings  W4        what they found — red until keyholder.js is fixed
    python -m harness.devices ios       I1        two simulators, one MLS room
    python -m harness.devices all       everything; the browser and the phones run concurrently
    python -m harness.devices w3        any one case by name

Every device is started here and reaped here: its own semaphore on a free port,
its own serve.py per browser device, its own Harness-* simulator per phone.
Nothing already running is touched — not :8100–8104, not :8787, not the E2E-*
simulators. Logs, transcripts and screenshots land in --out (a temp directory
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
        problems.append(f"no relay binary at {SEMAPHORE} (cargo build -p relay in arc/)")
    if any(c is not i1 for c in cases) and not shutil.which("bun"):
        problems.append("no bun on PATH — the browser agent is a bun sidecar (agent.mjs)")
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
    browser = [c for c in cases if c is not i1]
    phones = [c for c in cases if c is i1]

    async def in_turn(group):
        return [await c(out) for c in group]

    # Separate processes, separate relays: the phones do not wait on the browsers.
    groups = await asyncio.gather(in_turn(browser), in_turn(phones))
    order = {c: i for i, c in enumerate(cases)}
    done = [r for g in groups for r in g]
    return sorted(done, key=lambda r: [c.__name__.upper() for c in cases].index(r.name)
                  if r.name in [c.__name__.upper() for c in cases] else len(order))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="harness.devices", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("what", nargs="?", default="browser", choices=sorted(GROUPS))
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
