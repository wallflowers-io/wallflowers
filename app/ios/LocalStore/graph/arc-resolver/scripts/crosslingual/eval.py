#!/usr/bin/env python3
"""Does the embedding model cluster deliberation claims by MEANING or by LANGUAGE?

This decides one thing, before a multilingual corpus exists and locks it in:

    `OpinionGroup` is a centroid over the vote matrix with Statement embeddings
    breaking ties, and `threshold='consensus'` reads the bridging lens — "agreed
    across ALL opinion groups". If the embedder separates languages more strongly
    than it separates positions, opinion groups become LANGUAGE groups, fault lines
    become translation boundaries, and a ratification can never legitimately close.

Three metrics, in ascending order of how much they matter:

  1. TRANSLATION P@1 — for claim i in language A, is its own translation the nearest
     of all claims in language B? The standard cross-lingual retrieval check.

  2. MARGIN — mean cosine(same claim, across languages)
             minus mean cosine(different claims, same language).
     Positive means meaning beats language on average. Near zero or negative is fatal.

  3. NEAREST-NEIGHBOUR IDENTITY — the decisive one, because it is the clustering
     failure directly. For every embedding, is its single nearest neighbour the same
     claim in another language (meaning won) or a different claim in the same
     language (language won)? Anything below ~90% "meaning won" will fragment the
     vote matrix.

A per-language-pair breakdown is printed because the corpus translations are not all
equally trustworthy (see claims.json `_provenance`): a te reo result that lags the
others may be the corpus, not the model, and the breakdown is what tells them apart.

    ~/.mlx-venv/bin/python eval.py                     # candidate vs current baseline
    ~/.mlx-venv/bin/python eval.py --models A B ...    # any HF encoder ids
"""
import argparse
import itertools
import json
import pathlib
import sys

import numpy as np
import torch
from transformers import AutoModel, AutoTokenizer

HERE = pathlib.Path(__file__).resolve().parent

# (hf id, pooling, query prefix). E5 is trained with instruction prefixes and degrades
# without them; for a SYMMETRIC task (claim vs claim) its own card prescribes "query: "
# on both sides. BGE is CLS-pooled and takes no prefix for symmetric similarity.
DEFAULT_MODELS = [
    ("intfloat/multilingual-e5-small", "mean", "query: "),
    ("BAAI/bge-small-en-v1.5", "cls", ""),  # the CURRENT embedder — English-only, the baseline to beat
]


def load_corpus():
    data = json.loads((HERE / "claims.json").read_text())
    claims = data["claims"]
    langs = [k for k in data["_languages"] if not k.startswith("_")]
    texts, claim_ix, lang_ix = [], [], []
    for ci, c in enumerate(claims):
        for li, lg in enumerate(langs):
            texts.append(c[lg])
            claim_ix.append(ci)
            lang_ix.append(li)
    return data, claims, langs, texts, np.array(claim_ix), np.array(lang_ix)


@torch.no_grad()
def embed(model_id, pooling, prefix, texts, batch=16):
    tok = AutoTokenizer.from_pretrained(model_id)
    mdl = AutoModel.from_pretrained(model_id).eval()
    out = []
    for i in range(0, len(texts), batch):
        chunk = [prefix + t for t in texts[i : i + batch]]
        enc = tok(chunk, padding=True, truncation=True, max_length=512, return_tensors="pt")
        h = mdl(**enc).last_hidden_state
        if pooling == "cls":
            v = h[:, 0]
        else:  # mean pooling over real tokens only — padding must not drag the centroid
            m = enc["attention_mask"].unsqueeze(-1).float()
            v = (h * m).sum(1) / m.sum(1).clamp(min=1e-9)
        out.append(torch.nn.functional.normalize(v, p=2, dim=1).cpu().numpy())
    return np.vstack(out).astype(np.float32)


def norm(v):
    return v / np.linalg.norm(v, axis=1, keepdims=True).clip(min=1e-9)


def center_per_language(vecs, lang_ix):
    """Subtract each language's own centroid, then renormalise.

    A multilingual encoder places every language in a slightly different region: the
    raw space is a narrow cone in which "which language is this" is a large shared
    offset and "what does it claim" is a small residual. Removing the per-language
    mean deletes the offset and leaves the residual — the classic isotropy fix.

    NB this needs to know the language of each text, which the ontology does not
    currently record on a Statement. In production the centroid is estimated once
    per language from any sample of that language's statements, not from the batch
    being embedded; estimating it from the evaluation set (as here) is mildly
    optimistic, though the effect size makes that immaterial.
    """
    out = vecs.copy()
    for li in np.unique(lang_ix):
        m = lang_ix == li
        out[m] -= out[m].mean(axis=0, keepdims=True)
    return norm(out)


def all_but_the_top(vecs, k=2):
    """Remove the corpus mean and the top-k principal components, then renormalise.

    The unsupervised alternative to per-language centering (Mu & Viswanath): it needs
    NO language label, because in a multilingual corpus the dominant components tend
    to BE the language directions. Weaker in principle, deployable with less.
    """
    x = vecs - vecs.mean(axis=0, keepdims=True)
    # Right singular vectors are the principal directions; project them out.
    _, _, vt = np.linalg.svd(x, full_matrices=False)
    for i in range(min(k, vt.shape[0])):
        d = vt[i : i + 1]
        x = x - (x @ d.T) @ d
    return norm(x)


