//! W-98 FOLDS: the Site's answers to its Events, and its members' own words (ICD 2.3.1).
//!
//! group.rsvp (18, member), group.setRegistration (19, owner|role:admin) and group.rsvpDecide
//! (20, owner|role:admin), all commutative, on the Site's own log: an attendee is on no Event
//! roster, so the Site folds the answers, and group.rsvp's fold enforces the registration
//! register (capacity, waitlist, approval, guestsMax, maybe, closesMs), since no fold reads
//! another object. The facets `about` (base.publishAbout, member) and `questions`
//! (base.defineQuestion and base.retireQuestion, owner|role:admin; base.answerQuestion, member),
//! on `group`.
//!
//! Principals as the ICD's `principals` states them: `member` is a leaf on the roster;
//! `owner|role:admin` is the owner, or a roster member holding admin in the Site's own folded
//! roles, so a revoked admin's writes stop counting.
//!
//! Every test reads the view `fold::view_of` renders (the keys the ICD's `view` lines name),
//! folded from encoded deltas as every replica folds them. Ops are resolved by NAME from the
//! catalogue, so before the code declares one each test fails naming it.

use pacific_core::coordinator::{self, ArgVal, Args, Delta};
use pacific_core::object::{build_delta, LogPosition, ObjectKind};
use serde_json::{json, Value};

type Id = [u8; 32];

const OWNER: Id = [1; 32];
const ADMIN: Id = [2; 32];
const MEMBER: Id = [3; 32];
const ANN: Id = [4; 32];
const BEA: Id = [5; 32];
const CAL: Id = [6; 32];
const STRANGER: Id = [9; 32];
const EPOCH: u64 = 1;

