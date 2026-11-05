//  Wipe.swift
//  SIGN OUT — and sign out here means ERASE, not "forget the session".
//
//  A Pacific install is not an account you log into: the identity is an Ed25519 key
//  that lives on this device, sealed under the DeviceKey, with the message archive,
//  MLS group state and every System credential beside it. There is no server copy to
//  come back to. So the only sign-out that means anything is the one the user could
//  otherwise get by deleting the app: every byte the container holds, plus the
//  keychain items that would outlive it.
//
//  WHAT GOES
//    · the keychain      network.pacific.atrest (the device master key),
//                        network.pacific.credentials (System secrets) — and in fact
//                        EVERY item in the app's access group, by class, so a store
//                        added later cannot be forgotten here.
//    · the defaults      the whole persistent domain: relay, arc, tags, theme, the
//                        first-boot gate, the account tag.
//    · the container     Documents, Library and tmp emptied — pacific.db, id_ed25519,
//                        the graph, chats, notes, proposals, media caches.
//
//  WHAT STAYS, and why
//    · the PASSKEY. It is an iCloud Keychain credential owned by the platform for
//      its relying party, not by this app; erasing this install must not take away the thing
//      that lets the person come back. It is what UNWRAPS THE SEED: the Arc holds
//      76 bytes of ciphertext whose key never left the authenticator, so signing in
//      again on this device fetches those bytes and opens them. Erasing the app does
//      not touch either half.
//    · the ACCOUNT. This is the part the old comment understated by calling the
//      passkey a channel: what comes back is the identity itself, not a rendezvous.
//      The 24 words bring it back with no passkey at all.
//    · THERE IS NO SERVER SESSION TO CLEAR, and now there is none anywhere. This
//      used to say the cookie jar died with the `.ephemeral` URLSession, so POSTing
//      /auth/signout would carry nothing. The route is gone and so is the session
//      subsystem behind it: authority is a signature over a nonce, minted per
//      request and valid for one. There is nothing to revoke because nothing was
//      issued.
//
//  WHY IT QUITS. The Rust core is built once, in `CoreService.init`, and holds open
//  handles on the files this deletes; the identity it loaded is still in memory. There
//  is no supported way to hand a live core a new, empty state directory, so continuing
//  to run after the wipe would leave the shell showing data that no longer exists and
//  a core writing into unlinked inodes. Terminating is the honest end: the next launch
//  constructs everything from nothing and lands on the first-boot door. The disk is
//  erased LAST, immediately before the exit, so a background task has the smallest
//  possible window to recreate anything.

import Foundation
import Security
import UIKit

enum Wipe {
    /// What the wipe actually did — logged, and returned so a caller could show it.
    struct Report {
        var keychainItems = 0        // classes that reported a deletion
        var defaultsKeys = 0
        var filesRemoved = 0
        var bytesRemoved: Int64 = 0
        var failures: [String] = []
    }

    private static func log(_ m: String) { NSLog("[wipe] %@", m) }

    /// Erase everything and terminate. Does not return.
    @MainActor
    static func eraseAndQuit() {
        let r = erase()
        log(String(format: "done: keychain=%d defaults=%d files=%d bytes=%lld failures=%d",
                   r.keychainItems, r.defaultsKeys, r.filesRemoved, r.bytesRemoved,
                   r.failures.count))
        for f in r.failures { log("failure: \(f)") }
        // Give the log a beat to flush to the device console, then go. `exit(0)` is
        // deliberate — see the header: there is no in-process way back to a clean core.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { exit(0) }
    }

    /// The erase itself, in the order that matters: secrets, then preferences, then
    /// the files (last, so nothing running can put them back before we quit).
    @discardableResult
    static func erase() -> Report {
        var r = Report()
        eraseKeychain(&r)
        eraseDefaults(&r)
        eraseContainer(&r)
        return r
    }

    // MARK: - Keychain

    /// Every secure class, not a list of the three services we know about. A
    /// SecItemDelete with only a class is scoped to this app's access group, so it
    /// cannot reach another app's items — and it cannot miss one of ours either.
    private static func eraseKeychain(_ r: inout Report) {
        let classes: [(String, CFString)] = [
            ("generic", kSecClassGenericPassword),
            ("internet", kSecClassInternetPassword),
            ("certificate", kSecClassCertificate),
            ("key", kSecClassKey),
            ("identity", kSecClassIdentity),
        ]
        for (name, cls) in classes {
            let status = SecItemDelete([kSecClass as String: cls] as CFDictionary)
            switch status {
            case errSecSuccess:
                r.keychainItems += 1
                log("keychain \(name): deleted")
            case errSecItemNotFound:
                log("keychain \(name): empty")
            default:
                r.failures.append("keychain \(name): OSStatus \(status)")
            }
        }
    }

    // MARK: - Defaults

    private static func eraseDefaults(_ r: inout Report) {
        guard let domain = Bundle.main.bundleIdentifier else {
            r.failures.append("defaults: no bundle identifier")
            return
        }
        let d = UserDefaults.standard
        r.defaultsKeys = d.persistentDomain(forName: domain)?.count ?? 0
        d.removePersistentDomain(forName: domain)
        d.synchronize()
        log("defaults \(domain): removed \(r.defaultsKeys) key(s)")
    }

    // MARK: - Container

    /// Documents, Library and tmp EMPTIED — the directories themselves stay, because
    /// the OS owns them and a launch expects them to exist. Library carries Application
    /// Support (the core's state dir, the graph, chats, notes, proposals, media) and
    /// Caches; both go.
    private static func eraseContainer(_ r: inout Report) {
        let fm = FileManager.default
        let home = URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)
        for name in ["Documents", "Library", "tmp"] {
            let dir = home.appendingPathComponent(name, isDirectory: true)
            guard let kids = try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil) else {
                log("\(name): nothing to read")
                continue
            }
            var here = 0
            for kid in kids {
                let size = sizeOf(kid)
                do {
                    try fm.removeItem(at: kid)
                    here += 1
                    r.filesRemoved += 1
                    r.bytesRemoved += size
                } catch {
                    // Best effort, and SAID: a file the OS holds open (a live log, a
                    // mapped cache) may refuse, and the user deserves to know the wipe
                    // was partial rather than be told it was complete.
                    r.failures.append("\(name)/\(kid.lastPathComponent): \(error.localizedDescription)")
                }
            }
            log("\(name): removed \(here)/\(kids.count) entries")
        }
    }

    /// Bytes under a path, files and directories alike. Best effort — a size we could
    /// not read counts as zero rather than aborting the removal it was measuring.
    private static func sizeOf(_ url: URL) -> Int64 {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: url.path, isDirectory: &isDir) else { return 0 }
        if !isDir.boolValue {
            let attrs = try? fm.attributesOfItem(atPath: url.path)
            return (attrs?[.size] as? NSNumber)?.int64Value ?? 0
        }
        var total: Int64 = 0
        if let e = fm.enumerator(at: url, includingPropertiesForKeys: [.fileSizeKey]) {
            for case let f as URL in e {
                let v = try? f.resourceValues(forKeys: [.fileSizeKey])
                total += Int64(v?.fileSize ?? 0)
            }
        }
        return total
    }
}
