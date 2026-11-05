//  DeviceKey.swift
//  The device master key that seals pacific-core's at-rest secrets (workstream S5).
//  Generated once, stored in the Keychain (device-only, never synced to iCloud), and
//  injected into the Rust core at startup — BEFORE any identity/DB access. Losing the
//  key means the sealed store cannot be opened; that is the design (no key, no
//  plaintext). Injection order matters: DeviceKey.activate() must run before the core
//  opens `pacific.db` / `id_ed25519` (see CoreService.init, just before Core(stateDir:)).
//
//  FAIL CLOSED. Every path here distinguishes "the keychain says there is no key" from
//  "the keychain did not answer". They are opposite situations and conflating them was
//  a data-loss bug: the old `load()` returned nil on ANY OSStatus, so a transient read
//  failure minted a brand-new master key, and the previous one — the only thing that
//  could open the existing sealed store — was then overwritten. The user's identity,
//  MLS state and message history became permanently unopenable, with no error (the
//  add/delete statuses were discarded and `assertionFailure` compiles out in release)
//  and no escrow to recover from. The certain trigger was not an exotic race: the key
//  is ThisDeviceOnly and so is excluded from backup and device transfer, while the
//  state directory is backed up — so restoring to a new phone, or re-signing under a
//  different Team ID, hit it every time.
//
//  Now: mint ONLY on errSecItemNotFound; never delete before a successful write; and
//  on any other failure inject nothing and report it. Injecting nothing is safe and
//  recoverable — the sealed store simply stays shut until the keychain answers, so a
//  key that is merely unavailable (a background launch before first unlock) self-heals
//  on the next launch instead of destroying the store.

import Foundation
import Security
import PacificCore

enum DeviceKeyError: LocalizedError {
    /// The keychain failed to answer a read. NOT the same as "no key stored".
    case unreadable(OSStatus)
    /// The keychain refused to persist a newly minted key.
    case unwritable(OSStatus)
    /// A stored item exists but is not a 32-byte key.
    case malformed(Int)
    case randomFailed(OSStatus)
    case injectionFailed(Error)

    var errorDescription: String? {
        switch self {
        case .unreadable(let s):
            return "The secure enclave did not answer (\(s)). Your data is intact and "
                 + "sealed; unlock the device and reopen WallFlowers."
        case .unwritable(let s):
            return "Could not store the device key (\(s))."
        case .malformed(let n):
            return "The stored device key is \(n) bytes, not 32."
        case .randomFailed(let s):
            return "Secure random generation failed (\(s))."
        case .injectionFailed(let e):
            return "At-rest sealing could not be armed: \(e)"
        }
    }
}

enum DeviceKey {
    private static let service = "network.pacific.atrest"
    private static let account = "device-master-key"

    /// Provision the master key (generate on first run, else load) and inject it into
    /// the core's at-rest layer. Idempotent within a launch; call once at startup.
    /// THROWS rather than proceeding keyless-but-silent — see the file header.
    static func activate() throws {
        let key = try provision()
        do {
            // pacific-ffi: arms XChaCha20Poly1305 sealing of id_ed25519 and of the
            // secrets pacific-core seals in pacific.db (identity key, MLS signing key,
            // MLS group/epoch state, key packages, and `epoch_tag.secret`). NB it
            // does NOT currently cover `delta_log.envelope` or `me.card_json` — the
            // message archive itself is still plaintext at rest. A 32-byte key is never
            // rejected, so a throw here means the FFI binding is stale or misbuilt.
            try setDeviceKey(key: key)
        } catch {
            throw DeviceKeyError.injectionFailed(error)
        }
    }

    /// The existing 32-byte master key, or — ONLY when the keychain positively
    /// reports no such item — a freshly generated and persisted one.
    static func provision() throws -> Data {
        if let existing = try load() { return existing }
        var key = Data(count: 32)
        let status = key.withUnsafeMutableBytes { buf in
            SecRandomCopyBytes(kSecRandomDefault, 32, buf.baseAddress!)
        }
        guard status == errSecSuccess else { throw DeviceKeyError.randomFailed(status) }
        try store(key)
        return key
    }

    private static func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    /// `nil` means the keychain positively reported errSecItemNotFound — the only
    /// condition under which minting a new key is safe. Every other status throws,
    /// because a key we cannot read is not a key that is absent.
    private static func load() throws -> Data? {
        var q = baseQuery()
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: AnyObject?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw DeviceKeyError.unreadable(status) }
        guard let data = out as? Data else { throw DeviceKeyError.unreadable(errSecDecode) }
        // A short/corrupt item is reported, never silently replaced: overwriting it
        // would discard the only key that could open the existing sealed store.
        guard data.count == 32 else { throw DeviceKeyError.malformed(data.count) }
        return data
    }

    /// Add-then-update, never delete-then-add: a delete that succeeds followed by an
    /// add that fails would leave the device with no key and a sealed store it can
    /// never open again.
    private static func store(_ data: Data) throws {
        var add = baseQuery()
        add[kSecValueData as String] = data
        // Device-only, available after first unlock — the same class the credential
        // vault uses. (Biometric/SecAccessControl gating is a deliberate later dial.)
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(add as CFDictionary, nil)
        if status == errSecSuccess { return }
        guard status == errSecDuplicateItem else { throw DeviceKeyError.unwritable(status) }
        let updated = SecItemUpdate(
            baseQuery() as CFDictionary,
            [kSecValueData as String: data] as CFDictionary
        )
        guard updated == errSecSuccess else { throw DeviceKeyError.unwritable(updated) }
    }
}
