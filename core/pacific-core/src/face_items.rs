//! face_items — what a Site's Host carries for its public Face: the Site's own events
//! and posts, as `host.hydrate` items (O-48; Ralph, 27 Sep: "The Face needs to be
//! hydrated. This happens via the Group <-> Host route").
//!
//! A SITE'S OWN IS DECLARED AT BOTH ENDS (reciprocity, ruled 25 Sep 2026): the Site's
//! `group.setAffiliation {rel: created}` and the object's own `base.setBacklink
//! {object: <site>, rel: created}`. An object named at one end only stays off the Face,
//! and is said so.
//!
//! AN ITEM'S PAYLOAD IS ITS SOURCE OP'S ARGS, by their ICD names: `event.setProfile`'s
//! under `event:<id>`, `post.setProfile`'s under `post:<id>`, so the fields the Face
//! editor offers (face.js reads them from the same ops) find their keys. Plus `at`, when
//! the Site named it, which orders publications. For a repeating event `startMs` is the
//! occurrence the Face shows: the next one, and `endMs` that occurrence's end.
//!
//! ONLY `network` REACHES THE FACE (NC-139; Ralph 30 Sep). An event or post at `private` or
//! `connections` is members only: it is left off, and whatever it wrote is withdrawn. The Host
//! copy is the one way out of MLS, so what does not belong outside never gets a key.
//!
//! A POST'S PUBLIC PAGE (W-98 Resources, Ralph 30 Sep): its whole body in chunks
//! (`post:<id>:body:<n>`), its images (`post:<id>:asset:<assetId>`) and its document
//! (`post:<id>:document`), each a MediaRef's args as JSON, with O-79's keys when detached. A
//! post retracted or leaving `network` withdraws every one: `plan` withdraws any `post:` key
//! that is not wanted.
//!
//! Pure. The Node folds the Site, its sources and the Host, and applies `plan`'s writes;
//! nothing here reads a store or a clock.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::backlink::Backlink;
use crate::event::EventState;
use crate::post::PostState;
use crate::system::{HydratedItem, MAX_HYDRATED_PAYLOAD};

/// The relation both ends declare.
pub const CREATED: &str = "created";
/// Events from three hours ago on, face-render's own cut.
pub const EVENT_GRACE_MS: i64 = 3 * 3600 * 1000;
/// The newest posts: the Face draws four rows at most, and the Host's log stays bounded.
pub const MAX_POSTS: usize = 8;
/// How far ahead a repeating event's next occurrence is looked for.
const RECUR_HORIZON_MS: i64 = 400 * 86_400_000;
/// Cut first when an item is over the cap, in this order. The title never is.
const CLIPPABLE: [&str; 3] = ["body", "descriptor", "lineup"];

/// `event.setProfile`'s args the Face carries, by ICD name (G-6). With [`EVENT_PRIVATE_ARGS`]
/// they are every arg the ICD declares, held so by the test below: a new arg fails until it is
/// named one or the other.
pub const EVENT_ARGS: [&str; 13] = [
    "title", "startMs", "endMs", "descriptor", "venue", "recurrence", "lineup", "ticketUrl",
    "tz", "status", "videoUrl", "descriptorFormat", "allDay",
];
/// `event.setProfile`'s args that never leave MLS: the join link is for those who hold the
/// event (G-6; Luma's and Partiful's location for guests only).
pub const EVENT_PRIVATE_ARGS: [&str; 1] = ["online"];
/// `post.setProfile`'s args, by ICD name.
pub const POST_ARGS: [&str; 6] = ["title", "body", "form", "link", "bodyFormat", "excerpt"];

pub enum Source<'a> {
    Event(&'a EventState),
    Post(&'a PostState),
}

/// Whether an event or post may be on the Face at all: `network` alone (NC-139).
fn public(source: &Source) -> bool {
    let v = match source {
        Source::Event(e) => e.visibility(),
        Source::Post(p) => p.visibility(),
    };
    v == crate::visibility::Visibility::Network
}

/// One object the Site names as `created`, with the time it named it.
pub struct Attached<'a> {
    pub id: &'a str,
    pub at: i64,
    pub source: Source<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Put { key: String, payload: String },
    /// A tombstone: `withdrawn`, with an empty payload.
    Withdraw { key: String },
}

