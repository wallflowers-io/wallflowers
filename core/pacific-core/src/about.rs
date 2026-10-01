//! about — the ABOUT facet: `base.publishAbout`, each member's own bio and links in a Site.
//!
//! W-98 Members ("a developed, site scoped bio and info"; ICD 2.3.1 `facets.about`, band
//! `0xF00B`). Published by the member themselves, as `base.publishProfile` is; its SUBJECT is
//! its AUTHOR, never a field. Per-author LWW by the op's `gen`. An empty bio and no links clears.
//! A bio over [`MAX_BIO_BYTES`], more than [`MAX_LINKS`] links, or a link that is not https or
//! runs past [`MAX_LINK_CHARS`], is refused, never truncated.
//!
//! `member` is a leaf on the roster (ICD `principals`), so an author off it publishes nothing,
//! and the view is roster members only.
//!
//! The facet pattern, as [`crate::profiles`]: a type holds `about: BTreeMap<MemberId, About>`,
//! splices [`ABOUT_OPS`] into its `ops()` and delegates to [`reduce_about`].

use std::collections::BTreeMap;

use crate::coordinator::ArgVal;
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{req_int, req_text};

pub const OP_PUBLISH_ABOUT: u32 = 0xF00B_0000;

pub const MAX_BIO_BYTES: usize = 4096;
pub const MAX_LINKS: usize = 3;
pub const MAX_LINK_CHARS: usize = 2048;

pub static ABOUT_OPS: &[OpDecl] = &[OpDecl {
    op_id: OP_PUBLISH_ABOUT,
    name: "base.publishAbout",
    authority: Authority::AnyMember,
    commutativity: Commutativity::Commutative,
}];

pub fn is_about_op(op_id: u32) -> bool {
    op_id == OP_PUBLISH_ABOUT
}

/// A member's bio and links here, with the gen they were published at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct About {
    pub bio: String,
    pub links: Vec<String>,
    pub gen: u64,
}

fn link_ok(l: &str) -> bool {
    l.len() > "https://".len()
        && l.starts_with("https://")
        && l.chars().count() <= MAX_LINK_CHARS
        && !l.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The one reducer, over whatever State holds the facet. The author is the subject.
pub fn reduce_about(about: &mut BTreeMap<MemberId, About>, op: &Op<'_>) -> Result<(), DeltaRejection> {
    if op.op_id != OP_PUBLISH_ABOUT {
        return Err(DeltaRejection::UnknownType);
    }
    // Validate every arg before the first mutation — reduce is atomic.
    let bio = req_text(op.args, "bio")?;
    if bio.len() > MAX_BIO_BYTES {
        return Err(DeltaRejection::MalformedArgs);
    }
    let links: Vec<String> = match crate::arg_reads::get(op.args, "links") {
        None => Vec::new(),
        Some(ArgVal::Text(j)) => serde_json::from_str(j).map_err(|_| DeltaRejection::MalformedArgs)?,
        Some(_) => return Err(DeltaRejection::MalformedArgs),
    };
    if links.len() > MAX_LINKS || !links.iter().all(|l| link_ok(l)) {
        return Err(DeltaRejection::MalformedArgs);
    }
    let gen = u64::try_from(req_int(op.args, "gen")?).map_err(|_| DeltaRejection::MalformedArgs)?;
    if !op.ctx.is_member(op.author) {
        return Err(DeltaRejection::Unauthorized);
    }
    if about.get(op.author).is_some_and(|held| held.gen > gen) {
        return Ok(());
    }
    if bio.is_empty() && links.is_empty() {
        about.remove(op.author);
    } else {
        about.insert(*op.author, About { bio: bio.to_string(), links, gen });
    }
    Ok(())
}

/// The view's `about`: `{<member key>: {bio, links, gen}}`, members on `roster` only.
pub fn view(about: &BTreeMap<MemberId, About>, roster: &[MemberId]) -> serde_json::Value {
    serde_json::Value::Object(
        about
            .iter()
            .filter(|(m, _)| roster.contains(m))
            .map(|(m, a)| (hex::encode(m), serde_json::json!({ "bio": a.bio, "links": a.links, "gen": a.gen })))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The op the Group splices into its table is `ABOUT_OPS`' own.
    #[test]
    fn base_about_op_is_spliced_verbatim() {
        use crate::object::ObjectType;
        let key = |d: &OpDecl| (d.op_id, d.name, d.authority, d.commutativity);
        assert_eq!(crate::group::GroupType::op(OP_PUBLISH_ABOUT).map(key), Some(key(&ABOUT_OPS[0])));
    }

    /// The caps are the ICD's: the args' summaries state the numbers this fold holds.
    #[test]
    fn the_caps_are_the_icds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let args = &doc["facets"]["about"]["ops"]["base.publishAbout"]["args"];
        assert!(args["bio"]["summary"].as_str().unwrap().contains(&format!("at most {MAX_BIO_BYTES} bytes")));
        let links = args["links"]["summary"].as_str().unwrap();
        assert!(links.contains(&format!("at most {MAX_LINKS} https URLs, each at most {MAX_LINK_CHARS} characters")), "{links}");
    }
}
