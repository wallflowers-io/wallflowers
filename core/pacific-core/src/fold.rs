//! The fold — one log of deltas, in order, through one lens.
//!
//! IT LIVED IN `archive.rs`, which is why a browser that wanted to draw a list of
//! names was shipped an entire backup blob to fold. The two have nothing to do with
//! each other: this is the reducer's own path — used by the directory
//! (`object_store::folded`), by every `*_state` reader, and by the wasm surfaces —
//! and the archive is a backup artefact that happens to call it.
//!
//! Nothing here knows what an `Archive` is, and it must stay that way.

use crate::coordinator::{decode_delta, Coordinator, Delta, ForumState};
use crate::event::EventState;
use crate::group::{GroupState, Presence};
use crate::object::{MemberId, ObjectKind, ObjectType};
use crate::CoreError;

/// THE FOLD. One log, many op-groups: a Contact channel carries message deltas
/// (Forum lens) and prekey deltas (Contact lens) on the same log, and the
/// Coordinator hard-rejects a foreign `type_id`, so filtering to the lens is
/// mandatory. Then sequenced before commutative, by epoch, by seq — the order the
/// spine was written in. This is the function `object_store::folded` runs; it lives
/// here so the archive and the directory fold through ONE implementation.
pub fn fold_entries<T: ObjectType>(
    members: Vec<MemberId>,
    owner: MemberId,
    log: impl IntoIterator<Item = (MemberId, Vec<u8>)>,
) -> Result<Coordinator<T>, CoreError> {
    fold_entries_owned::<T>(members, vec![(0, owner)], log)
}

/// [`fold_entries`] for an object whose owner changed with the epoch (§10.2): the one
/// fold path the phone, the browser and the archive all run.
pub fn fold_entries_owned<T: ObjectType>(
    members: Vec<MemberId>,
    owners: Vec<(u64, MemberId)>,
    log: impl IntoIterator<Item = (MemberId, Vec<u8>)>,
) -> Result<Coordinator<T>, CoreError> {
    let want = T::KIND.type_id() as u32;
    let mut entries: Vec<(Delta, MemberId)> = Vec::new();
    for (author, envelope) in log {
        let d = decode_delta(&envelope)?;
        if d.type_id == want {
            entries.push((d, author));
        }
    }
    entries.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
    let mut coord = Coordinator::<T>::with_owners(members, owners);
    for (delta, author) in entries {
        // WHICH DELTA, NOT JUST WHICH REJECTION. `DeltaRejection` is a closed
        // taxonomy with no payload — `UnknownType` alone tells an operator that
        // SOMETHING in a log of nine hundred deltas was refused, and nothing else.
        // That is the difference between "this device is a version behind" and
        // "this log is corrupt", and it is the difference `Node::object_compliance`
        // exists to report, so the position and the op id are attached here, where
        // they are still in hand. The scalars are copied before `deliver` consumes
        // the delta and the message is only built on the failing path, so a fold
        // that succeeds pays nothing for this.
        let (type_id, op_id, epoch, seq) = (delta.type_id, delta.op_id, delta.epoch, delta.seq);
        coord.deliver(delta, author).map_err(|r| {
            CoreError::Coordinator(format!(
                "delta rejected: {r:?} — type {type_id}, op {op_id}, epoch {epoch}, \
                 seq {seq:?}, under the {} lens",
                T::KIND.name()
            ))
        })?;
    }
    Ok(coord)
}

pub fn digest_of<'a>(
    kind: ObjectKind,
    envelopes: impl Iterator<Item = &'a [u8]>,
    members: &[MemberId],
) -> Result<[u8; 32], CoreError> {
    use sha2::{Digest, Sha256};
    let want = kind.type_id() as u32;
    let mut ids: Vec<[u8; 32]> = Vec::new();
    for envelope in envelopes {
        let d = decode_delta(envelope)?;
        if d.type_id == want {
            ids.push(d.id());
        }
    }
    ids.sort_unstable();
    let mut members = members.to_vec();
    members.sort_unstable();
    // The exact framing `object_store::fold_digest` has always used — it delegates
    // here now, so this is the one place the formula lives.
    let mut h = Sha256::new();
    h.update(b"pacific-fold:v1");
    h.update((ids.len() as u64).to_be_bytes());
    for id in &ids {
        h.update(id);
    }
    h.update((members.len() as u64).to_be_bytes());
    for m in &members {
        h.update(m);
    }
    Ok(h.finalize().into())
}

