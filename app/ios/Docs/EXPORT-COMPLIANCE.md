# Export compliance — the form, and what Pacific actually owes

*Researched 4 Aug 2026. Not legal advice; it is the map of which filing applies and where it lives.
Cross-index: `Docs/TESTFLIGHT.md` (per-build prompt), `ui/project.yml` (the EXPORT COMPLIANCE note).*

## The short answer

**There is no up-front form to file before shipping.** The registration that used to exist —
BIS **Encryption Registration / ERN**, submitted through SNAP-R — was **deleted by BIS in the
2021 encryption rule**. Nothing replaced it as a precondition.

What Pacific owes instead is a single, retrospective filing:

> **The Annual Self-Classification Report** — 15 CFR **§742.15(b)**, for items self-classified
> under License Exception **ENC §740.17(b)(1)** as mass market **5D992.c**.

- **Form:** there is no numbered form. It is a **CSV file emailed as an attachment** (BIS accepts
  CSV *only* — not XLSX, not PDF), in the column order BIS publishes in Supplement No. 8 to Part 742.
- **Where:** `crypt@bis.doc.gov` **and** `enc@nsa.gov` (the ENC Encryption Request Coordinator).
  Both, in one message.
- **When:** by **1 February**, covering the **previous calendar year**. A build distributed in
  2026 is reported by 1 Feb 2027 — so **nothing is due before Pacific can ship**.
- **Contents:** product name, model/part number, ECCN (`5D992.c`), the encryption algorithms and
  key lengths, whether the crypto is "standard", and a URL or brief description of the item.

## Why this is the applicable route, not CCATS

CCATS (a formal BIS classification request, weeks of turnaround) is required for **non-standard
cryptography** or items outside the mass-market thresholds. Pacific is neither:

- It ships **MLS (RFC 9420)** via `mls-rs` + RustCrypto: AES-GCM, HMAC-SHA256, HKDF, ECDH
  (X25519/P-256), Ed25519 — all **published, standard** algorithms. No proprietary or modified
  cryptography, which is the specific trigger for CCATS.
- It is **mass market**: distributed to the general public through the App Store, at no charge,
  with no bespoke per-customer crypto engineering.

That places it squarely in §740.17(b)(1) self-classification → **ECCN 5D992.c** → annual report.

## What Apple asks, and the honest answers

App Store Connect asks per build. The chain for Pacific:

| Apple's question | Pacific's answer | Why |
|---|---|---|
| Does your app use encryption? | **Yes** | MLS end-to-end encryption |
| Does it qualify for any exemption? | **No** | The exemptions cover HTTPS-only, authentication-only, copy protection, or crypto called only through Apple's own frameworks. Pacific ships its own crypto stack. |
| Is your encryption limited to standard algorithms? | **Yes** | see above — no proprietary crypto |
| Do you have a CCATS? | **No** — self-classified | mass market, standard crypto |
| Available in France? | see below | Apple asks separately |

`ITSAppUsesNonExemptEncryption` therefore = **`true`** — as a statement about the app. But see the
next section: setting it in the plist is not, on its own, something Apple will accept.

## 27 Aug 2026 — tested against Apple, and the shortcut does not exist

The paragraph that used to sit here claimed **`true` alone is enough for uploads to stop
prompting**, and that `ITSEncryptionExportComplianceCode` is only for CCATS/ERN-era builds. Build
108 was the first upload carrying `true`. Apple refused it at `-exportArchive`:

```
90592 Invalid Export Compliance Code. The export compliance key value [] in the app's
Info.plist doesn't match the key value of the app's export compliance documentation.
```

The ASC API explains the mismatch:

- `GET /v1/appEncryptionDeclarations?filter[app]=6796835582` → **empty**. No declaration exists,
  so there is no key value on Apple's side for the plist to agree with.
- `GET /v1/builds` → the only build ever answered, 0.1.0 (55), is on file as
  `usesNonExemptEncryption: false`.

So `true` in the binary contradicts the only record Apple holds, and backs onto no declaration.
The claim is **falsified**: `true` is accepted once a declaration exists and the code it yields is
in the plist beside it — not before.

**What actually retires the per-build answering:** file the App Encryption Declaration in App
Store Connect (App Information → App Encryption Declaration — **web UI only**, the API can read
declarations and attach builds to them but cannot create one), then set **both** keys:

```yaml
ITSAppUsesNonExemptEncryption: true
ITSEncryptionExportComplianceCode: <code from the declaration>
```

Until that is done the key stays **absent**, and the questions are answered per build. Absent is
not a false declaration — it declares nothing and defers to the build's own answer. Setting
`false` would be the false one, and remains off the table.

## France

Apple asks whether the app is available in France because ANSSI requires a declaration for
encryption products distributed there. For **standard mass-market crypto** this is a
**declaration** (dossier submitted to ANSSI), not an authorisation. If France is in the storefront
list and this has not been filed, either file it or exclude France from availability — the second
is a one-click answer while the first is done.

## What to do, in order

1. **Now:** file the App Encryption Declaration in the App Store Connect web UI, then set
   `ITSAppUsesNonExemptEncryption: true` **and** `ITSEncryptionExportComplianceCode` together in
   `project.yml` → uploads stop prompting and TestFlight builds flow to testers without a manual
   answer. Doing the first half alone fails the upload (see above).
2. **Before external TestFlight:** answer Apple's France question honestly; file the ANSSI
   declaration or drop France from availability.
3. **By 1 Feb 2027:** email the CSV self-classification report to `crypt@bis.doc.gov` +
   `enc@nsa.gov` covering everything distributed in 2026. Put it in the calendar now — it is the
   only recurring obligation, and it is easy to forget precisely because nothing blocks on it.
4. **If Pacific ever ships non-standard crypto** (a bespoke primitive, a modified protocol), this
   whole analysis changes and CCATS becomes the route. The Arc's model/rail tiers do not change
   it; they use TLS and standard primitives.

## Sources

- BIS, *Mass market (§740.17)* and *Annual self-classification report* —
  https://www.bis.gov/learn-support/encryption-controls/mass-market ·
  https://www.bis.gov/learn-support/encryption-controls/annual-self-classification
- 15 CFR §740.17 (eCFR) — https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-740/section-740.17
- The 2021 rule deleting registration/ERN — Jones Day, Wiley, Baker McKenzie summaries
- Apple, *Complying with Encryption Export Regulations* —
  https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations
