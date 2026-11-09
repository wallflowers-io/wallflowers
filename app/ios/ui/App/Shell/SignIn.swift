//  SignIn.swift
//  ONE DOOR, and the seed behind it.
//
//  THE SEED IS THE ACCOUNT. 32 bytes the person owns, minted by pacific-core on
//  a device. A passkey WRAPS it and is never the account itself; 24 words
//  restore it when no passkey will. The Arc authenticates the identity public
//  key by challenge–response and never sees the passkey at all.
//
//  WHAT THIS FILE USED TO BE. Three doors — Kenjin, Apple, AI Passport — against
//  an auth service that ran a WebAuthn ceremony, minted session cookies and
//  exchanged OAuth codes. Every route they called has been deleted:
//  /auth/passkey/signin/{options,verify}, /auth/apple/native,
//  /auth/passport/start. Apple and Passport authenticate a PERSON TO A SERVICE,
//  which is a different fact from a device holding a key, and under this model
//  the second is the only one that means anything. So they are gone, and with
//  them the ephemeral cookie jar that existed to carry a flow cookie between two
//  calls that no longer exist.
//
//  THE CEREMONY IS STILL NATIVE and could not be otherwise: only the browserless
//  authenticator API can talk to the Secure Enclave, and the PRF secret it
//  derives never leaves it. What changed is what the ceremony is FOR. It used to
//  prove a person to a server. Now it unwraps a seed that never left this device
//  in the first place — the server's only role is to hold 76 bytes it cannot
//  read, and to check a signature.
//
//  A CREDENTIAL MINTED WITHOUT PRF CANNOT GAIN ONE. It is bound at creation,
//  which is why `createCredentialRegistrationRequest` below asks for PRF SUPPORT
//  with no salt, and the assertion asks for the EVALUATION with one. Those are
//  two different extension shapes and both are required; a credential made
//  without the first authenticates forever and derives nothing.

import AuthenticationServices
import Combine
import CryptoKit          // the PRF output arrives as a SymmetricKey, not Data
import Foundation
import PacificCore

/// Where the account lives. One host, so a dev build points at one place.
///
/// IT IS NO LONGER COMPILED IN. `https://arc.wallflowers.io` is what ships and what
/// every default resolves to, but a dev or harness run can name another Arc —
/// `PACIFIC_AUTH_URL` (the `PACIFIC_ARC_URL` idiom, so a simulator sets it as
/// `SIMCTL_CHILD_PACIFIC_AUTH_URL`), or the persisted `pacific.authUrl` default
/// when it has to survive a home-screen relaunch. Pointing this at a local Arc
/// (`https://127.0.0.1:8793`) is how the account routes are exercised against
/// something real; note that a self-signed local cert is refused by ATS, so the
/// local Arc has to be trusted by the simulator before the HTTP half will pass.
enum AuthHost {
    /// The origin every account route hangs off.
    static let base: URL = {
        let env = ProcessInfo.processInfo.environment["PACIFIC_AUTH_URL"]
        let stored = UserDefaults.standard.string(forKey: "pacific.authUrl")
        for candidate in [env, stored] {
            guard let raw = candidate?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !raw.isEmpty, let u = URL(string: raw), u.host != nil else { continue }
            // Before any actor exists: the file directly, as Trace would.
            NSLog("[account] auth host overridden → %@", u.absoluteString)
            LogFile.shared.append("[account] auth host overridden → \(u.absoluteString)")
            return u
        }
        // wallflowers.io since 15 Sep 2026: a passkey is bound to its RP ID for life
        // (`rpID` below).
        //
        // THE ARC IS ITS OWN HOST since 18 Sep 2026, as it is for the web client
        // (pacific-account.js, `arcOriginFor`): the account routes are the Arc's,
        // at arc.wallflowers.io, and wallflowers.io is the site. Same service
        // behind both today — which is exactly why the two could drift unnoticed.
        return URL(string: "https://arc.wallflowers.io")!
    }()

