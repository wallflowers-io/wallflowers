//  Credential.swift
//  A content-free credential handle + the device-local secret store it points into.
//  Mirrors the Rust `group.storeCredential` model: a group's log holds only a handle
//  { id, kind, label } (projected as its `vault_index`), while the SECRET lives here
//  in the Keychain, keyed by the handle id, and never enters the log or an export.
//
//  Authoring the actual group.storeCredential DELTA is a follow-on that lands with the
//  real System GroupObject (WS-A); until then this is the device-local half — the
//  secure store the handle points into — used directly by SystemService.

import Foundation

/// The content-free descriptor of a stored credential. The secret is NOT here — only
/// its id (the Keychain key), a kind, and a human label.
struct CredentialHandle: Codable, Hashable, Identifiable {
    enum Kind: String, Codable { case bearerToken, oauthToken, apiKey, password }
    let id: String        // UUID — the Keychain account under which the secret is stored
    let kind: Kind
    let label: String     // human label, e.g. "Sysiphus · nye"

    static func mint(kind: Kind, label: String) -> CredentialHandle {
        CredentialHandle(id: UUID().uuidString, kind: kind, label: label)
    }
}

/// The device-local secure store a vault handle points into: the plaintext secret in
/// the Keychain, keyed by `handle.id`. Only the content-free handle is ever surfaced
/// elsewhere — this is the "device-local secure store" `group.storeCredential` refers
/// to, so the secret never enters a group log or a vCard export.
enum CredentialVault {
    @discardableResult
    static func store(_ secret: String, for handle: CredentialHandle) -> Bool {
        Keychain.set(secret, for: handle.id)
    }

    static func secret(for handle: CredentialHandle) -> String? {
        Keychain.get(handle.id)
    }

    static func remove(_ handle: CredentialHandle) {
        Keychain.remove(handle.id)
    }
}