/// An Event the Site created (its rel=created edge), and one it did not.
const EVENT: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const ELSEWHERE: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn op(name: &str) -> u32 {
    pacific_core::authoring::op_on("group", name)
        .unwrap_or_else(|| panic!("{name} is not declared on group (ICD 2.3.1, W-98)"))
        .op_id
}
fn h(m: Id) -> String {
    hex::encode(m)
}
fn t(s: &str) -> ArgVal {
    ArgVal::Text(s.into())
}
fn i(n: i64) -> ArgVal {
    ArgVal::Int(n)
}
fn args(kv: &[(&str, ArgVal)]) -> Args {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// One Site's log: the owner's sequenced spine, hash-chained, and commutative deltas by
/// whoever wrote them, each with the gen it carries. Folded through `view_of`.
struct Site {
    roster: Vec<Id>,
    log: Vec<(Id, Delta)>,
    head: Option<(LogPosition, [u8; 32])>,
}

impl Site {
    /// OWNER, ADMIN (holding admin), MEMBER, ANN, BEA and CAL on the roster; EVENT created.
    fn new() -> Self {
        let mut s = Self { roster: vec![OWNER, ADMIN, MEMBER, ANN, BEA, CAL], log: vec![], head: None };
        s.seq(op("group.setAffiliation"), args(&[("peer", t(EVENT)), ("rel", t("created")), ("name", t("the fete")), ("at", i(1))]));
        s.seq(op("base.setRole"), args(&[("member", t(&h(ADMIN))), ("role", t("admin"))]));
        s
    }
    fn seq(&mut self, op_id: u32, a: Args) {
        let (seq, prev) = coordinator::next_sequenced_pos(self.head, EPOCH);
        let d = coordinator::sequenced_delta(ObjectKind::Group.type_id() as u32, op_id, a, EPOCH, seq, prev);
        self.head = Some((LogPosition { epoch: EPOCH, seq }, d.id()));
        self.log.push((OWNER, d));
    }
    fn comm(&mut self, author: Id, name: &str, mut a: Args, gen: u64) {
        a.insert("gen".into(), ArgVal::Int(gen as i64));
        self.log.push((author, build_delta(ObjectKind::Group, op(name), a, EPOCH, Some(gen))));
    }
    fn revoke_admin(&mut self) {
        self.seq(op("base.clearRole"), args(&[("member", t(&h(ADMIN)))]));
    }
    fn view_of(&self, log: &[(Id, Delta)]) -> Value {
        let rows = log.iter().map(|(a, d)| (*a, d.canonical_bytes()));
        let v = pacific_core::fold::view_of("group", OWNER, self.roster.clone(), vec![(0, OWNER)], rows, None)
            .expect("the Site folds");
        serde_json::from_str(&v).expect("the view is JSON")
    }
    fn view(&self) -> Value {
        self.view_of(&self.log)
    }
    /// The view with the commutative deltas delivered in reverse: the fold orders them.
    fn view_reversed(&self) -> Value {
        let (spine, mut comm): (Vec<_>, Vec<_>) = self.log.iter().cloned().partition(|(_, d)| d.seq.is_some());
        comm.reverse();
        self.view_of(&spine.into_iter().chain(comm).collect::<Vec<_>>())
    }
}

// ---- group.rsvp ---------------------------------------------------------------------

fn rsvp(site: &mut Site, who: Id, status: &str, guests: i64, at: i64, gen: u64) {
    site.comm(who, "group.rsvp", args(&[("event", t(EVENT)), ("status", t(status)), ("guests", i(guests)), ("at", i(at))]), gen);
}
fn answer(v: &Value, who: Id) -> &Value {
    &v["rsvps"][EVENT][h(who)]
}
fn counts(v: &Value) -> &Value {
    &v["rsvp_counts"][EVENT]
}
fn registration(site: &mut Site, who: Id, kv: &[(&str, ArgVal)], gen: u64) {
    let mut a = args(kv);
    a.insert("event".into(), t(EVENT));
    site.comm(who, "group.setRegistration", a, gen);
}
fn decide(site: &mut Site, who: Id, member: Id, decision: &str, gen: u64) {
    site.comm(who, "group.rsvpDecide", args(&[("event", t(EVENT)), ("member", t(&h(member))), ("decision", t(decision))]), gen);
}

#[test]
fn rsvp_member_counts() {
    let mut s = Site::new();
    rsvp(&mut s, MEMBER, "going", 2, 1_000, 3);
    let v = s.view();
    let a = answer(&v, MEMBER);
    assert_eq!((a["status"].as_str(), a["guests"].as_i64(), a["at"].as_i64()), (Some("going"), Some(2), Some(1_000)), "{v}");
    assert_eq!(counts(&v), &json!({"going": 1, "maybe": 0, "declined": 0, "guests": 2}));
}

#[test]
fn rsvp_non_member_refused() {
    let mut s = Site::new();
    rsvp(&mut s, STRANGER, "going", 0, 1_000, 3);
    let v = s.view();
    assert!(answer(&v, STRANGER).is_null(), "an author off the roster answers nothing: {v}");
    assert!(counts(&v).is_null() || counts(&v)["going"] == 0, "{v}");
}

#[test]
fn rsvp_to_an_event_the_site_did_not_create_is_refused() {
    let mut s = Site::new();
    s.comm(MEMBER, "group.rsvp", args(&[("event", t(ELSEWHERE)), ("status", t("going")), ("at", i(1_000))]), 3);
    assert!(s.view()["rsvps"][ELSEWHERE].is_null());
}

/// Guests counted only for going; maybe and declined counted apart; guests only with going.
#[test]
fn rsvp_counts_guests_only_for_going() {
    let mut s = Site::new();
    rsvp(&mut s, ANN, "going", 1, 1_000, 3);
    rsvp(&mut s, BEA, "maybe", 0, 1_000, 4);
    rsvp(&mut s, CAL, "declined", 0, 1_000, 5);
    rsvp(&mut s, MEMBER, "maybe", 2, 1_000, 6);
    let v = s.view();
    assert_eq!(counts(&v), &json!({"going": 1, "maybe": 1, "declined": 1, "guests": 1}), "{v}");
    assert!(answer(&v, MEMBER).is_null(), "guests on a maybe are refused");
}

/// One register per (author, event): the later gen wins, whatever order the deltas arrive in,
/// and a stale replay never undoes a newer answer. Declining withdraws.
#[test]
fn rsvp_lww_in_any_order() {
    let mut s = Site::new();
    rsvp(&mut s, ANN, "going", 1, 1_000, 3);
    rsvp(&mut s, ANN, "declined", 0, 2_000, 7);
    rsvp(&mut s, ANN, "maybe", 0, 1_500, 5);
    for v in [s.view(), s.view_reversed()] {
        assert_eq!(answer(&v, ANN)["status"], "declined", "{v}");
        assert_eq!(counts(&v), &json!({"going": 0, "maybe": 0, "declined": 1, "guests": 0}));
    }
}

/// guestNames: at most `guests`, each 1 to 80 characters; occurrence answers one startMs.
#[test]
fn rsvp_carries_guest_names_and_an_occurrence() {
    let mut s = Site::new();
    s.comm(ANN, "group.rsvp", args(&[("event", t(EVENT)), ("status", t("going")), ("guests", i(1)), ("at", i(1_000)),
        ("guestNames", t(r#"["Dot"]"#)), ("occurrence", i(1_760_000_000_000))]), 3);
    s.comm(BEA, "group.rsvp", args(&[("event", t(EVENT)), ("status", t("going")), ("guests", i(1)), ("at", i(1_000)),
        ("guestNames", t(r#"["Dot","Eve"]"#))]), 4);
    s.comm(CAL, "group.rsvp", args(&[("event", t(EVENT)), ("status", t("going")), ("guests", i(1)), ("at", i(1_000)),
        ("guestNames", t(&format!(r#"["{}"]"#, "x".repeat(81))))]), 5);
    let v = s.view();
    assert_eq!(answer(&v, ANN)["guestNames"], json!(["Dot"]), "{v}");
    assert_eq!(answer(&v, ANN)["occurrence"], 1_760_000_000_000i64);
    assert!(answer(&v, BEA).is_null(), "more names than guests is refused");
    assert!(answer(&v, CAL).is_null(), "a name over 80 characters is refused");
}

// ---- group.setRegistration ------------------------------------------------------------

#[test]
fn set_registration_owner_counts() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("capacity", i(40)), ("waitlist", i(1)), ("location", t("members"))], 3);
    let v = s.view();
    let r = &v["registration"][EVENT];
    assert_eq!(r["capacity"], 40, "{v}");
    assert_eq!(r["waitlist"], 1);
    assert_eq!(r["location"], "members");
    assert_eq!((r["approval"].as_i64(), r["guestsMax"].as_i64(), r["maybe"].as_i64()), (Some(0), Some(10), Some(1)), "the defaults");
    assert!(r["closesMs"].is_null(), "absent is open");
}

#[test]
fn set_registration_member_refused() {
    let mut s = Site::new();
    registration(&mut s, MEMBER, &[("capacity", i(1))], 3);
    assert!(s.view()["registration"][EVENT].is_null());
    rsvp(&mut s, ANN, "going", 0, 1_000, 4);
    rsvp(&mut s, BEA, "going", 0, 1_000, 5);
    assert_eq!(counts(&s.view())["going"], 2, "a member's capacity binds nobody");
}

#[test]
fn set_registration_revoked_admin_inert() {
    let mut s = Site::new();
    registration(&mut s, ADMIN, &[("capacity", i(5))], 3);
    assert_eq!(s.view()["registration"][EVENT]["capacity"], 5);
    s.revoke_admin();
    assert!(s.view()["registration"][EVENT].is_null(), "a revoked admin's register stops counting");
}

#[test]
fn set_registration_lww_in_any_order() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("capacity", i(9))], 3);
    registration(&mut s, ADMIN, &[("capacity", i(12))], 6);
    registration(&mut s, OWNER, &[("capacity", i(3))], 4);
    for v in [s.view(), s.view_reversed()] {
        assert_eq!(v["registration"][EVENT]["capacity"], 12, "{v}");
    }
}

// ---- the register, enforced by group.rsvp's fold -----------------------------------------

/// Capacity counts going places: an answer and its guests. Past it, with no waitlist, refused.
#[test]
fn rsvp_past_capacity_is_refused_without_a_waitlist() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("capacity", i(3))], 2);
    rsvp(&mut s, ANN, "going", 1, 1_000, 3);
    rsvp(&mut s, BEA, "going", 1, 1_000, 4);
    rsvp(&mut s, CAL, "going", 0, 1_000, 5);
    let v = s.view();
    assert!(answer(&v, BEA).is_null(), "two more places than the one left: refused: {v}");
    assert_eq!(answer(&v, CAL)["place"], "going", "one place, one answer");
    assert_eq!(counts(&v), &json!({"going": 2, "maybe": 0, "declined": 0, "guests": 1}));
}

/// With a waitlist, an answer past capacity waits; it counts nowhere until a host admits it.
#[test]
fn rsvp_past_capacity_waits_with_a_waitlist() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("capacity", i(1)), ("waitlist", i(1))], 2);
    rsvp(&mut s, ANN, "going", 0, 1_000, 3);
    rsvp(&mut s, BEA, "going", 0, 1_000, 4);
    let v = s.view();
    assert_eq!(answer(&v, ANN)["place"], "going", "{v}");
    assert_eq!(answer(&v, BEA)["place"], "waitlisted");
    assert_eq!(counts(&v)["going"], 1);
    decide(&mut s, ADMIN, BEA, "approved", 5);
    let v = s.view();
    assert_eq!(answer(&v, BEA)["place"], "going", "approved counts as going: {v}");
    assert_eq!(answer(&v, BEA)["decision"], "approved");
    assert_eq!(counts(&v)["going"], 2);
}

/// Declining frees the place for the next answer.
#[test]
fn a_decline_frees_its_place() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("capacity", i(1))], 2);
    rsvp(&mut s, ANN, "going", 0, 1_000, 3);
    rsvp(&mut s, ANN, "declined", 0, 1_100, 4);
    rsvp(&mut s, BEA, "going", 0, 1_200, 5);
    let v = s.view();
    assert_eq!(answer(&v, BEA)["place"], "going", "{v}");
    assert_eq!(counts(&v), &json!({"going": 1, "maybe": 0, "declined": 1, "guests": 0}));
}

/// With approval, a going answer counts only once approved (group.rsvpDecide).
#[test]
fn rsvp_with_approval_counts_once_approved() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("approval", i(1))], 2);
    rsvp(&mut s, ANN, "going", 1, 1_000, 3);
    rsvp(&mut s, BEA, "going", 0, 1_000, 4);
    rsvp(&mut s, CAL, "going", 0, 1_000, 5);
    let v = s.view();
    assert_eq!(answer(&v, ANN)["place"], "pending", "{v}");
    assert_eq!(counts(&v)["going"], 0);
    decide(&mut s, OWNER, ANN, "approved", 6);
    decide(&mut s, OWNER, BEA, "waitlisted", 7);
    decide(&mut s, ADMIN, CAL, "declined", 8);
    let v = s.view();
    assert_eq!(answer(&v, ANN)["place"], "going", "{v}");
    assert_eq!(answer(&v, BEA)["place"], "waitlisted");
    assert_eq!(answer(&v, CAL)["place"], "declined");
    assert_eq!(answer(&v, CAL)["decision"], "declined");
    assert_eq!(counts(&v), &json!({"going": 1, "maybe": 0, "declined": 0, "guests": 1}));
}

