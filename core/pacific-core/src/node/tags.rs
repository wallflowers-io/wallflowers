//! THE TAGS ON A NODE (`facets.tags`): reading an object's tags as a surface draws them, and
//! tagging with both halves of a link in one call. The facet's fold is `crate::tags`.

use super::*;
use crate::tags;

/// One tag on an object, as a surface draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagRow {
    pub label: String,
    /// The linked object's id, hex; empty for a label alone.
    pub object: String,
    /// How many members put it on.
    pub count: u32,
    /// Whether one of them is this account.
    pub mine: bool,
    /// Hidden by the owner or an admin: drawn only for them.
    pub hidden: bool,
    /// The linked object's half is held here: both ends say so.
    pub confirmed: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Node {
    fn view_json(&self, object_id_hex: &str) -> Result<serde_json::Value, CoreError> {
        serde_json::from_str(&self.object_view(object_id_hex)?).map_err(|e| CoreError::Coordinator(e.to_string()))
    }

    /// Whether `of`'s halves (its view's `taggedBy`) name `object` under `label`. False when
    /// `of` is not held here: unconfirmed is all this device can say.
    fn half_held(&self, of: &str, object: &str, label: &str) -> bool {
        self.view_json(of).is_ok_and(|v| {
            v["taggedBy"].as_array().is_some_and(|hs| hs.iter().any(|h| h["object"] == object && h["label"] == label))
        })
    }

    /// `object`'s tags, by (label, linked object), from its fold.
    pub fn tags_of(&self, object_id_hex: &str) -> Result<Vec<TagRow>, CoreError> {
        let me = hex::encode(self.id.identity_pk());
        let v = self.view_json(object_id_hex)?;
        Ok(v["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let label = t["label"].as_str().unwrap_or_default().to_string();
                let object = t["object"].as_str().unwrap_or_default().to_string();
                let confirmed = !object.is_empty() && self.half_held(&object, object_id_hex, &label);
                TagRow {
                    count: t["count"].as_u64().unwrap_or(0) as u32,
                    mine: t["by"].as_array().is_some_and(|b| b.iter().any(|a| *a == me)),
                    hidden: t["hidden"].as_bool().unwrap_or(false),
                    confirmed,
                    label,
                    object,
                }
            })
            .collect())
    }

    /// Tag `object` with `label`, linking `target` if given. A linked target must be held here,
    /// so its half (`base.setTagged`) is written by its own member in the same call; one not
    /// held is refused before anything is written.
    pub async fn tag(&self, object_id_hex: &str, label: &str, target: Option<&str>) -> Result<(), CoreError> {
        if let Some(t) = target {
            self.object_kind(t).map_err(|_| CoreError::Coordinator(format!("{t} is not held here: its half cannot be written")))?;
        }
        let at = now_ms();
        self.apply(object_id_hex, tags::OP_TAG, tags::tag_args(label, target, at)).await?;
        if let Some(t) = target {
            if !self.half_held(t, object_id_hex, label) {
                self.apply(t, tags::OP_SET_TAGGED, tags::set_tagged_args(object_id_hex, label, at)).await?;
            }
        }
        Ok(())
    }

    /// Take this account's own `label` off `object`. When its link was the last tag naming that
    /// object, the linked object's half goes too, if held here.
    pub async fn untag(&self, object_id_hex: &str, label: &str) -> Result<(), CoreError> {
        let rows = self.tags_of(object_id_hex)?;
        let mine = rows.iter().find(|r| r.label == label && r.mine).cloned();
        self.apply(object_id_hex, tags::OP_UNTAG, tags::untag_args(label)).await?;
        if let Some(r) = mine.filter(|r| !r.object.is_empty() && r.count == 1) {
            if self.half_held(&r.object, object_id_hex, label) {
                self.apply(&r.object, tags::OP_CLEAR_TAGGED, tags::clear_tagged_args(object_id_hex, label)).await?;
            }
        }
        Ok(())
    }
}