    /// The Door's own host, as the web's passkeys are (D-33, `DOOR_RP_ID`), so one
    /// passkey opens the account in both. A credential is bound to this for life.
    /// The Door's apple-app-site-association names this app, and the entitlement
    /// (`webcredentials:app.wallflowers.io`) names the host.
    ///
    /// It does not follow `base`, the Arc's host. A local Arc's own host is used on
    /// loopback (a localhost credential is stranded on purpose, as on the web), or
    /// `PACIFIC_AUTH_RPID` when a dev host differs.
    static let rpID: String = {
        if let override = ProcessInfo.processInfo.environment["PACIFIC_AUTH_RPID"],
           !override.isEmpty { return override }
        if let h = base.host, h == "localhost" || h == "127.0.0.1" { return h }
        return "app.wallflowers.io"
    }()

    /// What the wrap is bound to in its AAD: the Arc that serves it. Same string
    /// as the host in the passkey's user handle, and a wrap lifted onto another
    /// Arc will not open there.
    ///
    /// It is also the AUDIENCE this device will sign for (`signedHeaders`), with the
    /// port when there is one — the Arc names itself by `host[:port]`, and so does
    /// the web client (`location.host`).
    static var arcHost: String {
        guard let h = base.host else { return rpID }
        return base.port.map { "\(h):\($0)" } ?? h
    }
}

/// What a completed sign-in yields. There is no session and no token — the
/// account IS the identity the core now holds, so what comes back is the facts
/// about it rather than a credential.
struct SignInResult {
    /// `ed25519:…`, as `identityKey()` reports it.
    let identityKey: String
    /// How this device got in. Worth recording because it decides what to offer
    /// next: a words restore leaves this device with no passkey of its own.
    let via: Via
    /// What the Arc holds, from `/auth/users/<pk>/account`.
    let account: [String: Any]
    /// Every object the spine names and what became of it. Passkey only.
    var objects: [Resumption] = []

    enum Via { case created, passkey, words }
}

enum SignInError: LocalizedError {
    case cancelled
    /// THE CEREMONY SAID NOTHING AT ALL. Not a refusal — a silence.
    case timedOut
    case noPRF
    case noAccount
    case wrongKey
    /// The words fail their checksum: said at the field, where they can be fixed.
    case badWords
    case service(String)
    case malformed(String)

    var errorDescription: String? {
        switch self {
        case .cancelled: return "That was cancelled."
        case .timedOut:
            return "The passkey sheet never answered. Your twenty-four words still "
                 + "work, and Settings can add a passkey later."
        case .noPRF:
            // Said in full, because the fix is not obvious and the failure looks
            // like a bug in this app rather than a property of the credential.
            return "That passkey cannot hold an account — it was created before "
                 + "this app could ask it to. Use your twenty-four words, and "
                 + "this device will make a passkey that can."
        case .noAccount:
            return "This Arc has never held that account."
        case .wrongKey:
            // The one failure with two ordinary causes and no way to tell them
            // apart from here. Saying one would send people to fix the other.
            return "That passkey works, but it does not open your key. Either "
                 + "another device has enrolled its own passkey since — which "
                 + "replaces the one before it — or this platform does not carry "
                 + "the passkey secret between devices. Your words work either way."
        case .badWords: return "Those words do not check. One is mistyped or out of place."
        case .service(let m): return m
        case .malformed(let m): return "The Arc sent something unreadable: \(m)"
        }
    }
}

// MARK: - The service

@MainActor
final class SignInService: NSObject, ObservableObject {
    @Published var busy: String? = nil {        // which act is mid-ceremony
        didSet { if busy == nil { progress = nil } }
    }
    @Published var error: String? = nil
    /// The latest `pacific::resume` line while busy: what a sign-in has reached.
    @Published private(set) var progress: String? = nil

    /// THE PLAIN SHARED SESSION, and the absence of a cookie jar is the point.
    /// This was `URLSession(configuration: .ephemeral)`, carrying a per-session
    /// jar so an httpOnly flow cookie could survive between `options` and
    /// `verify`. There is no flow, no cookie and no session to keep: every
    /// request that needs authority carries a signature over a nonce the Arc
    /// issued moments earlier, and a request that does not need authority (the
    /// wrap) is unauthenticated on purpose.
    private let net = URLSession.shared

