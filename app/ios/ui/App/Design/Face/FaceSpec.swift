//  FaceSpec.swift
//  A Site's face, as `core/coordination/face.schema.json` specifies it — the one
//  specification the webapp's editor, core and the Arc's renderer build to — read from
//  the schema the core embeds (`faceCatalogue()`). Nothing here restates it.

import Foundation
import PacificCore

// MARK: - A JSON value

/// The face document as it is: a tree, edited in place and written back whole. Ints
/// stay ints (`rot`, `corners`, `precision` are integers to the schema) and the rest
/// of the numbers stay doubles, so a face loads and saves unchanged.
enum JSON: Equatable, Codable {
    case object([String: JSON])
    case array([JSON])
    case string(String)
    case int(Int)
    case double(Double)
    case bool(Bool)
    case null

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let i = try? c.decode(Int.self) { self = .int(i) }
        else if let d = try? c.decode(Double.self) { self = .double(d) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSON].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSON].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .object(let o): try c.encode(o)
        case .array(let a):  try c.encode(a)
        case .string(let s): try c.encode(s)
        case .int(let i):    try c.encode(i)
        case .double(let d): try c.encode(d)
        case .bool(let b):   try c.encode(b)
        case .null:          try c.encodeNil()
        }
    }

    init?(text: String) {
        guard let v = try? JSONDecoder().decode(JSON.self, from: Data(text.utf8)) else { return nil }
        self = v
    }

    var text: String {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return (try? e.encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }

    subscript(key: String) -> JSON {
        get { if case .object(let o) = self { return o[key] ?? .null }; return .null }
        set {
            var o: [String: JSON] = [:]
            if case .object(let held) = self { o = held }
            o[key] = newValue
            self = .object(o)
        }
    }

    subscript(index: Int) -> JSON {
        get { if case .array(let a) = self, a.indices.contains(index) { return a[index] }; return .null }
        set { if case .array(var a) = self, a.indices.contains(index) { a[index] = newValue; self = .array(a) } }
    }

    subscript(path: [String]) -> JSON {
        get { path.reduce(self) { $0[$1] } }
        set {
            guard let head = path.first else { self = newValue; return }
            var child = self[head]
            child[Array(path.dropFirst())] = newValue
            self[head] = child
        }
    }

    var string: String? { if case .string(let s) = self { return s }; return nil }
    var bool: Bool? { if case .bool(let b) = self { return b }; return nil }
    var number: Double? {
        switch self { case .int(let i): return Double(i); case .double(let d): return d; default: return nil }
    }
    var array: [JSON] { if case .array(let a) = self { return a }; return [] }
    var keys: [String] { if case .object(let o) = self { return o.keys.sorted() }; return [] }
}

// MARK: - The specification

/// What the schema says, read once from the core.
struct FaceSpec {
    let schema: JSON

    static let shared = FaceSpec(schema: (JSON(text: faceCatalogue()) ?? .null)["schema"])

    /// The face a new Site opens with: face.js's own, as the schema carries it.
    var opening: JSON { schema["x-wf-opening"] }
}
