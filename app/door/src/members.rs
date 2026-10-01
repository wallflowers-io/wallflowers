//! GET /v2/members (W-98): a Site's members as people are shown them, each with their own
//! about text, their answers to the Site's questions and their intentions, and the Site's
//! questions with their tallies. ONE route for both clients: a registered site's token is the
//! Site (`GET /v2/members`), the WallFlowers webapp names it (`GET /v2/members/<site id>`)
//! (Ralph, 30 Sep: "This will be a common route with egregore").
//!
//! Read from the graph the session already answers (`/v2/graph`, cut to the token's scope), so it
//! shows nothing a member could not read there: a presentation of the Site's own view, never a
//! second source. Writes stay on `/v2/apply` (base.publishAbout, base.defineQuestion,
//! base.retireQuestion, base.answerQuestion; ICD 2.3.1 draft, facets `about` and `questions`).

use serde_json::{json, Value};

/// The Site's members, their about, answers and intentions, and its questions, for `me` (bare
/// hex). `objects` is `/v2/graph`'s `objects`.
pub fn members_of(objects: &[Value], site: &str, me: &str) -> Result<Value, &'static str> {
    let group = objects
        .iter()
        .find(|o| o["id"] == site && o["kind"] == "group")
        .ok_or("no such Site in reach")?;
    if group["folds"] == false {
        return Err("this Site cannot be read here");
    }
    let view = &group["view"];
    let owner = group["owner"].as_str().unwrap_or_default();
    let role_of = |key: &str, role: &str| {
        view["roles"].as_array().into_iter().flatten().any(|r| r[0] == key && r[1] == role)
    };
    // THE ARC IS NOT A PERSON: the role `admitter` is its always-on node's, which may add a
    // member and record the Add and nothing else (ICD); BUILD's reading, 1 Oct, for Ralph to
    // overrule. The owner keeps their place whatever else they hold.
    let shown = |key: &str| key == owner || !role_of(key, "admitter");
    let roster: Vec<&str> = group["members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|k| shown(k))
        .collect();
    let on = |key: &str| roster.contains(&key);

    // A card wherever the graph folded one: the Site's own first, then any room's, which also
    // carries members who have left it (the same merge the clients make).
    let card = |key: &str| {
        std::iter::once(group)
            .chain(objects.iter().filter(|o| o["id"] != site))
            .find_map(|o| o["view"]["profiles"].get(key).filter(|c| c.is_object()))
            .cloned()
    };
    let questions: Vec<&Value> = view["questions"].as_array().map(|q| q.iter().collect()).unwrap_or_default();
    let id_of = |q: &Value| format!("{}:{}", q["author"].as_str().unwrap_or_default(), q["gen"]);

    let mut members: Vec<Value> = roster
        .iter()
        .map(|&key| {
            let card = card(key);
            let name = card.as_ref().and_then(|c| c["name"].as_str()).map(str::trim).filter(|n| !n.is_empty());
            let role = if key == owner { "owner" } else if role_of(key, "admin") { "admin" } else { "member" };
            let about = view["about"].get(key).filter(|a| a.is_object()).map(|a| json!({ "bio": a["bio"], "links": a["links"] }));
            let answers: serde_json::Map<String, Value> = questions
                .iter()
                .filter_map(|q| {
                    let a = q["answers"].get(key).filter(|a| a.is_object())?;
                    Some((id_of(q), json!({ "text": a["text"], "choices": a["choices"] })))
                })
                .collect();
            let mut intentions: Vec<&str> = Vec::new();
            for pair in view["intentions"].as_array().into_iter().flatten() {
                if let (Some(who), Some(choice)) = (pair[0].as_str(), pair[1].as_str()) {
                    if who == key && !intentions.contains(&choice) {
                        intentions.push(choice);
                    }
                }
            }
            json!({
                "key": key,
                "name": name,
                "icon": card.as_ref().map(|c| c["icon"].clone()).unwrap_or(Value::Null),
                "role": role,
                "about": about,
                "answers": answers,
                "intentions": intentions,
            })
        })
        .collect();
    // The owner, the admins, then everyone else, each by name; the nameless last, and a tie by key.
    let rank = |m: &Value| match m["role"].as_str() {
        Some("owner") => 0,
        Some("admin") => 1,
        _ => 2,
    };
    members.sort_by(|x, y| {
        let name = |m: &Value| m["name"].as_str().map(str::to_lowercase);
        rank(x)
            .cmp(&rank(y))
            .then_with(|| match (name(x), name(y)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| x["key"].as_str().cmp(&y["key"].as_str()))
    });

    let questions: Vec<Value> = questions
        .iter()
        .map(|q| {
            json!({
                "id": id_of(q),
                "text": q["text"],
                "hint": q["hint"],
                "options": q["options"],
                "multi": q["multi"] == true,
                "max": q["max"],
                "free": q["free"] == true,
                "textMax": q["textMax"],
                "retired": q["retired"] == true,
                "tally": q["tally"],
            })
        })
        .collect();
    let can_define = on(me) && (me == owner || role_of(me, "admin"));
    Ok(json!({ "site": site, "me": me, "can_define": can_define, "members": members, "questions": questions }))
}

/// THE SITES A WILD CLIENT MAY BE GIVEN (R3.1 (C)): of `graph` (`/v2/graph`, the person's own,
/// uncut), each Site that folds, with `me` (bare hex) on its roster as its owner or holding `admin`
/// in its view's roles: `(id, name)`, by name, then id. A Site as the webapp draws one: a group
/// that is neither the self record (the spine's first) nor a part of another object.
pub fn sites_run_by(graph: &Value, me: &str) -> Vec<(String, String)> {
    let objects = graph["objects"].as_array().map(Vec::as_slice).unwrap_or_default();
    let spine = graph["spine"].as_array().into_iter().flatten();
    let own = spine.min_by_key(|e| e["index"].as_u64().unwrap_or(u64::MAX)).map(|e| e["object"].clone());
    let part = |id: &Value| objects.iter().any(|o| o["view"]["parts"].as_array().into_iter().flatten().any(|p| p["part"] == *id));
    let mut out: Vec<(String, String)> = objects
        .iter()
        .filter(|o| o["kind"] == "group" && o["folds"] == true && Some(&o["id"]) != own.as_ref() && !part(&o["id"]))
        .filter(|o| o["members"].as_array().is_some_and(|m| m.iter().any(|k| k == me)))
        .filter(|o| o["owner"] == me || o["view"]["roles"].as_array().into_iter().flatten().any(|r| r[0] == me && r[1] == "admin"))
        .filter_map(|o| {
            let id = o["id"].as_str()?.to_string();
            let name = [&o["name"], &o["view"]["display_name"]].iter().find_map(|n| n.as_str().map(str::trim).filter(|n| !n.is_empty())).unwrap_or(&id).to_string();
            Some((id, name))
        })
        .collect();
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: char) -> String {
        c.to_string().repeat(64)
    }

    /// A Site as a member's session folds it: owner a, admin b, members c, d and g, the Arc e
    /// (role admitter), f who has left. A room carries g's card.
    fn site(extra: Value) -> Vec<Value> {
        let (a, b, c, d, e, f, g) = (k('a'), k('b'), k('c'), k('d'), k('e'), k('f'), k('9'));
        let mut view = json!({
            "display_name": "Egregore's Echoes",
            "roles": [[b, "admin"], [e, "admitter"]],
            "parts": [],
            "profiles": {
                a.clone(): { "name": "Ralph", "icon": null, "gen": 1 },
                b.clone(): { "name": "Antoine", "icon": { "mime": "image/webp", "data": "AAAA" }, "gen": 2 },
                d.clone(): { "name": "hana", "icon": null, "gen": 1 },
                e.clone(): { "name": "wallflowers_system", "icon": null, "gen": 1 }
            },
            "about": {
                c.clone(): { "bio": "I make things", "links": ["https://example.org"], "gen": 4 },
                e.clone(): { "bio": "system", "links": [], "gen": 1 },
                f.clone(): { "bio": "gone", "links": [], "gen": 2 }
            },
            "questions": [
                {
                    "author": a, "gen": 7, "text": "What do you offer?", "hint": "Up to four",
                    "options": ["TECH / AI", "MAKING"], "multi": true, "max": 2, "free": false, "textMax": null,
                    "retired": false,
                    "answers": {
                        c.clone(): { "text": null, "choices": [0, 1], "gen": 9 },
                        e.clone(): { "text": null, "choices": [0], "gen": 9 },
                        f.clone(): { "text": null, "choices": [1], "gen": 8 }
                    },
                    "tally": [1, 2]
                },
                { "author": b, "gen": 11, "text": "What do you seek?", "options": null, "multi": false, "free": true,
                  "textMax": 180, "retired": true, "answers": { d.clone(): { "text": "Soil", "choices": [], "gen": 12 } }, "tally": null }
            ],
            "intentions": [[c, "financial"], [d, "skills"], [d, "skills"], [e, "resources"], [f, "resources"]]
        });
        if let Some(o) = extra.as_object() {
            for (key, v) in o {
                view[key] = v.clone();
            }
        }
        vec![
            json!({ "id": k('5'), "kind": "group", "name": "Egregore's Echoes", "owner": a, "members": [d, c, b, a, e, g], "folds": true, "view": view }),
            json!({ "id": k('7'), "kind": "forum", "name": "A room", "owner": a, "members": [a, g], "folds": true,
                    "view": { "profiles": { g.clone(): { "name": "Zed", "icon": null, "gen": 1 } } } }),
        ]
    }

    fn answer(me: &str) -> Value {
        members_of(&site(json!({})), &k('5'), me).expect("the Site's members")
    }

    fn member(out: &Value, key: &str) -> Value {
        out["members"].as_array().unwrap().iter().find(|m| m["key"] == key).cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn the_owner_then_the_admins_then_everyone_by_name_and_never_the_arc() {
        let out = answer(&k('c'));
        let keys: Vec<&str> = out["members"].as_array().unwrap().iter().map(|m| m["key"].as_str().unwrap()).collect();
        assert_eq!(keys, [k('a'), k('b'), k('d'), k('9'), k('c')], "owner, admin, hana and Zed by name, then the nameless");
        let roles: Vec<&str> = out["members"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["owner", "admin", "member", "member", "member"]);
        assert!(!out.to_string().contains(&k('e')), "the Arc (admitter) is nowhere: not a member, an about, an answer or an intention");
        assert_eq!((out["site"].clone(), out["me"].clone()), (json!(k('5')), json!(k('c'))));
    }

    #[test]
    fn a_member_is_named_by_their_card_wherever_it_is_folded_and_a_key_is_never_a_name() {
        let out = answer(&k('c'));
        assert_eq!(member(&out, &k('b'))["name"], "Antoine");
        assert_eq!(member(&out, &k('b'))["icon"], json!({ "mime": "image/webp", "data": "AAAA" }));
        assert_eq!(member(&out, &k('9'))["name"], "Zed", "a card a room folded");
        assert_eq!(member(&out, &k('c'))["name"], Value::Null, "no card: null, and the client says 'New member'");
    }

    #[test]
    fn each_member_carries_their_own_about_answers_and_intentions_and_nobody_off_the_roster_shows() {
        let out = answer(&k('c'));
        let c = member(&out, &k('c'));
        assert_eq!(c["about"], json!({ "bio": "I make things", "links": ["https://example.org"] }));
        assert_eq!(c["answers"], json!({ format!("{}:7", k('a')): { "text": null, "choices": [0, 1] } }));
        assert_eq!(c["intentions"], json!(["financial"]));
        let d = member(&out, &k('d'));
        assert_eq!(d["about"], Value::Null);
        assert_eq!(d["answers"], json!({ format!("{}:11", k('b')): { "text": "Soil", "choices": [] } }));
        assert_eq!(d["intentions"], json!(["skills"]), "one label per choice");
        assert_eq!(member(&out, &k('a'))["intentions"], json!([]));
        assert!(!out.to_string().contains(&k('f')), "someone who has left shows nowhere");
    }

    #[test]
    fn the_questions_carry_their_id_their_form_and_the_folds_tally_oldest_first() {
        let out = answer(&k('c'));
        let q = out["questions"].as_array().unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(
            q[0],
            json!({ "id": format!("{}:7", k('a')), "text": "What do you offer?", "hint": "Up to four", "options": ["TECH / AI", "MAKING"],
                    "multi": true, "max": 2, "free": false, "textMax": null, "retired": false, "tally": [1, 2] })
        );
        assert_eq!(
            q[1],
            json!({ "id": format!("{}:11", k('b')), "text": "What do you seek?", "hint": null, "options": null,
                    "multi": false, "max": null, "free": true, "textMax": 180, "retired": true, "tally": null })
        );
    }

    #[test]
    fn only_the_owner_and_admins_may_define() {
        assert_eq!(answer(&k('a'))["can_define"], true);
        assert_eq!(answer(&k('b'))["can_define"], true);
        assert_eq!(answer(&k('c'))["can_define"], false);
        assert_eq!(answer(&k('e'))["can_define"], false, "the Arc is not an admin");
        assert_eq!(answer(&k('0'))["can_define"], false, "a reader off the roster");
    }

    #[test]
    fn a_site_with_nothing_yet_is_its_members_alone() {
        let objects = site(json!({ "about": null, "questions": null, "intentions": null }));
        let out = members_of(&objects, &k('5'), &k('c')).unwrap();
        assert_eq!(out["questions"], json!([]));
        assert!(out["members"].as_array().unwrap().iter().all(|m| m["about"].is_null() && m["answers"] == json!({}) && m["intentions"] == json!([])));
        assert_eq!(out["members"].as_array().unwrap().len(), 5);
    }

    /// R3.1 (C): the owner's and the admins' Sites, and no other: not a plain member's, not the
    /// Arc's (admitter), not one that does not fold, not a room, not one off the roster.
    #[test]
    fn the_sites_a_wild_client_may_be_given_are_the_owners_and_the_admins() {
        let graph = |objects: &[Value]| json!({ "objects": objects, "spine": [{ "index": 1, "object": k('5') }, { "index": 0, "object": k('3') }] });
        let ids = |objects: &[Value], me: &str| sites_run_by(&graph(objects), me).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        let mut objects = site(json!({}));
        assert_eq!(ids(&objects, &k('a')), [k('5')], "the owner");
        assert_eq!(sites_run_by(&graph(&objects), &k('b')), [(k('5'), "Egregore's Echoes".to_string())], "an admin, by the Site's name");
        for not in [k('c'), k('e'), k('f'), k('0')] {
            assert!(ids(&objects, &not).is_empty(), "{not}: a member, the Arc, someone who left, a stranger");
        }
        objects[1]["view"]["roles"] = json!([[k('9'), "admin"]]);
        assert!(ids(&objects, &k('9')).is_empty(), "an admin of a room is not given the room");
        let a = k('a');
        objects.push(json!({ "id": k('3'), "kind": "group", "name": "Ralph", "owner": a, "members": [a], "folds": true, "view": {} }));
        objects.push(json!({ "id": k('4'), "kind": "group", "name": "A part", "owner": a, "members": [a], "folds": true, "view": {} }));
        objects[0]["view"]["parts"] = json!([{ "part": k('4'), "role": "chapter" }]);
        assert_eq!(ids(&objects, &a), [k('5')], "not the self record, not a group that is a part");
        objects[0]["folds"] = json!(false);
        assert!(ids(&objects, &a).is_empty(), "a Site that does not fold here");
    }

    #[test]
    fn a_site_not_in_reach_is_said_so() {
        assert!(members_of(&site(json!({})), &k('6'), &k('c')).is_err(), "not in the graph");
        assert!(members_of(&site(json!({})), &k('7'), &k('c')).is_err(), "a room is not a Site");
        let mut unfolded = site(json!({}));
        unfolded[0]["folds"] = json!(false);
        assert!(members_of(&unfolded, &k('5'), &k('c')).is_err(), "a Site that does not fold here");
    }
}
