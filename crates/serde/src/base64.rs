//! Base64 (de)serialization

use std::borrow::Cow;

use base64::{Engine, engine::GeneralPurpose, prelude::BASE64_STANDARD_NO_PAD};
use serde::{Deserialize, Deserializer, Serializer, de};

const BASE64: GeneralPurpose = BASE64_STANDARD_NO_PAD;

/// Serializes to base64.
pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match bytes {
        Some(bytes) => serializer.serialize_str(&BASE64.encode(bytes)),
        None => serializer.serialize_none(),
    }
}

/// Deserializes from base64.
pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Cow<str>>::deserialize(deserializer)? {
        Some(s) => Ok(Some(BASE64.decode(s.as_ref()).map_err(de::Error::custom)?)),
        None => Ok(None),
    }
}
