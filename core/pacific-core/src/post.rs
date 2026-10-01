//! post — a PUBLISHED THING with its own spine. `ObjectKind::Post`, type id **30**.
//!
//! # What a Post is, and what it is not
//!
//! It is NOT `forum.post` and it is NOT `place.post`. Both of those are OPS authored onto
//! a spine that already exists — a message in a Channel, a note on a Place's wall. They
//! have no identity of their own: you cannot hand someone a `forum.post`, because there
//! is nothing to hand them but the Channel it lives in.
//!
//! A Post is the object you CAN hand over. It has its own MLS group, its own log, its own
//! audience, and it outlives whatever surface it was written on. That is why it is a
//! primitive rather than a row: a device-local row cannot be sent, cannot be co-held, and
//! has no log to fold.
//!
//! # Two roles, and why they need no new machinery
//!
//! Owner and Viewer, and nothing else (Ralph, 18 Aug). They land exactly on the two
//! authority levels the op system already has:
//!
//! ```text
//! Owner   -> Authority::Owner      writes the post: profile, media, retraction
//! Viewer  -> Authority::AnyMember  responds to it: reactions, comments
//! ```
//!
//! So the role model is not a layer on top of authority here — it IS the authority, which
//! is why a Post needs no `setMemberRole` op and has none. Contrast `GroupRole`, which has
//! five values layered over the same two levels and therefore cannot be enforced at fold
//! (see `GroupRole::may_mint`). A two-role object does not have that problem, and the
//! shape is worth keeping for exactly that reason.
//!
//! Everyone on the roster who is not the owner is a Viewer. There is no op to make one:
//! joining IS the grant, leaving IS the revocation, and the MLS roster is the record.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::coordinator::Args;
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{opt_text, req_text};

// ---- op ids (MUST match pacific-ffi/src/delta.rs `op::POST_*`) ---------------------
pub const OP_SET_PROFILE: u32 = 0; // owner / sequenced — the invariant: op 0 is the profile
pub const OP_SET_MEDIA: u32 = 1; // owner / sequenced
pub const OP_RETRACT: u32 = 2; // owner / sequenced
pub const OP_REACT: u32 = 3; // any-member / commutative
// W-98 Resources (ICD 2.3.1), owner / sequenced: the post's one PDF, and the images its body shows.
pub const OP_SET_DOCUMENT: u32 = 5;
pub const OP_ADD_ASSET: u32 = 6;
pub const OP_REMOVE_ASSET: u32 = 7;
// 4 WAS `post.comment`. A Post's comments are a FORUM now — the object declares
// one as a constituent part (`ObjectKind::parts`) and the mint brings it into
// being, so a comment is a `forum.post` in a room with its own roster rather than
// a string in a map on the Post. That is what makes comments role-gateable, and it
// is one fewer re-implementation of what a Forum already is. The id is left
// unused, never recycled: a delta from an older build must not fold as something
// else.

// ---- media caps (BYTES OF BASE64, enforced AT FOLD) --------------------------------
//
// The reducer is the one gate every device runs, so a cap here binds a patched peer as
// well as our own client. Matched to `event.setMedia` rather than invented: a post's
// banner is the same kind of thing as an event's, and two different ceilings for one
// idea is how a cap gets forgotten. REFUSED, never clamped — silently shrinking someone's
// image is a worse answer than telling them it did not fit.
pub const MAX_ICON_B64: usize = 500 * 1024;
pub const MAX_BANNER_B64: usize = 700 * 1024;
/// A comment is prose, not an essay. Long enough for a paragraph, short enough that a
/// spine stays foldable on a phone.
pub const MAX_COMMENT_CHARS: usize = 2_000;

/// One viewer's response, kept per author so a second reaction REPLACES the first rather
/// than stacking — the same per-(author, key) LWW `place.post` and `forum.vote` use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Responses {
    /// author -> the single emoji they are currently showing.
    pub reactions: BTreeMap<MemberId, String>,
}