/// Which lens a directory kind folds through. `None` for kinds with no fold
/// (an `arc-tether` is a machine link with no user-visible state).
pub fn lens_for(kind: &str) -> Option<ObjectKind> {
    Some(match kind {
        "forum" => ObjectKind::Forum,
        // FORKED 24 September 2026. A conversation is its own GroupObject with its
        // own wire type (#26), not a forum wearing a different kind string. Before
        // the fork this arm said Forum while `authoring::LENSES` said Conversation,
        // so `authoring::build` wrote 26 and every fold filtered for 19 —
        // stored, and then invisible, because an empty fold is a valid fold.
        "conversation" => ObjectKind::Conversation,
        // A notebook (group of 1) and a shared note (group of N) carry the Group
        // op vocabulary, so they fold through the Group lens. A kind `lens_for`
        // does not name does not fold.
        "group" => ObjectKind::Group,
        // Both strings, one kind. See authoring::LENSES — `notebook` is kept
        // readable so objects already stored under it still fold.
        "note" | "notebook" => ObjectKind::Note,
        "event" => ObjectKind::Event,
        "place" => ObjectKind::Place,
        "project" => ObjectKind::Project,
        "thing" => ObjectKind::Thing,
        "post" => ObjectKind::Post,
        // THE OTHER TWO VOCABULARIES. `contact` is the prekey/fan-out op-group
        // that rides a connection's log, and `system` is internal — neither is a
        // kind a person mints, and both are kinds the door will WRITE. A writable
        // kind with no fold is the same drift the fork just closed, pointing the
        // other way: authored, stored, and unreadable. `m31` sweeps for it.
        // THE PAIRING CHANNEL, and only that (migrated 24 September 2026). The ICD
        // calls channel 25 "the 1:1 pairing channel: prekey pool + pairwise
        // fan-out. Not an entity itself" — the messages are a Conversation. Until
        // the migration this said Forum, so a connection's log carried two
        // vocabularies and its lens claimed it was a chat room.
        "connection" => ObjectKind::Contact,
        "system" => ObjectKind::System,
        // A Treasury folds with its OWN vocabulary, not the Group one it used to
        // ride: its ops are a kind table numbered from 0, and the `0xF004` band is
        // vacated. A kind `lens_for` does not name does not fold, so this line is
        // what makes a treasury readable.
        "treasury" => ObjectKind::Treasury,
        // A Site's public copy. Named here so it folds: a Host holds the only copy
        // of what was actually served.
        "host" => ObjectKind::Host,
        // One sale (W-98 Trade). Named here so it folds.
        "transaction" => ObjectKind::Transaction,
        _ => return None,
    })
}

// ── THE VIEWS ──────────────────────────────────────────────────────────────
//
// What each lens renders, as JSON. These lived in `core-wasm`, which meant the
// browser could see a folded object and the door could not — the door would have
// had to restate the model to answer the same question, and a second statement of
// the model is the error this tree is built to refuse. They fold here now, and
// `view_of` below is the one door onto them.


/// A JSON string literal, quotes and escapes included. Text in a log is the peer's
/// to write, so it never goes into a format string by hand.
pub(crate) fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// THE PARTS AN OBJECT IS MADE OF, in ONE shape for every kind that has them.
///
/// An array of `{part, role, at}`, because that is what every consumer reads. A
/// wholesale state serialization would hand the same field over as a map keyed by
/// object id and the page would draw no edge and say nothing about it — the
/// silent-difference failure again, in a field seven kinds now carry. A part's name
/// is not here: the part's own MLS GroupContext holds it.
fn parts_view(parts: &std::collections::BTreeMap<String, crate::object::PartRef>) -> serde_json::Value {
    serde_json::Value::Array(
        parts
            .iter()
            .map(|(part, p)| {
                let mut v = serde_json::json!({ "part": part, "role": p.role, "at": p.at });
                if let Some(c) = &p.choice {
                    v["choice"] = c.as_str().into();
                }
                v
            })
            .collect(),
    )
}

/// A Thing — a listing when it takes a posture. The face still is left out: a
/// base64 image is not a field a view should carry on every read.
fn thing_view(st: &crate::thing::ThingState) -> String {
    let mut photos: Vec<&crate::thing::ThingPhoto> = st.photos.values().collect();
    photos.sort_by_key(|p| (p.at, p.id.clone()));
    serde_json::json!({
        "name": st.name,
        "descriptor": st.descriptor,
        "category": st.category.as_str(),
        "condition": st.condition,
        "description": st.description,
        "photos": photos,
        "disposition": st.disposition,
        "posture": st.posture.map(|p| p.as_str()),
        "price": st.price,
        "deadline": st.deadline,
        "reach": st.reach.as_str(),
        "area": st.area,
        "parts": parts_view(&st.parts),
        "backlinks": crate::backlink::listed(&st.backlinks),
    })
    .to_string()
}

/// A pairing channel's view: the prekeys it carries, the cards it has been told, and the
/// link (`contact.setLink`) from `reader`'s side; `null` with no reader.
fn contact_view(st: &crate::contact::ContactState, reader: Option<&MemberId>) -> String {
    let profiles: Vec<serde_json::Value> = st
        .profiles
        .iter()
        .map(|(who, p)| serde_json::json!({
            "member": hex::encode(who), "display_name": p.display_name,
            "shape": p.shape.as_str(), "gen": p.gen,
        }))
        .collect();
    serde_json::json!({
        "prekeys_available": st.prekeys.available.len(),
        "prekeys_spent": st.prekeys.spent.len(),
        "profiles": profiles,
        "listings": st.listings.len(),
        "link": reader.map(|r| {
            let (mine, theirs, state) = st.link_from(r);
            serde_json::json!({ "mine": mine.map(u8::from), "theirs": theirs.map(u8::from), "state": state.as_str() })
        }),
    })
    .to_string()
}

