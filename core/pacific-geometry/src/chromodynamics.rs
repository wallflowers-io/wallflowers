//! chromodynamics — LINGUISTIC CHROMODYNAMICS (7 Aug transmission): every span
//! of language carries three charges — a FROM, a TOWARDS, and an ABOUT (a
//! complexity measure). Red, Blue and Gold. Each associated with family
//! behaviours through time.
//!
//! ```text
//!     RED    from     the retrospective charge — origin, memory, where the
//!                     flow has been. (Doppler: red shifts away.)
//!     BLUE   towards  the prospective charge — intent, plan, approach, where
//!                     the flow is going. (Doppler: blue shifts toward.)
//!     GOLD   about    the complexity charge — how much subject-matter the span
//!                     actually carries; richness, not direction.
//! ```
//!
//! Composition follows the chromodynamic rule the FEED transmission drew: charges
//! COMPOSE INTO WHITE — a set of items whose reds, blues and golds are in balance
//! is colour-neutral, and the notification circle it composes into is white.
//! `whiteness` measures that balance on the charge simplex.
//!
//! RINGFENCE (invariant I-D): this is the EMBEDDING RING, not the model ring —
//! marker lexicons + token entropy, deterministic, versioned (`LEXICON`), no LLM
//! and no Apple model drift. A gauge in the Transmission sense: it names itself
//! so a frame that used it is reproducible in context.
//!
//! The charges are TIME-AVERAGED instruments by design ("predicting families of
//! behaviour, not individual states"): read them over clusters and windows, not
//! as verdicts on a single sentence.
//!
//! 1:1 port of PacificStore's `Chromodynamics.swift` (Swift `Float` → `f32`).
//! The Swift `enum Chromodynamics` namespace maps to this module's items.

use std::collections::BTreeMap;

/// The three charges of one span of language, each in [0, 1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Charge {
    /// from — retrospective
    pub red: f32,
    /// towards — prospective
    pub blue: f32,
    /// about — complexity
    pub gold: f32,
}

impl Charge {
    pub fn new(red: f32, blue: f32, gold: f32) -> Self {
        Charge { red, blue, gold }
    }

    pub const NEUTRAL: Charge = Charge { red: 0.0, blue: 0.0, gold: 0.0 };

    /// Balance on the charge simplex: 1 when the three charges are equal (the
    /// chromodynamic WHITE — colour confinement), 0 when one charge holds
    /// everything. A neutral (all-zero) charge is vacuously white.
    pub fn whiteness(&self) -> f32 {
        let total = self.red + self.blue + self.gold;
        if !(total > 0.0) {
            return 1.0;
        }
        let shares = [self.red / total, self.blue / total, self.gold / total];
        // Distance from the simplex centre, normalized by the farthest vertex.
        let d = shares
            .iter()
            .map(|s| (s - 1.0 / 3.0) * (s - 1.0 / 3.0))
            .fold(0.0f32, |acc, x| acc + x)
            .sqrt();
        // Swift: `Float(2.0 / 3).squareRoot()` — Double 2.0/3 narrowed to Float.
        let d_max = ((2.0f64 / 3.0) as f32).sqrt(); // a pure single-charge state
        1.0 - d / d_max
    }
}

/// The marker lexicons are the versioned instrument — bump when they change,
/// so any frame carrying charges names the ruler it measured with.
pub const LEXICON: &str = "lc-v1";

// FROM — retrospection, origin, the already-happened.
// (Swift `Set<String>`; membership-only, so a slice keeps the lexicon
// byte-identical in source with zero iteration-order surface.)
const RED_MARKERS: [&str; 27] = [
    "was", "were", "had", "did", "been", "ago", "yesterday", "remember",
    "remembered", "used", "back", "before", "last", "since", "came", "went",
    "left", "ended", "finished", "originally", "earlier", "once", "former",
    "began", "grew", "history", "past",
];

// TOWARDS — intent, plan, approach, the not-yet.
const BLUE_MARKERS: [&str; 33] = [
    "will", "gonna", "going", "shall", "plan", "plans", "planning", "tomorrow",
    "next", "soon", "later", "upcoming", "future", "toward", "towards", "let's",
    "lets", "should", "want", "wants", "hope", "hopes", "aim", "intend", "until",
    "book", "schedule", "scheduled", "rsvp", "join", "meet", "start", "launch",
];