/// WHAT SHAPE OF THING THIS POST IS — an article, a link, an image, or a PDF.
///
/// It is a discriminator and not a hint. Without it the shapes are told apart by
/// guessing at the other fields — a link is "a body that looks like a URL", an
/// image is "a banner with no body", a PDF "a URL ending .pdf" — and a reader that
/// guesses will be wrong on the article that opens with a URL and the essay that
/// carries a photograph. A lane cannot render a shape it cannot name.
///
/// `Article` is the `#[default]` so a post from a build that predates the field
/// folds as the thing it almost certainly was, rather than as nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Form {
    #[default]
    Article,
    Link,
    Image,
    /// ICD 2.1.0: the document is at `link`.
    Pdf,
}

impl Form {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "article" => Self::Article,
            "link" => Self::Link,
            "image" => Self::Image,
            "pdf" => Self::Pdf,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Article => "article",
            Self::Link => "link",
            Self::Image => "image",
            Self::Pdf => "pdf",
        }
    }
    /// What `link` is on this form: a link's destination, a PDF's document, an image's
    /// full-size source. An article has none.
    pub const fn takes_link(self) -> bool {
        !matches!(self, Self::Article)
    }
}

/// A link post's destination is capped at a length no honest URL reaches. The
/// cap is here rather than at the caller for the reason every other cap in this
/// file is: the reducer is the one gate every device runs.
pub const MAX_LINK_CHARS: usize = 2_048;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostState {
    /// Which shape this is. `#[serde(default)]` on the struct's
    /// derive covers a state folded before the field existed.
    pub form: Form,
    pub title: String,
    pub body: String,
    /// Where a `Link` post points, a `Pdf`'s document, an `Image`'s full-size
    /// source. Always empty on an `Article`, and empty is also legal on the others —
    /// a destination can be edited away without the post ceasing to be one.
    pub link: String,
    /// base64. Empty = none.
    pub icon: String,
    pub banner: String,
    /// Each picture's mime, from host.setMedia's set. Empty with a picture: written by
    /// a build before the field, so unknown.
    #[serde(default)]
    pub icon_mime: String,
    #[serde(default)]
    pub banner_mime: String,
    /// A retracted post keeps its log — the fold is append-only and a retraction is a
    /// statement, not an erasure. What it does is stop the object claiming to be readable:
    /// the UI shows the tombstone, not the words.
    pub retracted: bool,
    pub responses: Responses,
    /// THE PARTS THIS POST IS MADE OF, keyed by the part's object id — its comments
    /// section is one, a Forum GroupObject and not a field. `crate::parts`
    /// says why: a hosted room has its own roster, so comments can be open while
    /// the Post is members-only, or the other way round, with no new concept.
    pub parts: BTreeMap<String, crate::object::PartRef>,
    /// THE HALVES THIS OBJECT DECLARES of relations another object asserted.
    /// Keyed `(rel, object)`. Reciprocity is required (25 Sep 2026): a one-sided
    /// edge cannot be walked from the far end, and a kind that is not a Group had
    /// no way to write its half at all.
    #[serde(with = "crate::backlink::as_list")]
    pub backlinks: BTreeMap<(String, String), crate::backlink::Backlink>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
    /// How `body` is written, `plain | markdown`; empty is plain.
    #[serde(default)]
    pub body_format: String,
    /// The short summary a list row shows; empty for none.
    #[serde(default)]
    pub excerpt: String,
    /// The banner's alt text.
    #[serde(default)]
    pub banner_alt: String,
    /// `post.setDocument`'s PDF, and its file name; `None` for none. The view draws it.
    #[serde(skip)]
    pub document: Option<(pacific_media::MediaRef, String)>,
    /// `post.addAsset`'s images by id; the view draws them.
    #[serde(skip)]
    pub assets: BTreeMap<String, Asset>,
    /// Ids removed from `assets`: a removal is final.
    #[serde(skip)]
    pub assets_removed: std::collections::BTreeSet<String>,
    /// The visibility facet; `None` is the kind's default (`visibility::default_for`).
    #[serde(skip)]
    pub visibility: Option<crate::visibility::Visibility>,
}

/// One image a post's body shows as `asset:<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub media: pacific_media::MediaRef,
    pub alt: String,
    pub at: i64,
}

impl PostState {
    /// The visibility in force: the written one, else the kind's default from the ICD.
    pub fn visibility(&self) -> crate::visibility::Visibility {
        self.visibility.unwrap_or_else(|| crate::visibility::default_for(ObjectKind::Post.name()))
    }
}

pub struct PostType;