/// What a pass left off that someone should hear about: `(object id, why)`.
pub type Left = Vec<(String, &'static str)>;

pub fn key_of(source: &Source, id: &str) -> String {
    match source {
        Source::Event(_) => format!("event:{id}"),
        Source::Post(_) => format!("post:{id}"),
    }
}

fn declares(backlinks: &BTreeMap<(String, String), Backlink>, site: &str) -> bool {
    backlinks.contains_key(&(CREATED.to_string(), site.to_string()))
}

fn put(m: &mut Map<String, Value>, k: &str, v: &str) {
    if !v.is_empty() {
        m.insert(k.into(), Value::String(v.into()));
    }
}

/// The payload fields, or why there are none: `Ok(None)` is outside the window or
/// withdrawn by its author, which is no one's to hear about.
fn fields(a: &Attached, now_ms: i64, halves: &BTreeSet<String>, venue_public: bool) -> Result<Option<Map<String, Value>>, &'static str> {
    let mut m = Map::new();
    if !public(&a.source) {
        return Ok(None);
    }
    match a.source {
        Source::Event(e) => {
            if e.title.trim().is_empty() {
                return Err("no title");
            }
            let from = now_ms - EVENT_GRACE_MS;
            let start = match crate::recurrence::Recurrence::parse(&e.recurrence) {
                Ok(r) if !e.recurrence.is_empty() => {
                    match r.occurrences(e.start_ms, from, now_ms + RECUR_HORIZON_MS, 1).first() {
                        Some(&t) => t,
                        None => return Ok(None),
                    }
                }
                _ => e.start_ms,
            };
            if start < from {
                return Ok(None);
            }
            put(&mut m, "title", &e.title);
            m.insert("startMs".into(), Value::from(start));
            if let Some(end) = e.end_ms {
                m.insert("endMs".into(), Value::from(end + (start - e.start_ms)));
            }
            put(&mut m, "descriptor", &e.descriptor);
            // The venue only where the Site's registration keeps it public (group.setRegistration's
            // `location`; "members" keeps it off the public copy, which is outside MLS).
            if venue_public {
                put(&mut m, "venue", &e.venue);
            }
            put(&mut m, "recurrence", &e.recurrence);
            put(&mut m, "lineup", &e.lineup.join("\n"));
            put(&mut m, "ticketUrl", &e.ticket_url);
            put(&mut m, "tz", &e.tz);
            put(&mut m, "status", &e.status);
            put(&mut m, "videoUrl", &e.video_url);
            put(&mut m, "descriptorFormat", &e.descriptor_format);
            if e.all_day {
                m.insert("allDay".into(), Value::from(1));
            }
            // ASSURANCE 2: an act goes out only confirmed from the writer's own view (the
            // performer's performs_at half held); an unconfirmed person's act stays in MLS.
            let acts = e.acts.as_deref().unwrap_or_default();
            let ok = crate::event::confirmed(acts, halves);
            let shown: Vec<crate::event::Act> = acts.iter().zip(ok).filter(|(_, c)| *c).map(|(a, _)| a.clone()).collect();
            if !shown.is_empty() {
                let mut v = crate::event::acts_view(&shown, halves);
                for act in v.as_array_mut().into_iter().flatten() {
                    act.as_object_mut().map(|o| o.remove("confirmed"));
                }
                m.insert("acts".into(), v);
            }
        }
        Source::Post(p) => {
            if p.retracted {
                return Ok(None);
            }
            if p.title.trim().is_empty() {
                return Err("no title");
            }
            put(&mut m, "title", &p.title);
            put(&mut m, "body", &p.body);
            put(&mut m, "form", p.form.as_str());
            put(&mut m, "link", &p.link);
            put(&mut m, "bodyFormat", &p.body_format);
            put(&mut m, "excerpt", &p.excerpt);
        }
    }
    m.insert("at".into(), Value::from(a.at));
    Ok(Some(m))
}

