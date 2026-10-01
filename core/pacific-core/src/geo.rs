//! Location — a COMMON facet of every GroupObject (things, orgs, places, and
//! optionally people). Location is NOT place-specific: any object may carry one.
//!
//! # Two shapes, one primitive
//!
//! A location comes in two shapes, and the distinction is the whole point:
//!
//! - **Fixed** — a static coordinate (a Place, an org address). The literal point is
//!   carried in the delta and folds into the object's state.
//! - **Stream** — a LIVE position (a moving car, a person). *A live position is a
//!   stream, not a series of granular deltas* — authoring a `setLocation` per GPS
//!   tick would flood the append-only log. So the delta carries only the information
//!   needed to ACCESS the stream (a [`LocationStream`] reference), never the positions
//!   themselves; the positions flow out-of-band over that stream.
//!
//! The persisted primitive is identical for both shapes — one single-slot
//! location-source facet per object, owner-sequenced — only the payload
//! ([`LocationSource`]) differs. This mirrors the "live handle, not a copy" pattern
//! the System connectors use (a node persists *how to reach* a live value, not the
//! value), and the Instagram-Map separation of a canonical fixed coordinate from a
//! live device-position stream (the two are never conflated).
//!
//! # Wiring (the base op-group)
//!
//! Location is a BASE op-group, like the role/members base ops: available on every
//! object kind, carried by most. An object that carries location embeds a
//! `location: Option<LocationSource>` in its `State`, includes [`LOCATION_OPS`] in its
//! `ops()`, and delegates [`OP_SET_LOCATION`]/[`OP_CLEAR_LOCATION`] to
//! [`reduce_location`] from its `reduce`. Base ops take a RESERVED high op-id band so
//! they never collide with a type's own ops (which number from 0).

use serde::{Deserialize, Serialize};

use crate::coordinator::{ArgVal, Args};
use crate::object::{Authority, Commutativity, DeltaRejection, Op, OpDecl};
use crate::object_args::req_text;

/// A concrete geographic point + descriptive metadata.
///
/// Coordinates are stored as fixed-point **e7** integers (degrees × 1e7), NOT floats:
/// the value is then exact, `Eq`/hashable, and folds DETERMINISTICALLY on every
/// replica — float non-determinism would break log replay. This is the shape a fixed
/// location carries, and the shape a stream yields per tick once resolved.
///
/// Every non-coordinate field is `#[serde(default)]`, so the payload is additively
/// extensible (a peer on an older build folds unknown fields away), exactly like
/// `ContactCard`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GeoPoint {
    /// Latitude in 1e-7 degrees (WGS84). Valid range `[-900_000_000, 900_000_000]`.
    pub lat_e7: i32,
    /// Longitude in 1e-7 degrees (WGS84). Valid range `[-1_800_000_000, 1_800_000_000]`.
    pub lng_e7: i32,
    /// Human place name ("Bristol", "Sheep Lane"), "" when unnamed.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub category: String,
    /// Two-ID provenance (the Instagram `pk` + `external_id_source` pattern): a stable
    /// foreign key back to whatever POI source this point was picked from, so it can be
    /// deduped / re-resolved later. "" when hand-dropped.
    #[serde(default)]
    pub external_id: String,
    /// The provider that minted `external_id`: "apple_mapkit" | "manual" | … "" when none.
    #[serde(default)]
    pub external_source: String,
}

impl GeoPoint {
    pub const LAT_MAX_E7: i32 = 900_000_000;
    pub const LNG_MAX_E7: i32 = 1_800_000_000;

    /// Build a point from decimal degrees (rounds to e7). Metadata left empty.
    pub fn from_degrees(lat: f64, lng: f64) -> Self {
        Self {
            lat_e7: (lat * 1e7).round() as i32,
            lng_e7: (lng * 1e7).round() as i32,
            ..Default::default()
        }
    }

    pub fn lat(&self) -> f64 {
        self.lat_e7 as f64 / 1e7
    }
    pub fn lng(&self) -> f64 {
        self.lng_e7 as f64 / 1e7
    }

