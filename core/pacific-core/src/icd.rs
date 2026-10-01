//! icd — conformance between `coordination/delta-graph.icd.json` and the code.
//!
//! The ICD is an AsyncAPI 3.0 document describing every delta op and what it does
//! to the lodedb-graph read model. A document like that rots the day it is written
//! unless something fails when it drifts, so this module is that something.
//!
//! `ObjectType::ops()` is the anchor because it is already the enforced truth:
//! every op authored against a type must appear in its table or the Coordinator
//! rejects the delta. The checks below run in BOTH directions —
//!
//! ONE OP-GROUP IS ANCHORED ELSEWHERE, and it is not an exception so much as the
//! same rule pointed at the table that really enforces it. `Coordinator::deliver`
//! recognises the three base RATIFY ops AHEAD of the type's op table and
//! `Coordinator::ratify_state` folds them, so no `ObjectType` declares them and no
//! `T::reduce` has an arm — which is how they rode the wire through the whole of M4,
//! accepted on every object, in no op table and in no channel of this document. They
//! are checked against `coordinator::RATIFY_OPS`, on EVERY channel, because "accepted
//! on every object" is precisely the claim the document now has to carry.
//!
//!   code -> doc   an op declared by a reducer must appear in the ICD, with a
//!                 matching op id, authority and commutativity. Adding an op
//!                 without saying what it does to the graph fails here.
//!   doc -> code   a message in the ICD must be a real op. Renaming or removing
//!                 an op without updating the ICD fails here.
//!
//! The KINDS it visits are derived from `ObjectKind::ALL` by an exhaustive match
//! (`anchor`), not listed by hand. That is the third direction, and it was the one
//! that was missing: `SystemType` has a real op table, a real reducer, a real author
//! door on `Node` and a real FFI entry point, and it had no channel here until 15
//! Sep 2026 — so this module simply never visited it and the two could drift with
//! nothing to fail. A thirteenth kind now does not COMPILE until somebody has said
//! which channel holds it, or why it has none.
//!
//! What is deliberately NOT asserted: the `x-graph` clauses themselves. Whether
//! `event.setVenue` really mints `happens_at` is a claim about the projector, and
//! the projector lives in another crate (arc-resolver); asserting it here would
//! only compare the document to itself. This module pins the INVENTORY — the part
//! that silently drifts — and the `status` field carries the honest implemented /
//! specified split.

#[cfg(test)]
mod tests {
    use crate::object::{Authority, Commutativity, ObjectKind, ObjectType, OpDecl};
    use serde_json::Value;
    use std::collections::{BTreeMap, BTreeSet};