/// The payload as JSON within `MAX_HYDRATED_PAYLOAD`: the fold refuses a larger one and
/// never truncates, so it is cut here, `CLIPPABLE` first, halving at a char boundary.
fn fit(mut m: Map<String, Value>) -> Option<String> {
    for k in CLIPPABLE {
        loop {
            let s = Value::Object(m.clone()).to_string();
            if s.len() <= MAX_HYDRATED_PAYLOAD {
                return Some(s);
            }
            let Some(Value::String(v)) = m.get(k) else { break };
            let n = v.chars().count();
            if n == 0 {
                m.remove(k);
                break;
            }
            let cut: String = v.chars().take(n / 2).collect();
            m.insert(k.into(), Value::String(cut));
        }
    }
    let s = Value::Object(m).to_string();
    (s.len() <= MAX_HYDRATED_PAYLOAD).then_some(s)
}

/// `body` as the post page's `post:<id>:body:<n>` payloads, n from 0, read until a key is
/// absent or withdrawn (`plan` withdraws the tail of a longer body). Each is `{"text": piece}`,
/// a JSON object as every item's payload is, within `MAX_HYDRATED_PAYLOAD` bytes once escaped,
/// cut at a char boundary; the pieces joined are the body.
fn chunks(body: &str) -> Vec<String> {
    let json = |t: &str| serde_json::json!({ "text": t }).to_string();
    let mut out = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let mut end = rest.len().min(MAX_HYDRATED_PAYLOAD);
        loop {
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let over = json(&rest[..end]).len().saturating_sub(MAX_HYDRATED_PAYLOAD);
            if over == 0 {
                break;
            }
            end -= over.min(end - 1).max(1);
        }
        out.push(json(&rest[..end]));
        rest = &rest[end..];
    }
    out
}

/// A media ref's args as JSON under `prefix` (MediaRef::to_args), with `extra` beside them.
fn media_payload(m: &pacific_media::MediaRef, prefix: &str, extra: &[(&str, Value)]) -> String {
    let mut args = pacific_media::Args::new();
    m.to_args(prefix, &mut args);
    let mut o: Map<String, Value> = args
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                pacific_media::ArgVal::Text(t) => Value::String(t),
                pacific_media::ArgVal::Int(i) => Value::from(i),
            };
            (k, v)
        })
        .collect();
    for (k, v) in extra {
        o.insert(k.to_string(), v.clone());
    }
    Value::Object(o).to_string()
}

/// A network post's page beyond `post:<id>`: its body chunks, its assets and its document.
fn post_page(id: &str, p: &PostState, out: &mut BTreeMap<String, String>, left: &mut Left) {
    for (n, c) in chunks(&p.body).into_iter().enumerate() {
        out.insert(format!("post:{id}:body:{n}"), c);
    }
    let mut items: Vec<(String, String)> = p
        .assets
        .iter()
        .map(|(aid, a)| {
            let extra = [("id", Value::from(aid.as_str())), ("alt", Value::from(a.alt.as_str())), ("at", Value::from(a.at))];
            (format!("post:{id}:asset:{aid}"), media_payload(&a.media, "asset", &extra))
        })
        .collect();
    if let Some((m, name)) = &p.document {
        items.push((format!("post:{id}:document"), media_payload(m, "document", &[("name", Value::from(name.as_str()))])));
    }
    for (k, payload) in items {
        if payload.len() <= MAX_HYDRATED_PAYLOAD {
            out.insert(k, payload);
        } else {
            left.push((id.to_string(), "a picture or document too large for a Host item inline: detach it (O-79)"));
        }
    }
}

/// What the Host should carry for `site`: key → payload, and what was left off. No act goes
/// out: with no performer's half given, none is confirmed ([`desired_with`]).
pub fn desired(site: &str, attached: &[Attached], now_ms: i64) -> (BTreeMap<String, String>, Left) {
    desired_with(site, attached, now_ms, &BTreeMap::new(), &BTreeSet::new())
}