/// The forum/conversation view: `detailed()`, the same projection the app binds to,
/// with reactions grouped by emoji and votes already resolved. A reaction is `[emoji,
/// count, mine]`, `mine` whether `reader` is among its reactors (NC-111): no reactor's key
/// is served. A message's picture is served by the keys `forum.post` carries it under.
fn messages_view(st: &ForumState, reader: Option<&MemberId>) -> String {
    let msgs: Vec<String> = st
        .detailed()
        .iter()
        .map(|m| {
            let reply_to = match m.reply_to {
                Some((author, gen)) => {
                    format!(r#"{{"author":"{}","gen":{}}}"#, hex::encode(author), gen)
                }
                None => "null".to_string(),
            };
            let reactions: Vec<String> = m
                .reactions
                .iter()
                .map(|(emoji, who)| format!("[{},{},{}]", jstr(emoji), who.len(), reader.is_some_and(|r| who.contains(r))))
                .collect();
            let mut media = pacific_media::Args::new();
            if let Some(r) = &m.media {
                r.to_args("media", &mut media);
            }
            let media: String = media
                .iter()
                .map(|(k, v)| match v {
                    pacific_media::ArgVal::Int(i) => format!(",{}:{i}", jstr(k)),
                    pacific_media::ArgVal::Text(t) => format!(",{}:{}", jstr(k), jstr(t)),
                })
                .collect();
            format!(
                r#"{{"author":"{}","gen":{},"text":{},"ts":{},"reply_to":{},"up":{},"down":{},"reactions":[{}]{}}}"#,
                hex::encode(m.author),
                m.gen,
                jstr(&m.text),
                m.ts,
                reply_to,
                m.up.len(),
                m.down.len(),
                reactions.join(","),
                media
            )
        })
        .collect();
    // The rooms hung off this forum (`base.setPart`), each its own object.
    format!(
        r#"{{"messages":[{}],"parts":{},"parent":{}}}"#,
        msgs.join(","),
        parts_view(&st.parts),
        serde_json::to_string(&st.parent).unwrap_or_else(|_| "null".into())
    )
}

/// The group view. `Presence::Unknown` is rendered "undeclared" — the state a
/// fresh group-of-one is in before `setPresence`, which is a real state and not a
/// missing value.
///
/// The last three keys are the base facets a Group-typed object carries:
/// `notebook`, `wallet` and `publication`. They were folded and then dropped
/// here — `GroupState` has held all three since the facets landed, and this
/// function emitted the first four fields only, so a browser holding a real
/// archive could not see a note or a euro of a treasury it was a member of.
fn group_view(st: &GroupState, ratify: &crate::coordinator::RatifyState, owner: &[u8; 32]) -> String {
    let presence = match st.presence {
        Presence::Unknown => "undeclared",
        Presence::OnPlatform(_) => "onPlatform",
        Presence::OffPlatform(_) => "offPlatform",
    };
    let roles: Vec<String> = st
        .member_roles
        .iter()
        .map(|(m, r)| format!(r#"["{}","{}"]"#, hex::encode(m), r.as_str()))
        .collect();
    // THE SITE'S EDGES, which GroupState folds and no view carried: its offices
    // (`group.setOffice`: who holds what, and whether they accepted), its
    // affiliations with other Groups (`group.setAffiliation`), and the parts it is
    // made of (`base.setPart`). Without them a page could draw a Site's roles and
    // not its band table, its webring or its guestbook (thedoor, 22 Sep).
    let offices: Vec<serde_json::Value> = st
        .offices
        .iter()
        .map(|(office, h)| serde_json::json!({
            "office": office, "holder": hex::encode(h.holder), "status": h.status.as_str(), "at": h.at,
        }))
        .collect();
    let affiliations: Vec<serde_json::Value> = st
        .affiliations
        .iter()
        .map(|(peer, a)| serde_json::json!({
            "peer": peer, "rel": a.rel.as_str(), "name": a.name, "tether": a.tether, "at": a.at,
        }))
        .collect();
    let parts = parts_view(&st.parts);
    // WHO HAS BEEN IN IT, AND SINCE WHEN. The MLS ratchet tree is authoritative
    // for who is in TODAY; this is the history the tree cannot keep, and it is
    // where a founder's own tenure lands — `object_new` authors `memberJoined`
    // through `record_founder`, beside the commit. Without it the view said
    // nothing at all about the people in an object: `roles` is an OVERLAY that is
    // empty until somebody authors `group.setMemberRole`, so a Site with a
    // founder and no appointments read as unpeopled.
    let membership: Vec<serde_json::Value> = st
        .membership
        .transitions
        .keys()
        .map(|who| {
            let spells: Vec<serde_json::Value> = st
                .membership
                .tenures(who)
                .iter()
                .map(|t| serde_json::json!({
                    "joined_at": t.joined_at,
                    "left_at": t.left_at,
                    "departure": t.departure.as_str(),
                }))
                .collect();
            serde_json::json!({
                "member": who,
                "present": st.membership.is_present(who),
                "tenures": spells,
            })
        })
        .collect();
    let handovers: Vec<serde_json::Value> = st
        .membership
        .handovers
        .iter()
        .map(|h| serde_json::json!({ "to": h.to, "by": hex::encode(h.by), "at": h.at }))
        .collect();

    // THE VERTEBRAE, as the fold has them. `group.joinedObject` is the only
    // enumeration of a person's graph a client can READ — the spine is sealed —
    // and a view that dropped it left every consumer to guess the graph from a
    // roster. `tag` addresses the sealed way-in entry, which is how a device that
    // holds the list but not the objects gets them back.
    let joined: Vec<serde_json::Value> = st
        .joined
        .iter()
        .map(|(object, j)| serde_json::json!({
            "object": object, "kind": j.kind, "arc": j.arc, "tag": j.tag,
            "at": j.at, "left_at": j.left_at,
        }))
        .collect();
    // THE MEMBERS' INTENTIONS (W-98 Members; Ralph, 30 Sep: "Their choice of community will be
    // used to publish their intention on the W-98 Members page"): each spent claim that chose,
    // as its member and the choice. Never the claim, which the kiosk minted, nor the share,
    // which is the gift's: only what the member chose, which every member reads in the log.
    let mut intentions: Vec<(String, &str)> = st
        .claims_picked
        .iter()
        .filter_map(|(claim, pick)| Some((hex::encode(st.claims_spent.get(claim)?), pick.choice.as_deref()?)))
        .collect();
    intentions.sort();
    let intentions: Vec<serde_json::Value> = intentions.into_iter().map(|(m, c)| serde_json::json!([m, c])).collect();
    format!(
        r#"{{"display_name":{},"shape":"{}","presence":"{}","roles":[{}],"offices":{},"affiliations":{},"parts":{},"membership":{},"handovers":{},"joined":{},"decisions":{},"publication":{},"face":{},"intentions":{}}}"#,
        jstr(&st.display_name),
        st.shape.as_str(),
        presence,
        roles.join(","),
        serde_json::Value::Array(offices),
        serde_json::Value::Array(affiliations),
        parts,
        serde_json::Value::Array(membership),
        serde_json::Value::Array(handovers),
        serde_json::Value::Array(joined),
        decisions_view(ratify, owner),
        publication_view(&st.publication),
        // THE FACE, parsed rather than escaped: the webapp reads `face.look` to theme
        // the community being viewed, and a JSON string holding JSON would make every
        // reader parse twice. The reducer already refused anything that is not an
        // object, so this cannot fail open — `null` means no face has been set.
        if st.face.is_empty() {
            "null".to_string()
        } else {
            st.face.clone()
        },
        serde_json::Value::Array(intentions)
    )
}

/// The event as the ICD carries it: the profile, and the three facets beside it
/// that `event.setProfile` / `setTickets` / `setMedia` set. Leaving the facets out
/// (as this view once did) drew an event with its lineup, its tickets and its
/// poster missing, and nothing said they had been dropped.
fn event_view(st: &EventState) -> String {
    let opt = |s: &Option<String>| s.as_deref().map(jstr).unwrap_or_else(|| "null".into());
    let lineup: Vec<String> = st.lineup.iter().map(|s| jstr(s)).collect();
    // `null`, not an empty object: an event with no listing sells nothing, and a
    // zeroed listing would read as a free one.
    let tickets = match &st.listing {
        None => "null".to_string(),
        Some(t) => format!(
            r#"{{"price_cents":{},"currency":{},"capacity":{},"open":{},"acct":{},"terms":{}}}"#,
            t.price_cents,
            jstr(&t.currency),
            t.capacity,
            t.open,
            opt(&t.acct),
            opt(&t.terms)
        ),
    };
    let m = &st.media;
    let media = if m.is_empty() {
        "null".to_string()
    } else {
        format!(
            r#"{{"banner":{},"banner_mime":{},"photos":{},"clip":{},"clip_mime":{}}}"#,
            jstr(&m.banner),
            jstr(&m.banner_mime),
            serde_json::to_string(&m.photos).unwrap_or_else(|_| "[]".into()),
            jstr(&m.clip),
            jstr(&m.clip_mime)
        )
    };
    let v = format!(
        r#"{{"title":{},"descriptor":{},"start_ms":{},"end_ms":{},"venue":{},"lineup":[{}],"recurrence":{},"tickets":{},"ticket_url":{},"media":{}}}"#,
        jstr(&st.title),
        jstr(&st.descriptor),
        st.start_ms,
        st.end_ms.map(|e| e.to_string()).unwrap_or_else(|| "null".into()),
        jstr(&st.venue),
        lineup.join(","),
        jstr(&st.recurrence),
        tickets,
        // `null` when there is no link, as `tickets` is when nothing is on sale here.
        if st.ticket_url.is_empty() { "null".to_string() } else { jstr(&st.ticket_url) },
        media
    );
    // W-98 (ICD 2.3.1): setProfile's new fields, the co-hosts, the visibility, the acts, and the
    // banner, clip and photos as their own registers, each winning over setMedia's where written.
    let mut v: serde_json::Value = serde_json::from_str(&v).expect("the event view is JSON");
    let or_null = |s: &str| if s.is_empty() { serde_json::Value::Null } else { s.into() };
    v["tz"] = st.tz.clone().into();
    v["online"] = or_null(&st.online);
    v["status"] = if st.status.is_empty() { "scheduled".into() } else { st.status.clone().into() };
    v["video_url"] = or_null(&st.video_url);
    v["descriptor_format"] = if st.descriptor_format.is_empty() { "plain".into() } else { st.descriptor_format.clone().into() };
    v["all_day"] = u8::from(st.all_day).into();
    v["roles"] = st.member_roles.iter().map(|(m, r)| (hex::encode(m), r.as_str().into())).collect::<serde_json::Map<_, _>>().into();
    v["visibility"] = st.visibility().as_str().into();
    v["acts"] = crate::event::acts_view(st.acts.as_deref().unwrap_or_default(), &Default::default());
    let legacy = |data: &str, mime: &str| if data.is_empty() { serde_json::Value::Null } else { serde_json::json!({ "mime": mime, "data": data, "width": 0, "height": 0 }) };
    v["banner"] = match &st.banner {
        Some(b) => crate::event::media_view(b),
        None => legacy(&m.banner, &m.banner_mime),
    };
    v["clip"] = match &st.clip {
        Some(c) => crate::event::media_view(c),
        None => legacy(&m.clip, &m.clip_mime),
    };
    v["photos"] = match &st.photo_set {
        Some(set) => {
            let mut ph: Vec<(&String, &crate::event::SetPhoto)> = set.iter().collect();
            ph.sort_by_key(|(id, p)| (p.at, (*id).clone()));
            ph.into_iter()
                .map(|(id, p)| {
                    let mut o = crate::event::media_view(&p.media);
                    o["id"] = id.clone().into();
                    o["at"] = p.at.into();
                    o
                })
                .collect::<Vec<_>>()
                .into()
        }
        None => m.photos.iter().map(|p| serde_json::json!({ "id": "", "mime": p.mime, "data": p.data, "width": 0, "height": 0, "at": 0 })).collect::<Vec<_>>().into(),
    };
    v.to_string()
}

/// WHAT A VIEW STATES AGAINST THE READER'S CLOCK, applied to `view_of`'s product as it is
/// read, never inside it: `view_of` is a fold's product and is cached with it (O-69), so a
/// clock read there freezes at the first read (TB4). A Thing's hold past its `until` reads
/// available; a deal's silence past its confirmWithin reads completed. Other kinds unchanged.
pub fn at_read(kind: &str, view: String, now_ms: i64) -> String {
    let apply: fn(&mut serde_json::Value, i64) = match lens_for(kind) {
        Some(ObjectKind::Thing) => crate::thing::at_read,
        Some(ObjectKind::Transaction) => crate::transaction::at_read,
        _ => return view,
    };
    match serde_json::from_str::<serde_json::Value>(&view) {
        Ok(mut v) => {
            apply(&mut v, now_ms);
            v.to_string()
        }
        Err(_) => view,
    }
}

/// ONE OBJECT, FOLDED AND RENDERED — the answer to "what does this object say",
/// for any surface. The kind chooses the lens ([`lens_for`]); the lens chooses the
/// view. A kind with a lens but no renderer says so rather than rendering empty.
/// `reader` is whose view it is: a room tells its reader which reactions are theirs.
pub fn view_of(
    kind: &str,
    owner: MemberId,
    members: Vec<MemberId>,
    owners: Vec<(u64, MemberId)>,
    log: impl IntoIterator<Item = (MemberId, Vec<u8>)>,
    reader: Option<&MemberId>,
) -> Result<String, CoreError> {
    use crate::contact::ContactType;
    use crate::coordinator::ForumType;
    use crate::event::EventType;
    use crate::group::GroupType;
    use crate::post::PostType;
    let lens = lens_for(kind).ok_or_else(|| {
        CoreError::Coordinator(format!("no lens for kind '{kind}'"))
    })?;
    let owners = if owners.is_empty() { vec![(0, owner)] } else { owners };
    let log: Vec<(MemberId, Vec<u8>)> = log.into_iter().collect();
    // Each member's own card (O-77), a departed member's too.
    let with_profiles = |view: String, profiles: &std::collections::BTreeMap<MemberId, crate::profiles::Profile>| -> Result<String, CoreError> {
        let mut v: serde_json::Value = serde_json::from_str(&view).map_err(|e| CoreError::Coordinator(e.to_string()))?;
        v["profiles"] = crate::profiles::view(profiles);
        Ok(v.to_string())
    };
    Ok(match lens {
        ObjectKind::Forum => {
            let st = fold_entries_owned::<ForumType>(members, owners, log)?.state();
            let mut v: serde_json::Value = serde_json::from_str(&with_profiles(messages_view(&st, reader), &st.profiles)?)
                .map_err(|e| CoreError::Coordinator(e.to_string()))?;
            // The room's description (forum.editDescription): null when none, or cleared.
            v["description"] = st.description.as_ref().map(|d| d.text.as_str()).filter(|t| !t.is_empty()).into();
            // THIS room's roles, as the group view names a Site's: who may edit it is the page's to know.
            v["roles"] = st.roles.iter().map(|(m, r)| serde_json::json!([hex::encode(m), r.as_str()])).collect();
            v.to_string()
        }
        // Its own type id, the same state: the ICD's word is that a conversation
        // "shares Forum's mechanics", which is about the reducer, not the object.
        ObjectKind::Conversation => messages_view(
            &fold_entries_owned::<crate::coordinator::ConversationType>(members, owners, log)?
                .state(),
            reader,
        ),
        ObjectKind::Group => {
            // One fold, read twice: the domain projection and the governance one
            // over the same log, orthogonal by construction.
            let roster = members.clone();
            let c = fold_entries_owned::<GroupType>(members, owners, log)?;
            let st = c.state();
            let mut v: serde_json::Value = serde_json::from_str(&with_profiles(group_view(&st, &c.ratify_state(), &owner), &st.profiles)?)
                .map_err(|e| CoreError::Coordinator(e.to_string()))?;
            // W-98: the answers to the Site's Events, their counts and registers (the ICD's
            // `view` lines for group.rsvp, setRegistration and rsvpDecide), and the about and
            // questions facets, from roster members only.
            let (rsvps, rsvp_counts, registration) = crate::rsvp::view(&st.rsvps);
            v["rsvps"] = rsvps;
            v["rsvp_counts"] = rsvp_counts;
            v["registration"] = registration;
            v["about"] = crate::about::view(&st.about, &roster);
            v["questions"] = crate::questions::view(&st.questions, &roster);
            // The group's own card (group.setProfile): on the self record, the person's own
            // name and picture, as they see them (webapp REQUIREMENTS § 3). A card never written
            // is null, not an object of empty fields claiming one.
            v["card"] = if st.card == crate::group::ContactCard::default() {
                serde_json::Value::Null
            } else {
                serde_json::to_value(&st.card).map_err(|e| CoreError::Coordinator(e.to_string()))?
            };
            // The Trade board (group.publishListing): the live listings, by (lister, Thing).
            v["listings"] = serde_json::to_value(st.listings.values().collect::<Vec<_>>()).map_err(|e| CoreError::Coordinator(e.to_string()))?;
            v.to_string()
        }
        ObjectKind::Event => {
            let st = fold_entries_owned::<EventType>(members, owners, log)?.state();
            let mut v: serde_json::Value = serde_json::from_str(&event_view(&st))
                .map_err(|e| CoreError::Coordinator(e.to_string()))?;
            v["parts"] = parts_view(&st.parts);
            v["backlinks"] = crate::backlink::listed(&st.backlinks);
            v.to_string()
        }
        ObjectKind::Thing => {
            thing_view(&fold_entries_owned::<crate::thing::ThingType>(members, owners, log)?.state())
        }
        // The same view the wallet facet rendered when it hung off a Group, now on
        // the object that owns it. `wallet_view` is unchanged: what moved is whose
        // state it reads, not how a treasury is drawn.
        ObjectKind::Treasury => wallet_view(
            &fold_entries_owned::<crate::treasury::TreasuryType>(members, owners, log)?.state(),
        ),
        // The same view the note facet rendered on a Group, now on the object that
        // owns it. `notebook_view` is unchanged.
        ObjectKind::Note => notebook_view(
            &fold_entries_owned::<crate::note::NoteType>(members, owners, log)?.state(),
        ),
        // A sale, as the fold leaves it; completion by silence is the reader's clock's
        // (`at_read`).
        ObjectKind::Transaction => crate::transaction::view(
            &fold_entries_owned::<crate::transaction::TransactionType>(members, owners, log)?.state(),
        )
        .to_string(),
        // What the Arc folds and serves: the name, the address, the pictures, and the
        // live items. The face bundle is `items.face`, carried verbatim.
        ObjectKind::Host => {
            let st = fold_entries_owned::<crate::host::HostType>(members, owners, log)?.state();
            serde_json::json!({
                "name": st.name,
                "parent": st.parent,
                "publication": serde_json::from_str::<serde_json::Value>(&publication_view(&st.publication))
                    .unwrap_or(serde_json::Value::Null),
                "media": st.media.iter().map(|(k, m)| (k.clone(),
                    serde_json::json!({ "mime": m.mime, "data": m.data }))).collect::<serde_json::Map<_, _>>(),
                "items": st.live().iter().map(|i| serde_json::json!({
                    "key": i.key, "payload": i.payload, "rev": i.rev, "fetchedAt": i.fetched_at
                })).collect::<Vec<_>>(),
            }).to_string()
        }
        // What each is, and the parts it is made of. The rest of their state is not
        // rendered yet, and is absent rather than guessed.
        ObjectKind::Place => {
            let st = fold_entries_owned::<crate::place::PlaceType>(members, owners, log)?.state();
            serde_json::json!({ "name": st.name, "descriptor": st.descriptor,
                                "parts": parts_view(&st.parts),
                                "backlinks": crate::backlink::listed(&st.backlinks) }).to_string()
        }
        ObjectKind::Project => {
            let st = fold_entries_owned::<crate::project::ProjectType>(members, owners, log)?.state();
            serde_json::json!({ "title": st.title, "headline": st.headline, "goal": st.goal,
                                "parts": parts_view(&st.parts) }).to_string()
        }
        ObjectKind::Post => {
            let st = fold_entries_owned::<PostType>(members, owners, log)?.state();
            let mut v = serde_json::to_value(&st)
                .map_err(|e| CoreError::Coordinator(e.to_string()))?;
            v["parts"] = parts_view(&st.parts);
            // W-98 Resources (ICD 2.3.1): the document, the body's images, the visibility.
            let (document, assets) = crate::post::media_view(&st);
            v["document"] = document;
            v["assets"] = assets;
            v["visibility"] = st.visibility().as_str().into();
            if st.body_format.is_empty() {
                v["body_format"] = "plain".into();
            }
            v.to_string()
        }
        // THE PAIRING CHANNEL, rendered as what it holds: the prekey pool each side
        // stocked for the other, and the card each side published. Not messages —
        // those are the conversation's. A channel drawing as "unsupported" was a
        // connection saying nothing about itself.
        ObjectKind::Contact => {
            contact_view(&fold_entries_owned::<ContactType>(members, owners, log)?.state(), reader)
        }
        // A lens exists, so the fold and the digest are real; only the rendering
        // is not built, and it says so instead of drawing an empty object.
        _ => r#"{"unsupported":true}"#.to_string(),
    })
}


/// The notebook — THE SITE'S KNOWLEDGE BASE, as the fold already answers it.
///
/// One row per note, and the row is [`NoteBook::current`]'s winner. Which of two
/// co-authored versions IS the document is the fold's decision, and re-deciding
/// it in JavaScript would be the defect `delta_id` exists to prevent, wearing a
/// different hat. `versions` carries the count instead, so a surface can say a
/// note is co-edited without this emitting every draft.
///
/// A note whose every entry is retracted has no current winner and is ABSENT,
/// which is [`NoteBook::live`]'s own semantics — not emitted as an empty row,
/// because a blank note and a withdrawn one are different statements.
fn notebook_view(book: &crate::note::NoteBook) -> String {
    let notes: Vec<String> = book
        .note_ids()
        .iter()
        .filter_map(|id| {
            let cur = book.current(id)?;
            let versions = book.versions(id);
            // The author is on the KEY, not the entry, so it is recovered by
            // identity against the winner rather than by re-running the choice.
            let by = versions
                .iter()
                .find(|(_, e)| std::ptr::eq(*e, cur))
                .map(|(who, _)| hex::encode(who))
                .unwrap_or_default();
            Some(format!(
                r#"{{"note":{},"title":{},"text":{},"at":{},"created":{},"source":{},"by":"{}","versions":{}}}"#,
                jstr(&cur.note),
                jstr(&cur.title),
                jstr(&cur.text),
                cur.at,
                cur.created,
                jstr(&cur.source),
                by,
                versions.len()
            ))
        })
        .collect();
    let comments: Vec<String> = book
        .comments
        .iter()
        .map(|((who, _), c)| {
            format!(
                r#"{{"note":{},"comment":{},"text":{},"at":{},"by":"{}"}}"#,
                jstr(&c.note), jstr(&c.comment), jstr(&c.text), c.at, hex::encode(who)
            )
        })
        .collect();
    // An EMPTY emoji is the cleared state, and it is carried rather than
    // filtered: the row says "this member has cleared their reaction", which a
    // missing row cannot say.
    let reactions: Vec<String> = book
        .reactions
        .iter()
        .map(|((who, _), r)| {
            format!(
                r#"{{"note":{},"emoji":{},"at":{},"by":"{}"}}"#,
                jstr(&r.note), jstr(&r.emoji), r.at, hex::encode(who)
            )
        })
        .collect();
    format!(
        r#"{{"notes":[{}],"comments":[{}],"reactions":[{}]}}"#,
        notes.join(","), comments.join(","), reactions.join(",")
    )
}

/// The wallet — THE SITE'S WALLET.
///
/// A CLOSED wallet emits `null`, never an object with zeroes in it. `group.rs`
/// states the rule on the field itself: a group that has not opened a treasury
/// is a different thing from one holding nothing, and the two must not render
/// the same.
///
/// `balance` is emitted although it is derived, because [`Wallet::balance`] is
/// where the rule "deposits less settlements" lives. A reader that summed the
/// two arrays itself would be a second implementation of a money rule, which is
/// the arithmetic equivalent of the JS reducer this crate exists to retire.
///
/// WHAT IS NOT HERE, and the absence is load-bearing: the DECISIONS. A release
/// is a RATIFY proposal on this same log, folded by the Coordinator, not by the
/// wallet facet — `wallet.rs` opens by saying so. Nothing downstream may read
/// an empty decision list as "these payments were unauthorised"; it means this
/// view does not carry decisions at all.
fn wallet_view(w: &crate::wallet::Wallet) -> String {
    if !w.open {
        return "null".to_string();
    }
    let bands: Vec<String> = w
        .bands
        .iter()
        .map(|b| {
            format!(
                r#"{{"ceiling":{},"rule":"{}","quorum":{}}}"#,
                b.ceiling.map(|c| c.to_string()).unwrap_or_else(|| "null".into()),
                b.rule.as_str(),
                b.quorum
            )
        })
        .collect();
    let deposits: Vec<String> = w
        .deposits
        .iter()
        .map(|(reference, d)| {
            format!(
                r#"{{"reference":{},"amount":{},"at":{},"source":{},"by":"{}"}}"#,
                jstr(reference), d.amount, d.at, jstr(&d.source), hex::encode(d.by)
            )
        })
        .collect();
    let settlements: Vec<String> = w
        .settlements
        .iter()
        .map(|(reference, s)| {
            format!(
                r#"{{"reference":{},"amount":{},"at":{},"proposal":{},"memo":{},"by":"{}"}}"#,
                jstr(reference),
                s.amount,
                s.at,
                s.proposal.as_deref().map(jstr).unwrap_or_else(|| "null".into()),
                jstr(&s.memo),
                hex::encode(s.by)
            )
        })
        .collect();
    let counts: Vec<String> = w
        .counts
        .iter()
        .map(|(who, c)| {
            format!(
                r#"{{"by":"{}","amount":{},"at":{}}}"#,
                hex::encode(who), c.amount, c.at
            )
        })
        .collect();
    format!(
        r#"{{"open":true,"currency":{},"account":{},"disclosure":"{}","cooloff_hours":{},"balance":{},"bands":[{}],"deposits":[{}],"settlements":[{}],"counts":[{}]}}"#,
        jstr(&w.currency),
        jstr(&w.account),
        w.disclosure.as_str(),
        w.cooloff_hours,
        w.balance(),
        bands.join(","),
        deposits.join(","),
        settlements.join(","),
        counts.join(",")
    )
}

/// The DECISIONS on this log — RATIFY, which is the OTHER HALF of the wallet.
///
/// Carried because of what its absence does downstream, not for completeness. A
/// settlement names the proposal it discharges, so a reader holding the payments
/// and not the decisions concludes "paid with no passed proposal" about a NAMED
/// member, on every one of them. A false accusation manufactured by a mapping is
/// precisely the lie-with-a-UI this tree refuses, so the fold hands over both
/// halves or neither.
///
/// `outcome` is DERIVED at read time from the frozen tally and never stored —
/// `RatifyState::outcome`'s own rule, and the reason the owner has to be passed
/// in: roles do not live in the ratify fold.
fn decisions_view(r: &crate::coordinator::RatifyState, owner: &[u8; 32]) -> String {
    let rows: Vec<String> = r
        .proposals
        .iter()
        .map(|(pid, p)| {
            let ballots: Vec<String> = r
                .votes
                .get(pid)
                .map(|v| {
                    v.iter()
                        .map(|(who, b)| {
                            format!(r#"{{"by":"{}","ballot":"{}"}}"#, hex::encode(who), b.as_str())
                        })
                        .collect()
                })
                .unwrap_or_default();
            format!(
                r#"{{"id":"{}:{}","proposer":"{}","gen":{},"payload":{},"rule":"{}","outcome":"{}","closed":{},"ballots":[{}]}}"#,
                hex::encode(pid.0),
                pid.1,
                hex::encode(p.proposer),
                p.gen,
                jstr(&p.payload),
                p.rule.as_str(),
                r.outcome(pid, owner).as_str(),
                r.closed.contains_key(pid),
                ballots.join(",")
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

/// The public address — WHAT IS SERVED BEFORE ANYONE SIGNS IN, and who serves
/// it. `null` when unpublished, on [`Publication::is_published`]'s own test:
/// both halves must hold, since a slug with no publisher is an address nobody
/// answers.
fn publication_view(p: &crate::publication::Publication) -> String {
    match (p.is_published(), p.publisher) {
        (true, Some(publisher)) => format!(
            r#"{{"slug":{},"publisher":"{}"}}"#,
            jstr(&p.slug),
            hex::encode(publisher)
        ),
        _ => "null".to_string(),
    }
}
