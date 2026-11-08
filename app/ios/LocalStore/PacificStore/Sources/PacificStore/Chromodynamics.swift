//  Chromodynamics.swift
//  LINGUISTIC CHROMODYNAMICS (7 Aug transmission): every span of language carries
//  three charges — a FROM, a TOWARDS, and an ABOUT (a complexity measure).
//  Red, Blue and Gold. Each associated with family behaviours through time.
//
//      RED    from     the retrospective charge — origin, memory, where the
//                      flow has been. (Doppler: red shifts away.)
//      BLUE   towards  the prospective charge — intent, plan, approach, where
//                      the flow is going. (Doppler: blue shifts toward.)
//      GOLD   about    the complexity charge — how much subject-matter the span
//                      actually carries; richness, not direction.
//
//  Composition follows the chromodynamic rule the FEED transmission drew: charges
//  COMPOSE INTO WHITE — a set of items whose reds, blues and golds are in balance
//  is colour-neutral, and the notification circle it composes into is white.
//  `whiteness` measures that balance on the charge simplex.
//
//  RINGFENCE (invariant I-D): this is the EMBEDDING RING, not the model ring —
//  marker lexicons + token entropy, deterministic, versioned (`lexicon`), no LLM
//  and no Apple model drift. A gauge in the Transmission sense: it names itself
//  so a frame that used it is reproducible in context.
//
//  TWO INSTRUMENTS, ONE MECHANISM: lc-v1 (English, exact-token markers) and
//  lc-kr-v1 (the Seoul lane —
//  a SUPERSET adding Korean affix-matched markers; identical to lc-v1 on pure
//  English text). `charge(_:language:)` selects; `lexicon(for:)` names.
//
//  The charges are TIME-AVERAGED instruments by design ("predicting families of
//  behaviour, not individual states"): read them over clusters and windows, not
//  as verdicts on a single sentence.

import Foundation

/// The three charges of one span of language, each in [0, 1].
public struct Charge: Equatable, Sendable {
    public var red: Float     // from — retrospective
    public var blue: Float    // towards — prospective
    public var gold: Float    // about — complexity

    public init(red: Float, blue: Float, gold: Float) {
        self.red = red
        self.blue = blue
        self.gold = gold
    }

    public static let neutral = Charge(red: 0, blue: 0, gold: 0)

    /// Balance on the charge simplex: 1 when the three charges are equal (the
    /// chromodynamic WHITE — colour confinement), 0 when one charge holds
    /// everything. A neutral (all-zero) charge is vacuously white.
    public var whiteness: Float {
        let total = red + blue + gold
        guard total > 0 else { return 1 }
        let shares = [red / total, blue / total, gold / total]
        // Distance from the simplex centre, normalized by the farthest vertex.
        let d = shares.map { ($0 - 1.0 / 3) * ($0 - 1.0 / 3) }.reduce(0, +).squareRoot()
        let dMax = Float(2.0 / 3).squareRoot()   // a pure single-charge state
        return 1 - d / dMax
    }
}

public enum Chromodynamics {

    /// The marker lexicons are the versioned instrument — bump when they change,
    /// so any frame carrying charges names the ruler it measured with.
    public static let lexicon = "lc-v1"

    /// The KOREAN instrument: the
    /// lc-v1 English lexicons PLUS Korean affix-matched lexicons, one combined
    /// versioned ruler. Seoul streams are MIXED (11/13 rows Korean in Simulation
    /// 01), so lc-kr-v1 is a SUPERSET of lc-v1, never a replacement: on pure
    /// English text the two instruments are value-identical (Hangul markers
    /// cannot fire on Latin tokens, and the marker-boosted gold reduces to the
    /// lc-v1 entropy blend when no aboutness marker fires) — a property the
    /// package tests pin.
    public static let koreanLexicon = "lc-kr-v1"

    /// Which ruler measures under `language` — recorded exactly like `lexicon`
    /// is, so a frame carrying charges names its instrument. The language comes
    /// from the ONE union language authority (`UnionLanguageSetting`), the same
    /// setting that selects the union embedder (F-3) — never a second config.
    public static func lexicon(for language: NLLanguageIdentifier) -> String {
        switch language {
        case .english: return lexicon
        case .korean: return koreanLexicon
        }
    }

    // FROM — retrospection, origin, the already-happened.
    static let redMarkers: Set<String> = [
        "was", "were", "had", "did", "been", "ago", "yesterday", "remember",
        "remembered", "used", "back", "before", "last", "since", "came", "went",
        "left", "ended", "finished", "originally", "earlier", "once", "former",
        "began", "grew", "history", "past",
    ]

    // TOWARDS — intent, plan, approach, the not-yet.
    static let blueMarkers: Set<String> = [
        "will", "gonna", "going", "shall", "plan", "plans", "planning", "tomorrow",
        "next", "soon", "later", "upcoming", "future", "toward", "towards", "let's",
        "lets", "should", "want", "wants", "hope", "hopes", "aim", "intend", "until",
        "book", "schedule", "scheduled", "rsvp", "join", "meet", "start", "launch",
    ]

