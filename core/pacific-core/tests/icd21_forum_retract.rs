//! ICD 2.1.0 row 9: `forum.retract {target_author, target_gen}`. A message's author withdraws
//! their own; the room's owner hides any; the fold refuses anyone else. Grow-only: retracted
//! stays retracted, once or twice, whether the retract folds before the post or after.
//!
//! The op id and its args are the ICD's (`coordination/delta-graph.icd.json`).

use pacific_core::coordinator::{ArgVal, Args, ForumState, ForumType, FORUM_POST};
use pacific_core::object::{DeltaRejection, ObjectType, Op, ReduceContext};
use serde_json::Value;

/// The room's owner.
const ADA: [u8; 32] = [0xad; 32];
const BO: [u8; 32] = [0xb0; 32];
const CY: [u8; 32] = [0xc4; 32];

fn icd() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD")).expect("the ICD is JSON")
}

fn retract_op() -> u32 {
    icd()["kinds"]["forum"]["ops"]["forum.retract"]["op"].as_u64().expect("the ICD declares forum.retract") as u32
}

fn fold(state: &mut ForumState, op_id: u32, args: &Args, author: &[u8; 32]) -> Result<(), DeltaRejection> {
    let members = [ADA, BO, CY];
    let ctx = ReduceContext { members: &members, owner: ADA, epoch: 0 };
    ForumType::reduce(state, &Op { op_id, args, author, pos: None, ctx: &ctx })
}

fn post(state: &mut ForumState, author: &[u8; 32], gen: i64, text: &str) {
    let args: Args = [("text".to_string(), ArgVal::Text(text.into())), ("gen".to_string(), ArgVal::Int(gen))].into();
    fold(state, FORUM_POST, &args, author).expect("a post folds");
}

/// Every arg the ICD declares for forum.retract, naming `(author, gen)`; `gen` is the
/// retract's own.
fn retract(state: &mut ForumState, by: &[u8; 32], target: (&[u8; 32], i64), gen: i64) -> Result<(), DeltaRejection> {
    let doc = icd();
    let args: Args = doc["kinds"]["forum"]["ops"]["forum.retract"]["args"]
        .as_object()
        .expect("forum.retract declares args")
        .keys()
        .map(|k| {
            let v = match k.as_str() {
                "target_author" => ArgVal::Text(hex::encode(target.0)),
                "target_gen" => ArgVal::Int(target.1),
                "gen" => ArgVal::Int(gen),
                other => panic!("the ICD declares `{other}` on forum.retract; teach this test what it means"),
            };
            (k.clone(), v)
        })
        .collect();
    fold(state, retract_op(), &args, by)
}

fn texts(state: &ForumState) -> Vec<String> {
    state.detailed().into_iter().map(|m| m.text).collect()
}

fn threaded(state: &ForumState) -> Vec<String> {
    state.thread().into_iter().map(|n| n.msg.text).collect()
}

#[test]
fn an_author_withdraws_their_own_message() {
    let mut s = ForumState::default();
    post(&mut s, &BO, 1, "bo's");
    post(&mut s, &CY, 1, "cy's");
    assert_eq!(retract(&mut s, &BO, (&BO, 1), 2), Ok(()));
    assert_eq!(texts(&s), ["cy's"], "gone from the view");
    assert_eq!(threaded(&s), ["cy's"], "and from the thread");
}

#[test]
fn the_owner_hides_anyones() {
    let mut s = ForumState::default();
    post(&mut s, &BO, 1, "bo's");
    assert_eq!(retract(&mut s, &ADA, (&BO, 1), 1), Ok(()));
    assert!(texts(&s).is_empty());
}

#[test]
fn anyone_else_is_refused() {
    let mut s = ForumState::default();
    post(&mut s, &BO, 1, "bo's");
    assert_eq!(retract(&mut s, &CY, (&BO, 1), 2), Err(DeltaRejection::Unauthorized));
    assert_eq!(texts(&s), ["bo's"]);
}

#[test]
fn twice_is_once() {
    let mut once = ForumState::default();
    post(&mut once, &BO, 1, "bo's");
    retract(&mut once, &BO, (&BO, 1), 2).unwrap();
    let mut twice = once.clone();
    assert_eq!(retract(&mut twice, &ADA, (&BO, 1), 1), Ok(()));
    assert_eq!(retract(&mut twice, &BO, (&BO, 1), 3), Ok(()));
    assert_eq!(format!("{twice:?}"), format!("{once:?}"));
}

#[test]
fn a_retract_before_its_post_holds_when_the_post_arrives() {
    let mut post_first = ForumState::default();
    post(&mut post_first, &BO, 1, "bo's");
    retract(&mut post_first, &BO, (&BO, 1), 2).unwrap();

    let mut retract_first = ForumState::default();
    assert_eq!(retract(&mut retract_first, &BO, (&BO, 1), 2), Ok(()), "a retract needs no message to name");
    post(&mut retract_first, &BO, 1, "bo's");
    assert!(texts(&retract_first).is_empty(), "the late post stays retracted");
    assert_eq!(format!("{retract_first:?}"), format!("{post_first:?}"), "either order, one state");

    // The owner's hide, before the post, likewise.
    let mut hidden_first = ForumState::default();
    retract(&mut hidden_first, &ADA, (&BO, 1), 1).unwrap();
    post(&mut hidden_first, &BO, 1, "bo's");
    assert!(texts(&hidden_first).is_empty());
}

#[test]
fn a_retract_without_its_target_is_refused() {
    let mut s = ForumState::default();
    let args: Args = [("gen".to_string(), ArgVal::Int(1))].into();
    assert_eq!(fold(&mut s, retract_op(), &args, &BO), Err(DeltaRejection::MalformedArgs));
}

#[test]
fn forum_declares_retract_and_a_conversation_does_not() {
    let op = retract_op();
    let d = ForumType::op(op).expect("the Forum's table declares forum.retract");
    assert_eq!(d.name, "forum.retract");
    assert!(
        pacific_core::coordinator::ConversationType::op(op).is_none(),
        "the ICD's conversation kind does not declare it"
    );
}
