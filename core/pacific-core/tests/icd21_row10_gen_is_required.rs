//! ICD 2.1.0 row 10: `group.editFace` and `host.editMedia` declare `gen` required, and the
//! fold enforces it, as `forum.retract` does: a delta whose args lack it is refused
//! (BUILD, 29 Sep, keeping the JSON W-86 fixed). `authoring::build` stamps it, so a delta
//! written through `Node::apply` still folds.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::group::{GroupState, GroupType};
use pacific_core::parts::OP_SET_PART;
use pacific_core::host::{HostState, HostType};
use pacific_core::object::{DeltaRejection, ObjectType, Op, ReduceContext};

const OWNER: [u8; 32] = [1; 32];
/// A 1×1 png.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn op(kind: &str, name: &str) -> u32 {
    pacific_core::authoring::op_on(kind, name).unwrap_or_else(|| panic!("{name} is not declared on {kind}")).op_id
}

fn args(kv: &[(&str, ArgVal)]) -> Args {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn with_gen(mut a: Args) -> Args {
    a.insert("gen".into(), ArgVal::Int(1));
    a
}

fn reduce<T: ObjectType>(state: &mut T::State, op_id: u32, a: &Args) -> Result<(), DeltaRejection> {
    let members = [OWNER];
    let ctx = ReduceContext { members: &members, owner: OWNER, epoch: 1 };
    T::reduce(state, &Op { op_id, args: a, author: &OWNER, pos: None, ctx: &ctx })
}

fn a_site() -> GroupState {
    let mut st = GroupState::default();
    let host = args(&[("part", ArgVal::Text("44".repeat(32))), ("role", ArgVal::Text("host".into())), ("at", ArgVal::Int(1))]);
    let members = [OWNER];
    let ctx = ReduceContext { members: &members, owner: OWNER, epoch: 1 };
    let pos = Some(pacific_core::object::LogPosition { epoch: 1, seq: 1 });
    GroupType::reduce(&mut st, &Op { op_id: OP_SET_PART, args: &host, author: &OWNER, pos, ctx: &ctx }).expect("the Site's Host part");
    st
}

#[test]
fn a_face_edit_without_its_gen_is_refused() {
    let face = args(&[("face", ArgVal::Text(r#"{"v":1}"#.into()))]);
    let edit = op("group", "group.editFace");
    assert_eq!(reduce::<GroupType>(&mut a_site(), edit, &face), Err(DeltaRejection::MalformedArgs));
    assert_eq!(reduce::<GroupType>(&mut a_site(), edit, &with_gen(face)), Ok(()), "with it, it folds");
}

#[test]
fn a_media_edit_without_its_gen_is_refused() {
    let media = args(&[("slot", ArgVal::Text("mark".into())), ("media", ArgVal::Text(PNG.into())), ("mediaMime", ArgVal::Text("image/png".into()))]);
    let edit = op("host", "host.editMedia");
    assert_eq!(reduce::<HostType>(&mut HostState::default(), edit, &media), Err(DeltaRejection::MalformedArgs));
    assert_eq!(reduce::<HostType>(&mut HostState::default(), edit, &with_gen(media)), Ok(()), "with it, it folds");
}

/// Written as every writer writes, through `Node::apply` and `authoring::build`, with no
/// `gen` in the caller's args: both fold.
#[tokio::test]
async fn both_edits_through_the_one_write_path_fold() {
    let h = Harness::new(&["ada"]).await;
    let site = h.mint_group(0);
    h.group_set_profile(0, &site, "Mill Road", "community").await;
    let host = h.node(0).host_new(&site, "Mill Road").await.expect("the Site's Host");
    let face = args(&[("face", ArgVal::Text(r#"{"v":1,"by":"ada"}"#.into()))]);
    h.node(0).apply(&site, op("group", "group.editFace"), face).await.expect("the owner's face edit");
    let media = args(&[("slot", ArgVal::Text("mark".into())), ("media", ArgVal::Text(PNG.into())), ("mediaMime", ArgVal::Text("image/png".into()))]);
    h.node(0).apply(&host, op("host", "host.editMedia"), media).await.expect("the owner's media edit");
    h.settle().await;
    assert_eq!(h.node(0).group_state(&site).unwrap().face, r#"{"v":1,"by":"ada"}"#);
    let view: serde_json::Value = serde_json::from_str(&h.node(0).object_view(&host).unwrap()).unwrap();
    assert_eq!(view["media"]["mark"]["mime"], "image/png", "{view}");
    assert!(h.node(0).noncompliant_objects().unwrap().is_empty(), "everything held folds");
}