    fn doc() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../coordination/delta-graph.icd.json"
        );
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("the ICD must be readable at {path}: {e}"));
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("the ICD must be valid JSON: {e}"))
    }

    /// THE OPS ONE KIND DECLARES: name -> (op, ego, fold).
    ///
    /// A kind's own ops sit under `kinds.<kind>.ops`; the ops it shares with other
    /// kinds sit once under `facets.<facet>.ops`, with the facet naming the kinds
    /// that carry it. Both are the kind's, and this is where they come back
    /// together — the reader never has to know which is which.
    fn kind_ops(doc: &Value, kind: &str) -> BTreeMap<String, (u32, String, String)> {
        let k = doc["kinds"]
            .get(kind)
            .unwrap_or_else(|| panic!("the model must declare a kind `{kind}`"));
        let mut out = BTreeMap::new();
        let mut take = |name: &String, m: &Value| {
            out.insert(
                name.clone(),
                (
                    m["op"].as_u64().expect("op is an integer") as u32,
                    m["ego"].as_str().expect("ego").into(),
                    m["fold"].as_str().expect("fold").into(),
                ),
            );
        };
        for (name, m) in k["ops"].as_object().expect("kinds.<kind>.ops is a map") {
            take(name, m);
        }
        for (_facet, f) in doc["facets"].as_object().expect("facets is a map") {
            let on = f["on"].as_array().expect("facet.on is a list");
            if !on.iter().any(|c| c.as_str() == Some(kind)) {
                continue;
            }
            for (name, m) in f["ops"].as_object().expect("facet.ops is a map") {
                take(name, m);
            }
        }
        out
    }

    /// THE IDS A KIND RETIRED: op id -> the op that held it. A-13 rule 1: an id is never
    /// reassigned, so a delta from an older build can never fold as something else.
    fn retired(doc: &Value, kind: &str) -> BTreeMap<u32, String> {
        doc["kinds"][kind]["retired"]
            .as_object()
            .map(|r| {
                r.iter()
                    .map(|(id, was)| {
                        let id = id.parse().unwrap_or_else(|_| panic!("kinds.{kind}.retired: `{id}` is not an op id"));
                        (id, was.as_str().expect("a retired id names its op").to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every op, in the code's table or the model's, that takes an id this kind retired.
    fn retired_reuse(doc: &Value, kind: &str, code: &[OpDecl]) -> Vec<String> {
        let gone = retired(doc, kind);
        let model = kind_ops(doc, kind).into_iter().map(|(name, (id, _, _))| (name, id));
        let code = code.iter().map(|o| (o.name.to_string(), o.op_id));
        model
            .chain(code)
            .filter_map(|(name, id)| gone.get(&id).map(|was| format!("{name} takes {id}, retired from {was}")))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Both directions, for one object kind.
    fn check<T: ObjectType>(doc: &Value, channel: &str) {
        let documented = kind_ops(doc, channel);
        let reused = retired_reuse(doc, channel, T::ops());
        assert!(reused.is_empty(), "an id is never reassigned (A-13 rule 1): {reused:?}");

        // The channel must name the kind's real type id, since that is how a reader
        // maps a document channel back onto a delta on the wire.
        assert_eq!(
            doc["kinds"][channel]["typeId"]
                .as_u64()
                .expect("kinds.<kind>.typeId") as u16,
            T::KIND.type_id(),
            "channel `{channel}` declares the wrong typeId"
        );

        // The base RATIFY group, checked on EVERY channel against the table the
        // Coordinator obeys. `T::ops()` does not and must not contain these three (see
        // `coordinator::RATIFY_OPS`), so without this arm a ratify message in a channel
        // would fail the doc -> code loop below as "no such op" — which is how three
        // ops that every object accepts came to be documented nowhere at all. The
        // check runs per channel rather than once, because a channel that quietly
        // dropped them would be claiming a kind whose objects refuse a ratification,
        // and `deliver` has no such kind.
        for decl in crate::coordinator::RATIFY_OPS.iter() {
            let (op_id, authority, fold) = documented.get(decl.name).unwrap_or_else(|| {
                panic!(
                    "`{}` is a base op every object accepts, and channel `{channel}` does \
                     not document it. Every channel must $ref all three ratify messages.",
                    decl.name,
                )
            });
            assert_eq!(*op_id, decl.op_id, "`{}` has the wrong opId", decl.name);
            assert_eq!(*authority, decl.authority.ego(), "`{}` authority disagrees", decl.name);
            assert_eq!(
                fold,
                match decl.commutativity {
                    Commutativity::Sequenced => "sequenced",
                    Commutativity::Commutative => "commutative",
                },
                "`{}` fold disagrees",
                decl.name
            );
        }

        // code -> doc
        for op in T::ops() {
            let (op_id, authority, fold) = documented.get(op.name).unwrap_or_else(|| {
                panic!(
                    "`{}` is declared by {} but missing from the ICD.\n         \
                     Add it to coordination/delta-graph.icd.json under channel `{channel}` \
                     with an x-graph clause saying what it does to the graph \
                     (or why it is excluded).",
                    op.name,
                    std::any::type_name::<T>(),
                )
            });
            assert_eq!(
                *op_id, op.op_id,
                "`{}` has the wrong opId in the ICD",
                op.name
            );
            assert_eq!(*authority, op.authority.ego(), "`{}` authority disagrees", op.name);
            let want_fold = match op.commutativity {
                Commutativity::Sequenced => "sequenced",
                Commutativity::Commutative => "commutative",
            };
            assert_eq!(fold, want_fold, "`{}` fold disagrees", op.name);
        }

        // doc -> code
        for name in documented.keys() {
            // …except the ratify three, checked above against the table that owns them.
            if crate::coordinator::RATIFY_OPS.iter().any(|o| o.name == *name) {
                continue;
            }
            assert!(
                T::ops().iter().any(|o| o.name == *name),
                "the ICD documents `{name}` on channel `{channel}`, but {} declares no such op",
                std::any::type_name::<T>(),
            );
        }
    }

    /// Where one kind is anchored: `Ok((channel, the both-directions check))`, or
    /// `Err(reason)` for a kind that has no channel because it has no op table.
    ///
    /// A `match` rather than a list, and that is the whole mechanism. Until 15 Sep
    /// 2026 the kinds checked here were a hand-written array of nine strings sitting
    /// beside a twelve-variant `ObjectKind::ALL`, so `SystemType` — a real op table,
    /// a real reducer, a real author door on `Node`, a real FFI entry point — was
    /// simply never visited, and its inventory could drift from the document with
    /// nothing to fail. Matching exhaustively means a thirteenth variant does not
    /// COMPILE until somebody has said here which channel holds it, or why it has
    /// none; a kind can no longer be forgotten, only excluded on the record.
    fn anchor(kind: ObjectKind) -> Result<(&'static str, fn(&Value, &str)), &'static str> {
        match kind {
            ObjectKind::Group => Ok(("group", check::<crate::group::GroupType>)),
            ObjectKind::Forum => Ok(("forum", check::<crate::coordinator::ForumType>)),
            ObjectKind::System => Ok(("system", check::<crate::system::SystemType>)),
            ObjectKind::Project => Ok(("project", check::<crate::project::ProjectType>)),
            ObjectKind::Contact => Ok(("contact", check::<crate::contact::ContactType>)),
            ObjectKind::Conversation => Ok((
                "conversation",
                check::<crate::coordinator::ConversationType>,
            )),
            ObjectKind::Thing => Ok(("thing", check::<crate::thing::ThingType>)),
            ObjectKind::Place => Ok(("place", check::<crate::place::PlaceType>)),
            ObjectKind::Event => Ok(("event", check::<crate::event::EventType>)),
            ObjectKind::Post => Ok(("post", check::<crate::post::PostType>)),
            ObjectKind::Treasury => Ok((
                "treasury",
                check::<crate::treasury::TreasuryType>,
            )),
            ObjectKind::Note => Ok(("note", check::<crate::note::NoteType>)),
            ObjectKind::Host => Ok(("host", check::<crate::host::HostType>)),
            ObjectKind::Transaction => Ok(("transaction", check::<crate::transaction::TransactionType>)),

            // The two kinds that are a taxonomy entry and nothing else. Neither has an
            // `impl ObjectType`, so there is no op table to hold a channel to, no
            // reducer to fold one, and no author door on `Node` to emit one. Giving
            // them a channel would document ops that cannot be authored — exactly the
            // drift this module exists to stop, pointed the other way.
            ObjectKind::Field => Err(
                "Field (20) has no `impl ObjectType`: no op table, no reducer, no author \
                 path. Its wire id is reserved and nothing else.",
            ),
            ObjectKind::Topic => Err(
                "Topic (23) has no `impl ObjectType`: no op table, no reducer, no author \
                 path. Its wire id is reserved and nothing else.",
            ),
        }
    }

    #[test]
    fn icd_matches_every_op_table() {
        let d = doc();
        for &kind in ObjectKind::ALL {
            if let Ok((channel, check)) = anchor(kind) {
                check(&d, channel);
            }
        }
    }

    /// A retired id taken again, by the code's table or the model's, is named. Forum's 4 was
    /// forum.setRoom until 26 Sep 2026; ICD 2.1.0 put forum.retract on it, and only a
    /// comment in coordinator.rs said so.
    #[test]
    fn a_retired_id_taken_again_is_named() {
        let mut d = doc();
        assert_eq!(retired(&d, "forum").get(&4).map(String::as_str), Some("forum.setRoom"));
        let again = [OpDecl { op_id: 4, name: "forum.again", authority: Authority::AnyMember, commutativity: Commutativity::Commutative }];
        assert_eq!(retired_reuse(&d, "forum", &again), vec!["forum.again takes 4, retired from forum.setRoom"]);
        d["kinds"]["forum"]["ops"]["forum.post"]["op"] = 4.into();
        assert_eq!(retired_reuse(&d, "forum", &[]), vec!["forum.post takes 4, retired from forum.setRoom"]);
    }

    /// The set of channels and the set of anchored kinds are THE SAME SET.
    ///
    /// Both directions, because both have failed: a kind with an op table and no
    /// channel (System, until today) drifts unwatched, and a channel with no kind
    /// behind it documents ops nothing declares.
    #[test]
    fn every_built_kind_has_a_channel() {
        let d = doc();
        let channels: BTreeSet<&str> = d["kinds"]
            .as_object()
            .expect("the model has kinds")
            .keys()
            .map(|s| s.as_str())
            .collect();

        let mut anchored: BTreeSet<&str> = BTreeSet::new();
        for &kind in ObjectKind::ALL {
            match anchor(kind) {
                Ok((channel, _)) => {
                    assert!(
                        channels.contains(channel),
                        "`{}` declares an op table but the model has no `{channel}` kind. \
                         Add it, with a relation per reference arg saying what it does to the \
                         graph (or why it is excluded).",
                        kind.name(),
                    );
                    anchored.insert(channel);
                }
                Err(why) => {
                    assert!(
                        why.len() > 24,
                        "`{}` is excluded from the ICD with a reason too short to weigh",
                        kind.name(),
                    );
                    assert!(
                        !channels.contains(kind.name()),
                        "the ICD now has a `{}` channel, but `anchor` still excludes the \
                         kind: \"{why}\". Give it an op table and wire it up, or drop the \
                         channel — a documented op with no `ObjectType` behind it is an op \
                         nobody can author.",
                        kind.name(),
                    );
                }
            }
        }

        assert_eq!(
            channels, anchored,
            "a channel was added to the ICD without a conformance check in \
             icd_matches_every_op_table — every channel must be anchored to an op table"
        );
    }

    /// The graph clause is the whole point of the document, so no op may be silent
    /// about it: it either writes node fields, mints edges, or says why it is excluded.
    /// THE MLS POLICY IS MODEL TOO (membership-through-mls.md §13). The context
    /// extension ids, the commit-rules table, the protocol version, the self-update
    /// interval, and the two options every commit carries are stated ONCE, in the
    /// ICD's `info.x-mls`, and the code is held to them here in both directions — a
    /// rule changed in `mls.rs` without the document, or the document without the
    /// code, fails the build.
    #[test]
    fn the_mls_section_matches_the_code() {
        use crate::mls;
        let d = doc();
        let x = &d["mls"];
        assert!(x.is_object(), "the model must carry an `mls` section");

        let ext = |name: &str| x["groupContextExtensions"][name]["type"].as_u64().unwrap_or_else(|| {
            panic!("mls.groupContextExtensions.{name}.type must be an integer")
        }) as u16;
        assert_eq!(ext("name"), mls::GROUP_NAME_EXT.raw_value(), "name extension id");
        assert_eq!(ext("owner"), mls::OWNER_EXT.raw_value(), "owner extension id");
        assert_eq!(ext("floor"), mls::MIN_VERSION_EXT.raw_value(), "floor extension id");

        // THE PAYLOAD A DELTA TRAVELS IN (A-10 part 1): each key, as the code names it.
        let p = &x["applicationPayload"]["cbor"];
        for (k, name) in [
            (crate::delta_sig::PAYLOAD_ENVELOPE, "envelope"),
            (crate::delta_sig::PAYLOAD_AUTHOR, "author"),
            (crate::delta_sig::PAYLOAD_SIG, "sig"),
        ] {
            let v = p[k.to_string()]
                .as_str()
                .unwrap_or_else(|| panic!("mls.applicationPayload.cbor.{k} is missing"));
            assert!(v.starts_with(name), "mls.applicationPayload.cbor.{k} is {v:?}; the code's is {name}");
        }

        let doc_rules: BTreeMap<String, String> = x["commitRules"]
            .as_object()
            .expect("info.x-mls.commitRules is an object")
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().expect("a rule id is a string").to_string()))
            .collect();
        let code_rules: BTreeMap<String, String> = mls::COMMIT_RULES
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(doc_rules, code_rules, "the commit rules: the ICD is the anchor, mls.rs conforms");

        assert_eq!(
            x["protocolVersion"].as_u64(),
            Some(mls::PROTOCOL_VERSION as u64),
            "protocol version"
        );
        // No routine self-update (19 Sep ruling), so no interval: a document that
        // declares one is stating a behaviour the code does not have.
        assert!(
            x.get("selfUpdateIntervalSecs").is_none(),
            "the ICD declares a self-update interval; there is no routine self-update"
        );

        // The options every commit carries, read from the rules themselves.
        use mls_rs::MlsRules;
        let crypto = mls::crypto();
        let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
        let client = crate::mls_mem::build_client_mem(
            crate::mls_mem::MemGroupStateStorage::new(),
            crate::mls_mem::MemKeyPackageStorage::new(),
            mls::signing_identity(&[7u8; 32], pk.as_bytes()),
            sk,
        )
        .unwrap();
        let g = mls::create_group_named(&client, "icd").unwrap();
        let opts = mls::PacificRules
            .commit_options(&g.roster(), g.context(), &mls_rs::mls_rules::ProposalBundle::default())
            .unwrap();
        assert_eq!(x["pathRequired"].as_bool(), Some(opts.path_required), "pathRequired");
        let enc = mls::PacificRules.encryption_options(&g.roster(), g.context()).unwrap();
        assert_eq!(
            x["handshakeWireFormat"].as_str(),
            Some(if enc.encrypt_control_messages { "PrivateMessage" } else { "PublicMessage" }),
            "handshakeWireFormat"
        );
    }

    /// A REFERENCE ARG DECLARES WHAT IT MEANS.
    ///
    /// This used to assert that every message carried an `x-graph` clause — six
    /// fields per op, of which `edges` restated what the args already said,
    /// `exclude` (29 ops) said "nothing to project" and `status` was a claim about
    /// a different program. The projection is derived now: an arg that names
    /// another object carries its `rel`, and an op with no such arg projects
    /// nothing without having to say so.
    ///
    /// What is left to check is that no relation is invented: every `rel` an arg
    /// names must be one the vocabulary declares.
    #[test]
    fn every_relation_an_arg_names_is_declared() {
        let d = doc();
        let known: BTreeSet<&str> = d["relations"]
            .as_object()
            .expect("the model declares its relations")
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert!(!known.is_empty(), "the relation vocabulary is empty");

        let mut seen = 0usize;
        let mut check_op = |name: &str, m: &Value| {
            for (arg, spec) in m["args"].as_object().into_iter().flatten() {
                for r in spec["rel"].as_array().into_iter().flatten() {
                    let rel = r["name"].as_str().expect("rel.name");
                    assert!(
                        known.contains(rel),
                        "`{name}`.{arg} names the relation `{rel}`, which the \
                         vocabulary does not declare"
                    );
                    seen += 1;
                }
            }
            // An edge that could not be expressed on an arg — an endpoint resolved
            // from the fold, or from the ego — stays explicit, and is held to the
            // same vocabulary.
            for e in m["edges"].as_array().into_iter().flatten() {
                let rel = e["rel"].as_str().expect("edge.rel");
                assert!(known.contains(rel), "`{name}` names an undeclared relation `{rel}`");
                for side in ["from", "to"] {
                    let v = e[side].as_str().expect("edge endpoint");
                    assert!(
                        v == "ego" || v == "target" || v.starts_with("arg:") || v.starts_with("fold:"),
                        "`{name}`.{side} is `{v}` — an endpoint is ego, target, an arg or a fold"
                    );
                }
                seen += 1;
            }
        };
        for (_k, kind) in d["kinds"].as_object().expect("kinds") {
            for (name, m) in kind["ops"].as_object().expect("kind.ops") {
                check_op(name, m);
            }
        }
        for (_f, facet) in d["facets"].as_object().expect("facets") {
            for (name, m) in facet["ops"].as_object().expect("facet.ops") {
                check_op(name, m);
            }
        }
        assert!(seen > 0, "no op projects anything — the derivation lost the edges");
    }

    // ---- THE ARGS ARE THE ARGS A REDUCER READS (NC-9, O-12) ----

    /// An arg as the ICD declares it: its type, and the values the ICD names for it (its
    /// vocabulary, and the words its own and its op's summaries give).
    struct ArgSpec {
        ty: String,
        named: Vec<String>,
    }

    /// The words a summary offers as values: `a | b`, `{a,b}`, `a/b/c`, `` `a` ``, and a
    /// MIME type whole.
    fn words(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for w in text.split(|c: char| !(c.is_ascii_alphanumeric() || "/_-.+".contains(c))) {
            let w = w.trim_matches(|c: char| ".-".contains(c));
            if w.is_empty() || w.len() > 40 {
                continue;
            }
            out.push(w.to_string());
            if w.contains('/') {
                out.extend(w.split('/').filter(|p| !p.is_empty()).map(str::to_string));
            }
        }
        let lower: Vec<String> = out.iter().map(|w| w.to_ascii_lowercase()).collect();
        out.extend(lower);
        out.dedup();
        out
    }

    /// A value an arg's declared `pattern` admits, for the two shapes the ICD declares:
    /// `^[0-9a-f]{N}$` (a client-chosen id) and `^[A-Z]{N}$` (a currency). Any other pattern
    /// offers nothing, and the arg is probed with the rest.
    fn pattern_sample(p: &str) -> Option<String> {
        let (class, n) = p.strip_prefix("^[")?.strip_suffix("}$")?.split_once("]{")?;
        let n: usize = n.parse().ok()?;
        let alphabet = match class {
            "0-9a-f" => "0123456789abcdef",
            "A-Z" => "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            _ => return None,
        };
        Some(alphabet.chars().cycle().take(n).collect())
    }

    #[test]
    fn a_declared_pattern_offers_a_value_it_admits() {
        assert_eq!(pattern_sample("^[0-9a-f]{16}$").as_deref(), Some("0123456789abcdef"));
        assert_eq!(pattern_sample("^[A-Z]{3}$").as_deref(), Some("ABC"));
        assert_eq!(pattern_sample("^[0-9a-f]{64}$").map(|s| s.len()), Some(64));
        assert_eq!(pattern_sample("^.+$"), None);
    }

    /// Each op a kind carries, its own or a facet's, with its args as the ICD declares them.
    fn kind_args(doc: &Value, kind: &str) -> BTreeMap<String, BTreeMap<String, ArgSpec>> {
        let mut out = BTreeMap::new();
        let mut take = |name: &String, m: &Value| {
            let op_words = words(m["summary"].as_str().unwrap_or(""));
            let args = m["args"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(a, spec)| {
                    let mut named: Vec<String> = spec["vocabulary"].as_object().map(|v| v.keys().cloned().collect()).unwrap_or_default();
                    named.extend(spec["pattern"].as_str().and_then(pattern_sample));
                    named.extend(words(spec["summary"].as_str().unwrap_or("")));
                    named.extend(op_words.iter().cloned());
                    (a.clone(), ArgSpec { ty: spec["type"].as_str().expect("an arg's type").to_string(), named })
                })
                .collect();
            out.insert(name.clone(), args);
        };
        for (name, m) in doc["kinds"][kind]["ops"].as_object().expect("kinds.<kind>.ops is a map") {
            take(name, m);
        }
        for (_facet, f) in doc["facets"].as_object().expect("facets is a map") {
            if f["on"].as_array().expect("facet.on").iter().any(|c| c.as_str() == Some(kind)) {
                for (name, m) in f["ops"].as_object().expect("facet.ops is a map") {
                    take(name, m);
                }
            }
        }
        out
    }

    /// The words the reducers' own matches accept (every `"word" =>` arm in this crate's
    /// source and the media crate's): a value the ICD does not name is still one the code
    /// will fold. Each arg that needs one is also a finding for the revision: the ICD names
    /// no vocabulary for it.
    fn enum_words() -> Vec<String> {
        let mut out = BTreeSet::new();
        for dir in [concat!(env!("CARGO_MANIFEST_DIR"), "/src"), concat!(env!("CARGO_MANIFEST_DIR"), "/../pacific-media/src")] {
            for e in std::fs::read_dir(dir).expect("the source").flatten() {
                let Ok(src) = std::fs::read_to_string(e.path()) else { continue };
                let pieces: Vec<&str> = src.split('"').collect();
                for w in pieces.windows(2) {
                    if w[1].trim_start().starts_with("=>")
                        && !w[0].is_empty()
                        && w[0].len() <= 32
                        && w[0].chars().all(|c| c.is_ascii_alphanumeric() || "_/-.".contains(c))
                    {
                        out.insert(w[0].to_string());
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// JSON documents of the core's own types, for an arg that carries one.
    fn documents() -> Vec<String> {
        let point = crate::geo::GeoPoint::from_degrees(51.5, -0.1);
        let land = crate::place::LandClaim {
            parcel: "EX1".into(),
            proprietor: String::new(),
            company_no: String::new(),
            source: crate::place::LandSource::Inspire,
            as_of: 1,
            permission: Default::default(),
        };
        let core = crate::event::TicketCore {
            v: 1,
            listing: "l1".into(),
            ticket: "t1".into(),
            title: "a show".into(),
            start_ms: 1,
            buyer: hex::encode(PROBE_OTHER),
            issued_at: 1,
        };
        let core = serde_json::to_string(&core).expect("a ticket's core");
        vec![
            serde_json::to_string(&crate::geo::LocationSource::Fixed { point }).expect("a location"),
            serde_json::to_string(&land).expect("a land claim"),
            serde_json::json!([{ "core": core, "sig": "ab".repeat(32) }]).to_string(),
            // A member's card (O-77), as profiles::parse reads one.
            serde_json::to_string(&crate::profiles::Card { display_name: "probe".into(), photo: None, photo_mime: None }).expect("a card"),
            // A treasury's bands, one `ceiling|rule|quorum` a line; a currency (ISO 4217's
            // shape); and base64 as a picture is carried.
            "*|majority|1".into(),
            // A claim's share id, from the claim's own alphabet.
            String::from_utf8(crate::claim::SHARE_ALPHABET[..20].to_vec()).expect("the share alphabet is ASCII"),
            "EUR".into(),
            "image/png".into(),
            "cHJvYmU=".into(),
            // A question's options (base.defineQuestion): 2 to 24 distinct labels.
            serde_json::json!(["probe", "probe 2"]).to_string(),
        ]
    }

    /// The values a probe tries for an arg, in order: what the ICD names for it, then the
    /// shapes a reducer's typed lenses accept (a 64-hex key or id, a word, empty, a JSON list
    /// or object; a positive, zero, negative or millisecond integer). An arg the ICD does not
    /// declare has no type: both kinds are tried.
    fn candidates(spec: Option<&ArgSpec>) -> Vec<crate::coordinator::ArgVal> {
        use crate::coordinator::ArgVal;
        use base64::Engine;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let ints = move || [1, 0, 2, -1, now].into_iter().map(ArgVal::Int);
        let texts = |named: &[String]| {
            // A 64-hex key or id, a roster member's key (the probe's `other`), then what the
            // ICD names, then the plain shapes and a 32-byte key in base64url.
            [hex::encode(PROBE_OTHER), hex::encode(PROBE_OWNER)]
                .into_iter()
                .chain(named.iter().cloned())
                .chain(ENUM_WORDS.with(|w| w.clone()))
                .chain(documents())
                .chain(["x".into(), String::new(), "[]".into(), "{}".into(), "1".into()])
                .chain([base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32])])
                .map(ArgVal::Text)
                .collect::<Vec<_>>()
        };
        match spec {
            Some(ArgSpec { ty, .. }) if ty == "integer" => ints().collect(),
            Some(ArgSpec { ty, named }) if ty == "string" => texts(named),
            Some(ArgSpec { ty, .. }) => panic!("the ICD types an arg `{ty}`: the envelope carries only integer and string"),
            None => texts(&[]).into_iter().chain(ints()).collect(),
        }
    }

    /// A media slot as the media crate writes one (`MediaRef::to_args`), under `prefix`, for
    /// each delivery: inline, detached and live. Each is the args of one sample, mapped onto
    /// the probe's candidates; a sample a candidate list cannot say is skipped.
    fn media_seeds(prefix: &str, spec: &BTreeMap<String, ArgSpec>) -> Vec<BTreeMap<String, usize>> {
        use pacific_media::{Delivery, MediaKind, MediaRef};
        let png = || MediaKind::of_mime("image/png").expect("png is a still");
        let samples = [
            MediaRef::inline("image/png", "cHJvYmU=").expect("an inline picture").with_intrinsics(1, 1, 0),
            MediaRef {
                kind: png(),
                mime: "image/png".into(),
                delivery: Delivery::Detached { digest: [0xab; 32], bytes: 1 },
                width: 1,
                height: 1,
                duration_ms: 0,
            },
            MediaRef::live("x").expect("a live session"),
        ];
        samples
            .iter()
            .filter_map(|m| {
                let mut args = crate::coordinator::Args::new();
                m.to_args(prefix, &mut args);
                args.into_iter()
                    .map(|(k, v)| {
                        let at = candidates(spec.get(&k)).iter().position(|c| *c == v)?;
                        Some((k, at))
                    })
                    .collect()
            })
            .collect()
    }

    /// What probing one op found.
    struct Probed {
        /// Every key its reducer read, in any run.
        read: BTreeSet<String>,
        /// Keys it read only to refuse: no value of them folds, and without them it does.
        refused: BTreeSet<String>,
        /// The values it folded Ok with, if it did.
        folded: Option<BTreeMap<String, usize>>,
        /// The key it stopped at, when no ICD-typed value got it past it.
        stuck: Option<String>,
    }

    /// One op, probed on `state`, by the state's owner, with every arg the ICD declares; the
    /// keys its reducer reads are recorded (`arg_reads`).
    ///
    /// SETTLING. A value the reducer refuses as malformed is replaced: first one key at a time,
    /// the last read first; then two keys judged together, among the three last read, small
    /// values first. A key it reads that the ICD does not declare is supplied too, so what it
    /// reads after it is reached; one no value of which folds, alone or paired, is tried
    /// absent, and if the op gets further without it, it is a key the reducer reads only to
    /// refuse. Progress is a fold, or reading further before being refused.
    ///
    /// EXPLORING, once it folds: each key through its candidates, the rest held. A variation
    /// that reads a set of keys not read before is kept as a base in its turn, so a branch two
    /// values deep (a presence's kind and its key; a media's delivery and its digest) is
    /// reached, and each distinct branch is explored once.
    fn probe_op<T: ObjectType>(
        decl: &crate::object::OpDecl,
        spec: &BTreeMap<String, ArgSpec>,
        state: &T::State,
    ) -> Probed {
        use crate::coordinator::{ArgVal, Args};
        use crate::object::{DeltaRejection, LogPosition, Op, ReduceContext};
        let (owner, other) = (PROBE_OWNER, PROBE_OTHER);
        let members = [owner, other];
        let ctx = ReduceContext { members: &members, owner, epoch: 0 };
        let pos = matches!(decl.commutativity, Commutativity::Sequenced).then_some(LogPosition { epoch: 0, seq: 1 });
        // A ROLE-VALUED OP (`Authority::Roles`, W-98 Trade) is probed as a holder of its first
        // role, which the owner is not by being the owner: the state gains the roles a
        // Transaction's mint writes, through the kind's own reducer (the owner the settler,
        // the other the op's role).
        let (author, state) = match decl.authority {
            crate::object::Authority::Roles(rs) => {
                let mut seeded = state.clone();
                let role = rs[0];
                let mut give = |who: [u8; 32], r: crate::group::GroupRole| {
                    let args = crate::roles::set_role_args(&who, r);
                    let _ = T::reduce(&mut seeded, &Op { op_id: crate::roles::OP_SET_ROLE, args: &args, author: &owner, pos: Some(LogPosition { epoch: 0, seq: 0 }), ctx: &ctx });
                };
                give(owner, crate::group::GroupRole::Settler);
                if role != crate::group::GroupRole::Settler {
                    give(other, role);
                }
                (if role == crate::group::GroupRole::Settler { owner } else { other }, seeded)
            }
            _ => (owner, state.clone()),
        };
        let state = &state;
        let debug = std::env::var("ICD_PROBE_DEBUG").is_ok_and(|op| op == decl.name);
        let args_of = |pick: &BTreeMap<String, usize>| -> Args {
            let mut args: Args = pick.iter().map(|(k, i)| (k.clone(), candidates(spec.get(k))[*i].clone())).collect();
            // A claim issuer's `kid` is its key's (the code's own `kid_of`).
            if let (Some(ArgVal::Text(key)), true) = (args.get("key").cloned(), args.contains_key("kid")) {
                use base64::Engine;
                if let Ok(k) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key) {
                    if let Ok(k) = <[u8; 32]>::try_from(k.as_slice()) {
                        args.insert("kid".into(), ArgVal::Text(crate::claim::kid_of(&k)));
                    }
                }
            }
            args
        };
        let run = |pick: &BTreeMap<String, usize>| -> (Result<(), DeltaRejection>, Vec<String>) {
            let args = args_of(pick);
            let out = crate::arg_reads::recording(|| {
                T::reduce(&mut state.clone(), &Op { op_id: decl.op_id, args: &args, author: &author, pos, ctx: &ctx })
            });
            if debug {
                eprintln!("{}: {:?} read {:?} with {args:?}", decl.name, out.0, out.1);
            }
            out
        };
        let reach = |order: &[String]| order.iter().collect::<BTreeSet<_>>().len();
        let mut pick: BTreeMap<String, usize> = spec.keys().map(|k| (k.clone(), 0)).collect();
        let mut refused: BTreeSet<String> = BTreeSet::new();
        let mut read: BTreeSet<String> = BTreeSet::new();
        let (mut folded, mut settled, mut stuck) = (None, false, None);
        for _ in 0..10_000 {
            let (got, order) = run(&pick);
            read.extend(order.iter().cloned());
            if got == Ok(()) && folded.is_none() {
                folded = Some(pick.clone());
            }
            if let Some(k) = order.iter().find(|k| !pick.contains_key(*k) && !refused.contains(*k)) {
                pick.insert(k.clone(), 0);
                continue;
            }
            if got != Err(DeltaRejection::MalformedArgs) {
                settled = true;
                break;
            }
            let now = reach(&order);
            let better = |p: &BTreeMap<String, usize>| {
                let (g, o) = run(p);
                g != Err(DeltaRejection::MalformedArgs) || reach(&o) > now || o.iter().any(|k| !p.contains_key(k) && !refused.contains(k))
            };
            let mut seen = BTreeSet::new();
            let turned: Vec<String> = order.iter().rev().filter(|k| pick.contains_key(*k) && seen.insert((*k).clone())).cloned().collect();
            // One key at a time, the last read first.
            let one = turned.iter().find_map(|k| {
                (0..candidates(spec.get(k)).len()).find_map(|i| {
                    let mut p = pick.clone();
                    p.insert(k.clone(), i);
                    better(&p).then_some(p)
                })
            });
            if let Some(p) = one {
                pick = p;
                continue;
            }
            // Two keys judged together: the three last read, pairwise, small values first.
            let near: Vec<&String> = turned.iter().take(3).collect();
            let mut two = None;
            'pairs: for (x, a) in near.iter().enumerate() {
                for b in near.iter().skip(x + 1) {
                    let (na, nb) = (candidates(spec.get(*a)).len(), candidates(spec.get(*b)).len());
                    for sum in 0..(na + nb - 1) {
                        for i in sum.saturating_sub(nb - 1)..=sum.min(na - 1) {
                            let mut p = pick.clone();
                            p.insert((*a).clone(), i);
                            p.insert((*b).clone(), sum - i);
                            if better(&p) {
                                two = Some(p);
                                break 'pairs;
                            }
                        }
                    }
                }
            }
            if let Some(p) = two {
                pick = p;
                continue;
            }
            // An undeclared key no value of which helps: tried absent.
            let absent = turned.iter().filter(|k| !spec.contains_key(*k)).find_map(|k| {
                let mut p = pick.clone();
                p.remove(k);
                let (g, o) = run(&p);
                (g != Err(DeltaRejection::MalformedArgs) || reach(&o) > now).then(|| (p, k.clone()))
            });
            match absent {
                Some((p, k)) => {
                    pick = p;
                    refused.insert(k);
                }
                None => {
                    stuck = turned.first().cloned();
                    break;
                }
            }
        }
        if settled {
            let mut bases: Vec<BTreeMap<String, usize>> = vec![pick.clone()];
            let mut branches: BTreeSet<BTreeSet<String>> = BTreeSet::new();
            branches.insert(read.clone());
            while let Some(base) = bases.pop() {
                // A media slot, filled as the media crate fills one, wherever the reducer read
                // a key and its `…Mime` together.
                let (_, base_order) = run(&base);
                let media: Vec<String> = base_order.iter().filter(|k| base_order.contains(&format!("{k}Mime"))).cloned().collect();
                for prefix in media {
                    for seed in media_seeds(&prefix, spec) {
                        let mut p = base.clone();
                        p.extend(seed);
                        let (got, order) = run(&p);
                        read.extend(order.iter().cloned());
                        if got == Ok(()) && folded.is_none() {
                            folded = Some(p.clone());
                        }
                        if branches.insert(order.iter().cloned().collect()) {
                            bases.push(p);
                        }
                    }
                }
                for k in base.keys() {
                    for i in 0..candidates(spec.get(k)).len() {
                        let mut p = base.clone();
                        p.insert(k.clone(), i);
                        let (got, order) = run(&p);
                        read.extend(order.iter().cloned());
                        if got == Ok(()) && folded.is_none() {
                            folded = Some(p.clone());
                        }
                        let branch: BTreeSet<String> = order.iter().cloned().collect();
                        if branches.insert(branch) {
                            for r in &order {
                                if !p.contains_key(r) && !refused.contains(r) {
                                    p.insert(r.clone(), 0);
                                }
                            }
                            bases.push(p);
                        }
                    }
                }
            }
        }
        Probed { read, refused, folded, stuck }
    }

    thread_local! {
        static ENUM_WORDS: Vec<String> = enum_words();
    }

    const PROBE_OWNER: [u8; 32] = [0x0a; 32];
    const PROBE_OTHER: [u8; 32] = [0x0b; 32];

    /// One kind's ops, probed twice: on a fresh state, then, for an op that did not fold, on
    /// the state the others left (a sale needs its listing first). Then both directions:
    /// every key a reducer reads is declared, and every arg declared for an op that folded is
    /// read, or marked `unread`. An op that never folds is named, not passed.
    fn reads<T: ObjectType>(doc: &Value, channel: &str, wrong: &mut Vec<String>, unprobed: &mut Vec<String>) {
        use crate::coordinator::Args;
        use crate::object::{Op, ReduceContext};
        let declared = kind_args(doc, channel);
        let marked_unread = |op: &str, arg: &str| {
            let op_doc = doc["kinds"][channel]["ops"].get(op).cloned().or_else(|| {
                doc["facets"].as_object().and_then(|f| f.values().find_map(|f| f["ops"].get(op).cloned()))
            });
            op_doc.is_some_and(|o| o["args"][arg]["unread"].as_bool() == Some(true))
        };
        let mut world = T::State::default();
        // AN EVENT THE SITE CREATED (W-98): group.rsvp, setRegistration and rsvpDecide are
        // refused unless their `event` is one, so the world holds the edge the Site's mint of an
        // Event writes (group.setAffiliation, rel=created), through the kind's own reducer, at
        // the probe owner's id, which the probe's own setAffiliation (the other's) leaves alone.
        if let Some(aff) = T::ops().iter().find(|d| d.name == "group.setAffiliation") {
            use crate::coordinator::ArgVal;
            let members = [PROBE_OWNER, PROBE_OTHER];
            let ctx = crate::object::ReduceContext { members: &members, owner: PROBE_OWNER, epoch: 0 };
            let args: Args = [
                ("peer", ArgVal::Text(hex::encode(PROBE_OWNER))),
                ("rel", ArgVal::Text("created".into())),
                ("name", ArgVal::Text("probe".into())),
                ("at", ArgVal::Int(1)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
            let pos = Some(crate::object::LogPosition { epoch: 0, seq: 0 });
            T::reduce(&mut world, &Op { op_id: aff.op_id, args: &args, author: &PROBE_OWNER, pos, ctx: &ctx })
                .expect("the probe's created edge folds");
        }
        let mut found: BTreeMap<&'static str, Probed> = BTreeMap::new();
        for pass in 0..2 {
            for decl in T::ops() {
                let Some(spec) = declared.get(decl.name) else { continue };
                if found.get(decl.name).is_some_and(|p| p.folded.is_some()) {
                    continue;
                }
                let start = if pass == 0 { T::State::default() } else { world.clone() };
                let p = probe_op::<T>(decl, spec, &start);
                if let Some(pick) = &p.folded {
                    // The world moves on by what folded, so a later op can find it.
                    let args: Args = pick.iter().map(|(k, i)| (k.clone(), candidates(spec.get(k))[*i].clone())).collect();
                    let members = [PROBE_OWNER, PROBE_OTHER];
                    let ctx = ReduceContext { members: &members, owner: PROBE_OWNER, epoch: 0 };
                    let pos = matches!(decl.commutativity, Commutativity::Sequenced)
                        .then_some(crate::object::LogPosition { epoch: 0, seq: 1 });
                    let _ = T::reduce(&mut world, &Op { op_id: decl.op_id, args: &args, author: &PROBE_OWNER, pos, ctx: &ctx });
                }
                match found.get_mut(decl.name) {
                    Some(prev) => {
                        prev.read.extend(p.read);
                        prev.refused.extend(p.refused);
                        prev.folded = prev.folded.take().or(p.folded);
                        prev.stuck = p.stuck;
                    }
                    None => {
                        found.insert(decl.name, p);
                    }
                }
            }
        }
        for decl in T::ops() {
            let (Some(spec), Some(p)) = (declared.get(decl.name), found.get(decl.name)) else { continue };
            for k in &p.read {
                if !spec.contains_key(k) && !p.refused.contains(k) {
                    wrong.push(format!("{channel}: {} reads `{k}`, which the ICD does not declare", decl.name));
                }
            }
            for k in &p.refused {
                if spec.contains_key(k) {
                    wrong.push(format!("{channel}: {} declares `{k}`, which its reducer only refuses", decl.name));
                }
            }
            if p.folded.is_some() {
                for k in spec.keys() {
                    let read = p.read.contains(k);
                    if !read && !marked_unread(decl.name, k) {
                        wrong.push(format!("{channel}: {} declares `{k}`, which its reducer never reads", decl.name));
                    }
                    if read && marked_unread(decl.name, k) {
                        wrong.push(format!("{channel}: {} marks `{k}` unread, and its reducer reads it", decl.name));
                    }
                }
            } else {
                unprobed.push(format!("{channel}: {} (stopped at `{}`)", decl.name, p.stuck.clone().unwrap_or_default()));
            }
        }
    }

    /// The base RATIFY ops fold in `Coordinator::ratify_state`, not in any `T::reduce`, so
    /// `reads` never meets them. Each is delivered after what it needs, as the builders
    /// write it; its reads are what that fold reads beyond its prerequisite's.
    fn ratify_reads(doc: &Value, wrong: &mut Vec<String>) {
        use crate::coordinator::{ratify_close_frozen, ratify_propose, ratify_vote, Ballot, Coordinator, Delta, Rule, GENESIS_PREV};
        let tid = u32::from(ObjectKind::Group.type_id());
        let members = vec![PROBE_OWNER, PROBE_OTHER];
        let pid = (PROBE_OWNER, 1);
        let propose = || (ratify_propose("probe", Rule::Consent, 1, 0, tid), PROBE_OWNER);
        let vote = || (ratify_vote(pid, Ballot::Approve, 2, 0, tid), PROBE_OTHER);
        let close = (ratify_close_frozen(pid, &members, &[vote().0.id()], 0, 0, GENESIS_PREV, tid), PROBE_OWNER);
        let read = |ds: Vec<(Delta, crate::object::MemberId)>| -> BTreeSet<String> {
            let mut c = Coordinator::<crate::group::GroupType>::new(members.clone(), PROBE_OWNER);
            for (d, a) in ds {
                c.deliver(d, a).expect("a ratify delta the builders write delivers");
            }
            crate::arg_reads::recording(|| c.ratify_state()).1.into_iter().collect()
        };
        let before = read(vec![propose()]);
        for (name, got) in [
            ("ratify.propose", before.clone()),
            ("ratify.vote", read(vec![vote()])),
            ("ratify.close", read(vec![propose(), close]).difference(&before).cloned().collect()),
        ] {
            let spec = doc["facets"]["ratify"]["ops"][name]["args"].as_object().cloned().unwrap_or_default();
            let unread = |k: &str| spec[k]["unread"].as_bool() == Some(true);
            for k in &got {
                if !spec.contains_key(k) {
                    wrong.push(format!("ratify: {name} reads `{k}`, which the ICD does not declare"));
                }
            }
            for k in spec.keys() {
                match (got.contains(k), unread(k)) {
                    (false, false) => wrong.push(format!("ratify: {name} declares `{k}`, which its fold never reads")),
                    (true, true) => wrong.push(format!("ratify: {name} marks `{k}` unread, and its fold reads it")),
                    _ => {}
                }
            }
        }
    }

    /// The kinds, as `anchor` names them: a kind with a reducer has its reads probed.
    fn reader(kind: ObjectKind) -> Option<(&'static str, fn(&Value, &str, &mut Vec<String>, &mut Vec<String>))> {
        match kind {
            ObjectKind::Group => Some(("group", reads::<crate::group::GroupType>)),
            ObjectKind::Forum => Some(("forum", reads::<crate::coordinator::ForumType>)),
            ObjectKind::System => Some(("system", reads::<crate::system::SystemType>)),
            ObjectKind::Project => Some(("project", reads::<crate::project::ProjectType>)),
            ObjectKind::Contact => Some(("contact", reads::<crate::contact::ContactType>)),
            ObjectKind::Conversation => Some(("conversation", reads::<crate::coordinator::ConversationType>)),
            ObjectKind::Thing => Some(("thing", reads::<crate::thing::ThingType>)),
            ObjectKind::Place => Some(("place", reads::<crate::place::PlaceType>)),
            ObjectKind::Event => Some(("event", reads::<crate::event::EventType>)),
            ObjectKind::Post => Some(("post", reads::<crate::post::PostType>)),
            ObjectKind::Treasury => Some(("treasury", reads::<crate::treasury::TreasuryType>)),
            ObjectKind::Note => Some(("note", reads::<crate::note::NoteType>)),
            ObjectKind::Host => Some(("host", reads::<crate::host::HostType>)),
            ObjectKind::Transaction => Some(("transaction", reads::<crate::transaction::TransactionType>)),
            // As `anchor` says: no `impl ObjectType`, no reducer, nothing to read.
            ObjectKind::Field | ObjectKind::Topic => None,
        }
    }

    /// NC-9, O-12: THE ICD'S ARGS ARE THE ARGS A REDUCER READS, both ways, on every kind.
    /// An arg a reducer reads that the ICD does not declare is one no ICD-conformant writer
    /// can send; an arg the ICD declares that no reducer reads is one a writer sends for
    /// nothing. Either way the ICD is not the model, and the ICD is the model.
    #[test]
    fn icd_args_are_the_args_a_reducer_reads() {
        let d = doc();
        // The kinds are independent, and the read point records per thread: one thread each.
        let found: Vec<(Vec<String>, Vec<String>)> = std::thread::scope(|scope| {
            let d = &d;
            let each: Vec<_> = ObjectKind::ALL
                .iter()
                .filter_map(|&kind| reader(kind))
                .map(|(channel, probe)| {
                    scope.spawn(move || {
                        let (mut wrong, mut unprobed) = (Vec::new(), Vec::new());
                        probe(d, channel, &mut wrong, &mut unprobed);
                        (wrong, unprobed)
                    })
                })
                .collect();
            each.into_iter().map(|h| h.join().expect("a kind's probe")).collect()
        });
        let (mut wrong, mut unprobed) = (Vec::new(), Vec::new());
        ratify_reads(&d, &mut wrong);
        for (w, u) in found {
            wrong.extend(w);
            unprobed.extend(u);
        }
        // An op no ICD-typed value folds, even on the state the others left, cannot be held
        // to its declared args here; each is named, and they are few. Another is a probe
        // gone blind (base.setPart was, while `choice` was read around the read point).
        if !unprobed.is_empty() {
            eprintln!("{} op(s) never folded, so their declared args are not held to reads:\n{}", unprobed.len(), unprobed.join("\n"));
        }
        const NEVER_FOLD: [&str; 5] = [
            "group: group.clearClaimIssuer",
            "project: project.addDependency",
            "project: project.removeDependency",
            "project: project.touchSubscription",
            "event: event.recordSale",
        ];
        for u in unprobed.iter().filter(|u| !NEVER_FOLD.iter().any(|n| u.starts_with(&format!("{n} (")))) {
            wrong.push(format!("no longer probed: {u}"));
        }
        assert!(wrong.is_empty(), "{} disagreement(s) between the ICD's args and the reducers':\n{}", wrong.len(), wrong.join("\n"));
    }
}
