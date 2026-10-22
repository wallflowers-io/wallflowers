"""The strings that must be identical in three languages, pinned in one place.

WHY THIS EXISTS. Two payloads in this system are signed by a device and verified
here, and each one is written out separately in Rust, in Python and in
JavaScript:

    pacific-auth:v1        the challenge-response that proves an identity
    pacific-arc-write:v1   the write authorisation behind first-write-claims

A signer and a verifier that disagree by one byte fail in a way NEITHER SIDE CAN
SEE. `auth.verify` refuses without saying which part was wrong, deliberately --
so a drifted domain, a drifted separator and a wrong key all arrive as the same
opaque refusal. There is no log line that names it and no test that catches it,
unless it is this one.

The Arc-write payload is worse than the sign-in one. A refused sign-in is an
outage you notice; a refused WRITE under first-write-claims can be permanent,
because the Arc holds no seed and cannot arbitrate between a claimant and an
impostor. Whoever wrote first keeps the address.

THE CORE IS THE DEFINITION. `pacific_core::identity` and
`pacific_core::locator` are where these bytes are decided; this file reads them
out of the Rust source and holds the Python and JavaScript copies against them.

SKIPS RATHER THAN FAILS WHEN A TREE IS ABSENT. `core/`, `site/` and
`app/` are separate git repos and are not always checked out beside each other.
A missing tree is "cannot check", which is not the same as "checked and fine" --
so it skips loudly, with the path it looked for. A tree that IS present and
disagrees is a failure, every time.
"""
from __future__ import annotations

import re
from pathlib import Path

import pytest

# tests/ -> auth/ -> kenjin/ -> site/ -> the working tree root
# business/website/auth/tests/ -> website -> business -> the workspace. This has
# moved twice: parents[4] under site/kenjin/, parents[3] under site/, and back to
# parents[4] since the site became business/website on 18 Sep 2026. Left wrong it
# finds no core/ and skips every check reporting "not checked out" — green, and
# checking nothing.
ROOT = Path(__file__).resolve().parents[4]
PRODUCT = ROOT / "product"                # app, arc and core: one repository since 18 Sep
CORE = PRODUCT / "core" / "pacific-core" / "src"
WEB = PRODUCT / "app" / "web" / "account" / "pacific-account.js"
SMOKE = PRODUCT / "app" / "web" / "smoke" / "e2e.py"


def _read(p: Path) -> str:
    if not p.exists():
        pytest.skip(f"not checked out beside this repo: {p} — domain drift UNCHECKED")
    return p.read_text(encoding="utf-8")


def _rust_const(src: str, name: str) -> str:
    """The value of a `pub const NAME: &str = "…"` or `&[u8] = b"…"`.

    `AUTH_DOMAIN` did not exist as a named constant until this test asked for it
    -- the string was inlined in `auth_payload`'s format! call, which meant there
    was nothing for a cross-repo check to read. A string that cannot be read
    cannot be pinned, and one that cannot be pinned drifts.
    """
    m = re.search(rf'{name}\s*:\s*&(?:\[u8\]|str)\s*=\s*b?"([^"]+)"', src)
    assert m, (
        f"{name} is no longer a simple string constant in the core, so this test "
        f"cannot read it. Do not delete the test: give the constant back its name."
    )
    return m.group(1)


# ── the two domains ─────────────────────────────────────────────────────────

def test_auth_domain_matches_the_core():
    from app import auth

    core = _rust_const(_read(CORE / "identity.rs"), "AUTH_DOMAIN")
    assert auth.DOMAIN == core, (
        f"app/auth.py DOMAIN={auth.DOMAIN!r} but the core says {core!r}. "
        "Every sign-in fails, and the refusal says nothing about why."
    )


def test_arc_write_domain_matches_the_core():
    from app import arc_write

    core = _rust_const(_read(CORE / "locator.rs"), "ACCT_SIG_DOMAIN")
    assert arc_write.DOMAIN == core, (
        f"app/arc_write.py DOMAIN={arc_write.DOMAIN!r} but the core says {core!r}. "
        "Every write is refused, permanently, and the Arc cannot arbitrate."
    )


def test_key_prefix_matches_the_core():
    from app import auth

    core = _rust_const(_read(CORE / "identity.rs"), "IDENTITY_KEY_PREFIX")
    assert auth.KEY_PREFIX == core


# ── the JavaScript copy, which is the one nobody greps ──────────────────────

def test_the_browser_carries_the_same_auth_domain():
    from app import auth

    js = _read(WEB)
    m = re.search(r"var AUTH_DOMAIN\s*=\s*'([^']+)'", js)
    assert m, "pacific-account.js no longer declares AUTH_DOMAIN the way this test reads it"
    assert m.group(1) == auth.DOMAIN, (
        f"pacific-account.js says {m.group(1)!r}, this service says {auth.DOMAIN!r}"
    )


def test_the_smoke_suite_carries_the_same_auth_domain():
    from app import auth

    smoke = _read(SMOKE)
    found = set(re.findall(r'["\']((?:pacific-auth|pacific-arc-write):v\d+)["\']', smoke))
    assert found, "app/web/smoke/e2e.py no longer names a domain — did the smoke suite stop signing?"
    assert found <= {auth.DOMAIN, "pacific-arc-write:v1"}, (
        f"the smoke suite signs {found}, which this service does not verify"
    )


# ── the payload itself, not just its first line ─────────────────────────────

def test_the_arc_write_payload_is_shaped_the_way_the_core_builds_it():
    """A matching domain is not a matching payload. This pins the separators and
    the field order too, against a vector the core's own test also produces."""
    from app import arc_write

    got = arc_write.payload("arc.example", "ab" * 32, "n1", b"body-bytes")
    assert got == b"pacific-arc-write:v1\narc.example\n" + b"ab" * 32 + b"\nn1\n" + b"body-bytes"
    # THE ENCODING MUST BE INJECTIVE, and it is not for free: `nonce="n\\n",
    # body="\\n"` and `nonce="n", body="\\n\\n"` would build identical bytes, so one
    # signature would authorise both writes. The Arc issues the nonce, so the
    # ambiguity is reachable by the exact party this defends against. Both sides
    # refuse the newline rather than encoding it; this pins that they do.
    from app.errors import AuthError

    assert arc_write.payload("a", "cd" * 32, "n", b"\n\n")  # the honest one still encodes
    with pytest.raises(AuthError):
        arc_write.payload("a", "cd" * 32, "n\n", b"\n")
    with pytest.raises(AuthError):
        arc_write.payload("a\nb", "cd" * 32, "n", b"")


def test_the_auth_payload_is_shaped_the_way_the_core_builds_it():
    from app import auth

    got = auth.payload("kenjin.cc", "ef" * 32, "nonce-1")
    assert got == b"pacific-auth:v1\nkenjin.cc\ned25519:" + b"ef" * 32 + b"\nnonce-1"
