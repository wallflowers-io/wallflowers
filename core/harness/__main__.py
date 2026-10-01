"""The harness, from the command line.

    python -m harness cases      the F-cases against the state objects (no I/O)
    python -m harness objects    the O-cases: the real object catalogue, enforced
    python -m harness units      every Rust unit test we mirror, re-run in Python
    python -m harness live       the same cases against a real semaphore process
    python -m harness property   seeded random walk, diffed against the real relay
    python -m harness render     the same F-cases under rich.live.Live
    python -m harness all        everything, in that order

Exit status is 0 only if every check in every case is green.
"""
from __future__ import annotations

import argparse
import asyncio
import sys

from .cases import ALL, report
from .cases.rust_parity import rust_unit_parity
from .objects.cases import ALL as OBJECT_CASES


def _model_cases(on_step=None):
    return [case(on_step=on_step) for case in ALL]


def _object_cases(on_step=None):
    """The O-cases. No I/O either — they read `core/` and assert over it."""
    return [case(on_step=on_step) for case in OBJECT_CASES]


def _print(results) -> int:
    text, green = report(results)
    print(text)
    print("\nGREEN" if green else "\nRED")
    return 0 if green else 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="harness", description=__doc__)
    ap.add_argument("what", nargs="?", default="cases",
                    choices=["cases", "objects", "units", "live", "property",
                             "render", "all"])
    ap.add_argument("--pace", type=float, default=0.25,
                    help="seconds per step in the live render")
    ap.add_argument("--seed", type=int, default=20260914)
    ap.add_argument("--steps", type=int, default=120)
    args = ap.parse_args(argv)

    if args.what == "units":
        return _print([rust_unit_parity()])
    if args.what == "cases":
        return _print(_model_cases())
    if args.what == "objects":
        return _print(_object_cases())
    if args.what == "live":
        from .live.parity import run_all
        return _print(asyncio.run(run_all()))
    if args.what == "property":
        from .live.fuzz import rounds, walk

        async def both():
            return [await walk(steps=args.steps, seed=args.seed),
                    await rounds(seed=args.seed)]
        return _print(asyncio.run(both()))
    if args.what == "render":
        return _render(args.pace)
    if args.what == "all":
        from .live.fuzz import rounds, walk
        from .live.parity import run_all

        async def everything():
            return (await run_all()) + [await walk(steps=args.steps, seed=args.seed),
                                        await rounds(seed=args.seed)]
        results = [rust_unit_parity(), *_model_cases(), *_object_cases()]
        results += asyncio.run(everything())
        return _print(results)
    return 2


def _render(pace: float) -> int:
    """The F- and O-cases, one frame per step, with the backend footer on screen."""
    from .render import Dashboard, Scene

    results = []
    for case in [*ALL, *OBJECT_CASES]:
        # Each case exposes the hub/bucket/budget it is really mutating (see
        # Run.expose), so the panels show that state and not a decorative copy.
        with Dashboard(Scene(backend="model"), pace=pace) as dash:
            results.append(case(on_step=dash.on_step))
    return _print(results)


if __name__ == "__main__":
    sys.exit(main())
