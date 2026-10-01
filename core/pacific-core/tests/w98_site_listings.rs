//! W-98 Trade, T-7: a Site's Trade board. A member lists their own Thing on the Site
//! (`group.publishListing`), the latest by gen standing and `withdrawn` taking it down; the
//! owner or an admin takes one off for good (`group.removeListing`); the Site's own listing
//! (`site` 1, relation sold_through) is the owner's or an admin's alone.

use pacific_core::coordinator::{sequenced_delta, ArgVal, Args, Coordinator, GENESIS_PREV};
use pacific_core::group::{GroupRole, GroupState, GroupType, OP_PUBLISH_LISTING, OP_REMOVE_LISTING};
use pacific_core::object::{build_delta, MemberId, ObjectKind};

const OWNER: MemberId = [1u8; 32];
const ADMIN: MemberId = [2u8; 32];
const MEMBER: MemberId = [3u8; 32];

fn a(pairs: &[(&str, ArgVal)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn t(s: &str) -> ArgVal {
    ArgVal::Text(s.into())
}

struct Site(Coordinator<GroupType>, u64);

impl Site {
    fn new() -> Self {
        let mut c: Coordinator<GroupType> = Coordinator::new(vec![OWNER, ADMIN, MEMBER], OWNER);
        let d = sequenced_delta(ObjectKind::Group.type_id() as u32, pacific_core::roles::OP_SET_ROLE, pacific_core::roles::set_role_args(&ADMIN, GroupRole::Admin), 0, 0, GENESIS_PREV);
        c.deliver(d, OWNER).expect("the admin");
        Site(c, 1)
    }
    fn op(&mut self, op: u32, who: MemberId, mut args: Args) -> &mut Self {
        let gen = self.1;
        self.1 += 1;
        args.insert("gen".into(), ArgVal::Int(gen as i64));
        self.0.deliver(build_delta(ObjectKind::Group, op, args, 0, Some(gen)), who).expect("delivered");
        self
    }
    fn list(&mut self, who: MemberId, thing: &str, title: &str, extra: &[(&str, ArgVal)]) -> &mut Self {
        let mut args = a(&[("thingId", t(thing)), ("posture", t("selling")), ("title", t(title)), ("reach", t("private")), ("rev", ArgVal::Int(0)), ("price", t("£15"))]);
        for (k, v) in extra {
            args.insert(k.to_string(), v.clone());
        }
        self.op(OP_PUBLISH_LISTING, who, args)
    }
    fn state(&self) -> GroupState {
        self.0.state()
    }
}

fn thing(n: u8) -> String {
    hex::encode([n; 32])
}

#[test]
fn a_member_lists_their_thing_and_the_latest_stands() {
    let mut s = Site::new();
    s.list(MEMBER, &thing(9), "drill", &[]).list(MEMBER, &thing(9), "cordless drill", &[]);
    let st = s.state();
    assert_eq!(st.listings.len(), 1);
    assert_eq!(st.listings[&(MEMBER, thing(9))].title, "cordless drill");
    assert!(!st.listings[&(MEMBER, thing(9))].site);
    s.list(MEMBER, &thing(9), "cordless drill", &[("withdrawn", ArgVal::Int(1))]);
    assert!(s.state().listings.is_empty(), "withdrawn");
}

#[test]
fn the_owner_or_an_admin_takes_a_listing_off_for_good() {
    let mut s = Site::new();
    s.list(MEMBER, &thing(9), "drill", &[]);
    s.op(OP_REMOVE_LISTING, MEMBER, a(&[("author", t(&hex::encode(MEMBER))), ("thingId", t(&thing(9)))]));
    assert_eq!(s.state().listings.len(), 1, "a member cannot remove");
    s.op(OP_REMOVE_LISTING, ADMIN, a(&[("author", t(&hex::encode(MEMBER))), ("thingId", t(&thing(9)))]));
    assert!(s.state().listings.is_empty());
    s.list(MEMBER, &thing(9), "drill again", &[]);
    assert!(s.state().listings.is_empty(), "removed stays removed");
    s.list(MEMBER, &thing(8), "saw", &[]);
    assert_eq!(s.state().listings.len(), 1, "another Thing lists");
    s.op(OP_REMOVE_LISTING, OWNER, a(&[("author", t(&hex::encode(MEMBER))), ("thingId", t(&thing(8)))]));
    assert!(s.state().listings.is_empty(), "the owner removes too");
}

#[test]
fn the_sites_own_listing_is_the_owners_or_an_admins() {
    let mut s = Site::new();
    s.list(MEMBER, &thing(7), "tote", &[("site", ArgVal::Int(1))]);
    assert!(s.state().listings.is_empty(), "a member cannot list as the Site");
    s.list(ADMIN, &thing(7), "tote", &[("site", ArgVal::Int(1))]);
    assert!(s.state().listings[&(ADMIN, thing(7))].site);
}

#[test]
fn a_listing_is_the_models_shape() {
    let mut s = Site::new();
    s.list(MEMBER, "not-hex", "drill", &[]);
    s.list(MEMBER, &thing(9), " ", &[]);
    s.list(MEMBER, &thing(9), "drill", &[("posture", t("wants"))]);
    assert!(s.state().listings.is_empty(), "a bad id, an empty title, a price on a standing want");
}

/// THE UI'S FIXTURE IS CORE'S OWN SERIALISATION: a Site's listings as the group view serves
/// them (fold.rs: `listings`), written to app/web/webapp/trade.listings.json, which the
/// webapp's trade.test.mjs reads. Held here, so the two cannot spell a key differently.
/// `FIXTURE_WRITE=1` rewrites it.
#[test]
fn the_webapps_listing_fixture_is_cores_serialisation() {
    let mut s = Site::new();
    s.list(MEMBER, &thing(9), "A bike", &[("descriptor", t("blue, 56 cm")), ("area", t("gcpvj")), ("photo", t("cHJvYmU=")), ("photoMime", t("image/png"))]);
    s.list(ADMIN, &thing(7), "Tote", &[("site", ArgVal::Int(1))]);
    let st = s.state();
    let got = serde_json::to_string_pretty(&st.listings.values().collect::<Vec<_>>()).unwrap() + "\n";
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/web/webapp/trade.listings.json");
    if std::env::var("FIXTURE_WRITE").is_ok() {
        std::fs::write(path, &got).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap_or_default(), got, "trade.listings.json is not core's serialisation: FIXTURE_WRITE=1 rewrites it");
}
