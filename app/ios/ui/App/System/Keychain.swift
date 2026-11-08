//  Keychain.swift
//  Minimal Keychain-backed secret store — the device-local vault a CredentialHandle
//  points into (the secret keyed by the handle id). At-rest protection for ANY
//  System's credential; the secret never enters the plaintext store or a group log.

import Foundation
import Security

enum Keychain {
    private static let service = "network.pacific.credentials"

    @discardableResult
    static func setData(_ data: Data, for key: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    @discardableResult
    static func set(_ string: String, for key: String) -> Bool {
        setData(Data(string.utf8), for: key)
    }

    static func getData(_ key: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess else { return nil }
        return out as? Data
    }

    static func get(_ key: String) -> String? {
        getData(key).flatMap { String(data: $0, encoding: .utf8) }
    }

    static func remove(_ key: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