/// Answers after closesMs are refused, by the answer's own `at`.
#[test]
fn rsvp_after_closes_ms_is_refused() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("closesMs", i(5_000))], 2);
    rsvp(&mut s, ANN, "going", 0, 4_999, 3);
    rsvp(&mut s, BEA, "going", 0, 5_001, 4);
    let v = s.view();
    assert_eq!(answer(&v, ANN)["status"], "going", "{v}");
    assert!(answer(&v, BEA).is_null(), "after the close: refused");
}

/// guestsMax bounds a going answer's guests; 10 bounds them absent a register.
#[test]
fn rsvp_guests_over_guests_max_are_refused() {
    let mut s = Site::new();
    rsvp(&mut s, CAL, "going", 11, 1_000, 2);
    registration(&mut s, OWNER, &[("guestsMax", i(1))], 3);
    rsvp(&mut s, ANN, "going", 1, 1_000, 4);
    rsvp(&mut s, BEA, "going", 2, 1_000, 5);
    let v = s.view();
    assert!(answer(&v, CAL).is_null(), "over 10: refused: {v}");
    assert_eq!(answer(&v, ANN)["guests"], 1);
    assert!(answer(&v, BEA).is_null(), "over guestsMax: refused");
}

/// maybe 0: going or declined only.
#[test]
fn rsvp_maybe_is_refused_where_the_register_offers_none() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("maybe", i(0))], 2);
    rsvp(&mut s, ANN, "maybe", 0, 1_000, 3);
    rsvp(&mut s, BEA, "declined", 0, 1_000, 4);
    let v = s.view();
    assert!(answer(&v, ANN).is_null(), "{v}");
    assert_eq!(answer(&v, BEA)["status"], "declined");
}