    /// This surface's lines, into the one log (`Trace`, and its file).
    /// WHAT IS SAFE TO WRITE DOWN: byte LENGTHS, status codes, and the identity
    /// key, which is public and is the account's address. What never appears:
    /// the seed, the PRF secret, the wrap's plaintext, the words. A log that
    /// leaks the key it was added to debug is worse than no log.
    private func log(_ m: String) { Trace.shared.log("account", m) }

    private let core: Core
    private var following: AnyCancellable?
    init(core: Core) {
        self.core = core
        super.init()
        following = Trace.shared.$events.sink { [weak self] events in
            guard let self, self.busy != nil,
                  let e = events.first, e.stage == "core:pacific::resume" else { return }
            self.progress = e.detail
        }
    }

    // MARK: Making an account

    /// Mint, wrap, register. The seed is created HERE, by the core, and the Arc
    /// never sees it — only 76 bytes it cannot read.
    ///
    /// ORDER MATTERS: the identity must exist before a wrap can be exported from
    /// it, and the passkey must exist before it can be asked for a PRF. So:
    /// mint → create credential (asking for PRF SUPPORT) → assert (asking for
    /// the EVALUATION) → wrap → PUT.
    func createAccount(displayName: String) async throws -> SignInResult {
        busy = "create"; defer { busy = nil }

        log("minting an identity")
        let identityKey = try core.initIdentity(displayName: displayName)
        log("identity \(identityKey.prefix(24))…")

        let pk = Self.rawKey(identityKey)
        _ = try await registerPasskey(pk: pk, label: displayName)

        let body = try await account(identityKey)
        return SignInResult(identityKey: identityKey, via: .created, account: body)
    }

    /// Make a credential for this account and store the wrap it can open. Also
    /// the second half of a words restore, and the way a second device earns a
    /// local unlock of its own.
    ///
    /// ONE WRAP PER ACCOUNT, so this is revocation as much as enrolment: storing
    /// a new wrap makes the credential it replaces useless. Callers must say so.
    @discardableResult
    func registerPasskey(pk: Data, label: String) async throws -> Data {
        // THE HANDLE CARRIES THE ACCOUNT: public key ‖ arc host. Those are
        // exactly the two things a fresh device cannot derive for itself — the
        // address IS the public key, and the public key is downstream of the
        // seed it is fetching.
        var handle = pk
        handle.append(Data(AuthHost.arcHost.utf8))

        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: AuthHost.rpID)
        // Not verified by any server: this Arc runs no WebAuthn ceremony, so the
        // challenge is ceremony rather than proof.
        let reg = provider.createCredentialRegistrationRequest(
            challenge: Self.randomBytes(32),
            // The local part is the public key's first 8 hex, never anything
            // personal, and the host is what the authenticator shows in its
            // picker — the one human-visible cue about WHICH Arc a credential
            // belongs to. A label never displaces it.
            name: "\(pk.prefix(4).map { String(format: "%02x", $0) }.joined())@\(AuthHost.arcHost)",
            userID: handle)
        // PRF SUPPORT, with no salt. Empty means "this credential must be able
        // to do this", and it BINDS THE SECRET — it cannot be added later.
        // Evaluation happens at assertion, below, and carries the salt.
        reg.prf = ASAuthorizationPublicKeyCredentialPRFRegistrationInput.checkForSupport

        log("presenting the passkey sheet (rp \(AuthHost.rpID))")
        let made = try await performRegistration(reg)
        log("passkey made · handle carries the key and \(AuthHost.arcHost)")

        // PRF COMES FROM AN ASSERTION, NEVER FROM CREATION: several shipping
        // authenticators return nothing at creation and the full 32 bytes at
        // assertion. Name the credential just made so no picker appears.
        let prf = try await evaluatePRF(allowing: made.credentialID)
        let wrap = try core.exportWrap(prfSecret: prf, host: AuthHost.arcHost)
        log("seed wrapped · \(wrap.count) bytes of ciphertext")

