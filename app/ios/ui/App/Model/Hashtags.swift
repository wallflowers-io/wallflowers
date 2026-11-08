//  Hashtags.swift
//  THE tag extractor (2 Aug ruling): a tag is a #hashtag the author typed,
//  parsed from the text itself — never minted by a model. CHAT and NOTE share
//  this one definition, so "#parsnips" means the same thing in a room, a DM and
//  a note, and the two surfaces can never drift on what counts as a tag.
//
//  Tags are DERIVED wherever they are needed (at index time, at list build, at
//  note commit) — a pure function of the text, never a second store.

import Foundation

enum Hashtags {
    /// Tags in `text`, lowercased, first-seen order, deduped: letters/digits/_
    /// after a '#', at least one character. "#Parsnips!" → "parsnips".
    static func extract(_ text: String) -> [String] {
        var out: [String] = []
        var seen = Set<String>()
        for match in text.matches(of: /#([\p{L}\p{N}_]+)/) {
            let tag = String(match.1).lowercased()
            if seen.insert(tag).inserted { out.append(tag) }
        }
        return out
    }
}
