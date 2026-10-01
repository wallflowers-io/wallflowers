//! profiles — the PROFILE facet: `base.publishProfile`, a member's own card in an object.
//!
//! ## What it is for
//!
//! Ralph, 28 Sep (O-77; ICD 2.1.0 row 4): "All members of a community are required to publish
//! a profile card on entry, which includes Name, profile icon and public key." Before this a
//! Site member's name reached others only through a pairing channel (`contact.publishProfile`),
//! so a visitor the Arc admitted from a kiosk was a key in hex to everyone they had not paired
//! with. The card rides the object itself, and every member folds it.
//!
//! ## What a card is, and is not
//!
//! A name and, optionally, a picture: `{displayName, photo?, photo_mime?}`, the picture in the
//! ContactCard's own fields (MANAGE, 28 Sep), a still webp, png or jpeg in standard base64.
//! Nothing else (Software Assurance: only a name and a picture, RX.11). Its SUBJECT is
//! its AUTHOR: the key is the signer's, never a field, so a card has no way to name anyone
//! else, and one that carries any other field is refused. At most [`MAX_CARD_BYTES`], refused
//! rather than cut, as `host.hydrate`'s payload is.
//!
//! ## The facet pattern
//!
//! A type that carries cards holds `profiles: BTreeMap<MemberId, Profile>` in its `State`,
//! splices [`PROFILE_OPS`] into its `ops()`, and delegates to [`reduce_profiles`]. Per-author
//! LWW by the op's `gen`, which `authoring::build` stamps as it does every commutative op's.
//! Band `0xF00A`.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{req_int, req_text};

/// A member's card in this object, their own.
pub const OP_PUBLISH_PROFILE: u32 = 0xF00A_0000;

/// A card's size, as the ICD states it: the `card` arg's bytes. A 96 px still fits with room;
/// more is refused, never truncated.
pub const MAX_CARD_BYTES: usize = 16_384;

/// The pictures a card may carry, by `photo_mime`.
const PHOTO_TYPES: &[&str] = &["image/webp", "image/png", "image/jpeg"];

pub static PROFILE_OPS: &[OpDecl] = &[OpDecl {
    op_id: OP_PUBLISH_PROFILE,
    name: "base.publishProfile",
    authority: Authority::AnyMember,
    commutativity: Commutativity::Commutative,
}];

pub fn is_profile_op(op_id: u32) -> bool {
    op_id == OP_PUBLISH_PROFILE
}

/// A card: a name, and a picture if it has one, in ContactCard's own fields. No other field
/// exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo_mime: Option<String>,
}

/// An author's card as folded, with the gen it was published at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub card: Card,
    pub gen: u64,
}

impl Card {
    /// The card as its `card` arg carries it.
    pub fn to_arg(&self) -> String {
        serde_json::to_string(self).expect("a card serialises")
    }

    /// Why this card would be refused, or None: [`parse`]'s rule, for a card being built.
    pub fn refusal(&self) -> Option<String> {
        let arg = self.to_arg();
        if arg.len() > MAX_CARD_BYTES {
            return Some(format!("the card is {} bytes, over the {MAX_CARD_BYTES} a card holds", arg.len()));
        }
        if self.display_name.trim().is_empty() {
            return Some("a card has a name".into());
        }
        match (&self.photo, &self.photo_mime) {
            (None, None) => None,
            (Some(photo), Some(mime)) => photo_ok(photo, mime).err(),
            _ => Some("a picture is its photo and its photo_mime, both".into()),
        }
    }
}

/// A still webp, png or jpeg, in standard base64 that decodes.
fn photo_ok(photo: &str, mime: &str) -> Result<(), String> {
    if !PHOTO_TYPES.contains(&mime) {
        return Err(format!("a picture is {}, not {mime}", PHOTO_TYPES.join(", ")));
    }
    base64::engine::general_purpose::STANDARD.decode(photo).map_err(|_| "the picture's base64 does not decode".to_string())?;
    Ok(())
}

