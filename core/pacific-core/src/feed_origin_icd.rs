//! feed_origin_icd — conformance between `coordination/feed-origin.icd.json`
//! and the vocabularies it canonicalises.
//!
//! The feed-origin ICD is the product stance ("no opaque algorithm") stated as
//! schema: a CLOSED set of origins, a stamp per origin, typed refusals, and a
//! pure ordering contract. A stance document rots quietly, so this module makes
//! drift loud:
//!
//!   doc -> fixtures   the storable origins must be exactly the provenance enum
//!                     the shipped fixture schema uses (member/hop/system), and
//!                     the hop bound must agree.
//!   doc -> ops        every derived rule cites a source op; a rule marked
//!                     `implemented` must cite an op that exists in the
//!                     delta-graph ICD; `specified` may cite a planned op.
//!   doc -> doc        the origin set is closed, precedence is a total order
//!                     over exactly the storable origins, every origin carries
//!                     a stamp and a legibility line, and the four product
//!                     refusals are present by name.
//!
//! What is deliberately NOT asserted: that the feed assembly obeys the ordering
//! contract — that is a claim about the app's filter configs and belongs to
//! their tests. This module pins the INVENTORY and the REFUSALS: the parts that
//! would otherwise drift silently.

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn doc() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../coordination/feed-origin.icd.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("read feed-origin ICD"))
            .expect("feed-origin ICD parses")
    }

    /// The feed fixture schema. Core holds the one copy; the iOS app's fixtures follow it.
    fn fixture_schema() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../coordination/feed-dataset.schema.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("read feed fixture schema"))
            .expect("fixture schema parses")
    }

    fn delta_graph() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../coordination/delta-graph.icd.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("read delta-graph ICD"))
            .expect("delta-graph ICD parses")
    }

    fn origin_names(doc: &Value) -> BTreeSet<String> {
        doc["origins"]
            .as_object()
            .expect("origins object")
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn origin_set_is_closed_and_exact() {
        let names = origin_names(&doc());
        let expected: BTreeSet<String> = ["member", "hop", "system", "derived"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            names, expected,
            "the origin set is CLOSED — adding or removing an origin is a product \
             decision that must be made here, in review, not discovered in the app"
        );
    }

    #[test]
    fn storable_origins_match_the_shipped_fixture_enum() {
        let d = doc();
        let precedence: BTreeSet<String> = d["collision"]["precedence"]
            .as_array()
            .expect("precedence array")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        let fixture_enum: BTreeSet<String> = fixture_schema()["$defs"]["object"]["properties"]
            ["provenance"]["enum"]
            .as_array()
            .expect("fixture provenance enum")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert_eq!(
            precedence, fixture_enum,
            "storable origins (collision precedence) must equal the fixture \
             schema's provenance enum — the ICD canonicalises it, not forks it"
        );

        // Derived is read-time only: never storable, never in precedence.
        assert!(!precedence.contains("derived"));
        assert!(d["collision"]["derived_excluded"].is_string());
    }

    #[test]
    fn every_origin_carries_stamp_and_legibility() {
        let d = doc();
        for (name, o) in d["origins"].as_object().unwrap() {
            let required = o["stamp"]["required"].as_array().unwrap_or_else(|| {
                panic!("origin `{name}` missing stamp.required")
            });
            assert!(
                !required.is_empty(),
                "origin `{name}` must require at least its own stamp fields"
            );
            assert_eq!(
                required[0].as_str(),
                Some("origin"),
                "origin `{name}`: the stamp's first required field is `origin` itself"
            );
            let leg = o["legibility"].as_str().unwrap_or("");
            assert!(
                leg.contains('{'),
                "origin `{name}` needs a templated one-line why — `{leg}` renders nothing"
            );
        }
    }

    #[test]
    fn derived_rules_cite_real_or_declared_ops() {
        let d = doc();
        let dg = delta_graph();
        // Every op the model declares — a kind's own, and the facet ops the kinds
        // share. The document has no `components.messages` any more: an op is
        // declared where it belongs and nothing is reached by `$ref`.
        let mut known_ops: BTreeSet<String> = BTreeSet::new();
        for (_k, kind) in dg["kinds"].as_object().expect("delta-graph kinds") {
            known_ops.extend(kind["ops"].as_object().expect("kind.ops").keys().cloned());
        }
        for (_f, facet) in dg["facets"].as_object().expect("delta-graph facets") {
            known_ops.extend(facet["ops"].as_object().expect("facet.ops").keys().cloned());
        }

        for (rule, r) in d["origins"]["derived"]["rules"].as_object().unwrap() {
            let op = r["source_op"].as_str().unwrap_or_else(|| {
                panic!("derived rule `{rule}` cites no source_op")
            });
            let status = r["status"].as_str().unwrap_or("");
            match status {
                "implemented" => assert!(
                    known_ops.contains(op),
                    "derived rule `{rule}` claims implemented but `{op}` is not in the delta-graph ICD"
                ),
                "specified" => { /* planned op — existence not required yet */ }
                other => panic!("derived rule `{rule}` has unknown status `{other}`"),
            }
        }
    }

    #[test]
    fn hop_bound_agrees_with_fixture_schema() {
        let d = doc();
        let icd_max = d["origins"]["hop"]["hops_max"].as_u64().expect("hops_max");
        let fixture_max = fixture_schema()["$defs"]["object"]["properties"]["hops"]["maximum"]
            .as_u64()
            .expect("fixture hops maximum");
        assert_eq!(icd_max, fixture_max, "hop distance bound drifted");
    }

    #[test]
    fn the_four_refusals_stand() {
        let d = doc();
        let names: BTreeSet<String> = d["refusals"]
            .as_array()
            .expect("refusals array")
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect();
        for required in ["paid-origin", "learned-ranking", "ai-authored", "unstamped"] {
            assert!(
                names.contains(required),
                "refusal `{required}` was removed — that is a product decision \
                 nobody gets to make silently"
            );
        }
    }

    #[test]
    fn ordering_contract_names_its_forbidden_inputs() {
        let d = doc();
        let forbidden: Vec<String> = d["ordering"]["forbidden_inputs"]
            .as_array()
            .expect("forbidden_inputs")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        for f in ["engagement counters", "per-person learned weights", "remote configuration"] {
            assert!(
                forbidden.iter().any(|x| x == f),
                "ordering contract lost forbidden input `{f}`"
            );
        }
    }
}
