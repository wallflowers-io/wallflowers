//  CLIPTokenizer.swift
//  CLIP's byte-pair tokenizer, as MobileCLIP's text tower expects it.
//
//  The text encoder takes [1, 77] int32 token ids and nothing else — no tokenizer ships with
//  the CoreML package — so this reproduces OpenAI's `SimpleTokenizer` exactly. Getting it
//  wrong does not throw; it silently produces a vector for a DIFFERENT string, so the
//  cross-modal alignment quietly degrades. That is why it is verified against the real model
//  rather than eyeballed.
//
//  Vocabulary is 49408 = 256 byte characters + 256 byte characters with `</w>` +
//  48894 learned merges + 2 specials (<|startoftext|>, <|endoftext|>).

import Foundation

public struct CLIPTokenizer {
    public static let contextLength = 77

    private let ranks: [String: Int]        // "a b" -> merge rank
    private let vocab: [String: Int32]      // token -> id
    private let byteEncoder: [UInt8: String]
    private let sot: Int32
    private let eot: Int32
    private let pattern = try! NSRegularExpression(
        pattern: "<\\|startoftext\\|>|<\\|endoftext\\|>|'s|'t|'re|'ve|'m|'ll|'d|[\\p{L}]+|[\\p{N}]|[^\\s\\p{L}\\p{N}]+",
        options: [.caseInsensitive]
    )

    /// `mergesText` is the decompressed `bpe_simple_vocab_16e6.txt`.
    public init(mergesText: String) {
        // GPT-2's byte<->unicode map: every byte gets a printable character, so arbitrary
        // UTF-8 survives a text-only BPE.
        var byteEncoder: [UInt8: String] = [:]
        var printable: [UInt8] = []
        printable.append(contentsOf: UInt8(33)...UInt8(126))
        printable.append(contentsOf: UInt8(161)...UInt8(172))
        printable.append(contentsOf: UInt8(174)...UInt8(255))
        var n = 0
        for b in UInt8(0)...UInt8(255) {
            if printable.contains(b) {
                byteEncoder[b] = String(UnicodeScalar(b))
            } else {
                byteEncoder[b] = String(UnicodeScalar(256 + n)!)
                n += 1
            }
        }
        self.byteEncoder = byteEncoder

        // Line 0 is the version header; CLIP keeps the next 48894 merges.
        let lines = mergesText.split(separator: "\n", omittingEmptySubsequences: false)
        let merges = lines.dropFirst().prefix(48894).map(String.init)

        var ranks: [String: Int] = [:]
        for (i, m) in merges.enumerated() { ranks[m] = i }
        self.ranks = ranks

        var vocab: [String: Int32] = [:]
        var id: Int32 = 0
        let base = (UInt8(0)...UInt8(255)).map { byteEncoder[$0]! }
        for t in base { vocab[t] = id; id += 1 }
        for t in base { vocab[t + "</w>"] = id; id += 1 }
        for m in merges { vocab[m.replacingOccurrences(of: " ", with: "")] = id; id += 1 }
        vocab["<|startoftext|>"] = id; sot = id; id += 1
        vocab["<|endoftext|>"] = id; eot = id; id += 1
        self.vocab = vocab
    }

    /// Token ids padded/truncated to `contextLength`, wrapped in the start/end markers.
    /// Truncation keeps the end marker, matching CLIP: a query longer than the window loses
    /// its tail, it does not lose its terminator.
    public func encode(_ text: String) -> [Int32] {
        var ids: [Int32] = [sot]
        for token in split(clean(text)) {
            let encoded = token.utf8.map { byteEncoder[$0]! }.joined()
            for piece in bpe(encoded).split(separator: " ") {
                if let v = vocab[String(piece)] { ids.append(v) }
            }
        }
        ids.append(eot)
        if ids.count > Self.contextLength {
            ids = Array(ids.prefix(Self.contextLength))
            ids[Self.contextLength - 1] = eot
        }
        return ids + Array(repeating: 0, count: Self.contextLength - ids.count)
    }

    // MARK: - internals

    private func clean(_ s: String) -> String {
        s.lowercased().split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    private func split(_ s: String) -> [String] {
        let ns = s as NSString
        return pattern.matches(in: s, range: NSRange(location: 0, length: ns.length))
            .map { ns.substring(with: $0.range) }
    }

    /// Greedily merge the highest-ranked adjacent pair until none is known.
    private func bpe(_ token: String) -> String {
        guard !token.isEmpty else { return token }
        var word = token.map(String.init)
        word[word.count - 1] += "</w>"
        if word.count == 1 { return word[0] }

        while true {
            var bestRank = Int.max
            var bestIndex = -1
            for i in 0..<(word.count - 1) {
                if let r = ranks["\(word[i]) \(word[i + 1])"], r < bestRank {
                    bestRank = r
                    bestIndex = i
                }
            }
            guard bestIndex >= 0 else { break }
            let first = word[bestIndex], second = word[bestIndex + 1]
            var merged: [String] = []
            var i = 0
            while i < word.count {
                if i < word.count - 1, word[i] == first, word[i + 1] == second {
                    merged.append(first + second)
                    i += 2
                } else {
                    merged.append(word[i])
                    i += 1
                }
            }
            word = merged
            if word.count == 1 { break }
        }
        return word.joined(separator: " ")
    }
}
