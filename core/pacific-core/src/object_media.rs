//! object_media — the MEDIA facet: an object's icon and banner (ICD `facets.media`, band
//! `0xF00F`; #112, ruled 9 Nov 2026).
//!
//! Every kind's media is detached (#56), and none of the per-kind slots can carry a reference:
//! `post.setMedia`, `group.setCover`, `thing.setPhoto` and a Group's profile card take inline
//! base64 only. So the mint writes here, and where a slot of this facet has been written it
//! wins over the kind's own, which stays readable for objects made before it.
//!
//! Each slot is one register, LWW by (gen, author), as `event.setBanner`'s is: a write counts
//! from the owner, or from a roster member holding admin in the object's own roles. A kind
//! without the roles facet passes none, so there only the owner's count.
//!
//! The facet pattern, as [`crate::questions`]: a type holds [`Media`], splices [`MEDIA_OPS`]
//! into its `ops()` and delegates to [`reduce_media`].

use std::collections::BTreeMap;

use crate::coordinator::ArgVal;
use crate::group::GroupRole;
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::visibility::icd;

pub const OP_SET_ICON: u32 = 0xF00F_0000;
pub const OP_SET_BANNER: u32 = 0xF00F_0001;

pub static MEDIA_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_ICON,
        name: "base.setIcon",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_BANNER,
        name: "base.setBanner",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
];

pub fn is_media_op(op_id: u32) -> bool {
    op_id == OP_SET_ICON || op_id == OP_SET_BANNER
}

/// The two registers. `None` until a write counted, `Some(empty)` once cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Media {
    pub icon: Option<pacific_media::MediaRef>,
    pub banner: Option<pacific_media::MediaRef>,
}

impl Media {
    /// The facet's view keys (`facets.media.view`), onto a kind's view object.
    pub fn view_into(&self, v: &mut serde_json::Value) {
        if let Some(m) = self.icon.as_ref().filter(|m| !m.is_empty()) {
            v["icon"] = crate::event::media_view(m);
        }
        if let Some(m) = self.banner.as_ref().filter(|m| !m.is_empty()) {
            v["banner"] = crate::event::media_view(m);
        }
    }
}

/// Both ops, over whatever State holds the facet; `roles` is the object's own folded standing,
/// or `None` for a kind that carries none. Every arg is validated before the register moves.
pub fn reduce_media(
    media: &mut Media,
    roles: Option<&BTreeMap<MemberId, GroupRole>>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    if !matches!(crate::arg_reads::get(op.args, "gen"), Some(ArgVal::Int(_))) {
        return Err(DeltaRejection::MalformedArgs);
    }
    let admin = op.ctx.is_member(op.author)
        && roles.is_some_and(|r| r.get(op.author) == Some(&GroupRole::Admin));
    if *op.author != op.ctx.owner && !admin {
        return Err(DeltaRejection::PreconditionFailed);
    }
    match op.op_id {
        OP_SET_ICON => {
            media.icon = Some(crate::event::media_of(op.args, "icon", pacific_media::Slot::Icon, icd::MEDIA_ICON_MAX)?);
            Ok(())
        }
        OP_SET_BANNER => {
            media.banner = Some(crate::event::media_of(op.args, "banner", pacific_media::Slot::Banner, icd::MEDIA_BANNER_MAX)?);
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjectType;

    /// Every kind the ICD puts the facet on splices `MEDIA_OPS`' own declarations.
    #[test]
    fn media_ops_are_spliced_verbatim() {
        let key = |d: &OpDecl| (d.op_id, d.name, d.authority, d.commutativity);
        for want in MEDIA_OPS {
            assert_eq!(crate::group::GroupType::op(want.op_id).map(key), Some(key(want)), "group");
            assert_eq!(crate::event::EventType::op(want.op_id).map(key), Some(key(want)), "event");
            assert_eq!(crate::thing::ThingType::op(want.op_id).map(key), Some(key(want)), "thing");
            assert_eq!(crate::post::PostType::op(want.op_id).map(key), Some(key(want)), "post");
        }
    }
}