def evaluate(vecs, claim_ix, lang_ix, langs, n_claims):
    sim = vecs @ vecs.T
    n = len(vecs)
    same_claim = claim_ix[:, None] == claim_ix[None, :]
    same_lang = lang_ix[:, None] == lang_ix[None, :]
    eye = np.eye(n, dtype=bool)

    # 1. translation P@1, per ordered language pair
    pairs = {}
    for a, b in itertools.permutations(range(len(langs)), 2):
        rows = np.where(lang_ix == a)[0]
        cols = np.where(lang_ix == b)[0]
        sub = sim[np.ix_(rows, cols)]
        got = claim_ix[cols][sub.argmax(axis=1)]
        pairs[(langs[a], langs[b])] = float((got == claim_ix[rows]).mean())

    # 2. margin
    cross_same = sim[same_claim & ~eye & ~same_lang]
    within_diff = sim[~same_claim & same_lang]
    margin = float(cross_same.mean() - within_diff.mean())

    # 3. nearest-neighbour identity — the clustering failure, directly
    masked = np.where(eye, -np.inf, sim)
    nn = masked.argmax(axis=1)
    nn_same_claim = same_claim[np.arange(n), nn]
    nn_same_lang = same_lang[np.arange(n), nn]
    meaning_won = float(nn_same_claim.mean())
    language_won = float((~nn_same_claim & nn_same_lang).mean())

    # Per-language "meaning won", so a weak translation set is visible as itself.
    by_lang = {
        langs[li]: float(nn_same_claim[lang_ix == li].mean()) for li in range(len(langs))
    }
    return {
        "pairs": pairs,
        "margin": margin,
        "cross_same": float(cross_same.mean()),
        "within_diff": float(within_diff.mean()),
        "meaning_won": meaning_won,
        "language_won": language_won,
        "by_lang": by_lang,
        "dim": int(vecs.shape[1]),
    }


def report(model_id, r, langs):
    print(f"\n{'=' * 78}\n{model_id}   (dim {r['dim']})\n{'=' * 78}")
    print("\n  translation P@1")
    header = "        " + "".join(f"{l:>8}" for l in langs)
    print(header + "   (row = source, col = target)")
    for a in langs:
        row = f"    {a:<4}"
        for b in langs:
            row += "       ·" if a == b else f"{r['pairs'][(a, b)] * 100:7.0f}%"
        print(row)

    print(f"\n  margin  {r['margin']:+.3f}"
          f"   (same claim across languages {r['cross_same']:.3f}"
          f"  −  different claims same language {r['within_diff']:.3f})")

    print(f"\n  nearest neighbour is …")
    print(f"    the SAME CLAIM  (meaning won)   {r['meaning_won'] * 100:5.1f}%")
    print(f"    a same-language different claim {r['language_won'] * 100:5.1f}%   ← the failure mode")
    print("    per language: " + "  ".join(f"{l} {r['by_lang'][l] * 100:.0f}%" for l in langs))

    if r["meaning_won"] >= 0.90 and r["margin"] > 0.05:
        print("\n  VERDICT: usable. Opinion clustering will track opinion.")
    elif r["meaning_won"] >= 0.70:
        print("\n  VERDICT: marginal. Some claims will fragment by language — inspect before shipping.")
    else:
        print("\n  VERDICT: UNUSABLE for multilingual deliberation. Opinion groups would be language groups.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", nargs="*", default=None,
                    help="HF encoder ids; pooling is guessed (e5→mean+prefix, else cls)")
    args = ap.parse_args()

    models = DEFAULT_MODELS
    if args.models:
        models = [
            (m, "mean", "query: ") if "e5" in m.lower() else (m, "cls", "")
            for m in args.models
        ]

    data, claims, langs, texts, claim_ix, lang_ix = load_corpus()
    print(f"{len(claims)} claims × {len(langs)} languages = {len(texts)} embeddings")
    print(f"languages: {', '.join(f'{k} ({v})' for k, v in data['_languages'].items())}")
    print(f"P@1 chance level: {100 / (len(claims) - 1):.1f}%")

    results = {}
    for model_id, pooling, prefix in models:
        try:
            vecs = embed(model_id, pooling, prefix, texts)
        except Exception as e:  # a missing download should not lose the other models
            print(f"\n{model_id}: SKIPPED — {type(e).__name__}: {e}")
            continue
        for label, v in (
            ("raw", vecs),
            ("+ per-language centering", center_per_language(vecs, lang_ix)),
            ("+ all-but-the-top (k=2)", all_but_the_top(vecs, 2)),
        ):
            key = model_id if label == "raw" else f"{model_id}  {label}"
            results[key] = evaluate(v, claim_ix, lang_ix, langs, len(claims))
            report(key, results[key], langs)

    if len(results) > 1:
        print(f"\n{'=' * 78}\nSIDE BY SIDE\n{'=' * 78}")
        print(f"  {'model':<56}{'meaning won':>14}{'margin':>9}")
        for m, r in results.items():
            print(f"  {m:<56}{r['meaning_won'] * 100:>13.1f}%{r['margin']:>+9.3f}")

    (HERE / "results.json").write_text(json.dumps(
        {m: {k: (v if not isinstance(v, dict) else {str(kk): vv for kk, vv in v.items()})
             for k, v in r.items()} for m, r in results.items()}, indent=2, ensure_ascii=False))
    print(f"\nwrote {HERE / 'results.json'}")
    return 0 if results else 1


if __name__ == "__main__":
    sys.exit(main())