/// [`desired`], with what the Site's own state says: the performers whose `performs_at` half
/// the writer holds, per event id (`event::confirmed`'s `halves`), whose acts alone go on the
/// Face (ASSURANCE 2); and the events whose registration keeps the venue to members
/// (group.setRegistration {location: members}), whose venue does not.
pub fn desired_with(
    site: &str,
    attached: &[Attached],
    now_ms: i64,
    halves: &BTreeMap<String, BTreeSet<String>>,
    members_venue: &BTreeSet<String>,
) -> (BTreeMap<String, String>, Left) {
    let none = BTreeSet::new();
    let mut out = BTreeMap::new();
    let mut left = Left::new();
    let mut posts = 0;
    let mut order: Vec<&Attached> = attached.iter().collect();
    order.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(b.id)));
    for a in order {
        let backlinks = match a.source {
            Source::Event(e) => &e.backlinks,
            Source::Post(p) => &p.backlinks,
        };
        if !declares(backlinks, site) {
            left.push((a.id.to_string(), "the Site names it, and it does not name the Site"));
            continue;
        }
        let m = match fields(a, now_ms, halves.get(a.id).unwrap_or(&none), !members_venue.contains(a.id)) {
            Ok(Some(m)) => m,
            Ok(None) => continue,
            Err(why) => {
                left.push((a.id.to_string(), why));
                continue;
            }
        };
        if matches!(a.source, Source::Post(_)) {
            if posts == MAX_POSTS {
                continue;
            }
            posts += 1;
        }
        match fit(m) {
            Some(p) => {
                out.insert(key_of(&a.source, a.id), p);
                if let Source::Post(p) = a.source {
                    post_page(a.id, p, &mut out, &mut left);
                }
            }
            None => left.push((a.id.to_string(), "too large for a Host item with its text cut")),
        }
    }
    (out, left)
}