static OPS: &[OpDecl] = &[
    // The BASE parts op-group, spliced — written out because
    // `parts::PART_OPS` is a `static`. `base_part_ops_are_spliced_verbatim`
    // pins these to it.
    OpDecl {
        op_id: crate::parts::OP_SET_PART,
        name: "base.setPart",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::parts::OP_CLEAR_PART,
        name: "base.clearPart",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base BACKLINK ops, spliced. This is how this kind writes ITS half of a
    // relation something else asserted — `group.setAffiliation`'s job, for kinds
    // that are not a Group.
    OpDecl {
        op_id: crate::backlink::OP_SET_BACKLINK,
        name: "base.setBacklink",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::backlink::OP_CLEAR_BACKLINK,
        name: "base.clearBacklink",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_PROFILE,
        name: "post.setProfile",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_MEDIA,
        name: "post.setMedia",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_RETRACT,
        name: "post.retract",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REACT,
        name: "post.react",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    // The base PARENT ops: any kind can be a part (ICD `facets.parent`), and the
    // part names what it is part of so `part_of` walks from this end too.
    OpDecl {
        op_id: crate::parent::OP_SET_PARENT,
        name: "base.setParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::parent::OP_CLEAR_PARENT,
        name: "base.clearParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base VISIBILITY op, spliced (`crate::visibility`; NC-139).
    OpDecl {
        op_id: crate::visibility::OP_SET_VISIBILITY,
        name: "base.setVisibility",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_DOCUMENT,
        name: "post.setDocument",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_ADD_ASSET,
        name: "post.addAsset",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REMOVE_ASSET,
        name: "post.removeAsset",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

impl ObjectType for PostType {
    const KIND: ObjectKind = ObjectKind::Post;
    type State = PostState;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        // Base ops first: a reserved id band, so this can never shadow a Post op.
        if crate::parts::is_part_op(op.op_id) {
            return crate::parts::reduce_parts(&mut state.parts, op);
        }
        if crate::backlink::is_backlink_op(op.op_id) {
            return crate::backlink::reduce_backlink(&mut state.backlinks, op);
        }
        if crate::visibility::is_visibility_op(op.op_id) {
            let mut v = state.visibility();
            crate::visibility::reduce_visibility(&mut v, op)?;
            state.visibility = Some(v);
            return Ok(());
        }
        let args = op.args;
        match op.op_id {
            // Validate BEFORE the first mutation — a reduce-time rejection is skipped at
            // fold, so a half-applied profile would be the state forever.
            OP_SET_PROFILE => {
                // Validate every field before the first assignment — a reduce-time
                // rejection is skipped at fold, and a half-applied profile would
                // leave a post claiming a form its other fields do not support.
                let title = req_text(args, "title")?.to_string();
                let body = opt_text(args, "body").unwrap_or_default();
                // Absent is Article, not a rejection: `setProfile` predates the
                // field, and a peer on an older build authors no `form` at all.
                let form = match opt_text(args, "form") {
                    Some(f) => Form::parse(&f)?,
                    None => Form::default(),
                };
                let link = opt_text(args, "link").unwrap_or_default();
                if link.chars().count() > MAX_LINK_CHARS {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // A destination on an article is a contradiction the fold refuses
                // rather than silently drops: two devices would otherwise disagree
                // about whether the post has one.
                if !link.is_empty() && !form.takes_link() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let body_format = crate::event::word_of(args, "bodyFormat", icd::POST_BODY_FORMATS)?;
                let excerpt = opt_text(args, "excerpt").unwrap_or_default();
                if excerpt.chars().count() > icd::POST_EXCERPT_MAX {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.body_format = body_format;
                state.excerpt = excerpt;
                state.form = form;
                state.title = title;
                state.body = body;
                state.link = link;
                Ok(())
            }

            OP_SET_MEDIA => {
                let icon = opt_text(args, "icon").unwrap_or_default();
                let banner = opt_text(args, "banner").unwrap_or_default();
                if icon.len() > MAX_ICON_B64 || banner.len() > MAX_BANNER_B64 {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // host.setMedia's set, and only beside a picture that is there.
                let mime = |key: &str, pic: &str| -> Result<String, DeltaRejection> {
                    match opt_text(args, key) {
                        None => Ok(String::new()),
                        Some(m) if !pic.is_empty() && crate::host::MEDIA_MIMES.contains(&m.as_str()) => Ok(m),
                        Some(_) => Err(DeltaRejection::MalformedArgs),
                    }
                };
                let icon_mime = mime("iconMime", &icon)?;
                let banner_mime = mime("bannerMime", &banner)?;
                let banner_alt = opt_text(args, "bannerAlt").unwrap_or_default();
                if banner_alt.chars().count() > icd::POST_BANNER_ALT_MAX {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.banner_alt = banner_alt;
                state.icon = icon;
                state.banner = banner;
                state.icon_mime = icon_mime;
                state.banner_mime = banner_mime;
                Ok(())
            }

            OP_RETRACT => {
                state.retracted = true;
                Ok(())
            }

            // ---- W-98 Resources: the document and the body's images ---------------
            OP_SET_DOCUMENT => {
                let m = crate::event::media_of(args, "document", pacific_media::Slot::Document, icd::POST_DOCUMENT_MAX)?;
                let name = opt_text(args, "name").unwrap_or_default();
                if name.chars().count() > icd::POST_NAME_MAX {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.document = if m.is_empty() { None } else { Some((m, name)) };
                Ok(())
            }
            OP_ADD_ASSET => {
                let id = crate::event::key16(args)?;
                let media = crate::event::media_of(args, "asset", pacific_media::Slot::Photo, icd::POST_ASSET_MAX)?;
                let alt = opt_text(args, "alt").unwrap_or_default();
                let at = crate::object_args::req_int(args, "at")?;
                if media.is_empty() || at <= 0 || alt.chars().count() > icd::POST_ALT_MAX {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if state.assets_removed.contains(&id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                if !state.assets.contains_key(&id) && state.assets.len() >= icd::POST_ASSETS_LIVE {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                state.assets.insert(id, Asset { media, alt, at });
                Ok(())
            }
            OP_REMOVE_ASSET => {
                let id = crate::event::key16(args)?;
                state.assets.remove(&id);
                state.assets_removed.insert(id);
                Ok(())
            }

            // ---- the Viewer's two ops -------------------------------------------
            //
            // AnyMember. WHETHER THE AUTHOR WAS A MEMBER is not re-asked here: MLS
            // answered it at ingest, for the epoch the reaction was written in
            // (membership-through-mls.md §10.1). Asking today's roster would erase a
            // departed viewer's reactions and comments from the Post's history.
            OP_REACT => {
                let emoji = req_text(args, "emoji")?.to_string();
                if emoji.is_empty() {
                    // The way to take a reaction back is `clear`, which is this op with
                    // an explicit empty — so an absent emoji is malformed, not a clear.
                    state.responses.reactions.remove(op.author);
                } else {
                    state.responses.reactions.insert(*op.author, emoji);
                }
                Ok(())
            }

            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

use crate::visibility::icd;

/// The post's document and images as the view draws them: `document` {mime, bytes, data, name}
/// and `assets` [{id, mime, data, width, height, alt, at}] ordered by at.
pub fn media_view(st: &PostState) -> (serde_json::Value, serde_json::Value) {
    let document = match &st.document {
        None => serde_json::Value::Null,
        Some((m, name)) => {
            let mut o = crate::event::media_view(m);
            let bytes = match &m.delivery {
                pacific_media::Delivery::Inline { data } => (data.len() / 4 * 3) as u64,
                pacific_media::Delivery::Detached { bytes, .. } | pacific_media::Delivery::Sealed { bytes, .. } => *bytes,
                pacific_media::Delivery::Live { .. } => 0,
            };
            o["bytes"] = bytes.into();
            o["name"] = name.clone().into();
            o
        }
    };
    let mut assets: Vec<(&String, &Asset)> = st.assets.iter().collect();
    assets.sort_by_key(|(id, a)| (a.at, (*id).clone()));
    let assets = assets
        .into_iter()
        .map(|(id, a)| {
            let mut o = crate::event::media_view(&a.media);
            o["id"] = id.clone().into();
            o["alt"] = a.alt.clone().into();
            o["at"] = a.at.into();
            o
        })
        .collect::<Vec<_>>()
        .into();
    (document, assets)
}

/// The one op-0 arg builder, shared with the mint seam so the keys are named once.
///
/// `form` is an `Option` rather than a `Form` so the builder can author NOTHING
/// where the caller has no opinion — an absent arg folds to `Article` at the
/// reducer, which is a different statement from asserting "article" on the wire.
pub fn set_profile_args(
    title: &str,
    body: Option<&str>,
    form: Option<Form>,
    link: Option<&str>,
) -> Args {
    let mut a = Args::new();
    a.insert("title".into(), crate::coordinator::ArgVal::Text(title.into()));
    if let Some(b) = body {
        a.insert("body".into(), crate::coordinator::ArgVal::Text(b.into()));
    }
    if let Some(f) = form {
        a.insert("form".into(), crate::coordinator::ArgVal::Text(f.as_str().into()));
    }
    if let Some(l) = link {
        a.insert("link".into(), crate::coordinator::ArgVal::Text(l.into()));
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::ArgVal;
    use crate::object::ReduceContext;

    const OWNER: MemberId = [1u8; 32];

    fn apply(st: &mut PostState, op_id: u32, a: Args) -> Result<(), DeltaRejection> {
        let members = [OWNER];
        let ctx = ReduceContext { members: &members, owner: OWNER, epoch: 1 };
        PostType::reduce(
            st,
            &Op { op_id, args: &a, author: &OWNER, pos: None, ctx: &ctx },
        )
    }

    /// The whole point of the discriminator: absent is `article`, not a rejection
    /// and not nothing. A post authored before the field existed folds as the
    /// thing it almost certainly was.
    #[test]
    fn an_absent_form_folds_as_an_article() {
        let mut st = PostState::default();
        apply(&mut st, OP_SET_PROFILE, set_profile_args("Feast for forty", None, None, None))
            .expect("profile");
        assert_eq!(st.form, Form::Article);
        assert_eq!(st.title, "Feast for forty");
        assert!(st.link.is_empty());
    }

    #[test]
    fn the_three_forms_round_trip() {
        for f in [Form::Article, Form::Link, Form::Image] {
            assert_eq!(Form::parse(f.as_str()).unwrap(), f);
        }
        assert_eq!(Form::parse("essay"), Err(DeltaRejection::MalformedArgs));
    }

    #[test]
    fn a_link_post_carries_its_destination() {
        let mut st = PostState::default();
        apply(&mut st, OP_SET_PROFILE, set_profile_args(
            "The actual form", Some("What to file."), Some(Form::Link),
            Some("sevilla.org/urbanismo/licencias"),
        )).expect("profile");
        assert_eq!(st.form, Form::Link);
        assert_eq!(st.link, "sevilla.org/urbanismo/licencias");
    }

    /// A destination on an article is a contradiction, and the fold says so
    /// rather than dropping it — two devices would otherwise disagree about
    /// whether the post has one. (An image's link is its full-size source since
    /// ICD 2.1.0 row 7, so only the article refuses one.)
    #[test]
    fn a_destination_on_an_article_is_refused() {
        let mut st = PostState::default();
        assert_eq!(
            apply(&mut st, OP_SET_PROFILE,
                  set_profile_args("t", None, Some(Form::Article), Some("example.org"))),
            Err(DeltaRejection::MalformedArgs),
            "an article must not carry a link"
        );
        assert!(st.title.is_empty(), "reduce is atomic — nothing landed");
    }

    #[test]
    fn an_overlong_destination_is_refused() {
        let mut st = PostState::default();
        let long = "a".repeat(MAX_LINK_CHARS + 1);
        assert_eq!(
            apply(&mut st, OP_SET_PROFILE,
                  set_profile_args("t", None, Some(Form::Link), Some(&long))),
            Err(DeltaRejection::MalformedArgs)
        );
        // …and exactly at the cap is fine: a bound is a bound, not a suggestion.
        let ok = "a".repeat(MAX_LINK_CHARS);
        apply(&mut st, OP_SET_PROFILE,
              set_profile_args("t", None, Some(Form::Link), Some(&ok))).expect("at the cap");
    }

    /// Re-profiling an existing post must be able to CHANGE its form, and a form
    /// that stops being a link must not keep a destination behind it.
    #[test]
    fn a_form_can_be_edited_and_the_link_goes_with_it() {
        let mut st = PostState::default();
        apply(&mut st, OP_SET_PROFILE,
              set_profile_args("t", None, Some(Form::Link), Some("example.org"))).unwrap();
        assert_eq!(st.link, "example.org");
        apply(&mut st, OP_SET_PROFILE,
              set_profile_args("t", Some("now an essay"), Some(Form::Article), None)).unwrap();
        assert_eq!(st.form, Form::Article);
        assert!(st.link.is_empty(), "the destination does not outlive the form");
    }

    fn raw_profile(form: &str, link: Option<&str>) -> Args {
        let mut a = Args::new();
        a.insert("title".into(), ArgVal::Text("t".into()));
        a.insert("form".into(), ArgVal::Text(form.into()));
        if let Some(l) = link {
            a.insert("link".into(), ArgVal::Text(l.into()));
        }
        a
    }

    fn media(icon: &str, icon_mime: Option<&str>, banner: &str, banner_mime: Option<&str>) -> Args {
        let mut a = Args::new();
        a.insert("icon".into(), ArgVal::Text(icon.into()));
        a.insert("banner".into(), ArgVal::Text(banner.into()));
        if let Some(m) = icon_mime {
            a.insert("iconMime".into(), ArgVal::Text(m.into()));
        }
        if let Some(m) = banner_mime {
            a.insert("bannerMime".into(), ArgVal::Text(m.into()));
        }
        a
    }

    /// What a reader is served: the state as the view serializes it.
    fn served(st: &PostState, key: &str) -> String {
        serde_json::to_value(st).unwrap().get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    /// ICD 2.1.0 row 6: a PDF is its own form, not a link the reader guesses at from a URL.
    #[test]
    fn a_pdf_post_carries_its_document() {
        let mut st = PostState::default();
        apply(&mut st, OP_SET_PROFILE, raw_profile("pdf", Some("https://example.org/menu.pdf")))
            .expect("a pdf with its document");
        assert_eq!(served(&st, "form"), "pdf");
        assert_eq!(st.link, "https://example.org/menu.pdf");
    }

    /// ICD 2.1.0 row 7: an image may name its full-size source ("Manolin to use full size
    /// images from manolin.kr", Ralph).
    #[test]
    fn an_image_post_may_carry_its_full_size_source() {
        let mut st = PostState::default();
        apply(&mut st, OP_SET_PROFILE, raw_profile("image", Some("https://manolin.kr/full.jpg")))
            .expect("an image with its source");
        assert_eq!(served(&st, "form"), "image");
        assert_eq!(st.link, "https://manolin.kr/full.jpg");
    }

    /// ICD 2.1.0 row 8: each picture names its mime, so no reader sniffs bytes.
    #[test]
    fn media_carry_their_mimes() {
        let mut st = PostState::default();
        apply(&mut st, OP_SET_MEDIA, media("aWNvbg==", Some("image/webp"), "YmFubmVy", Some("image/png")))
            .expect("media with mimes");
        assert_eq!(served(&st, "icon_mime"), "image/webp");
        assert_eq!(served(&st, "banner_mime"), "image/png");
        // A later setMedia replaces both pictures, and their mimes with them.
        apply(&mut st, OP_SET_MEDIA, media("aWNvbg==", None, "", None)).expect("older writer");
        assert_eq!(served(&st, "icon_mime"), "", "no mime sent: unknown, as a build before the field");
        assert_eq!(served(&st, "banner_mime"), "", "a cleared banner keeps no mime");
    }

    /// The mimes are host.setMedia's closed set, and a mime names a picture that is there.
    #[test]
    fn a_mime_outside_the_set_or_without_its_picture_is_refused() {
        for (a, why) in [
            (media("aWNvbg==", Some("image/gif"), "", None), "a gif"),
            (media("aWNvbg==", Some("text/html"), "", None), "not a picture"),
            (media("", Some("image/png"), "", None), "a mime with no icon"),
            (media("", None, "", Some("image/jpeg")), "a mime with no banner"),
        ] {
            let mut st = PostState::default();
            assert_eq!(apply(&mut st, OP_SET_MEDIA, a), Err(DeltaRejection::MalformedArgs), "{why}");
        }
    }

    #[test]
    fn a_bad_form_is_loud() {
        let mut st = PostState::default();
        let mut a = Args::new();
        a.insert("title".into(), ArgVal::Text("t".into()));
        a.insert("form".into(), ArgVal::Text("essay".into()));
        assert_eq!(apply(&mut st, OP_SET_PROFILE, a), Err(DeltaRejection::MalformedArgs));
    }
}
