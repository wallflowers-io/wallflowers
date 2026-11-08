//! face.schema.json: every face the webapp's editor writes is one the specification allows,
//! and what it does not allow is refused before a write, with the place it went wrong.
//!
//! The fixtures are face.js's own `document()`, and in published/ what face.js publishes from it
//! (core/coordination/face-fixtures.mjs): core's `bundle` must publish exactly that.

use pacific_core::face_spec::{bundle, refusal};
use std::path::Path;

fn fixtures() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/faces");
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .expect("the face fixtures")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    out.sort();
    out
}

fn every_part() -> serde_json::Value {
    let (_, t) = fixtures().into_iter().find(|(n, _)| n == "every-part.json").unwrap();
    serde_json::from_str(&t).unwrap()
}

fn refused(f: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut v = every_part();
    f(&mut v);
    refusal(&v.to_string()).expect("refused")
}

#[test]
fn every_face_the_editor_writes_is_allowed() {
    let all = fixtures();
    assert!(all.len() >= 17, "the default, the fifteen themes and every-part");
    for (name, face) in all {
        assert_eq!(refusal(&face), None, "{name}");
    }
}

#[test]
fn what_the_specification_does_not_allow_is_refused_where_it_is() {
    assert_eq!(refused(|v| v["look"]["colours"]["text"] = "red".into()), "face.look.colours.text is not a #RRGGBB colour");
    assert_eq!(refused(|v| v["mark"] = "data:image/png;base64,AAAA".into()), "face.mark must be empty: pictures are the Host's");
    assert_eq!(refused(|v| v["look"]["text"]["font"] = "comic-sans".into()), "face.look.text.font: no font 'comic-sans'");
    assert_eq!(refused(|v| v["stickers"][0]["id"] = "nope".into()), "face.stickers[0].id: no sticker 'nope'");
    assert_eq!(refused(|v| v["look"]["blocks"]["corners"] = 4.into()), "face.look.blocks.corners: 4 is out of range");
    assert_eq!(refused(|v| v["door"]["access"] = "secret".into()), "face.door.access: \"secret\" is not one of the choices");
    assert_eq!(refused(|v| v["extra"] = 1.into()), "face.extra is not part of a face");
    assert_eq!(
        refused(|v| v["blocks"]["root"]["children"][0]["type"] = "wf-iframe".into()),
        "face.blocks.root.children[0]: \"wf-iframe\" is not a kind of widget"
    );
    assert_eq!(
        refused(|v| { v["blocks"]["root"]["children"][6].as_object_mut().unwrap().remove("url"); }),
        "face.blocks.root.children[6].url is missing"
    );
    assert_eq!(
        refused(|v| v["door"]["questions"] = serde_json::Value::Array(vec![v["door"]["questions"][0].clone(); 6])),
        "face.door.questions holds at most 5"
    );
    assert_eq!(refusal("[]"), Some("face must be an object".into()));
    assert!(refusal(&"x".repeat(16385)).unwrap().contains("over 16384 bytes"));
}

#[test]
fn core_publishes_a_face_as_the_editor_does() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/faces/published");
    for (name, face) in fixtures() {
        let working: serde_json::Value = serde_json::from_str(&face).unwrap();
        let want: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(&name)).unwrap()).unwrap();
        let b = bundle("Riverside", "community", "A garden.", &working);
        assert_eq!(b["face"], want["face"], "{name}: the published face");
        assert_eq!(b["profile"]["card"]["urls"], want["urls"], "{name}: the social links");
        assert_eq!(b.get("location").cloned().unwrap_or_default(), want["location"], "{name}: the location");
        assert_eq!(b["profile"]["displayName"], "Riverside");
    }
}
