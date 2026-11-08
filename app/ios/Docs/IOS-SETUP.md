# Shipping an iOS/iPadOS App with a Rust Core and a Thin Swift Wrapper — The Straightforward Path

*A first-app guide for an experienced engineer new to Apple's ecosystem. Current as of 2026-06-25. Version numbers carry their as-of date; re-check anything time-sensitive before you pin it.*

This guide assumes the durable givens of an app like Nova: a Rust core (MLS via `mls-rs`, SQLCipher, tokio, a serverless blind-relay client, on-device RAG), a single UniFFI bridge above the UI, a thin SwiftUI shell, one app target serving iPhone **and** iPad, iOS 17+ deployment, and an end-to-end-encrypted messenger. It assumes you are on **Apple Silicon macOS** (if you are on an Intel Mac, see the note in §5 — it changes one target and one `lipo` line, and Intel is the *last* macOS-Tahoe-capable generation, so move to Apple Silicon if you can).

---

## 1. TL;DR — the recommended stack

| Concern | Pick | One-liner why |
|---|---|---|
| **Project generation** | **XcodeGen** (`project.yml`) | A 30-line YAML you can diff; `.xcodeproj` is merge-hostile XML. Tuist is overkill solo; pure-SwiftPM can't carry Info.plist/icons/signing for an app target. |
| **FFI bridge** | **UniFFI 0.31.2** (proc-macros, no UDL) | Only option giving Swift `async`/`throws`/foreign-traits **and** re-emitting Kotlin for a future Android core. *(crates.io 2026-06-17)* |
| **Packaging** | **XCFramework → SwiftPM `binaryTarget` (local `path:`)** | Xcode consumes a finished binary → fast indexing, working Previews. Device slice + a fat simulator slice. |
| **Build tool** | **A hand-owned `build-ios.sh`**; `cargo-swift` v0.11.1 to bootstrap | The script is one path for laptop + CI and gives you control over the SQLCipher C-build env. `cargo-swift` *(2026-05-20)* gets you green on day one. |
| **Test frameworks** | **`cargo test` on host (the bulk) + Swift Testing** | ~70% of logic is platform-agnostic Rust. Swift Testing (`@Test`) is the Xcode 26 default; XCTest only for UI/perf. |
| **CI** | **GitHub Actions `macos-26`** | Runs *both* `cargo test` and `xcodebuild test` in one YAML; Xcode Cloud can't run native `cargo test`. |
| **Distribution** | **TestFlight Internal → App Store, signed Automatically; Fastlane later** | Internal testers need no review and go live in minutes. Manual Archive first; Fastlane when you automate. |

---

## 2. Mental model — one FFI boundary, placed above the UI

There is exactly **one** language boundary in the whole app, and it sits between your Rust core and your Swift UI. Everything below it is Rust; everything above is Swift. UniFFI generates the glue automatically from annotations on your Rust code.

The Rust core is compiled to a static library (`.a`), packaged per-platform into an **`.xcframework`**, and paired with **UniFFI-generated Swift sources**. A **thin Swift wrapper package** (`NovaCore`) re-exports that, hiding the raw FFI symbols behind an idiomatic API. The **SwiftUI app target** imports only `NovaCore`.

