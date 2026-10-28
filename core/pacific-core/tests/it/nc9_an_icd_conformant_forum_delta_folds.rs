//! NC-9: a delta built from the ICD's own args must fold. The ICD
//! (`coordination/delta-graph.icd.json`) is the model; a writer that conforms to it and is
//! refused `MalformedArgs` by the reducer is the drift NC-9 names.
//!
//! Each delta below carries exactly the args the ICD declares for its op, every one of
//! them, and nothing else. The values are the fixture's; the names are the ICD's.

use pacific_core::coordinator::{ArgVal, Args, ForumState, ForumType, FORUM_POST, FORUM_REACT};
use pacific_core::object::{ObjectType, Op, ReduceContext};
use serde_json::Value;

const ADA: [u8; 32] = [0xad; 32];
const BO: [u8; 32] = [0xb0; 32];
/// Ada's first post, the one reacted to and replied to.
const FIRST_GEN: i64 = 1;

fn icd() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD")).expect("the ICD is JSON")
}

/// The reply's picture, as the media crate writes one: an inline PNG.
fn picture() -> Args {
    let mut a = Args::new();
    pacific_media::MediaRef::inline("image/png", "cHJvYmU=").expect("a picture").with_intrinsics(1, 1, 0).to_args("media", &mut a);
    a
}

/// A value for each arg the ICD declares, by name: the fixture knows what each name means
/// in this story. A name it does not know is a new arg, and the fixture must learn it.
fn sample(name: &str) -> ArgVal {
    if let Some(v) = picture().get(name) {
        return v.clone();
    }
    match name {
        // The media keys an inline picture does not write: another delivery's, sent anyway.
        "mediaDigest" => ArgVal::Text("ab".repeat(32)),
        "mediaBytes" => ArgVal::Int(1),
        "mediaSession" => ArgVal::Text("a session".into()),
        "mediaMs" => ArgVal::Int(0),
        // O-79: a sealed ref's object key and content key.
        "mediaKey" => ArgVal::Text("blobs/ab12".into()),
        "mediaSecret" => ArgVal::Text("c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0".into()),
        "text" => ArgVal::Text("the reply".into()),
        "gen" => ArgVal::Int(2),
        "ts" => ArgVal::Int(1_790_000_000_000),
        "reply_author" | "target_author" => ArgVal::Text(hex::encode(ADA)),
        "reply_gen" | "target_gen" => ArgVal::Int(FIRST_GEN),
        // The ICD's former name for the reacted message: Ada's post, as its author.
        "target" => ArgVal::Text(hex::encode(ADA)),
        "emoji" => ArgVal::Text("👍".into()),
        "active" => ArgVal::Int(1),
        other => panic!("the ICD declares `{other}`, which this fixture does not know: teach it what the arg means"),
    }
}

/// Every arg the ICD declares for `forum.<op>`, with the fixture's value.
fn icd_args(op: &str) -> Args {
    let doc = icd();
    let spec = &doc["kinds"]["forum"]["ops"][op]["args"];
    spec.as_object()
        .unwrap_or_else(|| panic!("the ICD declares no args for {op}"))
        .keys()
        .map(|name| (name.clone(), sample(name)))
        .collect()
}

fn fold(state: &mut ForumState, op_id: u32, args: &Args, author: &[u8; 32]) -> Result<(), pacific_core::object::DeltaRejection> {
    let members = [ADA, BO];
    let ctx = ReduceContext { members: &members, owner: ADA, epoch: 0 };
    ForumType::reduce(state, &Op { op_id, args, author, pos: None, ctx: &ctx })
}

/// Ada's first post, as the reducer reads it: the story's premise, not the claim.
fn with_ada_first_post() -> ForumState {
    let mut state = ForumState::default();
    let args: Args = [("text".to_string(), ArgVal::Text("first".into())), ("gen".to_string(), ArgVal::Int(FIRST_GEN))].into();
    fold(&mut state, FORUM_POST, &args, &ADA).expect("the premise: Ada's first post folds");
    state
}

#[test]
fn nc9_an_icd_conformant_forum_react_folds() {
    let mut state = with_ada_first_post();
    let args = icd_args("forum.react");
    let got = fold(&mut state, FORUM_REACT, &args, &BO);
    assert_eq!(got, Ok(()), "bo's react, built from the ICD's args {:?}, is refused", args.keys().collect::<Vec<_>>());
    let on_first = state.reactions.get(&(ADA, FIRST_GEN as u64)).and_then(|r| r.get(&BO));
    assert_eq!(on_first.map(|(e, _)| e.as_str()), Some("👍"), "and lands on Ada's first post");
}

#[test]
fn nc9_an_icd_conformant_reply_post_folds_as_a_reply() {
    let mut state = with_ada_first_post();
    let args = icd_args("forum.post");
    let got = fold(&mut state, FORUM_POST, &args, &BO);
    assert_eq!(got, Ok(()), "bo's post, built from the ICD's args {:?}, is refused", args.keys().collect::<Vec<_>>());
    let reply = state.messages.values().find(|m| m.text == "the reply").expect("bo's post is held");
    assert_eq!(reply.reply_to, Some((ADA, FIRST_GEN as u64)), "and replies to Ada's first post");
    assert_eq!(reply.media.as_ref().map(|m| m.mime.as_str()), Some("image/png"), "with its picture");
}