// ---- group.rsvpDecide ------------------------------------------------------------------

#[test]
fn rsvp_decide_owner_counts() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("approval", i(1))], 2);
    rsvp(&mut s, ANN, "going", 0, 1_000, 3);
    decide(&mut s, OWNER, ANN, "approved", 4);
    let v = s.view();
    assert_eq!(answer(&v, ANN)["decision"], "approved", "{v}");
    assert_eq!(counts(&v)["going"], 1);
}

#[test]
fn rsvp_decide_member_refused() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("approval", i(1))], 2);
    rsvp(&mut s, ANN, "going", 0, 1_000, 3);
    decide(&mut s, MEMBER, ANN, "approved", 4);
    decide(&mut s, ANN, ANN, "approved", 5);
    let v = s.view();
    assert!(answer(&v, ANN)["decision"].is_null(), "{v}");
    assert_eq!(answer(&v, ANN)["place"], "pending");
}

#[test]
fn rsvp_decide_revoked_admin_inert() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("approval", i(1))], 2);
    rsvp(&mut s, ANN, "going", 0, 1_000, 3);
    decide(&mut s, ADMIN, ANN, "approved", 4);
    assert_eq!(counts(&s.view())["going"], 1);
    s.revoke_admin();
    let v = s.view();
    assert!(answer(&v, ANN)["decision"].is_null(), "{v}");
    assert_eq!(counts(&v)["going"], 0, "a revoked admin's approval stops counting");
}