/// The `card` arg, checked: within the cap, a card and nothing else, named, its picture a
/// picture.
pub fn parse(arg: &str) -> Result<Card, DeltaRejection> {
    if arg.len() > MAX_CARD_BYTES {
        return Err(DeltaRejection::MalformedArgs);
    }
    let card: Card = serde_json::from_str(arg).map_err(|_| DeltaRejection::MalformedArgs)?;
    if card.refusal().is_some() {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(card)
}

/// The one reducer, over whatever State holds the cards. The author is the subject.
pub fn reduce_profiles(profiles: &mut BTreeMap<MemberId, Profile>, op: &Op<'_>) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_PUBLISH_PROFILE => {
            // Validate every arg before the first mutation — reduce is atomic.
            let card = parse(req_text(op.args, "card")?)?;
            let gen = u64::try_from(req_int(op.args, "gen")?).map_err(|_| DeltaRejection::MalformedArgs)?;
            match profiles.get(op.author) {
                Some(held) if held.gen > gen => {}
                _ => {
                    profiles.insert(*op.author, Profile { card, gen });
                }
            }
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The args `base.publishProfile` carries; `authoring::build` adds the gen.
pub fn publish_args(card: &Card) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("card".into(), crate::coordinator::ArgVal::Text(card.to_arg()));
    a
}

/// The view's `profiles` (webapp REQUIREMENTS § 3): `{<member hex>: {name, icon: {mime, data}
/// | null, gen}}`, every author's card, a departed member's too: their lines never go back to
/// "New member". The picture is served as the icon, `{mime: photo_mime, data: photo}`.
pub fn view(profiles: &BTreeMap<MemberId, Profile>) -> serde_json::Value {
    serde_json::Value::Object(
        profiles
            .iter()
            .map(|(m, p)| {
                let icon = p.card.photo.as_ref().zip(p.card.photo_mime.as_ref()).map(|(data, mime)| serde_json::json!({ "mime": mime, "data": data }));
                (hex::encode(m), serde_json::json!({ "name": p.card.display_name, "icon": icon, "gen": p.gen }))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{ArgVal, Args};
    use crate::object::ReduceContext;

    fn run(profiles: &mut BTreeMap<MemberId, Profile>, author: MemberId, card: &str, gen: i64) -> Result<(), DeltaRejection> {
        let members = [author];
        let ctx = ReduceContext { members: &members, owner: author, epoch: 1 };
        let mut args = Args::new();
        args.insert("card".into(), ArgVal::Text(card.into()));
        args.insert("gen".into(), ArgVal::Int(gen));
        reduce_profiles(profiles, &Op { op_id: OP_PUBLISH_PROFILE, args: &args, author: &author, pos: None, ctx: &ctx })
    }

    /// The op the Group and the Forum splice into their tables is `PROFILE_OPS`' own.
    #[test]
    fn base_profile_op_is_spliced_verbatim() {
        use crate::object::ObjectType;
        let key = |d: &OpDecl| (d.op_id, d.name, d.authority, d.commutativity);
        for spliced in [crate::group::GroupType::op(OP_PUBLISH_PROFILE), crate::coordinator::ForumType::op(OP_PUBLISH_PROFILE)] {
            assert_eq!(spliced.map(key), Some(key(&PROFILE_OPS[0])));
        }
        assert!(crate::coordinator::ConversationType::op(OP_PUBLISH_PROFILE).is_none(), "a conversation is not on the facet");
    }

    /// The cap is the ICD's: its `card` arg states the same number this fold refuses above.
    #[test]
    fn the_cap_is_the_icds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let said = doc["facets"]["profiles"]["ops"]["base.publishProfile"]["args"]["card"]["summary"].as_str().expect("the card's summary");
        assert!(said.contains(&format!("At most {MAX_CARD_BYTES} bytes")), "{said}");
    }

    #[test]
    fn a_card_is_its_authors_and_the_later_gen_holds() {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let mut p = BTreeMap::new();
        run(&mut p, a, r#"{"displayName":"Ada"}"#, 5).unwrap();
        run(&mut p, b, r#"{"displayName":"Bo","photo":"UklGRg==","photo_mime":"image/webp"}"#, 6).unwrap();
        run(&mut p, a, r#"{"displayName":"Ada L."}"#, 7).unwrap();
        run(&mut p, a, r#"{"displayName":"Ada, stale"}"#, 4).unwrap();
        assert_eq!(p[&a].card.display_name, "Ada L.", "the later gen holds, whatever order it folds in");
        assert_eq!(p[&b].card.photo.as_deref(), Some("UklGRg=="));
        assert_eq!(
            view(&p),
            serde_json::json!({
                hex::encode(a): { "name": "Ada L.", "icon": null, "gen": 7 },
                hex::encode(b): { "name": "Bo", "icon": { "mime": "image/webp", "data": "UklGRg==" }, "gen": 6 },
            })
        );
    }

    #[test]
    fn a_card_that_is_more_than_a_name_and_a_picture_is_refused() {
        let a = [1u8; 32];
        let mut p = BTreeMap::new();
        let other = hex::encode([2u8; 32]);
        for bad in [
            format!(r#"{{"displayName":"Ada","member":"{other}"}}"#),
            format!(r#"{{"displayName":"Ada","publicKey":"{other}"}}"#),
            r#"{"displayName":""}"#.to_string(),
            r#"{"displayName":"  "}"#.to_string(),
            r#"{"photo":"UklGRg==","photo_mime":"image/webp"}"#.to_string(),
            r#"{"displayName":"Ada","icon":"data:image/webp;base64,UklGRg=="}"#.to_string(),
            r#"{"displayName":"Ada","photo":"UklGRg=="}"#.to_string(),
            r#"{"displayName":"Ada","photo_mime":"image/webp"}"#.to_string(),
            r#"{"displayName":"Ada","photo":"PHN2Zz4=","photo_mime":"image/svg+xml"}"#.to_string(),
            r#"{"displayName":"Ada","photo":"***","photo_mime":"image/png"}"#.to_string(),
            format!(r#"{{"displayName":"Ada","photo":"{}","photo_mime":"image/webp"}}"#, "A".repeat(MAX_CARD_BYTES)),
            "not json".to_string(),
        ] {
            assert_eq!(run(&mut p, a, &bad, 1), Err(DeltaRejection::MalformedArgs), "{}", &bad[..bad.len().min(80)]);
        }
        assert!(p.is_empty(), "nothing folded");
        let at_cap = format!(r#"{{"displayName":"Ada","photo":"{}","photo_mime":"image/webp"}}"#, "A".repeat(MAX_CARD_BYTES - 64));
        assert!(at_cap.len() <= MAX_CARD_BYTES);
        run(&mut p, a, &at_cap, 1).expect("a card within the cap");
    }
}
