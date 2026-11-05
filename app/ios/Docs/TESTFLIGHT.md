# Pacific iOS — TestFlight

*Status as of 27 Aug 2026. The pipeline was built and verified end-to-end on 3 Aug.*

## Current state

- **Team `9LFA85922G`** — paid Apple Developer Program, individual enrollment (~31 Jul 2026). The free-Personal-Team era (7-day profiles, `.dev` bundle id) is over; store profiles for `network.pacific` + `network.pacific.notifications` run to 2027.
- **App record:** `network.pacific`, Apple ID **6796835582**, store version 1.0 in "Prepare for Submission".
- **Builds in TestFlight:** 0.1.0 (55) uploaded manually via Xcode on 31 Jul; 0.1.0 (62) via this pipeline on 3 Aug (both `VALID`); 0.1.0 (109) on 27 Aug via this pipeline, `VALID`, build id `10929c3c-4dcc-4ef9-9f03-56e4153138d6`. 55 was answered `usesNonExemptEncryption: false`; 62 was never answered, which is why it never reached a tester — **the compliance answer is what gates installability, so answer it the same day you upload.**
- **Build numbers are the commit count on `feat/marketplace`**, which is the release line (main is 48 behind it). Cutting from a shorter branch would move the number backwards and Apple would reject the duplicate.

## Upload a build

```
just testflight        # → scripts/testflight.sh
```

The script: `version.sh set-build` (build number = git commit count, regenerates the project) → `xcodebuild archive` for `generic/platform=iOS` → `xcodebuild -exportArchive`, which **is** the upload (`ui/ExportOptions.plist` sets `destination: upload`). Fully headless — no Xcode GUI, no local distribution certificate (automatic **cloud signing**; the keychain only ever holds the Apple Development identity).

A successful upload takes ~5–15 min of Apple-side processing before the build shows up in App Store Connect → TestFlight (or in `GET /v1/builds`) — poll, don't conclude failure.

## Auth

Everything authenticates with the App Store Connect API key:

- key file: `~/.appstoreconnect/private_keys/AuthKey_$APPLE_KEY_ID.p8` (where `altool`/`xcodebuild` find it)
- key id, issuer id: `APPLE_KEY_ID`, `APPLE_ISSUER_ID` in `env/.env` (gitignored)

Both `xcodebuild` steps pass `-allowProvisioningUpdates -authenticationKeyPath/-KeyID/-KeyIssuerID`, which mints/refreshes profiles and the cloud distribution cert as needed. For ad-hoc ASC queries: `xcrun altool --list-apps --apiKey $APPLE_KEY_ID --apiIssuer $APPLE_ISSUER_ID`, or the REST API with an ES256 JWT (system `python3` has PyJWT).

## Per-build manual step: export compliance

`ITSAppUsesNonExemptEncryption` is deliberately **absent** from Info.plist — Pacific ships real e2e crypto (MLS), which is not an Apple exemption, and `true` needs an `ITSEncryptionExportComplianceCode` that only exists once export documentation is filed (see the EXPORT COMPLIANCE note in `ui/project.yml`, and IOS-SETUP.md §7.6 for the BIS self-classification obligation). Until then: App Store Connect → TestFlight → the new build → answer the encryption questions by hand. Once the code exists, set both keys in project.yml and this step disappears.

## Testers

- **Internal** (≤100, need an ASC role): no review, live minutes after the compliance answer.
- **External** (≤10,000): first build of each version passes Beta App Review (~24h).
- Builds expire 90 days after upload.
