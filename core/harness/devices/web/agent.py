"""web — a browser device and an embedded site, each in its own Chromium.

Two agents over one sidecar, `agent.mjs`:

    WebAgent     a keyholder origin with no UI in front of it — what a browser
                 device holds and does, reached through the same MessagePort a
                 host page is handed.
    EmbedAgent   pacific.js mounted in a member's own page, driven through its
                 open shadow root the way a person would, with its world read
                 back from the api the mount returns.

Both run keyholder.js unmodified, served by app/web/keyholder/serve.py on a port
of the agent's own. That port IS the device: app/web/README.md's "two origins
are two IndexedDB stores" is the whole reason a second agent is a second device
and not a second tab.
"""
from __future__ import annotations

import asyncio
import os
import sys
from pathlib import Path
from typing import Any

from ...live.relay import free_port
from ..protocol import Agent, Sidecar

HERE = Path(__file__).resolve().parent
AGENT = HERE / "agent.mjs"
KENJIN = HERE.parents[2]
KEYHOLDER_DIR = KENJIN / "app/web/keyholder"
BUN = os.environ.get("BUN", "bun")

#: The worlds the browser devices fold. Copied from docs/pacific-two-devices.html,
#: which is the shape keyholder.js's reducer is proven against there.
TWO_DEVICE_SEED = {
    "site": "t", "me": "ada", "sites": {"t": {"name": "T", "kind": "Community", "disc": "chat"}},
    "people": {"ada": {"n": "ada", "home": "t", "role": "Steward", "joined": "x", "on": 1}},
    "threads": [], "events": [], "rsvp": {},
    "convs": [{"id": "c1", "with": "grace", "unread": 0, "msgs": []}],
    "links": [], "pending": [],
}


class KeyholderServer:
    """serve.py on a free port — one per device, reaped on stop.

    AUTH_UPSTREAM is set to a closed loopback port, and not for convenience.
    serve.py's startup line reads `FIXTURE`, a name that no longer exists, so
    with AUTH_UPSTREAM unset it dies with NameError before serving a byte; with
    one set it never reaches that expression. Nothing a device agent does needs
    /auth — whether anyone is signed in is answered from the seed in the
    keyholder's own database — so an unreachable upstream is the honest value:
    anything that did need it fails with upstream_unreachable instead of
    appearing to work.
    """

    def __init__(self, workdir: Path, label: str) -> None:
        self.workdir = Path(workdir)
        self.label = label
        self.port = 0
        self.proc: asyncio.subprocess.Process | None = None
        self._log = None

    @property
    def origin(self) -> str:
        return f"http://localhost:{self.port}"

    async def start(self, timeout: float = 20.0) -> "KeyholderServer":
        self.workdir.mkdir(parents=True, exist_ok=True)
        self.port = free_port()
        self._log = open(self.workdir / f"{self.label}.keyholder.log", "ab")
        self.proc = await asyncio.create_subprocess_exec(
            sys.executable, "serve.py", str(self.port), cwd=str(KEYHOLDER_DIR),
            env={**os.environ, "AUTH_UPSTREAM": f"http://127.0.0.1:{free_port()}"},
            stdout=self._log, stderr=self._log)
        loop = asyncio.get_running_loop()
        deadline = loop.time() + timeout
        while loop.time() < deadline:
            if self.proc.returncode is not None:
                raise RuntimeError(f"serve.py exited {self.proc.returncode} — see "
                                   f"{self.workdir / (self.label + '.keyholder.log')}")
            try:
                _, w = await asyncio.open_connection("127.0.0.1", self.port)
                w.close()
                return self
            except OSError:
                await asyncio.sleep(0.05)
        raise TimeoutError(f"serve.py did not bind {self.port} in {timeout:.0f}s")

    async def stop(self) -> None:
        if self.proc and self.proc.returncode is None:
            self.proc.terminate()
            try:
                await asyncio.wait_for(self.proc.wait(), 5)
            except asyncio.TimeoutError:
                self.proc.kill()
                await self.proc.wait()
        if self._log:
            self._log.close()
            self._log = None