        try await put("/auth/users/\(Self.hex(pk))", body: wrap)
        log("registered")
        return wrap
    }

    // MARK: Signing in

    /// The passkey door: discoverable, so nothing is typed and no account is
    /// named. The handle says who, the wrap says what, the signature proves it.
    func signInWithPasskey() async throws -> SignInResult {
        busy = "passkey"; defer { busy = nil }

        log("asking for a passkey — no account named, the credential knows")
        let (prf, userHandle) = try await assertDiscoverable()

        // Anything shorter than 33 bytes is not one of ours: the key alone is 32
        // and the host is at least one character.
        guard userHandle.count > 32 else {
            throw SignInError.malformed("that passkey carries no host in its handle")
        }
        let pk = userHandle.prefix(32)
        let host = String(decoding: userHandle.dropFirst(32), as: UTF8.self)
        log("the handle says \(Self.hex(pk).prefix(16))… at \(host)")

        // UNAUTHENTICATED, deliberately: this read is how a device gets the key
        // it would otherwise have to authenticate with.
        let wrap = try await getData("/auth/users/\(Self.hex(pk))")
        log("wrap fetched · \(wrap.count) bytes")
        let head = try await head(pk)
        log(head.map { "head at \($0.position)" } ?? "no head yet")

        // A DEVICE THAT HOLDS NOTHING, signed in by the core in one step: the wrap
        // must open to the handle's key, then every object the spine names is
        // joined over the relay. Once; this device's state persists after.
        let c = core, routes = CoreService.routeSpec, expect = "ed25519:\(Self.hex(pk))"
        let signed: SignedIn
        do {
            signed = try await Task.detached {
                try c.signInFromWrap(displayName: "", prfSecret: prf, wrap: wrap, host: host,
                                     expectKey: expect, routes: routes,
                                     head: head?.blob, headPosition: head?.position ?? 0)
            }.value
        } catch {
            log("sign-in failed: \(error)")
            // Nothing written: the wrap did not open to this key. Two ordinary
            // causes, indistinguishable from here — see SignInError.wrongKey.
            if !core.hasIdentity() { throw SignInError.wrongKey }
            throw SignInError.service(error.localizedDescription)
        }
        log("signed in · \(signed.objects.count) objects named by the spine")

        let body = try await account(signed.identityKey)
        return SignInResult(identityKey: signed.identityKey, via: .passkey, account: body,
                            objects: signed.objects)
    }

    /// The words door. No passkey is involved at all, which is what makes it the
    /// one that always opens: after a lost device, on a platform whose PRF does
    /// not travel, or on a device that never had a credential.
    ///
    /// THE SAME SIGN-IN AS THE PASSKEY'S, from the words instead of a wrap: the key
    /// the words name (nothing written), the head, then the core's
    /// `signInFromWords`, which walks the spine before any self record is made.
    func restoreWithWords(_ phrase: String, displayName: String = "") async throws -> SignInResult {
        busy = "words"; defer { busy = nil }

        let words = phrase.trimmingCharacters(in: .whitespacesAndNewlines)
        log("reading a recovery key — the checksum holds or it does not")
        let identityKey: String
        do { identityKey = try wordsKey(recoveryKey: words) } catch {
            log("the words do not check")
            throw SignInError.badWords
        }
        let pk = Self.rawKey(identityKey)
        log("the words name \(Self.hex(pk).prefix(16))…")
        let head = try await head(pk)
        log(head.map { "head at \($0.position)" } ?? "no head yet")

        let c = core, routes = CoreService.routeSpec
        let signed: SignedIn
        do {
            signed = try await Task.detached {
                try c.signInFromWords(displayName: displayName, recoveryKey: words, routes: routes,
                                      head: head?.blob, headPosition: head?.position ?? 0)
            }.value
        } catch {
            log("sign-in failed: \(error)")
            throw SignInError.service(error.localizedDescription)
        }
        log("signed in · \(signed.objects.count) objects named by the spine")

        let body = try await account(signed.identityKey)
        return SignInResult(identityKey: signed.identityKey, via: .words, account: body,
                            objects: signed.objects)
    }

    // MARK: Proving the account to the Arc

    /// The Arc issues a single-use nonce; the core signs
    /// `pacific-auth:v1 \n audience \n ed25519:<pk> \n nonce` and the signature
    /// rides in headers. No cookie, no session, nothing at rest to steal.
    ///
    /// THE CORE SIGNS, NOT THIS FILE. `signChallenge` is domain-separated and is
    /// not a general signing oracle — the private half never crosses the FFI,
    /// which is the constraint that made `arcMemberCredential` the only signing
    /// primitive before it.
    ///
    /// REFUSE TO SIGN FOR SOMEBODY ELSE. The audience is inside the signed bytes so
    /// a signature captured by one Arc cannot be replayed at another — but the Arc
    /// DECLARES its audience, so signing whatever it says rests the whole property
    /// on the Arc being honest about its own name. This device decides what it
    /// expects (`AuthHost.arcHost`) and checks, as the web client does
    /// (pacific-account.js, `expectedAudience`).
    private func signedHeaders() async throws -> [String: String] {
        let c = try await getJSON("/auth/challenge")
        guard let nonce = c["nonce"] as? String,
              let audience = c["audience"] as? String else {
            throw SignInError.malformed("challenge without a nonce")
        }
        guard audience == AuthHost.arcHost else {
            throw SignInError.service(
                "This Arc calls itself \(audience) but it is \(AuthHost.arcHost) — "
                + "refusing to sign, because a signature for another Arc is one this "
                + "Arc could replay there.")
        }
        let sig = try core.signChallenge(audience: audience, nonce: nonce)
        return ["X-Pacific-Challenge": nonce, "X-Pacific-Signature": sig]
    }

    /// Everything the Arc holds for this key. Replaces `/auth/session`: there is
    /// no session to describe, so the question is not "who am I" — the caller
    /// proved that by signing — but "what do you have of mine".
    private func account(_ identityKey: String) async throws -> [String: Any] {
        let pk = Self.hex(Self.rawKey(identityKey))
        return try await getJSON("/auth/users/\(pk)/account", headers: try await signedHeaders())
    }

    // MARK: The ceremonies

    private func evaluatePRF(allowing credentialID: Data) async throws -> Data {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: AuthHost.rpID)
        let req = provider.createCredentialAssertionRequest(challenge: Self.randomBytes(32))
        req.allowedCredentials = [
            ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: credentialID)
        ]
        // The salt is the CORE'S, read rather than retyped: a byte of difference
        // would wrap the seed under a key nothing else can derive, and nothing
        // would say so until a second device tried to open it.
        req.prf = .inputValues(.init(saltInput1: wrapDomain()))
        let a = try await performCeremony(req)
        return try Self.prfBytes(a)
    }

    private func assertDiscoverable() async throws -> (prf: Data, userHandle: Data) {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: AuthHost.rpID)
        let req = provider.createCredentialAssertionRequest(challenge: Self.randomBytes(32))
        // No allowedCredentials: a discoverable credential means the
        // authenticator already knows which accounts exist, and naming them
        // would leak which ones we hold.
        req.prf = .inputValues(.init(saltInput1: wrapDomain()))
        let a = try await performCeremony(req)
        return (try Self.prfBytes(a), a.userID)
    }

    private static func prfBytes(
        _ a: ASAuthorizationPlatformPublicKeyCredentialAssertion
    ) throws -> Data {
        // The PRF output arrives as a CryptoKit SymmetricKey — the API's way of
        // saying these bytes are key material and not a payload — so it is
        // copied out only to hand straight to the core.
        guard let out = a.prf else { throw SignInError.noPRF }
        let d = out.first.withUnsafeBytes { Data($0) }
        guard d.count == 32 else { throw SignInError.malformed("PRF is \(d.count) bytes") }
        return d
    }

    /// HOW LONG A CEREMONY MAY SAY NOTHING BEFORE IT HAS FAILED.
    ///
    /// `ASAuthorizationController` is supposed to call back exactly once —
    /// completed, or an error, and a user's cancel is an error. On a device it
    /// does. It does NOT always: a simulator with no associated-domain to
    /// validate, a Personal Team build whose `webcredentials:` entitlement the
    /// platform will not honour, or a rp id that resolves to nothing presents no
    /// sheet AND reports no failure. The continuation below then never resumes,
    /// the `await` never returns, and — before this — the whole first-boot
    /// sequence stopped dead on the NAME screen with an identity already minted
    /// and its owner's twenty-four words never shown.
    ///
    /// A ceremony that has not spoken in this long is a failure whether or not
    /// the platform ever says so. Saying it ourselves is what lets the flow go on.
    /// Long enough for a person choosing among several passkeys: 15 s was not.
    private static let ceremonyTimeout: TimeInterval = 120

    private func performCeremony(
        _ request: ASAuthorizationPlatformPublicKeyCredentialAssertionRequest
    ) async throws -> ASAuthorizationPlatformPublicKeyCredentialAssertion {
        try await withCheckedThrowingContinuation { k in
            let once = Once(k)
            let c = ASAuthorizationController(authorizationRequests: [request])
            let box = AssertionDelegate(once)
            c.delegate = box
            c.presentationContextProvider = box
            self.ceremony = box            // the controller holds neither strongly
            self.arm(once, what: "assertion")
            c.performRequests()
        }
    }
    private var ceremony: AssertionDelegate?

    private func performRegistration(
        _ request: ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest
    ) async throws -> ASAuthorizationPlatformPublicKeyCredentialRegistration {
        try await withCheckedThrowingContinuation { k in
            let once = Once(k)
            let c = ASAuthorizationController(authorizationRequests: [request])
            let box = RegistrationDelegate(once)
            c.delegate = box
            c.presentationContextProvider = box
            self.registration = box
            self.arm(once, what: "registration")
            c.performRequests()
        }
    }
    private var registration: RegistrationDelegate?

    /// The deadline. `Once` makes this safe to fire unconditionally: whichever of
    /// the delegate and the clock gets there first wins, and the loser is a no-op.
    private func arm<T>(_ once: Once<T>, what: String) {
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.ceremonyTimeout) { [weak self] in
            guard once.pending else { return }
            self?.log("\(what): the passkey sheet never answered in "
                      + "\(Int(Self.ceremonyTimeout))s — giving up on it, not on the identity")
            once.resume(throwing: SignInError.timedOut)
        }
    }

    // MARK: A Site's address
    //
    // The Arc's site routes, as the Door calls them (app/door/src/auth.rs claim_site,
    // bind_host): the address claimed for this account, then bound to the Host it is
    // served from. The publication itself is a delta (`base.publish`), the core's.

    /// Whether the Arc holds this account (its wrap). An address is claimed by an account,
    /// so none means no Publish. Only the Arc's own "no account" says no: a link that is
    /// down is BLUE's to report.
    func arcHoldsAccount() async -> Bool {
        do { _ = try await account(try core.identityKey()); return true }
        catch SignInError.noAccount { return false }
        catch { return true }
    }

    #if DEBUG
    /// THE TEST PRF (Ralph, 9 Nov; DEBUG builds only): the wrap a passkey would store,
    /// made under a fixed PRF a UI suite passes (`-pacificTestPRF <64 hex>`), so a suite
    /// can keep the account at a local Arc without the system sheet it cannot reach.
    /// face-smoke.sh's SMOKE_PRF, on the phone. A Release build has no such path.
    func registerWithTestPRF(pk: Data, prf: Data) async throws {
        let wrap = try core.exportWrap(prfSecret: prf, host: AuthHost.arcHost)
        try await put("/auth/users/\(Self.hex(pk))", body: wrap)
        log("registered under the test PRF")
    }
    #endif

    /// `/auth/site/check`: whether an address is free, and why not.
    func checkAddress(_ slug: String) async throws -> AddressCheck {
        var q = URLComponents(url: AuthHost.base.appendingPathComponent("/auth/site/check"), resolvingAgainstBaseURL: false)!
        q.queryItems = [URLQueryItem(name: "slug", value: slug)]
        let (data, _) = try await net.data(from: q.url!)
        let j = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        if j?["available"] as? Bool == true { return .free }
        switch j?["reason"] as? String {
        case "taken": return .refused("That address is taken")
        case "held": return .refused("Someone is signing up with that address")
        case let why?: return .refused(why)
        case nil: return .refused("The Arc did not say")
        }
    }

    /// Claim `slug` for this account and bind it to `host`.
    func claimAddress(_ slug: String, host: String) async throws {
        let pk = Self.hex(Self.rawKey(try core.identityKey()))
        var h = try await signedHeaders()
        h["Content-Type"] = "application/json"
        _ = try await send("/auth/users/\(pk)/sites", method: "POST",
                           body: try JSONSerialization.data(withJSONObject: ["slug": slug]), headers: h)
        var b = try await signedHeaders()
        b["Content-Type"] = "application/json"
        _ = try await send("/auth/users/\(pk)/sites/\(slug)/host", method: "PUT",
                           body: try JSONSerialization.data(withJSONObject: ["host": host]), headers: b)
    }

    /// The Arc's pairing bundle (`/v1/bundle`), single-use, for adding it to a Host. The
    /// Arc's gateway, which is the account routes' host in production; `PACIFIC_ARC_HTTP`
    /// names it where the two are apart (the e2e stack).
    func arcBundle() async throws -> String {
        let base = ProcessInfo.processInfo.environment["PACIFIC_ARC_HTTP"].flatMap(URL.init(string:)) ?? AuthHost.base
        let (data, response) = try await net.data(from: base.appendingPathComponent("/v1/bundle"))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw SignInError.service("the Arc gave no bundle (HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0))")
        }
        let t = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        if let j = try? JSONSerialization.jsonObject(with: Data(t.utf8)) as? [String: Any], let b = j["bundle"] as? String {
            return b.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return t
    }

    // MARK: HTTP

    private func getJSON(_ path: String,
                         headers: [String: String] = [:]) async throws -> [String: Any] {
        let data = try await send(path, method: "GET", body: nil, headers: headers)
        guard let j = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw SignInError.malformed("not JSON")
        }
        return j
    }

    private func getData(_ path: String) async throws -> Data {
        try await send(path, method: "GET", body: nil, headers: [:])
    }

    /// The account's head and the position it declares. UNAUTHENTICATED, as the
    /// wrap is (pacific-account.js `fetchHead`). Nil: no head yet, a state.
    private func head(_ pk: Data) async throws -> (blob: Data, position: UInt64)? {
        let r = URLRequest(url: AuthHost.base.appendingPathComponent("/auth/users/\(Self.hex(pk))/head"))
        let (data, response) = try await net.data(for: r)
        let http = response as? HTTPURLResponse
        if http?.statusCode == 404 { return nil }
        guard http?.statusCode == 200,
              let at = http?.value(forHTTPHeaderField: "X-Chain-Position").flatMap(UInt64.init) else {
            throw SignInError.service("could not fetch the head (HTTP \(http?.statusCode ?? 0))")
        }
        return (data, at)
    }

    private func put(_ path: String, body: Data) async throws {
        var h = try await signedHeaders()
        h["Content-Type"] = "application/octet-stream"
        _ = try await send(path, method: "PUT", body: body, headers: h)
    }

    private func send(_ path: String, method: String,
                      body: Data?, headers: [String: String]) async throws -> Data {
        var r = URLRequest(url: AuthHost.base.appendingPathComponent(path))
        r.httpMethod = method
        r.httpBody = body
        for (k, v) in headers { r.setValue(v, forHTTPHeaderField: k) }

        let (data, response) = try await net.data(for: r)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(code) else {
            if code == 404 { throw SignInError.noAccount }
            // The Arc's refusals are JSON with an `error` and often a `message`.
            // Say what it said rather than the status line: "no_wrap" is a fact
            // a person can act on and 401 is not.
            let j = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            let what = (j?["message"] as? String) ?? (j?["error"] as? String) ?? "HTTP \(code)"
            log("\(method) \(path) → \(code) \(what)")
            throw SignInError.service(what)
        }
        return data
    }

    // MARK: Small things

    /// `ed25519:<64 hex>` → the 32 raw bytes. The prefix is the core's and is
    /// part of what gets signed, so it is stripped here and nowhere else.
    private static func rawKey(_ identityKey: String) -> Data {
        let hex = identityKey.contains(":")
            ? String(identityKey.split(separator: ":").last!)
            : identityKey
        var out = Data(capacity: hex.count / 2)
        var i = hex.startIndex
        while i < hex.endIndex, let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex) {
            if let b = UInt8(hex[i..<j], radix: 16) { out.append(b) }
            i = j
        }
        return out
    }

    private static func hex<C: Collection>(_ d: C) -> String where C.Element == UInt8 {
        d.map { String(format: "%02x", $0) }.joined()
    }

    private static func randomBytes(_ n: Int) -> Data {
        var b = [UInt8](repeating: 0, count: n)
        _ = SecRandomCopyBytes(kSecRandomDefault, n, &b)
        return Data(b)
    }
}

