//! newitem_icd — conformance between `coordination/newitem.icd.json` and the declaration
//! the newitem modal actually renders.
//!
//! The contract was written FROM the surface (app/ui/App/Design/New.swift), which is the
//! honest direction: the modal is what a person meets, and a document describing it is
//! only worth having if it cannot quietly stop being true.
//!
//! What this pins:
//!
//!   doc -> declaration   every kind's field list, IN ORDER, is exactly what
//!                        `mint::fields` hands the surface — order is row order, so a
//!                        reordering is a redesign and should read as one.
//!   doc -> catalog       the type ids are the real ones, and the kinds the doc says
//!                        are not minted are exactly the ones `mint::classify` refuses.
//!   doc -> inputs        every `MintInput` the declaration can emit has a control
//!                        described. A field the surface cannot draw is a blank row.
//!   doc -> carriage      the three carriages are described, and `dependent` names the
//!                        one field whose delivery hangs on another's value.
//!
//! What this does NOT assert: that the SwiftUI renderer obeys the input table. That is a
//! claim about the app and belongs to its own tests (`NewComponentTests`). This module
//! pins the INVENTORY and the ORDER — the parts that would otherwise drift in silence,
//! which is exactly how the four dropped fields happened.

#[cfg(test)]
mod tests {
    use crate::mint::{classify, fields, is_mintable, Carriage, MintInput};
    use crate::object::ObjectKind;
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn doc() -> Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/newitem.icd.json");
        serde_json::from_str(&std::fs::read_to_string(path).expect("read newitem ICD"))
            .expect("parse newitem ICD")
    }

    /// The heart of it: the doc's field list per kind is the declaration's, in order.
    #[test]
    fn every_kind_declares_exactly_what_the_contract_says() {
        let d = doc();
        let kinds = d["kinds"].as_object().expect("kinds");
        for (name, spec) in kinds {
            let kind = crate::mint::kind_from_name(name)
                .unwrap_or_else(|| panic!("`{name}` is not an ObjectKind"));
            assert_eq!(
                spec["typeId"].as_u64().expect("typeId") as u16,
                kind.type_id(),
                "{name}: the contract's type id is not the catalog's"
            );
            let declared: Vec<String> =
                fields(kind).iter().map(|f| f.label.to_string()).collect();
            let documented: Vec<String> = spec["fields"]
                .as_array()
                .expect("fields")
                .iter()
                .map(|v| v.as_str().expect("field label").to_string())
                .collect();
            assert_eq!(
                declared, documented,
                "{name}: the contract and the declaration disagree. Order is ROW ORDER — \
                 if this is a redesign, say so in the doc; if it is a slip, the doc is right."
            );
        }
    }

    /// Every mintable kind is in the doc, and every kind in the doc is mintable. A kind
    /// with a `+` and no contract is undocumented UI; a contract for a kind with no `+`
    /// describes a screen nobody can reach.
    #[test]
    fn the_contract_covers_exactly_the_mintable_kinds() {
        let d = doc();
        let documented: BTreeSet<String> =
            d["kinds"].as_object().expect("kinds").keys().cloned().collect();
        let mintable: BTreeSet<String> = ObjectKind::ALL
            .iter()
            .filter(|k| is_mintable(**k))
            .map(|k| k.name().to_string())
            .collect();
        assert_eq!(documented, mintable);
    }

    /// And the refusals are named. An absent `+` should be a decision on the record.
    #[test]
    fn every_unminted_kind_says_why() {
        let d = doc();
        let excused: BTreeSet<String> = d["notMinted"]
            .as_object()
            .expect("notMinted")
            .keys()
            .filter(|k| *k != "summary")
            .cloned()
            .collect();
        let refused: BTreeSet<String> = ObjectKind::ALL
            .iter()
            .filter(|k| classify(**k).is_err())
            .map(|k| k.name().to_string())
            .collect();
        assert_eq!(
            excused, refused,
            "a kind without a `+` must say why in the contract, and only such kinds may"
        );
    }

    /// Every input the declaration can emit has a control described. A field whose input
    /// the contract does not cover is a row the surface has no instructions for.
    #[test]
    fn every_declared_input_has_a_control() {
        let d = doc();
        let described: BTreeSet<&str> =
            d["inputs"].as_object().expect("inputs").keys()
                .filter(|k| *k != "summary")
                .map(|s| s.as_str())
                .collect();
        for &kind in ObjectKind::ALL {
            for f in fields(kind) {
                let name = match f.input {
                    MintInput::Line => "line",
                    MintInput::Multiline => "multiline",
                    MintInput::Choice(_) => "choice",
                    MintInput::Date => "date",
                    MintInput::Recurrence => "recurrence",
                    MintInput::Money => "money",
                    MintInput::Place => "place",
                    MintInput::Fix => "fix",
                    MintInput::Icon => "icon",
                    MintInput::Banner => "banner",
                };
                assert!(
                    described.contains(name),
                    "{:?} declares a `{name}` field ({}) the contract does not describe",
                    kind, f.label
                );
            }
        }
    }

    /// The carriage vocabulary is complete, and `deferred` is described as a promise the
    /// contract deliberately does not make. If a carriage ever loses its paragraph, the
    /// flag becomes a way to hide a dropped field again.
    #[test]
    fn every_carriage_is_described() {
        let d = doc();
        let c = d["carriage"].as_object().expect("carriage");
        for key in ["op0", "separate", "deferred", "dependent"] {
            let text = c[key].as_str().unwrap_or_default();
            assert!(!text.is_empty(), "carriage `{key}` has no description");
        }
        // And the flag is only claimed where it is used.
        let used: BTreeSet<&str> = ObjectKind::ALL
            .iter()
            .flat_map(|k| fields(*k))
            .map(|f| match f.carriage {
                Carriage::Op0 => "op0",
                Carriage::Separate => "separate",
                Carriage::Deferred => "deferred",
            })
            .collect();
        for u in used {
            assert!(c.contains_key(u), "carriage `{u}` is used but not described");
        }
    }
}
