//! THE DOOR'S BOUND, READ FROM THE ICD AT BUILD (`mls.door.relay`, B3, ruled 9 Nov 2026).
//! Parametric, as pacific-core's build.rs: the relay that enforces the bound and the client
//! that knocks both link this crate, so this is the one read of it and both sides agree.

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(&manifest).join("../coordination/delta-graph.icd.json");
    println!("cargo:rerun-if-changed={}", path.display());
    let icd: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("the ICD, beside pacific-wire"))
        .expect("the ICD is JSON");
    let r = &icd["mls"]["door"]["relay"];
    let num = |k: &str| r[k].as_u64().unwrap_or_else(|| panic!("the ICD states mls.door.relay.{k}"));
    let prefix = r["prefix"].as_str().expect("the ICD states mls.door.relay.prefix");
    assert_eq!(r["evict"].as_str(), Some("oldest"), "mls.door.relay.evict: the relay evicts the oldest");
    let evicted_v = r["evicted"]["v"].as_u64().expect("the ICD states mls.door.relay.evicted.v");
    let evicted_frame = r["evicted"]["frame"].as_str().expect("the ICD states mls.door.relay.evicted.frame");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_door.rs");
    std::fs::write(
        out,
        format!(
            "/// `mls.door.relay.prefix`: what makes a tag a door's.\npub const DOOR_PREFIX: &str = {prefix:?};\n\
             /// `mls.door.relay.cap`: knocks a door tag holds; the oldest goes past it.\npub const DOOR_CAP: u64 = {};\n\
             /// `mls.door.relay.ttlDays`.\npub const DOOR_TTL_DAYS: u64 = {};\n\
             /// `mls.door.relay.maxBytes`: one knock, as published.\npub const DOOR_MAX_BYTES: u64 = {};\n\
             /// `mls.door.relay.perConnectionPerHour`: knocks one connection may make at one door.\npub const DOOR_PER_CONN_PER_HOUR: u64 = {};\n\
             /// `mls.door.relay.evicted.v`: the level at which a drain is told what was evicted.\npub const DOOR_EVICTED_V: u8 = {evicted_v};\n\
             /// `mls.door.relay.evicted.frame`.\npub const DOOR_EVICTED_FRAME: &str = {evicted_frame:?};\n",
            num("cap"),
            num("ttlDays"),
            num("maxBytes"),
            num("perConnectionPerHour"),
        ),
    )
    .expect("icd_door.rs");

    // THE HOSTED OBJECT'S BOUND (`media.hosted`): the level its overwrite travels at, and the
    // window a request's `at` must fall in. The relay enforces both; the client signs to them.
    let h = &icd["media"]["hosted"];
    let host_v = h["v"].as_u64().expect("the ICD states media.hosted.v");
    let host_frame = h["frame"].as_str().expect("the ICD states media.hosted.frame");
    let window = h["windowSecs"].as_u64().expect("the ICD states media.hosted.windowSecs");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_host.rs");
    std::fs::write(
        out,
        format!(
            "/// `media.hosted.v`: the level a hosted object's overwrite travels at.\npub const HOST_V: u8 = {host_v};\n\
             /// `media.hosted.frame`.\npub const HOST_FRAME: &str = {host_frame:?};\n\
             /// `media.hosted.windowSecs`: how far a `MediaHost`'s `at` may be from the relay's clock.\npub const HOST_WINDOW_SECS: u64 = {window};\n"
        ),
    )
    .expect("icd_host.rs");
}