// MARK: - Once

/// A continuation that has two possible finishers and is resumed by exactly one
/// of them. Resuming a `CheckedContinuation` twice is a crash; never resuming it
/// is a hang. A ceremony now races the platform's delegate against a deadline, so
/// the arbitration has to live somewhere — it lives here, and it is atomic.
private final class Once<T> {
    private var k: CheckedContinuation<T, Error>?
    private let lock = NSLock()

    init(_ k: CheckedContinuation<T, Error>) { self.k = k }

    /// Whether nobody has finished this yet. Advisory — the decision is `take()`.
    var pending: Bool { lock.lock(); defer { lock.unlock() }; return k != nil }

    func resume(returning v: T) { take()?.resume(returning: v) }
    func resume(throwing e: Error) { take()?.resume(throwing: e) }

    private func take() -> CheckedContinuation<T, Error>? {
        lock.lock(); defer { lock.unlock() }
        let c = k; k = nil; return c
    }
}

// MARK: - Delegates

/// Assertion and registration yield different credential types, and conflating
/// them would mean one cast that silently fails for the other.
private final class AssertionDelegate: NSObject, ASAuthorizationControllerDelegate,
                                       ASAuthorizationControllerPresentationContextProviding {
    private let k: Once<ASAuthorizationPlatformPublicKeyCredentialAssertion>
    init(_ k: Once<ASAuthorizationPlatformPublicKeyCredentialAssertion>) {
        self.k = k
    }
    func authorizationController(controller: ASAuthorizationController,
                                 didCompleteWithAuthorization auth: ASAuthorization) {
        if let a = auth.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion {
            k.resume(returning: a)
        } else {
            k.resume(throwing: SignInError.malformed("not a passkey assertion"))
        }
    }
    func authorizationController(controller: ASAuthorizationController,
                                 didCompleteWithError error: Error) {
        let e = error as NSError
        k.resume(throwing: e.code == ASAuthorizationError.canceled.rawValue
                 ? SignInError.cancelled : SignInError.service(e.localizedDescription))
    }
    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        Anchor.current()
    }
}