class WebAgent(Agent):
    platform = "web"
    PAGE = "keyholder"
    CANNOT = {
        "obj_post": "the browser folds the interior's own ops (say, post, rsvp…) into a "
                    "per-site log; it holds no GroupObject to post into until the GroupObject "
                    "conformity work lands (multi-platform-harness.html §06, ninth row)",
        "obj_view": "no GroupObject on this side to view — see obj_post",
        "room_new": "no GroupObject on this side to mint — see obj_post",
        "room_add": "no GroupObject on this side to add to — see obj_post",
        "pair_scan": "keyholder.js can mint an MLS contact bundle (device.bundle) but has no "
                     "method that consumes one, so a browser cannot scan a phone; it pairs "
                     "two of one person's devices over X25519 with device_offer and device_accept",
        "pair_accept": "a phone's pair_accept confirms another PERSON by space id after an MLS "
                       "pair_scan; the browser has no MLS contact path — device_accept is its "
                       "X25519 pairing of one person's own devices, a different operation",
        "dm_post": "a browser conversation is a 'say' op in the site's log, not an MLS DM — "
                   "commit {t:'say'} is the honest verb, and it will not reach a phone",
    }

    def __init__(self, name: str, *, workdir: Path, relay_url: str | None = None,
                 site: str = "cambridge-dd") -> None:
        super().__init__(name)
        self.workdir = Path(workdir)
        self.relay_url = relay_url
        self.site = site
        slug = name.replace("/", "-")
        self.keyholder = KeyholderServer(self.workdir, slug)
        self.sidecar = Sidecar([BUN, str(AGENT)], cwd=str(HERE),
                               log=self.workdir / f"{slug}.agent.log")
        self.opened: dict[str, Any] = {}

    def backend(self) -> str:
        chromium = self.sidecar.hello.get("version", "?")
        return (f"{self.platform} — keyholder.js unmodified on {self.keyholder.origin}, "
                f"page on http://localhost:8100 (routed from app/web/docs, granted "
                f"local-network-access), Chromium {chromium} headless shell; real relay")

    async def start(self) -> "WebAgent":
        await self.keyholder.start()
        await self.sidecar.start()
        self.opened = await self.call("open", page=self.PAGE,
                                      keyholder=self.keyholder.origin, site=self.site)
        return self

    async def stop(self) -> None:
        await self.sidecar.stop()
        await self.keyholder.stop()

    async def call(self, verb: str, timeout: float = 90.0, **args: Any) -> Any:
        msg = await self.sidecar.call(verb, args, timeout)
        if not msg.get("ok"):
            raise RuntimeError(msg.get("e") or "the page said no and gave no reason")
        return msg.get("v")

    async def rpc(self, m: str, **a: Any) -> Any:
        return await self.call("rpc", m=m, a=a)

    # -- the verbs -------------------------------------------------------------

    async def v_identity(self) -> dict[str, Any]:
        return await self.rpc("device.identity")

    async def v_seed_world(self, seed: dict[str, Any]) -> Any:
        """Genesis for this site on this device — store.load with a seed is a
        checkpoint at an empty vector, not a delta anybody authored."""
        return await self.rpc("store.load", seed=seed)

    async def v_world(self) -> Any:
        return await self.call("world")

    async def v_device_offer(self) -> str:
        """The X25519 offer a QR carries — pairing two of ONE person's devices."""
        return (await self.rpc("pair.offer"))["offer"]

    async def v_device_accept(self, offer: str) -> str:
        """Accept the other device's offer; returns the tag both sides derive and neither sends."""
        return (await self.rpc("pair.accept", offer=offer))["tag"]

    async def v_sync(self, relay: str | None = None) -> dict[str, Any]:
        relay = relay or self.relay_url
        if not relay:
            raise ValueError("no relay: this device was started without one")
        return await self.rpc("sync.start", relay=relay)

    async def v_sync_status(self) -> dict[str, Any]:
        return await self.rpc("sync.status")

    async def v_commit(self, op: dict[str, Any]) -> Any:
        return await self.rpc("store.commit", op=op)

    async def v_deltas(self, since: dict[str, int] | None = None) -> list[dict[str, Any]]:
        return await self.rpc("store.deltas", since=since or {})

    async def v_pushes(self) -> int:
        return await self.call("pushes")

    async def v_host(self) -> dict[str, Any]:
        return await self.call("host")

    async def v_rpc(self, m: str, a: dict[str, Any] | None = None) -> Any:
        """Any keyholder method, verbatim — the escape hatch, named so a scenario
        that reaches for it is visibly off the shared vocabulary."""
        return await self.call("rpc", m=m, a=a or {})

    async def v_rpc_many(self, calls: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Several keyholder calls issued together from inside the page — how two
        quick actions, or two tabs of one site, arrive at a single keyholder. The
        sidecar answers one request at a time, so without this a harness could
        never make two calls overlap."""
        return await self.call("rpc_many", calls=calls)

    async def v_screenshot(self, path: str, full: bool = False) -> str:
        return await self.call("screenshot", path=path, full=full)

    async def v_console(self, since: int = 0) -> dict[str, Any]:
        return await self.call("console", since=since)


class EmbedAgent(WebAgent):
    platform = "embed"
    PAGE = "embed"

    async def v_seed(self) -> dict[str, Any]:
        """The world pacific.js mounts by default (Pacific.seed) — what a second
        device of this site needs as its genesis to fold the same conversations."""
        return await self.call("seed")

    async def v_ui_open(self) -> dict[str, Any]:
        return await self.call("ui_open")

    async def v_ui_view(self, view: str) -> dict[str, Any]:
        return await self.call("ui_view", view=view)

    async def v_ui_chat_say(self, text: str, conv: str | None = None) -> dict[str, Any]:
        return await self.call("ui_chat_say", text=text, conv=conv)

    async def v_ui_text(self, selector: str) -> str:
        return await self.call("ui_text", selector=selector)