Callbacks go *inward* too: Swift implements a handful of Rust traits (a Keychain byte-store, a Secure-Enclave Signer, an `EmbeddingProvider` over Apple's `NLEmbedding`) and hands instances into Rust — that's how `mls-rs`'s signature provider reaches the Secure Enclave.

```
┌──────────────────────────────────────────────┐
│  SwiftUI app target  (Nova.app)              │   import NovaCore
│   views · navigation · camera/Vision QR      │
├──────────────────────────────────────────────┤
│  NovaCore  (thin Swift wrapper package)      │   idiomatic Swift API
│   + Swift impls of Rust traits ───callbacks──┼──┐  (Keychain, SE Signer,
├──────────────────────────────────────────────┤  │   EmbeddingProvider)
│  UniFFI-generated Swift  +  NovaCoreRS.xcframework │
│  ════════════════ THE ONE FFI BOUNDARY ═════════ │
├──────────────────────────────────────────────┤  │
│  nova-ffi   (#[uniffi::export] facade)  ◄───────┘
│  nova-core  (sync log · mls-rs · sqlcipher · tokio · RAG)
└──────────────────────────────────────────────┘   Rust workspace
```

---

## 3. Recommended repo structure

Two top-level folders: `rust/` (the Cargo workspace) and `apple/` (the SwiftPM package + generated project). Generated artifacts are committed for local dev so the app builds without first running cargo.

```
nova/
├── rust/
│   ├── Cargo.toml                     # [workspace]
│   ├── nova-core/                     # all the real logic — pure, platform-agnostic
│   │   ├── src/lib.rs
│   │   └── tests/                     # cargo integration tests (run on host)
│   ├── nova-ffi/                      # the UniFFI facade: #[uniffi::export] only
│   │   ├── src/lib.rs                 #   uniffi::setup_scaffolding!();  (no build.rs)
│   │   └── Cargo.toml                 #   [lib] crate-type = ["staticlib"]
│   └── uniffi-bindgen/                # tiny bin crate that runs the bindgen
│       └── src/main.rs                #   fn main() { uniffi::uniffi_bindgen_main() }
│
├── apple/
│   ├── build-ios.sh                   # the canonical build script (§5)
│   ├── Artifacts/
│   │   └── NovaCoreRS.xcframework/    # built: ios-arm64 + ios-sim-fat slices
│   ├── Sources/
│   │   ├── UniFFI/                    # generated *.swift bindings (checked in)
│   │   └── NovaCore/                  # YOUR thin wrapper + Swift trait impls
│   │       ├── NovaCore.swift         #   re-export + idiomatic helpers
│   │       ├── KeychainStore.swift    #   Swift impl of the Rust byte-store trait
│   │       └── SecureEnclaveSigner.swift
│   ├── Package.swift                  # 3 targets: NovaCoreRS ← UniFFI ← NovaCore
│   ├── project.yml                    # XcodeGen: the app target
│   ├── App/                           # SwiftUI sources, Assets.xcassets, Info.plist
│   │   ├── NovaApp.swift              #   @main
│   │   └── Nova.entitlements
│   └── Nova.xcodeproj/                # GENERATED by xcodegen — git-ignored
│
├── fixtures/cbor/                     # golden vectors, read by BOTH languages (§6)
└── .github/workflows/test.yml
```

`apple/Package.swift` wires three targets so the app imports only `NovaCore`:

```swift
// swift-tools-version: 5.10
import PackageDescription

let package = Package(
  name: "NovaCore",
  platforms: [.iOS(.v17)],
  products: [.library(name: "NovaCore", targets: ["NovaCore"])],
  targets: [
    .binaryTarget(name: "NovaCoreRS", path: "Artifacts/NovaCoreRS.xcframework"),
    .target(name: "UniFFI",   dependencies: ["NovaCoreRS"], path: "Sources/UniFFI"),
    .target(name: "NovaCore", dependencies: ["UniFFI"],     path: "Sources/NovaCore"),
  ]
)
```

> **On signing the XCFramework:** a SwiftPM `binaryTarget` referenced by a **local `path:`** does **not** need to be code-signed. Since Xcode 15, Xcode *verifies* a framework's signature on a trust-on-first-use basis but explicitly permits unsigned frameworks. The only hard SwiftPM requirement is for **remote** binary targets, which need a SHA-256 `checksum:`, not a signature. Don't waste time signing the `.xcframework` for local development.

---

## 4. The FFI boundary (UniFFI)

**Confirmed pick: UniFFI 0.31.2, driven by proc-macros.** It uniquely gives you native Swift `async`/`throws`, foreign traits with compiler-enforced `Send + Sync` (your inward callbacks), and the *same annotations also generate Kotlin* for the later Android core. cxx is disqualified (Rust↔C++ only, no Swift). swift-bridge is Swift-only and effectively single-maintainer. Hand-rolled cbindgen is weeks of boilerplate UniFFI generates for free.

**Use raw `mls-rs`, not `mls-rs-uniffi`.** The published wrapper is stuck at **v0.1.0 (2024-03-26)**, hardcodes its crypto provider (`OpensslCryptoProvider`) and identity provider — so it does **not** let you plug in a Swift Secure-Enclave signature provider, which Nova requires. (One nuance worth knowing: it *does* expose group-state **storage** as a foreign-pluggable trait via `uniffi::export(with_foreign)` — so mine its `src/config/group_state.rs` as a worked reference for the callback pattern, then write your own facade over raw `mls-rs`.)

### Minimal mechanics

Annotate Rust in `nova-ffi/src/lib.rs`. With proc-macros you add `setup_scaffolding!()` and need **no** `build.rs` and **no** UDL file:

```rust
uniffi::setup_scaffolding!();

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum NovaError {
    #[error("not paired")] NotPaired,
    #[error("relay: {0}")] Relay(String),
}

#[derive(uniffi::Object)]
pub struct Core { /* Arc-able state; interior mutability only */ }

#[uniffi::export]
impl Core {
    #[uniffi::constructor]
    pub fn new(keychain: Arc<dyn KeychainStore>) -> Arc<Self> { /* ... */ }

    // async fn → Swift `func ... async throws`. Tokio is wrapped automatically.
    #[uniffi::export(async_runtime = "tokio")]
    pub async fn drain_relay(&self) -> Result<u32, NovaError> { /* ... */ }
}

// An inward callback: Swift implements this, Rust calls it.
#[uniffi::export(with_foreign)]
pub trait KeychainStore: Send + Sync {
    fn get(&self, key: String) -> Result<Option<Vec<u8>>, NovaError>;
    fn set(&self, key: String, value: Vec<u8>) -> Result<(), NovaError>;
}
```

The generated Swift is idiomatic — `Object`→`final class` (Arc-backed), `Record`→`struct`, `Enum`→`enum`, snake_case→camelCase, `Result<T,E>`→`throws`, `async fn`→`async throws`. Swift calls it like native code:

```swift
import NovaCore

let core = try Core(keychain: KeychainStore())   // Swift class implements the Rust trait
let n = try await core.drainRelay()              // async across the FFI; main thread never blocks
```

### The rules you must follow

- **Async:** mark FFI async methods `#[uniffi::export(async_runtime = "tokio")]`. Own **one** long-lived `multi_thread` tokio runtime, created once at init — `current_thread` would serialize your relay + MLS + SQLite work. Known sharp edge: keep async methods on **concrete exported structs**, not on trait-object callbacks. Build **cancellation as a separate `cancel()` method**, not via the future (UniFFI's own guidance).
- **Errors:** map your Rust error tree to **one** `#[derive(uniffi::Error)]` enum at the boundary → native Swift `Error` used with `do/catch`.
- **Callbacks (foreign traits):** the trait must be `Send + Sync`; **every method must return `Result<>`**; implement `From<uniffi::UnexpectedUniFFICallbackError>` for your error type **or Rust panics across the FFI**; **parameters pass by value** — no references (use `String`, not `&str`). The old standalone "callback interfaces" are soft-deprecated — use foreign traits.
- **Streams:** there is no native stream type. Expose an **observer foreign trait** (`onEvent(...)`) that Rust calls from a task draining a **bounded `tokio::sync::mpsc`/`broadcast`**, and bridge to a Swift `AsyncStream`.
- **Blobs — file-URL handoff, not `Data`:** `Vec<u8>` returns still full-copy through a RustBuffer. (UniFFI's zero-copy `&[u8]` path is foreign→Rust **only**, requires a `[ByRef]` annotation, and as of 0.31.2 is still **unreleased** — do not design around it.) Large MLS/media payloads must cross as a **file-URL string** pointing into the app sandbox (ideally the App Group container), never as `Data`.
- **Thread safety is enforced:** every `Object` is `Send + Sync` with **no `&mut self`** — use `Mutex`/`RwLock`/atomics for interior mutability.

---

> **Day-to-day, you do not run any of this.** `just build|test|run` regenerate
> the bindings and the project file themselves — see [FFI-REGEN.md](FFI-REGEN.md).
> This section is how the pipeline was BUILT, and is written generically (it says
> "Nova" throughout); the names in this repo are `pacific-ffi` / `PacificCore` /
> `PacificCoreRS.xcframework`.

## 5. Build pipeline — zero to XCFramework

### Step 0 — install full Xcode (Command Line Tools are a dead end)

CLT gives you clang/swift/git but **not** the iOS SDK, Simulator runtimes, or a usable `xcodebuild` — `xcodebuild` refuses to run against a CLT-only developer directory, and `-create-xcframework` lives only in Xcode. As of mid-2026 the current release is **Xcode 26.5 (2026-05-11)**, which bundles the iOS 26 SDK and requires **macOS Tahoe 26.2+** (Xcode 26.0–26.3 ran on Sequoia 15.6). The iOS 26 SDK is **mandatory for App Store Connect uploads since 2026-04-28**.

```bash
# Install Xcode from the Mac App Store (~12-15 GB), then:
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
sudo xcodebuild -license accept
xcodebuild -runFirstLaunch            # installs bundled components incl. simctl
xcodebuild -downloadPlatform iOS      # installs the iOS Simulator runtime
xcodebuild -version                   # must succeed before continuing
```

### Step 1 — add the Rust targets

On **Apple Silicon** you need the device target plus the Apple-Silicon simulator target. Add the Intel-simulator slice too so the XCFramework runs on *any* Mac (CI, teammates):

```bash
rustup target add aarch64-apple-ios       # real iPhone/iPad (device)
rustup target add aarch64-apple-ios-sim   # Simulator on Apple-Silicon Macs
rustup target add x86_64-apple-ios        # Simulator on Intel Macs (host-agnostic XCFramework)
```

`aarch64-apple-ios` (device) and `aarch64-apple-ios-sim` (simulator) are **distinct targets** even though both are arm64 — they carry different Mach-O platform tags, so `lipo` *refuses* to merge them. The correct structure is: **device slice alone**, plus a **fat simulator slice** (`aarch64-apple-ios-sim` + `x86_64-apple-ios` lipo'd together, same platform tag). Forgetting the `-sim` slice is the #1 first-build failure ("no such module" on Run).

> **Intel Mac?** Your live simulator is `x86_64-apple-ios`. The script below already builds it, so nothing changes — but note Tahoe 26 is the last Intel-capable macOS and Xcode 26.4+ needs Tahoe, so plan to move to Apple Silicon.

### Step 2 — Cargo.toml

```toml
[lib]
crate-type = ["staticlib"]   # a .a linked into the app — the simplest correct choice

[profile.release]
lto = "fat"; codegen-units = 1; panic = "abort"; opt-level = "s"; strip = true
```

Ship `--release`: SQLCipher and MLS crypto are unusably slow in debug. **Bitcode is dead** — removed in Xcode 15, rejected by the App Store; add no bitcode flags and ignore any pre-2023 tutorial that does.

### Step 3 — `apple/build-ios.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
CRATE="nova_ffi"; LIB="lib${CRATE}.a"; OUT="target/ios"
export IPHONEOS_DEPLOYMENT_TARGET=17.0

# 1) compile all three slices
for T in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build -p "$CRATE" --release --target "$T"
done

# 2) fat SIMULATOR lib = arm64-sim + x86_64-sim  (device stays separate!)
mkdir -p "$OUT/sim"
lipo -create \
  "target/aarch64-apple-ios-sim/release/$LIB" \
  "target/x86_64-apple-ios/release/$LIB" \
  -output "$OUT/sim/$LIB"

# 3) generate UniFFI Swift bindings from the COMPILED .a (library mode → sees proc-macros)
HEADERS="$OUT/headers"; mkdir -p "$HEADERS"
cargo run -p uniffi-bindgen -- generate \
  --library "target/aarch64-apple-ios/release/$LIB" \
  --language swift --out-dir "$OUT/swift"
cp "$OUT"/swift/*.h "$HEADERS"/
# Xcode only finds a modulemap named exactly module.modulemap:
cp "$OUT"/swift/*.modulemap "$HEADERS/module.modulemap"
cp "$OUT"/swift/*.swift apple/Sources/UniFFI/

# 4) assemble the XCFramework: device slice + fat simulator slice
rm -rf apple/Artifacts/NovaCoreRS.xcframework
xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/release/$LIB" -headers "$HEADERS" \
  -library "$OUT/sim/$LIB"                          -headers "$HEADERS" \
  -output apple/Artifacts/NovaCoreRS.xcframework
echo "→ apple/Artifacts/NovaCoreRS.xcframework + apple/Sources/UniFFI/*.swift"
```

Run this **only when the FFI surface changes**. Between FFI changes, iterate on logic with plain `cargo test`. Plug the artifact into the app via the local SwiftPM `binaryTarget` (§3); `xcodegen generate` then produces `Nova.xcodeproj`.

> **Day-1 shortcut:** before committing the script, run `cargo install cargo-swift` (**v0.11.1, 2026-05-20**, pairs with UniFFI 0.31.x) and `cargo swift package` once. It wraps targets+bindgen+xcframework into a ready Swift package and *proves your toolchain is sane* end-to-end. Then own the script for control over the SQLCipher C-build env and CI. *(cargo-lipo is deprecated — it only makes obsolete fat libs; do not use it.)*

### Cross-compile notes for the C dependencies

- **SQLCipher (`rusqlite`):** use **CommonCrypto, not OpenSSL** — the single biggest gotcha. On Apple platforms SQLCipher uses Apple's CommonCrypto natively; cross-compiling OpenSSL for iOS is painful and bloats the binary.
  ```toml
  rusqlite = { version = "0.38", features = ["bundled-sqlcipher"] }  # NOT *-vendored-openssl
  ```
  Define `-DSQLCIPHER_CRYPTO_CC` and link `Security.framework`; this drops `openssl-sys` from your tree entirely. The classic failure — `'TargetConditionals.h' not found` during the `cc` compile — means the C compiler isn't pointed at the iOS SDK. On a healthy Xcode, rustc/`cc` infer the SDK; only if it fails, set `SDKROOT` **inside the per-target loop** (`xcrun --sdk iphoneos --show-sdk-path` for device, `iphonesimulator` for sim) — **never a single global `SDKROOT`** across a device+sim loop.
- **`mls-rs` / tokio / `ring` / `getrandom`:** these "just work" on the macOS→iOS path (the scary GitHub issues are about cross-compiling *from Linux*). Prefer `rustls` + `ring`/`aws-lc-rs` over OpenSSL to avoid an OpenSSL cross-build; `mls-rs` already favors this.
- **Speed:** `cargo install sccache && export RUSTC_WRAPPER=sccache` — a real win since each crate compiles three times.

---

## 6. Testing

The pyramid **inverts** the usual iOS shape: most logic is platform-agnostic Rust, so most tests are host `cargo test`. Only the FFI boundary must touch a simulator.

| Layer | Tool | Where | Effort | Command |
|---|---|---|---|---|
| **1. Rust unit/integration** | `cargo test` | **Host** (native arm64) | ~70% | `cargo test --workspace` |
| **2. Cross-language golden vectors** | `cargo test` + Swift Testing read same `.cbor` | Host + Sim | ~10% | `cargo test golden` / `xcodebuild test` |
| **3. FFI smoke test** | **Swift Testing** over UniFFI | Simulator | ~15% | `xcodebuild test` |
| **4. UI (XCUITest)** | XCTest only | Simulator | ~5%, **defer** | `xcodebuild test` |

**Run Rust tests on the host, not on iOS.** `cargo test --target …-sim` is possible but needs a custom runner/`cargo-dinghy` and getting the exit status out of the simulator is, per dinghy's author, "more or less cursed." Your sync protocol, CBOR codec, delta log, store, and MLS group logic are platform-agnostic — test them with plain `cargo test`. The one thing that genuinely needs a simulator is the FFI boundary (layer 3).

**Use Swift Testing (`@Test`), not XCTest, for everything except UI.** It ships with Xcode 16+/26 out of the box, runs parallel by default, and `#expect`/`#require` replace 40+ `XCTAssert*`. XCTest is **not** deprecated and remains **required** for XCUITest (UI automation) and `XCTMetric` performance — the two coexist in one target, so this is zero-lock-in.

**Build the FFI smoke test first** — it proves the whole architecture in one shot:

```swift
import Testing
@testable import NovaCore

@Test func core_roundtrips_an_async_call() async throws {
    let core = try Core(keychain: SpyKeychain())
    #expect(try await core.drainRelay() == 0)        // async across the FFI
}

@Test func typed_error_surfaces_to_swift() async {
    await #expect(throws: NovaError.notPaired) {
        try await Core(keychain: SpyKeychain()).requireSession()
    }
}

@Test func swift_keychain_callback_is_invoked() async throws {
    let spy = SpyKeychain()                           // Swift impl of the Rust trait
    let core = try Core(keychain: spy)
    try await core.storeIdentityKey()
    #expect(spy.writes.count == 1)                    // Rust called back INTO Swift
}
```

**Golden CBOR vectors — commit the bytes, both languages read them, never regenerate in-test.** Store `fixtures/cbor/handshake_v1.cbor` (plus a human-readable `.json` of the input). The Rust test asserts `encode(value) == fixture_bytes`; the Swift test asserts the same. Use a **deterministic** codec on both sides (RFC 8949 §4.2: shortest ints/lengths, bytewise-sorted map keys). `ciborium` gives preferred encodings in Rust; for strict byte-identity, **Blockchain Commons `dcbor`** has matched Rust + Swift reference implementations — the lowest-risk cross-language pairing. Regenerate the golden only via a deliberate `--bless` script reviewed in git.

**Simulator vs device:** develop and CI on the **simulator** (free, fast, no signing — set `CODE_SIGNING_ALLOWED=NO`). Use a **physical device** only for what the simulator fakes badly: **Secure Enclave** (the simulator has *no* SEP — `SecureEnclave.isAvailable` is false; SE key ops fail), camera/Vision QR pairing, real APNs, and true performance. Gate those behind a manual device checklist before each release.

### Minimal CI (`.github/workflows/test.yml`)

```yaml
name: test
on: [push, pull_request]
jobs:
  test:
    runs-on: macos-26                       # Apple Silicon; Xcode 26 + sim runtimes preinstalled
    steps:
      - uses: actions/checkout@v4
      - run: sudo xcode-select -s /Applications/Xcode_26.5.app   # PIN explicitly (check runner README)
      - name: Rust + iOS targets
        run: |
          rustup default stable
          rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
      - uses: Swatinem/rust-cache@v2
      - name: Rust tests (host) — includes golden vectors
        run: cargo test --workspace
      - name: Build xcframework
        run: ./apple/build-ios.sh
      - name: Swift / FFI tests (simulator)
        run: |
          xcodebuild test -scheme NovaCore \
            -destination 'platform=iOS Simulator,name=iPhone 17,OS=latest' \
            CODE_SIGNING_ALLOWED=NO
```

Pin `macos-26` and the exact `Xcode_26.5.app` path (verify against the [runner-images README](https://github.com/actions/runner-images)) — `macos-latest` and the default Xcode both rotate. Prefer GitHub Actions over Xcode Cloud: it runs *both* halves in one place; Xcode Cloud can't run a native `cargo test` step. (macOS runner minutes bill at ~10× — fine for PRs at solo volume.)

---

## 7. Deployment

The ordered path, with the two messenger-specific wrinkles (export compliance, GPL) flagged.

1. **Free first (no $99 yet).** A free Apple Account gives a **Personal Team**: run on **your own** devices to validate the bridge. Limits: signing/profiles **expire every 7 days**, **10 App IDs** max, **3 devices per platform**, **no push**, no TestFlight, no App Store. Good enough to prove Rust↔Swift runs on real hardware before paying — but Nova needs APNs, so you'll outgrow it fast.

2. **Apple Developer Program — $99/yr, enroll as Individual.** $99 is current (2026-06-25), same for Individual and Organization. Organization requires a **D-U-N-S number** + legal-entity verification (days–weeks); Individual enrolls fast and lists your legal name. Pick Individual now, migrate to Org later. *(D-U-N-S is optional only for government orgs.)*

3. **App record + bundle ID.** Register an explicit App ID (e.g. `network.nova`), enable **Push Notifications**. Create the App Store Connect app record (name, primary language, SKU) before any upload. One target serves iPhone **and** iPad — set destination to "iPhone, iPad" (`TARGETED_DEVICE_FAMILY = "1,2"`).

4. **Automatic signing.** Tick "Automatically manage signing" in Xcode → Signing & Capabilities. Xcode creates/rotates the certificate, App ID, and profiles. Pick **one** signing path across machines — never automatic locally + manual in CI.
   - *Mental model:* **certificate** = who you are; **provisioning profile** = binds App ID + cert + devices + entitlements; the **"Untrusted Developer"** prompt is a separate one-time on-device step (Settings → General → VPN & Device Management → Trust). On iOS 16+ you must also enable **Developer Mode** (Settings → Privacy & Security → Developer Mode → restart) *before* the trust step, or sideloaded builds silently fail to launch.

5. **Capabilities & entitlements this app needs** (Signing & Capabilities writes the `.entitlements` + Info.plist):
   - **Keychain Sharing** — at-rest key store; use `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (readable for background drains after first unlock, device-bound, excluded from iCloud/backup).
   - **Push Notifications (APNs)** — the only reliable way to wake a serverless app to drain the relay; adds `aps-environment`.
   - **Background Modes** → `remote-notification` (silent-push wake) + `processing` (catch-up `BGProcessingTask`). You **must** add `BGTaskSchedulerPermittedIdentifiers` (your task IDs) to Info.plist or registration crashes.
   - **App Groups** (`group.network.nova`) — so a Notification-Service/Share extension and the app share the container for the **file-URL blob handoff**.
   - **Secure Enclave** — **no** separate entitlement (works via the default app-identifier on a signed app), but it does **not exist in the Simulator** — test on device.
   - **Usage strings (Info.plist):** `NSCameraUsageDescription` (AVFoundation + Vision QR — required or the app crashes). Skip `NSLocalNetworkUsageDescription`/`NSBonjourServices` for v1 unless you do LAN discovery (it adds a scary prompt and has an iOS 17/18 permission-state bug). Add `NSContactsUsageDescription` only when you read Contacts.
   - **Data protection** for the SQLite store: `NSFileProtectionCompleteUntilFirstUserAuthentication` (set on the DB's parent directory). `Complete` would lock you out during background push-wake drains; SQLCipher provides at-rest encryption on top.

6. **Export compliance (MLS = strong crypto — read carefully).** Set **`ITSAppUsesNonExemptEncryption = true`** in Info.plist (you implement MLS, beyond OS HTTPS). MLS uses **standard, published** algorithms (AES, ECDH, RFC 9420), so you qualify for **mass-market self-classification under EAR §740.17(b)(1)** — **no CCATS** needed. **But "self-classify" is not "exempt":** you must file an **annual self-classification report to BIS + NSA by February 1** (email `crypt@bis.doc.gov`, `enc@nsa.gov`, subject "Self-Classification Report for Encryption Items") covering the prior year's builds. The widespread blog claim that mass-market apps owe *no* yearly filing is wrong for an app shipping its own crypto layer — only apps using *exempt* OS-provided TLS owe nothing. The old Encryption Registration Number (ERN) was eliminated in 2021. (Confirm current EAR text with an export attorney before first ship.)

7. **TestFlight.** **Internal testers** (≤100, must hold an App Store Connect role) need **no Beta App Review** and go live within minutes — your fastest path. **External testers** (≤10,000) need Beta App Review on the **first build of each version** (~24h). Every build **expires after 90 days**; ≤6 builds submitted for review per 24h. Path: Product → Archive → Distribute → App Store Connect → assign to your Internal group.

8. **App Store review + privacy labels.** Review is ~24–48h. For the **App Privacy "nutrition labels,"** an E2E blind-relay messenger can honestly answer **"Data Not Collected"** for message content (it never leaves the device readable) — but **do** declare APNs device tokens, any analytics/crash SDK, and Contacts if read. A **`PrivacyInfo.xcprivacy` privacy manifest** is **required since 2024-05-01**: it must cover required-reason APIs used by the Rust core **and** any SDK; Xcode merges all manifests (app + each XCFramework) into the Privacy Report. If your Rust core or a dep touches file-timestamp / `UserDefaults` / disk-space / boot-time APIs, declare the reason or the upload is rejected.

9. **GPL on the App Store — fix this before first submit.** Apple's App Store terms add usage/DRM restrictions that conflict with the GPL's "no further restrictions" clause (**GPLv2 §6 / GPLv3 §10** — note this is *not* GPLv3-specific; plain GPLv2 conflicts too). This is what got GNU Go (2010) and VLC (2011) pulled. As the **copyright holder**, add an explicit **"App Store additional permission"** to your LICENSE granting recipients the right to obtain/use the binary via Apple's terms — a few lines, clean and durable. Keep core deps permissive (`mls-rs` is Apache-2.0) so a GPL dependency you don't own can't re-introduce the conflict.

10. **CI/CD: manual first, then Fastlane (not Xcode Cloud).** Archive + Upload by hand for the first few builds so you understand every failure. Then adopt **Fastlane** (MIT, cross-platform — it follows you to Android/desktop; Xcode Cloud is Apple-only and adds a second signing path): `gym` (build/sign) → `pilot` (TestFlight) → `deliver` (App Store) → `match` (Git-stored signing when CI/a 2nd machine arrives). For a beginner, **Fastlane is the right durable pick.**

```ruby
# fastlane/Fastfile  (Phase 2)
platform :ios do
  lane :beta do
    sh("cd ../apple && ./build-ios.sh")               # build the Rust XCFramework
    build_app(scheme: "Nova", export_method: "app-store")
    upload_to_testflight(skip_waiting_for_build_processing: true)
  end
end
```

---

## 8. The beginner happy-path checklist (zero → TestFlight)

In realistic learning order. **[R]** = required, **[O]** = optional/defer.

1. **[R]** Install full Xcode 26.x; `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`; `sudo xcodebuild -license accept`; `xcodebuild -runFirstLaunch`; `xcodebuild -downloadPlatform iOS`.
2. **[R]** `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`.
3. **[R]** `cargo install cargo-swift` → `cargo swift package` once to prove the toolchain works end-to-end.
4. **[R]** New SwiftUI App project (or `xcodegen generate`); run it in the **Simulator** (⌘R) — no Apple ID, no cable. Validates the build before signing exists.
5. **[R]** Replace the scaffold with the durable layout (§3): `nova-core` + `nova-ffi` + the thin `NovaCore` wrapper; commit `build-ios.sh`.
6. **[R]** Write the **FFI smoke test** (async call + typed error + one inward callback) in Swift Testing; get it green on the simulator.
7. **[O]** Add a free Apple ID (Personal Team) and run on **your own iPhone** to validate Secure Enclave, camera/QR, Keychain — the things the simulator can't do. (7-day expiry; re-run from Xcode to refresh.)
8. **[R]** Write host `cargo test` coverage for the core + golden CBOR vectors; wire the GitHub Actions CI (§6).
9. **[R]** Enroll in the **Apple Developer Program ($99, Individual)** — forced once you need APNs/TestFlight.
10. **[R]** Register the App ID (with Push), create the App Store Connect record, add capabilities/entitlements + usage strings + `PrivacyInfo.xcprivacy` (§7.5/§7.8).
11. **[R]** Set `ITSAppUsesNonExemptEncryption = true`; note the Feb-1 BIS self-classification report obligation (§7.6).
12. **[R]** Add the GPL "App Store additional permission" to LICENSE (§7.9).
13. **[R]** Enable **Automatic signing**; Product → Archive → Distribute → **TestFlight Internal** (live in minutes, no review).
14. **[O]** Add **External** testers (first build → ~24h Beta App Review) when you want a wider beta.
15. **[O]** Adopt **Fastlane** once manual Archive feels repetitive; **[O]** add a 1–2 test XCUITest launch smoke; submit to the App Store with privacy labels.

---

## 9. Pitfalls & gotchas

- **CLT-only is a dead end.** `xcodebuild` won't run against a Command-Line-Tools developer directory; install full Xcode and `xcode-select -s` to it.
- **The simulator-vs-device arch trap.** `aarch64-apple-ios` (device) and `aarch64-apple-ios-sim` (simulator) are different targets with different Mach-O platform tags; `lipo` refuses to merge them. Device slice stays alone; lipo the two *simulator* slices together. Missing `-sim` = "no such module" on Run.
- **`module.modulemap` rename.** UniFFI emits `…FFI.modulemap`; Xcode finds only `module.modulemap`. Rename it (the script does) or the module won't import.
- **Blobs through `Data` will copy.** Rust→Swift returns full-copy through a RustBuffer; the zero-copy `&[u8]` path is foreign→Rust only, needs `[ByRef]`, and is still unreleased. Use the file-URL handoff.
- **Foreign-trait panics.** A callback method that doesn't return `Result<>`, or a missing `From<UnexpectedUniFFICallbackError>` impl, **panics across the FFI** (which aborts). And parameters must be by value.
- **Secure Enclave is not in the Simulator.** `SecureEnclave.isAvailable` is false; SE key generation/signing fails. The SE Signer callback can only be tested on a physical device — don't let a simulator fallback give you a false green.
- **Untrusted-developer + Developer Mode.** Two distinct gates on iOS 16+: enable **Developer Mode** (with restart) *then* trust the cert. Skipping the first makes the app silently fail to launch.
- **Free-provisioning 7-day expiry.** Personal-Team builds/profiles die after 7 days (re-run from Xcode); 10 App IDs, 3 devices/platform, no push/App Groups/iCloud. APNs forces the paid program.
- **No reliable always-on background networking.** A serverless relay client **cannot** hold a background socket. Drain model = **APNs silent push-wake** (~30s runtime, throttled to ~2–3/hr, de-prioritized in Low Power Mode) **+ `BGTaskScheduler` best-effort** (runs at OS discretion) **+ drain-on-foreground**. Architect for *eventual* delivery, never background real-time. (VoIP/PushKit wakes reliably but is restricted to genuine CallKit calls — not a chat backdoor.)
- **`processing` background mode needs the allowlist.** Add `BGTaskSchedulerPermittedIdentifiers` to Info.plist or `BGTaskScheduler` registration crashes.
- **OpenSSL is the binary-size villain.** Use SQLCipher's CommonCrypto backend (`bundled-sqlcipher` + `-DSQLCIPHER_CRYPTO_CC`), not vendored OpenSSL.
- **Export compliance is self-classification, not exemption.** Set `ITSAppUsesNonExemptEncryption = true` (don't set `false` to dodge the prompt — that's a false declaration for an MLS app) and file the Feb-1 BIS report.
- **GPL conflict applies to GPLv2 too.** Add the App Store additional permission before submitting; keep core deps non-GPL.
- **Binary-framework signing is *not* required** for a local-path SwiftPM `binaryTarget` (only a checksum, for *remote* targets). Don't burn time signing the `.xcframework`. *(Separately, the app bundle is re-signed at build time — that's automatic.)*
- **Bitcode is dead** (removed Xcode 15). Add no bitcode flags. Xcode 16+ even rejects uploads whose embedded third-party frameworks still contain bitcode.
- **Pin CI Xcode + runner.** `macos-latest` and the default Xcode rotate; pin `macos-26` and `Xcode_26.5.app`.

---

## 10. Curated learning resources

**UniFFI (the bridge)**
- User guide — Swift overview: https://mozilla.github.io/uniffi-rs/latest/swift/overview.html
- Foreign traits (`with_foreign`, `Send+Sync`, `Result<>` rule): https://mozilla.github.io/uniffi-rs/latest/foreign_traits.html
- Async/Future support (tokio, cancellation guidance): https://mozilla.github.io/uniffi-rs/latest/futures.html
- Proc-macros (preferred over UDL): https://mozilla.github.io/uniffi-rs/latest/proc_macro/index.html
- `uniffi-bindgen-swift` (library-mode flags): https://mozilla.github.io/uniffi-rs/next/swift/uniffi-bindgen-swift.html
- CHANGELOG (version/date truth): https://github.com/mozilla/uniffi-rs/blob/main/CHANGELOG.md

**Rust-core + iOS packaging (worked precedents)**
- `ianthetechie/uniffi-starter` — the clean `rust/` + `apple/` layout: https://github.com/ianthetechie/uniffi-starter
- Ferrostar iOS packaging (3 targets, lipo, `-create-xcframework`, local-vs-remote `Package.swift`): https://stadiamaps.com/news/ferrostar-building-a-cross-platform-navigation-sdk-in-rust-part-2/
- Mozilla application-services `build-xcframework.sh` (production reference): https://github.com/mozilla/application-services/blob/main/megazords/ios-rust/build-xcframework.sh
- `cargo-swift` (day-1 on-ramp): https://github.com/antoniusnaumann/cargo-swift
- "Building a Zero-Trust iOS Architecture: SwiftUI meets Rust and MLS — Part 2" (Jun 2026; tokio + UniFFI + MLS, directly analogous): https://medium.com/@alfred.weirich/building-a-zero-trust-ios-architecture-swiftui-meets-rust-and-mls-part-2-3fd6a955745f

**MLS core**
- `awslabs/mls-rs` (depend on this directly): https://github.com/awslabs/mls-rs
- `mls-rs-uniffi` `lib.rs` + `config/group_state.rs` (reference for the proc-macro + foreign-trait patterns, not a dependency): https://github.com/awslabs/mls-rs/blob/main/mls-rs-uniffi/src/lib.rs

**Swift / iOS fundamentals (for a newcomer)**
- Apple's SwiftUI tutorials (`@main` App / `Scene` / `WindowGroup` / Previews): https://developer.apple.com/tutorials/swiftui
- Hacking with Swift — 100 Days of SwiftUI (continuously updated): https://www.hackingwithswift.com/100/swiftui
- Swift Testing: https://developer.apple.com/xcode/swift-testing/
- Binary targets in modern Swift packages (Pol Piella): https://www.polpiella.dev/binary-targets-in-modern-swift-packages
- `NavigationSplitView` + size classes (one-target iPhone/iPad adaptivity): https://www.hackingwithswift.com/quick-start/swiftui/how-to-create-a-two-column-or-three-column-layout-with-navigationsplitview

**Apple platform truth (re-check before you rely on a number)**
- Xcode SDK & system requirements (current Xcode/macOS minimums): https://developer.apple.com/xcode/system-requirements/
- Downloading additional Xcode components (`-downloadPlatform`, simulator runtimes): https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components
- Choosing a membership ($99, Individual vs Org, D-U-N-S, free-tier limits): https://developer.apple.com/support/compare-memberships/
- TestFlight overview (100 internal / 10k external, review, 90-day expiry): https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/
- Complying with Encryption Export Regulations + `ITSAppUsesNonExemptEncryption`: https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations
- BIS encryption / §740.17 self-classification: https://www.bis.doc.gov/index.php/policy-guidance/encryption
- App Privacy details + privacy manifest: https://developer.apple.com/app-store/app-privacy-details/ · https://developer.apple.com/documentation/bundleresources/adding-a-privacy-manifest-to-your-app-or-third-party-sdk
- Pushing background updates (silent-push throttle, 30s window): https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app
- `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`: https://developer.apple.com/documentation/security/ksecattraccessibleafterfirstunlockthisdeviceonly
- FSF — GPL enforcement in Apple's App Store: https://www.fsf.org/blogs/licensing/more-about-the-app-store-gpl-enforcement

**Deterministic CBOR (golden vectors)**
- Blockchain Commons dCBOR (matched Rust + Swift impls): https://developer.blockchaincommons.com/dcbor/
- `ciborium` (Rust canonical codec): https://github.com/enarx/ciborium

**Build tooling (for context)**
- GitHub `actions/runner-images` `macos-26` README (pin the exact Xcode path): https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md
- Fastlane docs (`gym`/`pilot`/`deliver`/`match`): https://docs.fastlane.tools/actions/match/