private final class RegistrationDelegate: NSObject, ASAuthorizationControllerDelegate,
                                          ASAuthorizationControllerPresentationContextProviding {
    private let k: Once<ASAuthorizationPlatformPublicKeyCredentialRegistration>
    init(_ k: Once<ASAuthorizationPlatformPublicKeyCredentialRegistration>) {
        self.k = k
    }
    func authorizationController(controller: ASAuthorizationController,
                                 didCompleteWithAuthorization auth: ASAuthorization) {
        if let r = auth.credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration {
            k.resume(returning: r)
        } else {
            k.resume(throwing: SignInError.malformed("not a passkey registration"))
        }
    }
    func authorizationController(controller: ASAuthorizationController,
                                 didCompleteWithError error: Error) {
        let e = error as NSError
        k.resume(throwing: e.code == ASAuthorizationError.canceled.rawValue
                 ? SignInError.cancelled : SignInError.service(e.localizedDescription))
    }
    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        Anchor.current()
    }
}

private enum Anchor {
    static func current() -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first ?? ASPresentationAnchor()
    }
}

// MARK: - base64url

extension Data {
    /// WebAuthn speaks base64url throughout; Foundation only speaks base64.
    init?(base64URLEncoded s: String) {
        var t = s.replacingOccurrences(of: "-", with: "+")
                 .replacingOccurrences(of: "_", with: "/")
        while t.count % 4 != 0 { t += "=" }
        guard let d = Data(base64Encoded: t) else { return nil }
        self = d
    }
    func base64URLEncodedString() -> String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