    /// A real WGS84 coordinate. A reducer rejects an out-of-range point (MalformedArgs)
    /// rather than storing nonsense.
    pub fn is_valid(&self) -> bool {
        self.lat_e7.abs() <= Self::LAT_MAX_E7 && self.lng_e7.abs() <= Self::LNG_MAX_E7
    }
}

/// How to ACCESS a live location stream — carried in the delta IN PLACE OF the
/// positions. The transport is intentionally abstract so a new one needs no schema
/// bump: `kind` names it, `reference` addresses it within that kind, and `credential`
/// is the (content-free) vault handle id when the source needs auth to read — the SAME
/// `CredentialHandle` the device-local vault stores a secret under, so a location
/// System is reached exactly like any other System tool.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocationStream {
    /// What vends the positions: "device" (a member's own CoreLocation self-report),
    /// "system" (a connector/tool that returns a position), "relay" (a topic positions
    /// are published to), …
    #[serde(default)]
    pub kind: String,
    /// The address of the stream within `kind` — a channel/topic id, a tool name, a
    /// member IdentityKey for a device self-report, ….
    #[serde(default)]
    pub reference: String,
    /// Vault credential handle id when the stream needs auth; "" when public.
    #[serde(default)]
    pub credential: String,
    /// Freshness hint in seconds (how often the source updates); 0 = unknown.
    #[serde(default)]
    pub cadence_secs: u32,
}

/// The common location facet's value — the ONE thing a location delta carries and a
/// GroupObject folds, single-slot per object (its CURRENT source). Either a fixed
/// literal point, or a reference for accessing a live stream. A `Stream` deliberately
/// carries NO coordinate: positions are out-of-band, so a "last known" point is stream
/// cache, never a persisted delta.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum LocationSource {
    /// A static coordinate carried literally — places, org addresses.
    Fixed { point: GeoPoint },
    /// A live position — positions stream out-of-band; this is ONLY the access.
    Stream { access: LocationStream },
}

// ---- the base location op-group (common to every GroupObject) ----------------

/// Reserved base-op band: per-type ops are small integers from 0, base ops live up
/// here so the two can never collide.
pub const OP_SET_LOCATION: u32 = 0xF000_0000;
pub const OP_CLEAR_LOCATION: u32 = 0xF000_0001;

/// The base location ops, to be included in a type's `ops()`. Owner/sequenced: the
/// object's owner curates its location SOURCE (which changes rarely — positions
/// stream separately), matching `setProfile`/`setPresence`.
pub static LOCATION_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_LOCATION,
        name: "base.setLocation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_LOCATION,
        name: "base.clearLocation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// True if `op_id` is a base location op — so an object can route it to
/// [`reduce_location`] before (or within) its own op match.
pub fn is_location_op(op_id: u32) -> bool {
    op_id == OP_SET_LOCATION || op_id == OP_CLEAR_LOCATION
}