    // KOREAN MARKERS (lc-kr-v1). Korean is agglutinative: the FROM/TOWARDS
    // signal lives in PARTICLES (조사) and VERB ENDINGS (어미) that attach to
    // the eojeol the tokenizer yields ("학교에서" is ONE token), so an exact
    // set-lookup genuinely cannot see them. Matching is therefore AFFIX-based
    // (`matchesKoreanMarker`): a marker hits as the token's SUFFIX (particles,
    // endings, and Korean's head-final compounds — 서울출신, 개인정보) or, for
    // markers of two or more syllables, as its PREFIX (a content word with a
    // particle attached after it — 내일은, 계획을; the ≥2-syllable floor keeps
    // 갈 from firing inside 갈비). Deterministic, no analyzer, no LLM.
    //
    // AGENT-DRAFTED (9 Aug 2026), PENDING NATIVE-SPEAKER REVIEW — the whole
    // lexicon, not just the additions. Known coarseness for that review:
    // 에서 is both ablative-from AND locative-at; 으로 is both directional and
    // instrumental; the seed's "에 대해" is realised as the tokens 대해/대해서
    // (the bare particle 에 is far too greedy to be a marker on its own).

    // FROM — origin, the already-happened: ablative particles, past-tense 오다
    // forms, and retrospective time words.
    static let koreanRedMarkers: [String] = [
        "에서", "부터", "로부터", "출신", "왔", "왔어", "왔다", "왔어요", "왔습니다",
        "지난", "어제", "이전", "예전", "작년", "원래", "때문에",
    ]

    // TOWARDS — intent, plan, approach: directional/purposive particles,
    // intentive endings, prospective time words.
    static let koreanBlueMarkers: [String] = [
        "으로", "에게", "향해", "향해서", "하러", "으러", "하려고", "려고", "예정",
        "계획", "내일", "다음", "갈", "갈게", "할게", "위해", "위해서",
    ]

    // ABOUT — aboutness carriers ("~에 대해", topic/content/information words).
    // Unlike lc-v1 (where gold is pure entropy), these can RAISE gold — see
    // `charge`.
    static let koreanGoldMarkers: [String] = [
        "대해", "대해서", "대하여", "관련", "관해", "관해서", "주제", "내용", "정보",
    ]

    /// Charge one span of text. Deterministic: tokenization is plain
    /// letters/numbers lowercased, charges are marker densities (red, blue) and
    /// normalized token entropy blended with type–token ratio (gold).
    ///
    /// `language` selects the instrument — `.english` = lc-v1 (bit-identical to
    /// the pre-F-2 behaviour), `.korean` = lc-kr-v1 (the superset: English exact
    /// markers + Korean affix markers + marker-boosted gold). Callers pass the
    /// union language authority's value; the default keeps every existing call
    /// site on lc-v1.
    public static func charge(_ text: String, language: NLLanguageIdentifier = .english) -> Charge {
        let tokens = tokenize(text)
        guard !tokens.isEmpty else { return .neutral }
        let n = Float(tokens.count)

        let korean = language == .korean
        let red = Float(tokens.filter {
            redMarkers.contains($0) || (korean && matchesKoreanMarker($0, in: koreanRedMarkers))
        }.count)
        let blue = Float(tokens.filter {
            blueMarkers.contains($0) || (korean && matchesKoreanMarker($0, in: koreanBlueMarkers))
        }.count)

        // Density → [0,1] with a soft knee: 1 marker in 8 tokens already reads
        // as a strong directional charge.
        func density(_ hits: Float) -> Float { min(1, (hits / n) * 8) }

        // GOLD — about-ness as information: Shannon entropy of the token
        // distribution over its own maximum, blended with type–token ratio.
        var counts: [String: Int] = [:]
        for t in tokens { counts[t, default: 0] += 1 }
        // Summation order is FIXED (sorted keys): float addition is not
        // associative, and Dictionary iteration order is per-instance random —
        // unordered accumulation breaks bit-determinism in the last ulp.
        var entropy: Float = 0
        for (_, c) in counts.sorted(by: { $0.key < $1.key }) {
            let p = Float(c) / n
            entropy -= p * log2(p)
        }
        let maxEntropy = log2(n)
        let normalizedEntropy = maxEntropy > 0 ? entropy / maxEntropy : 0
        let typeToken = Float(counts.count) / n
        let entropyGold = min(1, 0.5 * normalizedEntropy + 0.5 * typeToken)

        // lc-kr-v1 gold: aboutness markers can RAISE gold, never lower it —
        // max() reduces to the lc-v1 entropy blend when no marker fires, so the
        // formula is one instrument, not a fork.
        let gold: Float
        if korean {
            let goldHits = Float(tokens.filter { matchesKoreanMarker($0, in: koreanGoldMarkers) }.count)
            gold = max(entropyGold, density(goldHits))
        } else {
            gold = entropyGold
        }

        return Charge(red: density(red), blue: density(blue), gold: gold)
    }

    /// Whether a Korean marker hits `token` under the affix mechanics: any
    /// marker as the token's suffix (particles/endings attach at the end;
    /// compounds are head-final), a ≥2-syllable marker also as its prefix (a
    /// content word wearing a particle). A token counts once per charge however
    /// many markers hit — the same per-token semantics as the English sets.
    static func matchesKoreanMarker(_ token: String, in markers: [String]) -> Bool {
        for marker in markers {
            if token.hasSuffix(marker) { return true }
            if marker.count >= 2 && token.hasPrefix(marker) { return true }
        }
        return false
    }

    /// Compose many charges into one — the notification-circle rule: each charge
    /// contributes its mean, and the result's `whiteness` says how balanced the
    /// set is. An empty set is neutral (and therefore white).
    public static func compose(_ charges: [Charge]) -> Charge {
        guard !charges.isEmpty else { return .neutral }
        let n = Float(charges.count)
        return Charge(red: charges.map(\.red).reduce(0, +) / n,
                      blue: charges.map(\.blue).reduce(0, +) / n,
                      gold: charges.map(\.gold).reduce(0, +) / n)
    }

    static func tokenize(_ text: String) -> [String] {
        text.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }
    }
}