/// The writes that bring `held` (the Host's items, tombstones included) to `desired`: a
/// put where the payload is new or different, a tombstone for a live `event:` or `post:`
/// item no longer wanted. `face` and any other key are never touched.
pub fn plan(desired: &BTreeMap<String, String>, held: &BTreeMap<String, HydratedItem>) -> Vec<Write> {
    let mut w = Vec::new();
    for (k, p) in desired {
        match held.get(k) {
            Some(h) if !h.withdrawn && &h.payload == p => {}
            _ => w.push(Write::Put { key: k.clone(), payload: p.clone() }),
        }
    }
    for (k, h) in held {
        if (k.starts_with("event:") || k.starts_with("post:")) && !h.withdrawn && !desired.contains_key(k) {
            w.push(Write::Withdraw { key: k.clone() });
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::Form;

    const SITE: &str = "5117e5117e5117e5117e5117e5117e5117e5117e5117e5117e5117e5117e5117";
    const NOW: i64 = 1_790_000_000_000;
    const DAY: i64 = 86_400_000;

    fn backlinked(site: &str) -> BTreeMap<(String, String), Backlink> {
        let mut b = BTreeMap::new();
        b.insert((CREATED.into(), site.into()), Backlink { rel: CREATED.into(), object: site.into(), at: 1 });
        b
    }
    fn event(title: &str, start: i64) -> EventState {
        EventState { title: title.into(), start_ms: start, backlinks: backlinked(SITE), ..Default::default() }
    }
    fn post(title: &str) -> PostState {
        PostState { title: title.into(), backlinks: backlinked(SITE), ..Default::default() }
    }
    fn payload(d: &BTreeMap<String, String>, k: &str) -> Value {
        serde_json::from_str(&d[k]).unwrap()
    }
    fn held(key: &str, payload: &str, withdrawn: bool) -> (String, HydratedItem) {
        (key.into(), HydratedItem { key: key.into(), payload: payload.into(), fetched_at: 0, rev: 1, withdrawn, author: [7; 32] })
    }

    /// The keys a payload carries are the source op's args, by their ICD-0 names, and
    /// every one of those args is carried: face.js offers a field by that name, and a
    /// renamed arg must fail here, not on a Face.
    #[test]
    fn a_payload_carries_its_source_ops_args_by_their_icd_names() {
        let icd: Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json")).unwrap(),
        )
        .unwrap();
        // Every arg the ICD declares is carried or named private, and none is both (G-6).
        for (kind, op, ours, private) in [
            ("event", "event.setProfile", &EVENT_ARGS[..], &EVENT_PRIVATE_ARGS[..]),
            ("post", "post.setProfile", &POST_ARGS[..], &[][..]),
        ] {
            let mut theirs: Vec<&str> = icd["kinds"][kind]["ops"][op]["args"].as_object().unwrap().keys().map(|k| k.as_str()).collect();
            let mut mine: Vec<&str> = ours.iter().chain(private).copied().collect();
            theirs.sort();
            mine.sort();
            assert_eq!(mine, theirs, "{op}");
            assert!(ours.iter().all(|a| !private.contains(a)), "{op}: an arg is carried or private, not both");
        }

        let e = EventState {
            descriptor: "Bring gloves".into(),
            venue: "The shed".into(),
            lineup: vec!["A".into(), "B".into()],
            ticket_url: "https://example.org/t".into(),
            end_ms: Some(NOW + DAY + 2 * 3600 * 1000),
            ..event("Dig day", NOW + DAY)
        };
        let p = PostState { body: "Long read".into(), link: "https://example.org/p".into(), form: Form::Link, ..post("Notes") };
        let (d, left) = desired(SITE, &[Attached { id: "e1", at: 5, source: Source::Event(&e) }, Attached { id: "p1", at: 6, source: Source::Post(&p) }], NOW);
        assert!(left.is_empty(), "{left:?}");
        let ev = payload(&d, "event:e1");
        for k in ev.as_object().unwrap().keys().filter(|k| *k != "at") {
            assert!(EVENT_ARGS.contains(&k.as_str()), "{k}");
        }
        assert_eq!(ev["startMs"], NOW + DAY);
        assert_eq!(ev["endMs"], NOW + DAY + 2 * 3600 * 1000);
        assert_eq!(ev["lineup"], "A\nB");
        assert_eq!(ev["at"], 5);
        let po = payload(&d, "post:p1");
        assert_eq!((po["title"].as_str(), po["form"].as_str(), po["link"].as_str()), (Some("Notes"), Some("link"), Some("https://example.org/p")));
        for k in po.as_object().unwrap().keys().filter(|k| *k != "at") {
            assert!(POST_ARGS.contains(&k.as_str()), "{k}");
        }
    }

    #[test]
    fn an_object_named_at_one_end_only_is_left_off_and_said_so() {
        let mut e = event("Dig day", NOW + DAY);
        e.backlinks.clear();
        let other = EventState { backlinks: backlinked("0ther"), ..event("Elsewhere", NOW + DAY) };
        let (d, left) = desired(SITE, &[Attached { id: "e1", at: 1, source: Source::Event(&e) }, Attached { id: "e2", at: 1, source: Source::Event(&other) }], NOW);
        assert!(d.is_empty());
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|(_, why)| why.contains("does not name the Site")));
    }

    #[test]
    fn the_window_is_events_still_to_come_and_the_newest_posts() {
        let past = event("Long past", NOW - 40 * DAY);
        let recent = event("Just now", NOW - 3600 * 1000);
        let weekly = EventState { recurrence: "FREQ=WEEKLY".into(), end_ms: Some(NOW - 70 * DAY + 3600 * 1000), ..event("Every Sunday", NOW - 70 * DAY) };
        let posts: Vec<PostState> = (0..10).map(|i| post(&format!("Post {i}"))).collect();
        let ids: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        let mut attached = vec![
            Attached { id: "past", at: 1, source: Source::Event(&past) },
            Attached { id: "recent", at: 1, source: Source::Event(&recent) },
            Attached { id: "weekly", at: 1, source: Source::Event(&weekly) },
        ];
        for (i, p) in posts.iter().enumerate() {
            attached.push(Attached { id: &ids[i], at: i as i64, source: Source::Post(p) });
        }
        let (d, left) = desired(SITE, &attached, NOW);
        assert!(left.is_empty(), "{left:?}");
        assert!(!d.contains_key("event:past"), "an event that has been and gone is not on the Face");
        assert!(d.contains_key("event:recent"), "three hours' grace, as face-render");
        let next = payload(&d, "event:weekly")["startMs"].as_i64().unwrap();
        assert!(next >= NOW - EVENT_GRACE_MS && next < NOW + 8 * DAY, "a repeating event shows its next occurrence: {next}");
        assert_eq!(payload(&d, "event:weekly")["endMs"].as_i64(), Some(next + 3600 * 1000), "and that occurrence's end");
        let kept: Vec<&String> = d.keys().filter(|k| k.starts_with("post:")).collect();
        assert_eq!(kept.len(), MAX_POSTS);
        assert!(!d.contains_key("post:p0") && !d.contains_key("post:p1"), "the oldest two fall outside the newest eight");
    }

    #[test]
    fn a_retracted_post_is_withdrawn_and_an_untitled_object_is_said() {
        let gone = PostState { retracted: true, ..post("Was here") };
        let blank = post("  ");
        let (d, left) = desired(SITE, &[Attached { id: "gone", at: 2, source: Source::Post(&gone) }, Attached { id: "blank", at: 1, source: Source::Post(&blank) }], NOW);
        assert!(d.is_empty());
        assert_eq!(left, vec![("blank".to_string(), "no title")]);

        let held: BTreeMap<String, HydratedItem> = [held("post:gone", "{\"title\":\"Was here\"}", false)].into();
        assert_eq!(plan(&d, &held), vec![Write::Withdraw { key: "post:gone".into() }]);
    }

    #[test]
    fn an_item_over_the_cap_has_its_text_cut_and_a_title_that_cannot_fit_is_left_off() {
        let long = PostState { body: "é".repeat(20_000), ..post("A long read") };
        let (d, left) = desired(SITE, &[Attached { id: "long", at: 1, source: Source::Post(&long) }], NOW);
        assert!(left.is_empty());
        assert!(d["post:long"].len() <= MAX_HYDRATED_PAYLOAD);
        let p = payload(&d, "post:long");
        assert_eq!(p["title"], "A long read");
        assert!(!p["body"].as_str().unwrap().is_empty(), "cut, not dropped");

        let huge = post(&"t".repeat(MAX_HYDRATED_PAYLOAD));
        let (d, left) = desired(SITE, &[Attached { id: "huge", at: 1, source: Source::Post(&huge) }], NOW);
        assert!(d.is_empty());
        assert_eq!(left, vec![("huge".to_string(), "too large for a Host item with its text cut")]);
    }

    #[test]
    fn the_plan_writes_the_difference_and_nothing_when_nothing_moved() {
        let e = event("Dig day", NOW + DAY);
        let (d, _) = desired(SITE, &[Attached { id: "e1", at: 1, source: Source::Event(&e) }], NOW);
        let p = d["event:e1"].clone();

        let fresh = plan(&d, &BTreeMap::new());
        assert_eq!(fresh, vec![Write::Put { key: "event:e1".into(), payload: p.clone() }]);

        let same: BTreeMap<String, HydratedItem> = [held("event:e1", &p, false)].into();
        assert!(plan(&d, &same).is_empty(), "nothing moved, nothing written");

        let stale: BTreeMap<String, HydratedItem> = [held("event:e1", "{\"title\":\"Old\"}", false)].into();
        assert_eq!(plan(&d, &stale), fresh, "a changed payload is put again");

        let tomb: BTreeMap<String, HydratedItem> = [held("event:e1", "", true)].into();
        assert_eq!(plan(&d, &tomb), fresh, "a withdrawn item that is wanted again is put again");

        let other: BTreeMap<String, HydratedItem> = [
            held("face", "{}", false),
            held("ra:event:1", "{}", false),
            held("post:old", "{}", true),
        ]
        .into();
        assert_eq!(plan(&BTreeMap::new(), &other), vec![], "the face, another source's keys and a tombstone are left alone");
    }
}
