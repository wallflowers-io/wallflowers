//! Byte fields as CBOR byte strings, for `#[serde(with = ...)]`. Shared with the
//! engine's intro payload so both encode a byte field one way.

pub mod bytes_vec {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let b = serde_bytes::ByteBuf::deserialize(d)?;
        Ok(b.into_vec())
    }
}

pub mod bytes32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let b = serde_bytes::ByteBuf::deserialize(d)?;
        b.into_vec()
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

pub mod bytes64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let b = serde_bytes::ByteBuf::deserialize(d)?;
        b.into_vec()
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))
    }
}
