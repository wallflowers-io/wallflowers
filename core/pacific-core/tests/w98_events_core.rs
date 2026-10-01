//! W-98 Events, core (EVENTS-CORE): the ICD 2.3.1 draft's event rows, folded through the one
//! fold path (`fold::view_of`) and read back as the view the ICD declares.
//!
//! Grounding: core/docs/launch/pdr/w98-grounding-events.md. Op ids come from the ICD by name,
//! so a renumbering fails here and not in a copy.

use pacific_core::coordinator::{sequenced_delta, ArgVal, Args, GENESIS_PREV};
use pacific_core::object::{build_delta, MemberId, ObjectKind};
use serde_json::Value;

const OWNER: MemberId = [7u8; 32];
const COHOST: MemberId = [8u8; 32];
const MEMBER: MemberId = [9u8; 32];
const PNG: &str = "cHJvYmU=";

fn icd() -> Value {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json")).expect("the ICD");
    serde_json::from_str(&raw).expect("the ICD is JSON")
}

/// An op's id, by name, from the event kind or a facet on it.
fn op(name: &str) -> u32 {
    let d = icd();
    let found = d["kinds"]["event"]["ops"].get(name).cloned().or_else(|| {
        d["facets"].as_object().unwrap().values().find_map(|f| {
            let on = f["on"].as_array().unwrap().iter().any(|k| k == "event");
            if on { f["ops"].get(name).cloned() } else { None }
        })
    });
    found.unwrap_or_else(|| panic!("the ICD declares {name} on event"))["op"].as_u64().unwrap() as u32
}