/// Fold a base location op into an object's single-slot `location` facet. Owner /
/// sequenced (the spine already gated authority), so this just parses + applies
/// atomically — every arg is validated BEFORE the first mutation, so a rejected op
/// leaves the slot untouched. Returns `UnknownType` for a non-location op.
pub fn reduce_location(
    location: &mut Option<LocationSource>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_LOCATION => {
            let json = req_text(op.args, "source")?;
            let source: LocationSource =
                serde_json::from_str(json).map_err(|_| DeltaRejection::MalformedArgs)?;
            match &source {
                // A fixed point must be a real coordinate; a stream must at least name
                // a transport, or it addresses nothing.
                LocationSource::Fixed { point } if !point.is_valid() => {
                    return Err(DeltaRejection::MalformedArgs);
                }
                LocationSource::Stream { access } if access.kind.is_empty() => {
                    return Err(DeltaRejection::MalformedArgs);
                }
                _ => {}
            }
            *location = Some(source);
            Ok(())
        }
        OP_CLEAR_LOCATION => {
            *location = None;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The one text arg a `setLocation` delta carries: the JSON-encoded [`LocationSource`].
/// Author helper for node/FFI, mirroring how `setProfile` carries its `card` JSON arg.
pub fn set_location_args(source: &LocationSource) -> Args {
    let mut a = Args::new();
    a.insert(
        "source".into(),
        ArgVal::Text(serde_json::to_string(source).expect("LocationSource always serializes")),
    );
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ReduceContext;

    const OWNER: [u8; 32] = [7u8; 32];

    /// Apply one base location op to a slot, building a minimal owner-sequenced Op.
    fn apply(
        location: &mut Option<LocationSource>,
        op_id: u32,
        args: &Args,
    ) -> Result<(), DeltaRejection> {
        let members = [OWNER];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let op = Op {
            op_id,
            args,
            author: &OWNER,
            pos: None,
            ctx: &ctx,
        };
        reduce_location(location, &op)
    }

    #[test]
    fn e7_round_trips_and_validates() {
        let p = GeoPoint::from_degrees(51.4545, -2.5879); // Bristol
        assert!((p.lat() - 51.4545).abs() < 1e-6);
        assert!((p.lng() - (-2.5879)).abs() < 1e-6);
        assert!(p.is_valid());
        assert!(!GeoPoint {
            lat_e7: GeoPoint::LAT_MAX_E7 + 1,
            lng_e7: 0,
            ..Default::default()
        }
        .is_valid());
    }

    #[test]
    fn set_fixed_folds_a_literal_point() {
        let src = LocationSource::Fixed {
            point: GeoPoint::from_degrees(51.4545, -2.5879),
        };
        let mut loc = None;
        apply(&mut loc, OP_SET_LOCATION, &set_location_args(&src)).unwrap();
        assert_eq!(loc, Some(src));
    }

    #[test]
    fn set_stream_folds_only_the_access_not_a_position() {
        let src = LocationSource::Stream {
            access: LocationStream {
                kind: "device".into(),
                reference: "ed25519:abc".into(),
                credential: String::new(),
                cadence_secs: 30,
            },
        };
        let mut loc = None;
        apply(&mut loc, OP_SET_LOCATION, &set_location_args(&src)).unwrap();
        assert_eq!(loc, Some(src));
    }

    #[test]
    fn clear_removes_the_source() {
        let mut loc = Some(LocationSource::Fixed {
            point: GeoPoint::from_degrees(0.0, 0.0),
        });
        apply(&mut loc, OP_CLEAR_LOCATION, &Args::new()).unwrap();
        assert_eq!(loc, None);
    }

    #[test]
    fn rejects_malformed_out_of_range_and_kindless_stream_atomically() {
        let mut loc = None;

        let mut bad_json = Args::new();
        bad_json.insert("source".into(), ArgVal::Text("not json".into()));
        assert_eq!(
            apply(&mut loc, OP_SET_LOCATION, &bad_json),
            Err(DeltaRejection::MalformedArgs)
        );

        let out_of_range = LocationSource::Fixed {
            point: GeoPoint {
                lat_e7: 999_000_000,
                lng_e7: 0,
                ..Default::default()
            },
        };
        assert_eq!(
            apply(&mut loc, OP_SET_LOCATION, &set_location_args(&out_of_range)),
            Err(DeltaRejection::MalformedArgs)
        );

        let kindless = LocationSource::Stream {
            access: LocationStream::default(),
        };
        assert_eq!(
            apply(&mut loc, OP_SET_LOCATION, &set_location_args(&kindless)),
            Err(DeltaRejection::MalformedArgs)
        );

        // A non-location op is not ours.
        assert_eq!(
            apply(&mut loc, 0, &Args::new()),
            Err(DeltaRejection::UnknownType)
        );

        // None of the rejects mutated the slot.
        assert_eq!(loc, None);
    }

    #[test]
    fn json_is_tagged_and_a_stream_carries_no_coordinate() {
        let stream = serde_json::to_string(&LocationSource::Stream {
            access: LocationStream {
                kind: "device".into(),
                ..Default::default()
            },
        })
        .unwrap();
        assert!(stream.contains("\"shape\":\"stream\""));
        assert!(!stream.contains("lat_e7")); // the whole point: a stream carries no position

        let fixed = serde_json::to_string(&LocationSource::Fixed {
            point: GeoPoint::from_degrees(1.0, 2.0),
        })
        .unwrap();
        assert!(fixed.contains("\"shape\":\"fixed\""));
        assert!(fixed.contains("lat_e7"));
    }
}