/// Charge one span of text. Deterministic: tokenization is plain
/// letters/numbers lowercased, charges are marker densities (red, blue) and
/// normalized token entropy blended with type–token ratio (gold).
pub fn charge(text: &str) -> Charge {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return Charge::NEUTRAL;
    }
    let n = tokens.len() as f32;

    let red = tokens.iter().filter(|t| RED_MARKERS.contains(&t.as_str())).count() as f32;
    let blue = tokens.iter().filter(|t| BLUE_MARKERS.contains(&t.as_str())).count() as f32;

    // Density → [0,1] with a soft knee: 1 marker in 8 tokens already reads
    // as a strong directional charge.
    let density = |hits: f32| -> f32 { ((hits / n) * 8.0).min(1.0) };

    // GOLD — about-ness as information: Shannon entropy of the token
    // distribution over its own maximum, blended with type–token ratio.
    //
    // Summation order is FIXED (sorted keys): float addition is not
    // associative, and Swift Dictionary iteration order is per-instance
    // random — unordered accumulation breaks bit-determinism in the last
    // ulp. A BTreeMap gives the sorted-key walk directly.
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &tokens {
        *counts.entry(t.as_str()).or_insert(0) += 1;
    }
    let mut entropy: f32 = 0.0;
    for &c in counts.values() {
        let p = c as f32 / n;
        entropy -= p * p.log2();
    }
    let max_entropy = n.log2();
    let normalized_entropy = if max_entropy > 0.0 { entropy / max_entropy } else { 0.0 };
    let type_token = counts.len() as f32 / n;
    let gold = (0.5 * normalized_entropy + 0.5 * type_token).min(1.0);

    Charge { red: density(red), blue: density(blue), gold }
}

/// Compose many charges into one — the notification-circle rule: each charge
/// contributes its mean, and the result's `whiteness` says how balanced the
/// set is. An empty set is neutral (and therefore white).
pub fn compose(charges: &[Charge]) -> Charge {
    if charges.is_empty() {
        return Charge::NEUTRAL;
    }
    let n = charges.len() as f32;
    Charge {
        red: charges.iter().map(|c| c.red).fold(0.0, |acc, x| acc + x) / n,
        blue: charges.iter().map(|c| c.blue).fold(0.0, |acc, x| acc + x) / n,
        gold: charges.iter().map(|c| c.gold).fold(0.0, |acc, x| acc + x) / n,
    }
}

/// Swift: lowercase, split on `CharacterSet.alphanumerics.inverted`, drop
/// empties. Rust mirror: `char::is_alphanumeric` (identical on ASCII; see the
/// M*-category caveat in the port notes).
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

// The three charges proven on plain language — deterministic, no model.
//
// What must hold: retrospective text reads RED, prospective text reads BLUE,
// rich text carries more GOLD than repetition; balanced charge sets compose
// toward WHITE and single-charge sets do not; and the whole instrument is
// deterministic run to run.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrospective_text_reads_red() {
        let c = charge("we were there last summer, remember how it began and how it ended");
        assert!(c.red > c.blue);
        assert!(c.red > 0.3);
    }

    #[test]
    fn prospective_text_reads_blue() {
        let c = charge("let's plan the launch for next month, we will meet soon and book the venue");
        assert!(c.blue > c.red);
        assert!(c.blue > 0.3);
    }

    #[test]
    fn rich_text_carries_more_gold_than_repetition() {
        let rich = charge(
            "the harbour market gathers ceramicists, luthiers and foragers under one tide chart",
        );
        let flat = charge("yes yes yes yes yes yes yes yes yes yes yes yes");
        assert!(rich.gold > flat.gold);
    }

    #[test]
    fn balanced_charges_compose_toward_white() {
        let balanced = compose(&[
            Charge::new(0.8, 0.1, 0.1),
            Charge::new(0.1, 0.8, 0.1),
            Charge::new(0.1, 0.1, 0.8),
        ]);
        let pure = Charge::new(0.9, 0.0, 0.0);
        assert!(balanced.whiteness() > 0.95);
        assert!(pure.whiteness() < 0.1);
    }

    #[test]
    fn neutral_is_vacuously_white_and_empty_composes_neutral() {
        assert_eq!(Charge::NEUTRAL.whiteness(), 1.0);
        assert_eq!(compose(&[]), Charge::NEUTRAL);
    }

    #[test]
    fn deterministic_run_to_run() {
        let text = "we came from the old studio and now we will build the new one";
        assert_eq!(charge(text), charge(text));
    }

    #[test]
    fn empty_text_is_neutral() {
        assert_eq!(charge("   "), Charge::NEUTRAL);
    }
}
