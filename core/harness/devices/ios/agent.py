"""ios — a phone, driven the one way iOS 26 leaves open.

WHY A FILE AND NOT A URL. `simctl openurl` stopped being drivable on iOS 26:
every custom-scheme open raises an "Open in Pacific?" confirmation, and a host
script cannot tap it. DevLinks.swift carries the other doorway — `DevCommands`
drains `dev-commands.txt` in the app's own container once a second and hands
each line to the same `DevLinks.handle` that `.onOpenURL` calls. So a verb here
is a `pacific://` line appended to that file, and every line is still exactly
the core call the tapped surface makes. Nothing in the app was changed for this.

DELIVERY IS ACKNOWLEDGED, NOT SLEPT ON. The drain records how many lines it has
consumed in UserDefaults, under `pacific.dev.commandCursor`. This agent reads
that cursor back through `simctl spawn … defaults read`, and a verb returns once
the app has taken its line. That is delivery, not completion: most handlers start
a Task. What a line DID is proven by reading a result file afterwards — never by
waiting a plausible number of seconds, which is what the shell scripts in
ios/scripts do and what sleeps turn into under load.

RESULTS ARE FILES. DevLinks writes `dev-*.txt` beside the command file. A read
verb deletes its file first and waits for the app to write it again, so an
answer can never be a stale one left by an earlier run.

ITS OWN SIMULATORS. A device is `Harness-<name>`, created if it does not exist.
Every start clean-installs the app and resets the simulator's keychain — the
identity lives in the keychain and survives an uninstall — which wipes that
simulator's Pacific. That is why this never touches a simulator it did not name:
E2E-Feed-*, E2E-Ticketing-* and Pacific Demo belong to the scripts in
ios/scripts.
"""
from __future__ import annotations

import asyncio
import json
import os
import plistlib
import time
import urllib.parse
from datetime import datetime
from pathlib import Path
from typing import Any, Callable

from ..protocol import Agent

BUNDLE = "network.pacific"
KENJIN = Path(__file__).resolve().parents[4]   # devices/ios -> harness -> core -> product

#: Where a simulator build of the app lands. The newest wins; PACIFIC_APP overrides.
APP_GLOBS = [
    (Path.home() / "Library/Developer/Xcode/DerivedData",
     "Pacific-*/Build/Products/Debug-iphonesimulator/Pacific.app"),
    (KENJIN.parent / "ios/app/ui/build", "Build/Products/Debug-iphonesimulator/Pacific.app"),
]
DEVICE_TYPE = os.environ.get("HARNESS_SIM_TYPE",
                             "com.apple.CoreSimulator.SimDeviceType.iPhone-17")


def newest_app() -> Path:
    if os.environ.get("PACIFIC_APP"):
        return Path(os.environ["PACIFIC_APP"])
    found = [p for root, pattern in APP_GLOBS if root.exists() for p in root.glob(pattern)]
    if not found:
        raise FileNotFoundError(
            "no Pacific.app built for a simulator — build the Pacific scheme for "
            "iphonesimulator (the Debug build: Release compiles DevLinks out)")
    return max(found, key=lambda p: p.stat().st_mtime)


def app_version(app: Path) -> str:
    info = plistlib.loads((app / "Info.plist").read_bytes())
    built = datetime.fromtimestamp(app.stat().st_mtime).strftime("%d %b %H:%M")
    return (f"{info.get('CFBundleShortVersionString', '?')} "
            f"({info.get('CFBundleVersion', '?')}), built {built}")


