//  UnionLanguageSetting.swift
//  The ONE persisted language authority for the Seoul lane.
//
//  One setting selects BOTH language-sensitive instruments, so they can never
//  disagree:
//    • which union embedder `UnionEmbedder.makeUnion(language:)` builds — and
//      therefore which model identity the store binds to (F-3), and
//    • which Chromodynamics instrument reads spans — lc-v1 or lc-kr-v1 (F-2).
//
//  Persistence is a UserDefaults key, the app's existing pattern for small
//  config ("life.rail.order", DevCommands' cursor). Absent or unrecognized
//  values read as .english — the shipped default, so a fresh install and every
//  pre-F-3 install behave exactly as before.
//
//  CHANGING the language is not a plain settings write: the union store's
//  vectors were embedded under the old model, so the store must be rebuilt
//  from retained text (`LodeLocalStore.migrateStore`). The composition root
//  reconciles setting ↔ store at launch; the DEBUG `union-language` dev verb
//  writes the setting and defers to that relaunch reconcile.

import Foundation

/// Load/save the configured union language. Reads default to `.english`.
public enum UnionLanguageSetting {

    /// The UserDefaults key. The stored value is `NLLanguageIdentifier.rawValue`
    /// ("en" / "ko") — frozen, it is also the model-identity suffix.
    public static let defaultsKey = "pacific.union.language"

    /// The configured language; `.english` when unset or unrecognized (never a
    /// throw — an unreadable setting means the shipped default, loudly normal).
    public static func load(from defaults: UserDefaults = .standard) -> NLLanguageIdentifier {
        defaults.string(forKey: defaultsKey).flatMap(NLLanguageIdentifier.init(rawValue:)) ?? .english
    }

    /// Persist `language` as the union language. The caller owns the consequences
    /// (the launch-time store reconcile); this is just the authority's memory.
    public static func save(_ language: NLLanguageIdentifier, to defaults: UserDefaults = .standard) {
        defaults.set(language.rawValue, forKey: defaultsKey)
    }
}
