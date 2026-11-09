//! summary — THE sweep: every object this device holds, with who holds it and what
//! it says, in one pass; and every person, with the profile they published.
//!
//! One walk serves every reader that used to make its own: the Door's graph
//! (`/v2/graph`), the app's object index and its graph projection. A reader that wants
//! one kind filters here, not after.

use crate::node::Node;
use crate::object::ObjectKind;
use crate::parent::ParentRef;
use crate::CoreError;

/// One object as the sweep reads it.
pub struct ObjectSummary {
    pub id: String,
    pub kind: String,
    /// The MLS group context's name; "" when the group will not load.
    pub name: String,
    /// The current owner (the `owner` extension), if recorded.
    pub owner: Option<[u8; 32]>,
    /// Who made it: the first owner the group had (`owner_history`, entry 0). A
    /// handover changes the owner, never the creator.
    pub creator: Option<[u8; 32]>,
    pub members: Vec<[u8; 32]>,
    /// The part's half of `part_of` (`base.setParent`), from the object's own fold.
    pub parent: Option<ParentRef>,
    /// For a connection's objects: the peer, and which of the pair this is.
    pub peer: Option<[u8; 32]>,
    pub link: Option<&'static str>,
    /// The folded view (`Node::object_view`), or the core's reason it will not fold.
    pub view: Result<String, String>,
}

/// One person: a connection, or this device's own identity.
pub struct Person {
    pub identity: [u8; 32],
    pub name: String,
    pub connected: bool,
    pub is_me: bool,
    /// The avatar's block colour, "RRGGBB", or "" when none was chosen.
    pub color: String,
    /// When the avatar photo was captured (the card's `captured_at`, unix secs; 0 =
    /// none): moves exactly when the photo does, so a reader keys its decoded copy on it.
    pub photo_at: u64,
}

impl Node {
    /// Every object this device holds, connections' objects included, each with its
    /// owner, creator, members, back-edge and view. `kind` keeps one kind.
    pub fn object_rows(&self, kind: Option<ObjectKind>) -> Result<Vec<ObjectSummary>, CoreError> {
        let want = |k: &str| kind.is_none_or(|w| w.name() == k);
        let mut held: Vec<(String, String, String)> = self.objects_named()?;
        // `objects_named` leaves connections out by design; a pair holds a CHANNEL
        // and a CHAT, and the reader names which.
        let mut links: std::collections::HashMap<String, ([u8; 32], &'static str)> = Default::default();
        for (peer, peer_name, _connected) in self.connections()? {
            if let Some((id, k)) = self.connection_object(&peer)? {
                links.insert(id.clone(), (peer, "channel"));
                if !held.iter().any(|(h, _, _)| *h == id) {
                    held.push((id, k, peer_name));
                }
            }
            if let Some(id) = self.conversation_with(&peer)? {
                links.insert(id, (peer, "chat"));
            }
        }
        let mut out = Vec::new();
        for (id, k, name) in held.into_iter().filter(|(_, k, _)| want(k)) {
            let gid = hex::decode(&id).map_err(|e| CoreError::Directory(format!("object id: {e}")))?;
            let owner = self.dir.group_owner(&gid)?.and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok());
            let creator = self.dir.owner_history(&gid)?.first().map(|(_, pk)| *pk);
            let members = self.dir.group_members(&gid)?;
            let view = self.object_view(&id).map_err(|e| e.to_string());
            let parent = view
                .as_ref()
                .ok()
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
                .and_then(|v| serde_json::from_value::<Option<ParentRef>>(v["parent"].clone()).ok())
                .flatten();
            let (peer, link) = links.get(&id).map_or((None, None), |(p, l)| (Some(*p), Some(*l)));
            out.push(ObjectSummary { id, kind: k, name, owner, creator, members, parent, peer, link, view });
        }
        Ok(out)
    }

    /// Every person: this device's identity first, then each connection, with the
    /// profile they published to us. Photos are not here: [`Node::person_photo`].
    pub fn people(&self) -> Result<Vec<Person>, CoreError> {
        let me = self.id.identity_pk();
        let (my_name, _shape, my_card) = self.my_profile()?;
        let mut out = vec![Person {
            identity: me,
            name: my_name,
            connected: true,
            is_me: true,
            photo_at: my_card.captured_at,
            color: my_card.avatar_color,
        }];
        for (peer, name, connected) in self.connections()? {
            let profile = self.peer_profile(&peer)?;
            out.push(Person {
                identity: peer,
                name,
                connected,
                is_me: false,
                color: profile.as_ref().map(|p| p.card.avatar_color.clone()).unwrap_or_default(),
                photo_at: profile.as_ref().map_or(0, |p| p.card.captured_at),
            });
        }
        Ok(out)
    }

    /// A person's avatar photo and its mime type, or None. The one read of photo
    /// bytes, so moving media out of the profile (#56) changes only this.
    pub fn person_photo(&self, identity: &[u8; 32]) -> Result<Option<(String, String)>, CoreError> {
        let card = if *identity == self.id.identity_pk() {
            Some(self.my_profile()?.2)
        } else {
            self.peer_profile(identity)?.map(|p| p.card)
        };
        Ok(card.filter(|c| !c.photo.is_empty()).map(|c| (c.photo, c.photo_mime)))
    }
}