async def simctl(*args: str, env: dict[str, str] | None = None,
                 timeout: float = 180.0, check: bool = True) -> str:
    proc = await asyncio.create_subprocess_exec(
        "xcrun", "simctl", *args, env={**os.environ, **(env or {})},
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
    try:
        out, err = await asyncio.wait_for(proc.communicate(), timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
        raise TimeoutError(f"simctl {' '.join(args[:3])} took over {timeout:.0f}s") from None
    if check and proc.returncode != 0:
        raise RuntimeError(f"simctl {' '.join(args[:3])} exited {proc.returncode}: "
                           f"{err.decode(errors='replace').strip()[:400]}")
    return out.decode(errors="replace")


class Simulator:
    def __init__(self, name: str) -> None:
        self.name = name
        self.udid: str | None = None

    async def ensure(self) -> str:
        listing = json.loads(await simctl("list", "devices", "-j"))
        for runtime, devices in listing["devices"].items():
            if "iOS" not in runtime:
                continue
            for d in devices:
                if d["name"] == self.name and d.get("isAvailable", True):
                    self.udid = d["udid"]
                    return self.udid
        runtimes = [r for r in json.loads(await simctl("list", "runtimes", "-j"))["runtimes"]
                    if r.get("isAvailable") and r.get("platform", "") == "iOS"]
        if not runtimes:
            raise RuntimeError("no available iOS simulator runtime")
        newest = max(runtimes, key=lambda r: tuple(int(x) for x in r["version"].split(".")))
        self.udid = (await simctl("create", self.name, DEVICE_TYPE, newest["identifier"])).strip()
        return self.udid

    async def boot(self) -> None:
        await simctl("boot", self.udid, check=False)      # already booted is not an error
        await simctl("bootstatus", self.udid, "-b", timeout=300)

    async def install_fresh(self, app: Path) -> None:
        await simctl("terminate", self.udid, BUNDLE, check=False)
        for _ in range(3):
            await simctl("uninstall", self.udid, BUNDLE, check=False)
            if not (await simctl("get_app_container", self.udid, BUNDLE, check=False)).strip():
                break
        await simctl("keychain", self.udid, "reset", check=False)
        await simctl("install", self.udid, str(app), timeout=300)
        await simctl("privacy", self.udid, "grant", "all", BUNDLE, check=False)

    async def launch(self, env: dict[str, str]) -> None:
        # simctl passes SIMCTL_CHILD_X to the launched app as X.
        await simctl("launch", self.udid, BUNDLE,
                     env={f"SIMCTL_CHILD_{k}": v for k, v in env.items()})

    async def data_dir(self) -> Path:
        container = (await simctl("get_app_container", self.udid, BUNDLE, "data")).strip()
        return Path(container) / "Library/Application Support/pacific"

    async def cursor(self) -> int | None:
        out = await simctl("spawn", self.udid, "defaults", "read", BUNDLE,
                           "pacific.dev.commandCursor", check=False, timeout=30)
        try:
            return int(out.strip())
        except ValueError:
            return None

    async def screenshot(self, path: Path) -> Path:
        await simctl("io", self.udid, "screenshot", str(path))
        return path

    async def terminate(self) -> None:
        await simctl("terminate", self.udid, BUNDLE, check=False)

    async def shutdown(self) -> None:
        await simctl("shutdown", self.udid, check=False)


def _line(text: str, i: int) -> str:
    rows = text.split("\n")
    return rows[i].strip() if len(rows) > i else ""


class IOSAgent(Agent):
    platform = "ios"
    CANNOT = {
        "dm_view": "no DevLinks verb writes a DM transcript out — dump-thread covers rooms "
                   "and dump-profiles covers connections; a DM read needs a dump handler "
                   "in DevLinks.swift",
        "commit": "that is the web keyholder's op vocabulary (say, post, rsvp…); the phone "
                  "authors core deltas into GroupObjects, and the two do not fold the same "
                  "thing yet",
        "device_offer": "that is the web's X25519 pairing of one person's own devices; the "
                        "phone pairs by MLS contact bundle (bundle, then pair_scan or room_add)",
        "device_accept": "that is the web's X25519 pairing of one person's own devices — see "
                         "device_offer",
    }

    #: How long the app gets to take a line off the command file. It drains every second.
    DRAIN = 30.0
    #: How long a Task the line started gets to write its result file.
    RESULT = 60.0

    def __init__(self, name: str, relay_url: str, *, display_name: str,
                 app: str | os.PathLike | None = None, sim_name: str | None = None,
                 keep_booted: bool = False) -> None:
        super().__init__(name)
        self.relay_url = relay_url
        self.display_name = display_name
        self.app = Path(app) if app else newest_app()
        self.sim = Simulator(sim_name or "Harness-" + name.replace("/", "-"))
        self.keep_booted = keep_booted
        self.dir: Path | None = None
        self._cmd = asyncio.Lock()
        self._read = asyncio.Lock()

    def backend(self) -> str:
        return (f"ios — Pacific.app {app_version(self.app)} on simulator {self.sim.name}; "
                f"real core, real MLS, real relay")

    async def start(self) -> "IOSAgent":
        await self.sim.ensure()
        await self.sim.boot()
        await self.sim.install_fresh(self.app)
        await self.sim.launch({
            "PACIFIC_RELAY_URL": self.relay_url,
            "PACIFIC_IDENTITY_NAME": self.display_name,
            # Without this the app mints sample threads on top of the real ones,
            # and a convergence check would be comparing two copies of a fixture.
            "PACIFIC_SKIP_MOCK_SEED": "1",
        })
        self.dir = await self.sim.data_dir()
        self.dir.mkdir(parents=True, exist_ok=True)
        await self._wait_file("dev-contact.txt", 120.0, lambda t: bool(_line(t, 0)),
                              "the app to mint an identity and export its contact")
        return self

    async def stop(self) -> None:
        if self.sim.udid:
            await self.sim.terminate()
            if not self.keep_booted:
                await self.sim.shutdown()

    # -- the doorway -----------------------------------------------------------

    async def send(self, host: str, **query: Any) -> dict[str, Any]:
        """Append one pacific:// line and return once the app has drained it."""
        pairs = [f"{k}={urllib.parse.quote(str(v), safe='')}"
                 for k, v in query.items() if v is not None]
        url = f"pacific://{host}" + ("?" + "&".join(pairs) if pairs else "")
        async with self._cmd:
            path = self.dir / "dev-commands.txt"
            with open(path, "a", encoding="utf-8") as fh:
                fh.write(url + "\n")
            # The drain counts lines exactly as Swift's split(omittingEmptySubsequences:) does.
            line = sum(1 for row in path.read_text(encoding="utf-8").split("\n") if row)
            t0, cursor = time.monotonic(), None
            while time.monotonic() - t0 < self.DRAIN:
                cursor = await self.sim.cursor()
                if cursor is not None and cursor >= line:
                    return {"link": host, "line": line,
                            "drained_ms": int((time.monotonic() - t0) * 1000)}
                await asyncio.sleep(0.25)
        raise TimeoutError(f"the app did not take line {line} ({host}) off dev-commands.txt "
                           f"in {self.DRAIN:.0f}s (cursor={cursor}) — is it still running?")

    async def _wait_file(self, name: str, timeout: float,
                         ready: Callable[[str], bool] = lambda t: True, what: str = "") -> str:
        path = self.dir / name
        t0 = time.monotonic()
        while time.monotonic() - t0 < timeout:
            try:
                text = path.read_text(encoding="utf-8")
                if ready(text):
                    return text
            except FileNotFoundError:
                pass
            await asyncio.sleep(0.25)
        raise TimeoutError(f"no {name} after {timeout:.0f}s waiting for {what or name}")

    async def fresh(self, name: str, host: str, ready: Callable[[str], bool] = lambda t: True,
                    **query: Any) -> str:
        """Delete a result file, send the verb that writes it, and read what comes back."""
        async with self._read:
            (self.dir / name).unlink(missing_ok=True)
            await self.send(host, **query)
            return await self._wait_file(name, self.RESULT, ready, f"{host} to write it")

    # -- the verbs -------------------------------------------------------------

    async def v_identity(self) -> dict[str, str]:
        text = (self.dir / "dev-contact.txt").read_text(encoding="utf-8")
        return {"space": _line(text, 0), "bundle": _line(text, 1), "name": self.display_name}

    async def v_bundle(self) -> str:
        """A FRESH contact bundle. Key packages are single-use, so every add wants
        a new one, and a scanner handed a consumed one fails "key package not found"."""
        old = (await self.v_identity())["bundle"]
        async with self._read:
            await self.send("export-contact")
            text = await self._wait_file(
                "dev-contact.txt", self.RESULT,
                lambda t: bool(_line(t, 1)) and _line(t, 1) != old,
                "export-contact to write a new bundle")
        return _line(text, 1)

    async def v_sync(self) -> dict[str, Any]:
        return await self.send("sync")

    async def v_pair_scan(self, bundle: str) -> dict[str, Any]:
        return await self.send("pair-scan", bundle=bundle)

    async def v_pair_accept(self, peer: str) -> dict[str, Any]:
        return await self.send("pair-accept", peer=peer)

    async def v_dm_post(self, peer: str, text: str) -> dict[str, Any]:
        return await self.send("dm-send", peer=peer, text=text)

    async def v_dm_reply(self, peer: str, text: str, author: str | None = None,
                         gen: int | None = None) -> dict[str, Any]:
        return await self.send("dm-reply", peer=peer, text=text, author=author, gen=gen)

    async def v_room_new(self) -> str:
        return _line(await self.fresh("dev-room.txt", "new-room", lambda t: bool(_line(t, 0))), 0)

    async def v_room_add(self, object: str, bundle: str) -> dict[str, Any]:
        return await self.send("room-add-member", object=object, bundle=bundle)

    async def v_obj_post(self, object: str, text: str) -> dict[str, Any]:
        return await self.send("room-post", object=object, text=text)

    async def v_obj_reply(self, object: str, text: str, match: str | None = None) -> dict[str, Any]:
        return await self.send("room-reply", object=object, text=text, match=match)

    async def v_obj_view(self, object: str) -> list[dict[str, Any]]:
        """The folded thread as `depth|author8|text` rows — CoreService.dumpThread."""
        text = await self.fresh("dev-thread.txt", "dump-thread", object=object)
        rows = []
        for row in text.split("\n"):
            if not row:
                continue
            depth, author, body = (row.split("|", 2) + ["", ""])[:3]
            rows.append({"depth": int(depth) if depth.isdigit() else depth,
                         "author": author, "text": body})
        return rows

    async def v_profiles(self) -> list[list[str]]:
        text = await self.fresh("dev-profiles.txt", "dump-profiles")
        return [row.split("|") for row in text.split("\n") if row]

    async def v_set_profile(self, name: str | None = None, org: str | None = None,
                            title: str | None = None, note: str | None = None) -> dict[str, Any]:
        return await self.send("set-profile", name=name, org=org, title=title, note=note)

    async def v_link(self, url: str) -> dict[str, Any]:
        """Any pacific:// line, verbatim — the escape hatch. Named so that a scenario
        reaching for it is visibly off the shared vocabulary."""
        parts = urllib.parse.urlsplit(url)
        if parts.scheme != "pacific":
            raise ValueError("only pacific:// links reach DevLinks")
        return await self.send(parts.netloc, **dict(urllib.parse.parse_qsl(parts.query)))

    async def v_screenshot(self, path: str) -> str:
        return str(await self.sim.screenshot(Path(path)))