fn args(pairs: &[(&str, ArgVal)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn t(s: &str) -> ArgVal {
    ArgVal::Text(s.into())
}
fn i(n: i64) -> ArgVal {
    ArgVal::Int(n)
}

/// One event's log: the owner's spine, in order, then commutative writes `(op, args, author, gen)`.
struct Log {
    seq: Vec<(u32, Args)>,
    comm: Vec<(u32, Args, MemberId, u64)>,
}

impl Log {
    fn new() -> Self {
        Log { seq: Vec::new(), comm: Vec::new() }
    }
    fn spine(mut self, name: &str, a: Args) -> Self {
        self.seq.push((op(name), a));
        self
    }
    fn by(mut self, name: &str, mut a: Args, author: MemberId, gen: u64) -> Self {
        a.insert("gen".into(), i(gen as i64));
        self.comm.push((op(name), a, author, gen));
        self
    }
    fn view(&self) -> Value {
        let tid = ObjectKind::Event.type_id() as u32;
        let mut log = Vec::new();
        let mut prev = GENESIS_PREV;
        for (n, (op_id, a)) in self.seq.iter().enumerate() {
            let d = sequenced_delta(tid, *op_id, a.clone(), 0, n as u64, prev);
            prev = d.id();
            log.push((OWNER, d.canonical_bytes()));
        }
        for (op_id, a, author, gen) in &self.comm {
            log.push((*author, build_delta(ObjectKind::Event, *op_id, a.clone(), 0, Some(*gen)).canonical_bytes()));
        }
        let v = pacific_core::fold::view_of("event", OWNER, vec![OWNER, COHOST, MEMBER], vec![], log, Some(&OWNER)).expect("the event folds");
        serde_json::from_str(&v).expect("the view is JSON")
    }
}

fn profile(title: &str, extra: &[(&str, ArgVal)]) -> Args {
    let mut a = args(&[("title", t(title)), ("startMs", i(1_790_000_000_000))]);
    for (k, v) in extra {
        a.insert(k.to_string(), v.clone());
    }
    a
}

fn cohost() -> Args {
    args(&[("member", t(&hex::encode(COHOST))), ("role", t("admin"))])
}

/// setProfile's W-98 args fold and read back as the ICD's view names them.
#[test]
fn the_profile_carries_zone_online_status_video_format_and_all_day() {
    let v = Log::new()
        .spine("event.setProfile", profile("Night market", &[
            ("tz", t("Asia/Seoul")),
            ("online", t("https://meet.example.org/night")),
            ("status", t("postponed")),
            ("videoUrl", t("https://video.example.org/1")),
            ("descriptorFormat", t("markdown")),
            ("allDay", i(1)),
        ]))
        .view();
    assert_eq!(v["tz"], "Asia/Seoul");
    assert_eq!(v["online"], "https://meet.example.org/night");
    assert_eq!(v["status"], "postponed");
    assert_eq!(v["video_url"], "https://video.example.org/1");
    assert_eq!(v["descriptor_format"], "markdown");
    assert_eq!(v["all_day"], 1);
}

/// Absent args read as the ICD's absences: the viewer's zone, no link, scheduled, plain, timed.
#[test]
fn an_old_profile_reads_as_scheduled_plain_and_timed() {
    let v = Log::new().spine("event.setProfile", profile("Night market", &[])).view();
    assert_eq!(v["tz"], "");
    assert_eq!(v["online"], Value::Null);
    assert_eq!(v["status"], "scheduled");
    assert_eq!(v["video_url"], Value::Null);
    assert_eq!(v["descriptor_format"], "plain");
    assert_eq!(v["all_day"], 0);
}

/// Each malformed value refuses the whole profile: the earlier one stands.
#[test]
fn a_malformed_profile_value_is_refused_whole() {
    for (k, bad) in [
        ("online", t("javascript:alert(1)")),
        ("videoUrl", t("data:text/html,x")),
        ("status", t("maybe")),
        ("descriptorFormat", t("html")),
        ("allDay", i(2)),
        ("tz", t(&"A".repeat(65))),
        ("tz", t("Asia/Seoul; drop")),
    ] {
        let v = Log::new()
            .spine("event.setProfile", profile("Before", &[]))
            .spine("event.setProfile", profile("After", &[(k, bad.clone())]))
            .view();
        assert_eq!(v["title"], "Before", "{k} = {bad:?} must refuse the profile");
    }
}

/// A co-host (admin on THIS event) edits the profile; a plain member's edit does not count; a
/// revoked admin's edits stop counting, and the owner's spine stands again.
#[test]
fn a_cohost_edits_the_profile_and_a_revoked_one_stops_counting() {
    let base = || {
        Log::new()
            .spine("event.setProfile", profile("Owner's", &[]))
            .spine("base.setRole", cohost())
    };
    let v = base().by("event.editProfile", profile("Co-host's", &[("status", t("cancelled"))]), COHOST, 5).view();
    assert_eq!(v["title"], "Co-host's");
    assert_eq!(v["status"], "cancelled");

    let v = base().by("event.editProfile", profile("A member's", &[]), MEMBER, 5).view();
    assert_eq!(v["title"], "Owner's", "a member without admin does not edit");

    let v = base()
        .spine("base.clearRole", args(&[("member", t(&hex::encode(COHOST)))]))
        .by("event.editProfile", profile("Co-host's", &[]), COHOST, 5)
        .view();
    assert_eq!(v["title"], "Owner's", "a revoked admin's write stops counting");

    // LWW by (gen, author): the higher gen wins whoever wrote it.
    let v = base()
        .by("event.editProfile", profile("Later", &[]), COHOST, 9)
        .by("event.editProfile", profile("Earlier", &[]), OWNER, 3)
        .view();
    assert_eq!(v["title"], "Later");
}

/// The event carries its co-hosts: the roles facet, read back.
#[test]
fn the_view_names_the_cohosts() {
    let v = Log::new().spine("event.setProfile", profile("x", &[])).spine("base.setRole", cohost()).view();
    assert_eq!(v["roles"][hex::encode(COHOST)], "admin");
}

/// Each act is a person (member) or an organisation (object), exactly one; the view keeps the
/// billing order and marks each unconfirmed until the reader holds the performer's half.
#[test]
fn the_lineup_is_profiles_in_billing_order() {
    let org = "ab".repeat(32);
    let acts = serde_json::json!([
        { "object": org, "role": "headliner", "start": 1_790_000_000_000i64, "end": 1_790_003_600_000i64 },
        { "member": hex::encode(MEMBER), "role": "support" },
    ]);
    let v = Log::new()
        .spine("event.setProfile", profile("x", &[]))
        .by("event.setLineup", args(&[("acts", t(&acts.to_string()))]), OWNER, 1)
        .view();
    let got = v["acts"].as_array().expect("acts");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["object"], org.as_str());
    assert_eq!(got[0]["role"], "headliner");
    assert_eq!(got[0]["end"], 1_790_003_600_000i64);
    assert_eq!(got[1]["member"], hex::encode(MEMBER));
    assert_eq!(got[0]["confirmed"], false);
    assert_eq!(got[1]["confirmed"], false);
}

/// A malformed lineup is refused whole: the list before it stands.
#[test]
fn a_malformed_lineup_is_refused_whole() {
    let m = hex::encode(MEMBER);
    let good = serde_json::json!([{ "member": m, "role": "dj" }]).to_string();
    let many: Vec<Value> = (0..51u8).map(|n| serde_json::json!({ "member": hex::encode([n; 32]), "role": "dj" })).collect();
    for bad in [
        serde_json::json!([{ "role": "dj" }]).to_string(),
        serde_json::json!([{ "member": m, "object": m, "role": "dj" }]).to_string(),
        serde_json::json!([{ "member": "zz", "role": "dj" }]).to_string(),
        serde_json::json!([{ "member": m, "role": "juggler" }]).to_string(),
        serde_json::json!([{ "member": m, "role": "dj" }, { "member": m, "role": "mc" }]).to_string(),
        serde_json::json!([{ "member": m, "role": "dj", "start": 10, "end": 9 }]).to_string(),
        serde_json::json!([{ "member": m, "role": "dj", "name": "a copy of the card" }]).to_string(),
        Value::Array(many).to_string(),
        "not json".to_string(),
    ] {
        let v = Log::new()
            .spine("event.setProfile", profile("x", &[]))
            .by("event.setLineup", args(&[("acts", t(&good))]), OWNER, 1)
            .by("event.setLineup", args(&[("acts", t(&bad))]), OWNER, 2)
            .view();
        assert_eq!(v["acts"][0]["role"], "dj", "refused: {bad}");
        assert_eq!(v["acts"].as_array().unwrap().len(), 1, "refused: {bad}");
    }
}

fn photo(id: &str, at: i64) -> Args {
    args(&[("id", t(id)), ("photo", t(PNG)), ("photoMime", t("image/png")), ("at", i(at))])
}

/// At most six live photos, ordered by `at`; a removed id stays removed, whichever arrives first.
#[test]
fn the_photo_set_caps_at_six_and_a_removal_is_final() {
    let mut log = Log::new().spine("event.setProfile", profile("x", &[]));
    for n in 0..7u64 {
        log = log.by("event.addPhoto", photo(&format!("{:016x}", n + 1), 100 - n as i64), OWNER, n + 1);
    }
    let v = log.view();
    let ph = v["photos"].as_array().expect("photos");
    assert_eq!(ph.len(), 6, "a seventh live add is refused");
    assert_eq!(ph[0]["id"], format!("{:016x}", 6), "ordered by at");
    assert_eq!(ph[0]["mime"], "image/png");

    let id = format!("{:016x}", 42);
    let v = Log::new()
        .spine("event.setProfile", profile("x", &[]))
        .by("event.removePhoto", args(&[("id", t(&id))]), OWNER, 1)
        .by("event.addPhoto", photo(&id, 5), OWNER, 2)
        .view();
    assert_eq!(v["photos"].as_array().unwrap().len(), 0, "an add after its removal is refused");
}

/// The banner and the clip are their own registers, winning over setMedia's; empty clears.
#[test]
fn the_banner_and_clip_are_their_own_registers() {
    let media = args(&[("banner", t(PNG)), ("bannerMime", t("image/png"))]);
    let v = Log::new()
        .spine("event.setProfile", profile("x", &[]))
        .spine("event.setMedia", media)
        .by("event.setBanner", args(&[("banner", t("QUJDRA==")), ("bannerMime", t("image/png")), ("bannerW", i(4)), ("bannerH", i(3))]), COHOST, 1)
        .spine("base.setRole", cohost())
        .by("event.setClip", args(&[("clip", t(PNG)), ("clipMime", t("video/mp4")), ("clipMs", i(3000))]), OWNER, 2)
        .view();
    assert_eq!(v["banner"]["data"], "QUJDRA==", "setBanner wins over setMedia");
    assert_eq!(v["banner"]["width"], 4);
    assert_eq!(v["clip"]["mime"], "video/mp4");
    assert_eq!(v["clip"]["duration_ms"], 3000);

    let v = Log::new()
        .spine("event.setProfile", profile("x", &[]))
        .by("event.setBanner", args(&[("banner", t(PNG)), ("bannerMime", t("image/png"))]), OWNER, 1)
        .by("event.setBanner", args(&[("banner", t("")), ("bannerMime", t(""))]), OWNER, 2)
        .view();
    assert_eq!(v["banner"], Value::Null, "an empty ref clears it");
}

/// Inline media past the carriage ceiling (the ICD's maxLength) is refused.
#[test]
fn inline_media_past_the_ceiling_is_refused() {
    let cap = icd()["kinds"]["event"]["ops"]["event.setBanner"]["args"]["banner"]["maxLength"].as_u64().unwrap() as usize;
    let big = "A".repeat(cap + 4);
    let v = Log::new()
        .spine("event.setProfile", profile("x", &[]))
        .by("event.setBanner", args(&[("banner", t(&big)), ("bannerMime", t("image/png"))]), OWNER, 1)
        .view();
    assert_eq!(v["banner"], Value::Null);
}

/// NC-139: an event with no visibility is `network` (public, as every Site event was under
/// O-48); the owner can make it members only.
#[test]
fn an_event_is_network_until_its_owner_says_otherwise() {
    let v = Log::new().spine("event.setProfile", profile("x", &[])).view();
    assert_eq!(v["visibility"], "network");
    let v = Log::new()
        .spine("event.setProfile", profile("x", &[]))
        .spine("base.setVisibility", args(&[("visibility", t("private"))]))
        .view();
    assert_eq!(v["visibility"], "private");
}

// ---- per op, as Step 0's record names them (ASSURANCE 5): the owner's write counts, a plain
// member's is refused, a revoked co-host's is inert. The co-host ops, event 8-13.

type Counted = fn(&Value) -> bool;

/// For one op: what the log holds before the write, the write, and whether the view shows it.
fn case(name: &str) -> (Vec<(&'static str, Args)>, Args, Counted) {
    let photo_id = format!("{:016x}", 7);
    match name {
        "event.editProfile" => (vec![], profile("Edited", &[]), |v| v["title"] == "Edited"),
        "event.setLineup" => (
            vec![],
            args(&[("acts", t(&serde_json::json!([{ "member": hex::encode(MEMBER), "role": "dj" }]).to_string()))]),
            |v| v["acts"].as_array().is_some_and(|a| a.len() == 1),
        ),
        "event.setBanner" => (vec![], args(&[("banner", t(PNG)), ("bannerMime", t("image/png"))]), |v| v["banner"]["data"] == PNG),
        "event.setClip" => (vec![], args(&[("clip", t(PNG)), ("clipMime", t("video/mp4"))]), |v| v["clip"]["mime"] == "video/mp4"),
        "event.addPhoto" => (vec![], photo(&photo_id, 5), |v| v["photos"].as_array().is_some_and(|p| p.len() == 1)),
        "event.removePhoto" => (
            vec![("event.addPhoto", photo(&photo_id, 5))],
            args(&[("id", t(&photo_id))]),
            |v| v["photos"].as_array().is_some_and(|p| p.is_empty()),
        ),
        _ => unreachable!("{name}"),
    }
}

/// The log for one case: a profile, `spine` extra (roles), the prior writes by the owner, then
/// the write by `author` at a later gen.
fn written(name: &str, spine: &[(&str, Args)], author: MemberId) -> Value {
    let (prior, write, _) = case(name);
    let mut log = Log::new().spine("event.setProfile", profile("Owner's", &[]));
    for (op, a) in spine {
        log = log.spine(op, a.clone());
    }
    for (n, (op, a)) in prior.into_iter().enumerate() {
        log = log.by(op, a, OWNER, n as u64 + 1);
    }
    log.by(name, write, author, 10).view()
}

fn owner_counts(name: &str) {
    assert!(case(name).2(&written(name, &[], OWNER)), "{name}: the owner's write counts");
}
fn member_refused(name: &str) {
    assert!(!case(name).2(&written(name, &[], MEMBER)), "{name}: a plain member's write is refused");
}
fn revoked_admin_inert(name: &str) {
    let revoked = [("base.setRole", cohost()), ("base.clearRole", args(&[("member", t(&hex::encode(COHOST)))]))];
    assert!(!case(name).2(&written(name, &revoked, COHOST)), "{name}: a revoked co-host's write is inert");
    assert!(case(name).2(&written(name, &revoked[..1], COHOST)), "{name}: the same write counts while they hold admin");
}

#[test]
fn set_lineup_owner_counts() {
    owner_counts("event.setLineup")
}
#[test]
fn set_lineup_member_refused() {
    member_refused("event.setLineup")
}
#[test]
fn set_lineup_revoked_admin_inert() {
    revoked_admin_inert("event.setLineup")
}
#[test]
fn set_banner_owner_counts() {
    owner_counts("event.setBanner")
}
#[test]
fn set_banner_member_refused() {
    member_refused("event.setBanner")
}
#[test]
fn set_banner_revoked_admin_inert() {
    revoked_admin_inert("event.setBanner")
}
#[test]
fn add_photo_owner_counts() {
    owner_counts("event.addPhoto")
}
#[test]
fn add_photo_member_refused() {
    member_refused("event.addPhoto")
}
#[test]
fn add_photo_revoked_admin_inert() {
    revoked_admin_inert("event.addPhoto")
}
#[test]
fn remove_photo_owner_counts() {
    owner_counts("event.removePhoto")
}
#[test]
fn remove_photo_member_refused() {
    member_refused("event.removePhoto")
}
#[test]
fn remove_photo_revoked_admin_inert() {
    revoked_admin_inert("event.removePhoto")
}
#[test]
fn set_clip_owner_counts() {
    owner_counts("event.setClip")
}
#[test]
fn set_clip_member_refused() {
    member_refused("event.setClip")
}
#[test]
fn set_clip_revoked_admin_inert() {
    revoked_admin_inert("event.setClip")
}
#[test]
fn edit_profile_owner_counts() {
    owner_counts("event.editProfile")
}
#[test]
fn edit_profile_member_refused() {
    member_refused("event.editProfile")
}
#[test]
fn edit_profile_revoked_admin_inert() {
    revoked_admin_inert("event.editProfile")
}