#[test]
fn rsvp_decide_lww_in_any_order() {
    let mut s = Site::new();
    registration(&mut s, OWNER, &[("approval", i(1))], 2);
    rsvp(&mut s, ANN, "going", 0, 1_000, 3);
    decide(&mut s, OWNER, ANN, "approved", 4);
    decide(&mut s, ADMIN, ANN, "declined", 9);
    decide(&mut s, OWNER, ANN, "waitlisted", 6);
    for v in [s.view(), s.view_reversed()] {
        assert_eq!(answer(&v, ANN)["decision"], "declined", "{v}");
        assert_eq!(counts(&v)["going"], 0);
    }
}

// ---- about: base.publishAbout -----------------------------------------------------------

fn about(site: &mut Site, who: Id, bio: &str, links: Option<&str>, gen: u64) {
    let mut a = args(&[("bio", t(bio))]);
    if let Some(l) = links {
        a.insert("links".into(), t(l));
    }
    site.comm(who, "base.publishAbout", a, gen);
}

#[test]
fn publish_about_member_counts() {
    let mut s = Site::new();
    about(&mut s, ANN, "I print zines.", Some(r#"["https://ann.example/zines"]"#), 3);
    let v = s.view();
    assert_eq!(v["about"][h(ANN)], json!({"bio": "I print zines.", "links": ["https://ann.example/zines"], "gen": 3}), "{v}");
}

#[test]
fn publish_about_non_member_refused() {
    let mut s = Site::new();
    about(&mut s, STRANGER, "not here", None, 3);
    let v = s.view();
    assert!(v["about"][h(STRANGER)].is_null(), "roster members only: {v}");
}

#[test]
fn publish_about_holds_its_caps_and_empty_clears() {
    let mut s = Site::new();
    about(&mut s, ANN, &"a".repeat(4097), None, 3);
    about(&mut s, BEA, "b", Some(r#"["https://1.example","https://2.example","https://3.example","https://4.example"]"#), 4);
    about(&mut s, CAL, "c", Some(r#"["http://plain.example"]"#), 5);
    about(&mut s, MEMBER, &"m".repeat(4096), None, 6);
    let v = s.view();
    for who in [ANN, BEA, CAL] {
        assert!(v["about"][h(who)].is_null(), "refused, never truncated: {v}");
    }
    assert_eq!(v["about"][h(MEMBER)]["bio"].as_str().map(str::len), Some(4096));
    about(&mut s, MEMBER, "", None, 7);
    assert!(s.view()["about"][h(MEMBER)].is_null(), "an empty bio and no links clears");
}

#[test]
fn publish_about_lww_in_any_order() {
    let mut s = Site::new();
    about(&mut s, ANN, "first", None, 3);
    about(&mut s, ANN, "last", None, 8);
    about(&mut s, ANN, "middle", None, 5);
    for v in [s.view(), s.view_reversed()] {
        assert_eq!(v["about"][h(ANN)]["bio"], "last", "{v}");
    }
}

// ---- questions: base.defineQuestion, base.retireQuestion, base.answerQuestion -------------

fn define(site: &mut Site, who: Id, kv: &[(&str, ArgVal)], gen: u64) {
    site.comm(who, "base.defineQuestion", args(kv), gen);
}
fn retire(site: &mut Site, who: Id, q: (Id, u64), gen: u64) {
    site.comm(who, "base.retireQuestion", args(&[("target_author", t(&h(q.0))), ("target_gen", i(q.1 as i64))]), gen);
}
fn answer_q(site: &mut Site, who: Id, q: (Id, u64), kv: &[(&str, ArgVal)], gen: u64) {
    let mut a = args(kv);
    a.insert("target_author".into(), t(&h(q.0)));
    a.insert("target_gen".into(), i(q.1 as i64));
    site.comm(who, "base.answerQuestion", a, gen);
}
fn questions(v: &Value) -> &Vec<Value> {
    v["questions"].as_array().unwrap_or_else(|| panic!("the view carries `questions`: {v}"))
}
fn poll(site: &mut Site, who: Id, gen: u64) {
    define(site, who, &[("text", t("What do you bring?")), ("options", t(r#"["food","music","hands"]"#)), ("multi", i(1)), ("max", i(2)),
        ("hint", t("up to two"))], gen);
}

#[test]
fn define_question_owner_counts() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    define(&mut s, OWNER, &[("text", t("Why here?")), ("textMax", i(180))], 4);
    let v = s.view();
    let q = questions(&v);
    assert_eq!(q.len(), 2, "{v}");
    assert_eq!((q[0]["author"].as_str(), q[0]["gen"].as_i64()), (Some(h(OWNER).as_str()), Some(3)), "oldest first");
    assert_eq!(q[0]["options"], json!(["food", "music", "hands"]));
    assert_eq!((q[0]["multi"].as_bool(), q[0]["free"].as_bool(), q[0]["retired"].as_bool()), (Some(true), Some(false), Some(false)));
    assert_eq!((q[0]["max"].as_i64(), q[0]["hint"].as_str()), (Some(2), Some("up to two")));
    assert_eq!(q[0]["tally"], json!([0, 0, 0]));
    assert!(q[1]["options"].is_null(), "a free question");
    assert_eq!((q[1]["free"].as_bool(), q[1]["textMax"].as_i64()), (Some(true), Some(180)));
}

#[test]
fn define_question_member_refused() {
    let mut s = Site::new();
    poll(&mut s, MEMBER, 3);
    assert!(questions(&s.view()).is_empty());
}

#[test]
fn define_question_revoked_admin_inert() {
    let mut s = Site::new();
    poll(&mut s, ADMIN, 3);
    answer_q(&mut s, ANN, (ADMIN, 3), &[("choices", t("[0]"))], 4);
    assert_eq!(questions(&s.view()).len(), 1);
    s.revoke_admin();
    assert!(questions(&s.view()).is_empty(), "a question counts while its author holds admin");
}

#[test]
fn define_question_holds_its_shape() {
    let mut s = Site::new();
    define(&mut s, OWNER, &[("text", t(""))], 3);
    define(&mut s, OWNER, &[("text", t("one?")), ("options", t(r#"["only"]"#))], 4);
    define(&mut s, OWNER, &[("text", t("twice?")), ("options", t(r#"["a","a"]"#))], 5);
    define(&mut s, OWNER, &[("text", t("free multi?")), ("multi", i(1))], 6);
    define(&mut s, OWNER, &[("text", t("max past options?")), ("options", t(r#"["a","b"]"#)), ("multi", i(1)), ("max", i(3))], 7);
    define(&mut s, OWNER, &[("text", t("nothing to answer?")), ("free", i(0))], 8);
    define(&mut s, OWNER, &[("text", t(&"q".repeat(1025)))], 9);
    assert!(questions(&s.view()).is_empty(), "{}", s.view());
}

#[test]
fn retire_question_owner_counts() {
    let mut s = Site::new();
    poll(&mut s, ADMIN, 3);
    answer_q(&mut s, ANN, (ADMIN, 3), &[("choices", t("[0,2]"))], 4);
    retire(&mut s, OWNER, (ADMIN, 3), 5);
    answer_q(&mut s, BEA, (ADMIN, 3), &[("choices", t("[1]"))], 6);
    let v = s.view();
    let q = &questions(&v)[0];
    assert_eq!(q["retired"], true, "{v}");
    assert_eq!(q["answers"][h(ANN)]["choices"], json!([0, 2]), "it and its answers stay");
    assert!(q["answers"][h(BEA)].is_null(), "closed to new answers");
    assert_eq!(q["tally"], json!([1, 0, 1]));
}

#[test]
fn retire_question_member_refused() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    retire(&mut s, MEMBER, (OWNER, 3), 4);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[1]"))], 5);
    let v = s.view();
    assert_eq!(questions(&v)[0]["retired"], false, "{v}");
    assert_eq!(questions(&v)[0]["tally"], json!([0, 1, 0]));
}

#[test]
fn retire_question_revoked_admin_inert() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    retire(&mut s, ADMIN, (OWNER, 3), 4);
    assert_eq!(questions(&s.view())[0]["retired"], true);
    s.revoke_admin();
    assert_eq!(questions(&s.view())[0]["retired"], false, "a revoked admin's retirement stops counting");
}

#[test]
fn answer_question_member_counts() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    define(&mut s, OWNER, &[("text", t("Seek?")), ("textMax", i(12))], 4);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[1,2]"))], 5);
    answer_q(&mut s, BEA, (OWNER, 3), &[("choices", t("[1]"))], 6);
    answer_q(&mut s, ANN, (OWNER, 4), &[("text", t("a quiet room"))], 7);
    let v = s.view();
    let q = questions(&v);
    assert_eq!(q[0]["answers"][h(ANN)], json!({"text": null, "choices": [1, 2], "gen": 5}), "{v}");
    assert_eq!(q[0]["tally"], json!([0, 2, 1]));
    assert_eq!(q[1]["answers"][h(ANN)]["text"], "a quiet room");
    assert_eq!(q[1]["tally"], json!([]));
}

#[test]
fn answer_question_non_member_refused() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    answer_q(&mut s, STRANGER, (OWNER, 3), &[("choices", t("[0]"))], 4);
    let v = s.view();
    assert!(questions(&v)[0]["answers"][h(STRANGER)].is_null(), "{v}");
    assert_eq!(questions(&v)[0]["tally"], json!([0, 0, 0]));
}

#[test]
fn answer_question_holds_the_questions_rules() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    define(&mut s, OWNER, &[("text", t("One?")), ("options", t(r#"["a","b"]"#))], 4);
    define(&mut s, OWNER, &[("text", t("Seek?")), ("textMax", i(5))], 5);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[0,1,2]"))], 6); // over max
    answer_q(&mut s, BEA, (OWNER, 3), &[("choices", t("[3]"))], 7); // out of range
    answer_q(&mut s, CAL, (OWNER, 3), &[("choices", t("[1,1]"))], 8); // not distinct
    answer_q(&mut s, ANN, (OWNER, 4), &[("choices", t("[0,1]"))], 9); // several, not multi
    answer_q(&mut s, BEA, (OWNER, 4), &[("text", t("free?"))], 10); // no free text taken
    answer_q(&mut s, CAL, (OWNER, 5), &[("text", t("too long"))], 11); // over textMax
    answer_q(&mut s, MEMBER, (OWNER, 5), &[("choices", t("[0]"))], 12); // no options to choose
    let v = s.view();
    for q in questions(&v) {
        assert_eq!(q["answers"], json!({}), "{v}");
    }
}

#[test]
fn answer_question_lww_in_any_order_and_empty_clears() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[0]"))], 4);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[2]"))], 8);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[1]"))], 6);
    for v in [s.view(), s.view_reversed()] {
        assert_eq!(questions(&v)[0]["answers"][h(ANN)]["choices"], json!([2]), "{v}");
        assert_eq!(questions(&v)[0]["tally"], json!([0, 0, 1]));
    }
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[]"))], 9);
    assert!(questions(&s.view())[0]["answers"][h(ANN)].is_null(), "an empty answer clears");
}

/// "A later answer replaces it, an empty one clears it": choices "[]" on an options question and
/// text "" on a free one, at a higher gen, take the member's answer out of the view and the tally.
/// Both Members clients send exactly these to clear.
#[test]
fn answer_question_empty_clears() {
    let mut s = Site::new();
    poll(&mut s, OWNER, 3);
    define(&mut s, OWNER, &[("text", t("Seek?"))], 4);
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[0,1]"))], 5);
    answer_q(&mut s, BEA, (OWNER, 3), &[("choices", t("[1]"))], 6);
    answer_q(&mut s, ANN, (OWNER, 4), &[("text", t("a quiet room"))], 7);
    let v = s.view();
    assert_eq!(questions(&v)[0]["tally"], json!([1, 2, 0]), "{v}");
    answer_q(&mut s, ANN, (OWNER, 3), &[("choices", t("[]"))], 8);
    answer_q(&mut s, ANN, (OWNER, 4), &[("text", t(""))], 9);
    for v in [s.view(), s.view_reversed()] {
        let q = questions(&v);
        assert!(q[0]["answers"][h(ANN)].is_null(), "choices [] clears: {v}");
        assert_eq!(q[0]["tally"], json!([0, 1, 0]), "and leaves the tally");
        assert_eq!(q[0]["answers"][h(BEA)]["choices"], json!([1]), "another's stands");
        assert!(q[1]["answers"][h(ANN)].is_null(), "text \"\" clears");
    }
}

/// The one write path (`authoring::build`, the browser's and the phone's) holds the same
/// principals: a member answers and a stranger is refused by the fold; an admin sets a register
/// and asks a question, and a plain member is refused before any fold.
#[test]
fn the_write_path_holds_the_principals() {
    use pacific_core::authoring::{build, Ctx, Refusal};
    let s = Site::new();
    let log: Vec<(Delta, Id)> = s.log.iter().map(|(a, d)| (d.clone(), *a)).collect();
    let owners = [(0u64, OWNER)];
    let ctx = |me: Id| Ctx { me, epoch: EPOCH, members: &s.roster, owners: &owners, log: &log, watermark: Some(0) };
    let answer = args(&[("event", t(EVENT)), ("status", t("going")), ("at", i(1_000))]);
    assert!(build("group", op("group.rsvp"), answer.clone(), &ctx(MEMBER)).is_ok());
    let refused = build("group", op("group.rsvp"), answer, &ctx(STRANGER));
    assert!(matches!(refused, Err(Refusal::Reducer { .. })), "{refused:?}");
    let about = args(&[("bio", t("hello"))]);
    assert!(build("group", op("base.publishAbout"), about.clone(), &ctx(ANN)).is_ok());
    assert!(matches!(build("group", op("base.publishAbout"), about, &ctx(STRANGER)), Err(Refusal::Reducer { .. })));
    for (name, a) in [
        ("group.setRegistration", args(&[("event", t(EVENT)), ("capacity", i(3))])),
        ("base.defineQuestion", args(&[("text", t("Why here?"))])),
    ] {
        assert!(build("group", op(name), a.clone(), &ctx(ADMIN)).is_ok(), "{name}");
        let refused = build("group", op(name), a, &ctx(MEMBER));
        assert!(matches!(refused, Err(Refusal::OwnerOrRoleOnly { .. })), "{name}: {refused:?}");
    }
}

/// A Site none of this has touched reads empty in every new field.
#[test]
fn an_untouched_site_reads_empty() {
    let v = Site::new().view();
    for k in ["rsvps", "rsvp_counts", "registration", "about"] {
        assert_eq!(v[k], json!({}), "{k}: {v}");
    }
    assert_eq!(v["questions"], json!([]));
}
