"""The passkey endpoints document, and the one member that is the point of it.

W3C, "A Well-Known URL for Relying Party Passkey Endpoints". A credential
manager holding a list of saved passkeys fetches
`https://<rp id>/.well-known/passkey-endpoints` and gets back where to make one
(`enroll`), where to manage them (`manage`), and — optionally, and almost never
in practice — `prfUsageDetails`: a page saying what the site does with the PRF
extension.

THAT LAST ONE IS WHY THIS FILE EXISTS. Deriving an encryption key from a
passkey's PRF turns a replaceable credential into a non-replaceable encryption
root, and the manager offering the delete button cannot see that it has done so.
Pacific is the case the warning is not about — the seed is the root, the PRF only
wraps it, and the 24 words are the seed — and the only way to say so where anyone
will read it is to publish the field and keep the page it points at alive.

So the tests below are mostly about DRIFT, not about shape: a document naming a
page that has moved is worse than no document, because it fails at the moment
someone is deciding whether to press delete.
"""
from __future__ import annotations

import os
from pathlib import Path
from urllib.parse import urlparse

import pytest

WELL_KNOWN = "/.well-known/passkey-endpoints"

# The website's site/ tree: another repository, beside this one in the workspace
# (business/website), or wherever WEBSITE names. Absent, these checks skip, saying so.
SITE = Path(os.environ.get("WEBSITE") or Path(__file__).resolve().parents[4] / "business" / "website") / "site"


def _page_for(url: str) -> Path | None:
    """Where nginx would find the page this URL names, or None.

    Two places, because the account pages are served from the ROOT and stored
    outside the web root: /signup is site/soon/signup.html, while /passkey is
    site/passkey.html. Same rule `scripts/check.py` uses on the site's own links.
    """
    name = urlparse(url).path.strip("/")
    for candidate in (SITE / f"{name}.html", SITE / "soon" / f"{name}.html"):
        if candidate.is_file():
            return candidate
    return None


def test_the_passkey_endpoints_document_is_published(client):
    """Free, unauthenticated and the same for everyone: it describes the relying
    party, not an account, so there is nothing in it to prove and nothing in it
    to leak."""
    r = client.get(WELL_KNOWN)
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("application/json")

    doc = r.json()
    assert set(doc) == {"enroll", "manage", "prfUsageDetails"}
    assert all(u.startswith("https://") for u in doc.values())

    # Revalidated rather than cached, like the site's other well-known
    # documents: a corrected page has to reach a credential manager without
    # waiting out an edge TTL.
    assert r.headers["cache-control"] == "no-cache, must-revalidate"
    assert r.headers["access-control-allow-origin"] == "*"


def test_the_document_names_this_deployments_own_pages(client):
    """The URLs are built from PUBLIC_ORIGIN, which is where the audience a
    device signs for comes from too. A staging Arc with kenjin.cc hardcoded here
    would walk its tester out of the copy under test and into the live site."""
    audience = client.get("/auth/health").json()["audience"]
    doc = client.get(WELL_KNOWN).json()
    for member, url in doc.items():
        assert urlparse(url).netloc == audience, member
        assert urlparse(url).scheme == "https", member


def test_prf_usage_details_is_present_and_is_the_reason_this_is_published(client):
    """An empty object would be legal and would say only "there are passkeys
    here". Publishing one without this member would leave a credential manager
    exactly where it is today: unable to tell a site that would lose your data
    when you delete a passkey from a site that would not."""
    doc = client.get(WELL_KNOWN).json()
    assert doc["prfUsageDetails"], "the optional member is the point of the document"
    assert doc["prfUsageDetails"] != doc["manage"], (
        "pointing it at the account page would answer a different question"
    )


@pytest.mark.skipif(not SITE.is_dir(), reason=f"no site tree at {SITE}")
def test_every_url_in_the_document_is_a_page_that_exists(client):
    """The drift test, and the one worth having. These three URLs are read by
    software we do not control, at a moment when the person reading them is
    about to delete something. A renamed page turns this document into three
    dead ends, and nothing else in either tree would notice."""
    doc = client.get(WELL_KNOWN).json()
    for member, url in doc.items():
        assert _page_for(url) is not None, f"{member} → {url} is not a page in {SITE}"


@pytest.mark.skipif(not SITE.is_dir(), reason=f"no site tree at {SITE}")
def test_the_prf_page_answers_the_three_questions_it_is_there_for(client):
    """Not a style check. Someone arrives from a delete button, and the page has
    to say what the passkey unlocks, what deleting it costs, and what to keep
    instead — the words. A page that reassured without naming the words would be
    the dangerous kind of honest."""
    page = _page_for(client.get(WELL_KNOWN).json()["prfUsageDetails"])
    text = page.read_text()
    for phrase in ("passkey", "twenty-four words", "delete"):
        assert phrase in text, phrase
    assert "prf" in text.lower(), "the PRF is what the member promises to describe"


def test_the_page_it_points_at_cannot_be_claimed_as_somebody_elses_site(client):
    """A slug is a path on kenjin.cc, so a slug that collides is a page nobody
    can reach. `_RESERVED` is the only list of those, and the URL this service
    publishes has to be in it — otherwise the answer to "what happens if I
    delete my passkey" is whatever the person who claimed /passkey wrote."""
    from app.main import PRF_USAGE_DETAILS_PATH

    slug = PRF_USAGE_DETAILS_PATH.strip("/")
    verdict = client.get(f"/auth/site/check?slug={slug}").json()
    assert verdict["available"] is False
    assert verdict["reason"] == "reserved"
