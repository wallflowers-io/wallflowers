"""run_all — drive E10, E11, E12 against a real relay and render what each LEARNED.

Brings up the real semaphore relay (or attaches to RELAY_URL), runs the three
adversaries, and renders each one's growing knowledge set with `rich`. The footer
carries the honesty rule from the-harness §00: these three legs are REAL today.

Usage:
    .venv/bin/python run_all.py
    RELAY_URL=ws://host:8787 .venv/bin/python run_all.py
"""

from __future__ import annotations

import asyncio

from emitter import EmitterUnavailable
from relay_harness import relay
from showcase import run_e10, run_e11, run_e12

try:
    from rich.console import Console
    from rich.panel import Panel
    from rich.table import Table
    from rich import box

    _RICH = True
except Exception:  # noqa: BLE001
    _RICH = False


def _render(e10: dict, e11: dict, e12: dict) -> None:
    if not _RICH:
        import json

        print(json.dumps({"E10": e10, "E11": e11, "E12": e12}, indent=2, default=str))
        return

    console = Console()

    def kn_table(title: str, kn: dict, ok: bool) -> Panel:
        t = Table(box=box.SIMPLE, show_header=False, expand=True)
        t.add_column("k", style="bold", no_wrap=True)
        t.add_column("v")
        t.add_row("tags seen", str(len(kn.get("tags_seen", []))))
        t.add_row("plaintext opened", str(kn.get("plaintext_opened", 0)))
        names = ", ".join(kn.get("names_learned", [])) or "—"
        t.add_row("names learned", names)
        facts = kn.get("facts", [])
        t.add_row("facts learned", str(len(facts)))
        for f in facts[:8]:
            t.add_row("", f"[dim]{f}[/dim]")
        for n in kn.get("notes", [])[:4]:
            t.add_row("note", f"[italic]{n}[/italic]")
        border = "green" if ok else "red"
        verdict = "OPEN (vuln live)" if ok else "closed"
        return Panel(t, title=f"{title}  [{border}]{verdict}[/{border}]", border_style=border)

    console.print(kn_table("E10 · eve @ relay · intro leak", e10["eve_knowledge"], e10["pass"]))
    console.print("  recovered intro fields:", e10.get("recovered_fields"))
    console.print(f"  blob source : {e10['blob_source']}")
    console.print(f"  per-run nonce recovered from ciphertext: {e10['per_run_nonce_recovered']}")
    sweep = e10["operator_sweep"]
    if sweep["available"]:
        console.print(
            f"  operator sweep: {sweep['blobs_opened']}/{sweep['blobs_stored']} stored blobs "
            f"opened under their own routing tag ({sweep['distinct_tags']} tags)"
        )
    console.print()
    console.print(kn_table("E11 · eve @ relay · global-seq correlation", e11["eve_knowledge"], e11["pass"]))
    console.print(f"  published : {e11['published_interleaving']}")
    console.print(f"  recovered : {e11['recovered_interleaving']}")
    console.print(f"  total order across {e11['distinct_tags']} tags: {e11['total_order_across_tags']}")
    console.print()
    console.print(kn_table("E12 · operator @ R2 · confirmation oracle", e12["eve_knowledge"], e12["pass"]))
    for r in e12["oracle_results"]:
        mark = "[green]PRESENT[/green]" if r["present"] else "[red]absent[/red]"
        console.print(f"  probe {r['candidate']:48s} {mark}  key {r['key']}")
    console.print(f"  cross-epoch dedup: {e12['cross_epoch_dedup']}")
    console.print()
    console.print(
        "[bold]BACKEND: real — sockets, crypto, and boto3 are the genuine article "
        "(no ModelDevice here).[/bold]"
    )


async def _main() -> int:
    # store=True: E10's operator-side sweep reads the relay's own SQLite, which is the
    # HostileRelay's view. Torn down with the relay.
    async with relay(store=True) as r:
        e10 = await run_e10(r.url, store_path=str(r.store) if r.store else None)
        e11 = await run_e11(r.url)
        e12 = run_e12()
    _render(e10, e11, e12)
    ok = e10["pass"] and e11["pass"] and e12["pass"]
    print()
    print(f"E10 pass={e10['pass']}  E11 pass={e11['pass']}  E12 pass={e12['pass']}")
    print(
        "E10 is a TRIPWIRE: pass means the intro-seal vulnerability is STILL OPEN. The day "
        "it stops passing, the fix has landed — read harness/adversary/test_e10_intro_tripwire.py."
    )
    return 0 if ok else 1


if __name__ == "__main__":
    try:
        raise SystemExit(asyncio.run(_main()))
    except EmitterUnavailable as e:
        # NO VERDICT. E10 refuses to grade without a live blob out of node.rs's own
        # call sites, and there is deliberately no recorded blob to fall back to — a
        # landed fix and a failed emitter look identical from here.
        print("\nE10: NO VERDICT — could not obtain a live intro blob.\n")
        print(e)
        raise SystemExit(2) from None
