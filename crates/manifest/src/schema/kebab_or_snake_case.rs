use serde::{Deserialize, Serialize};
pub use spin_serde::{KebabId, SnakeId};

pub fn serialize<S>(value: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    if value
        .iter()
        .all(|s| KebabId::try_from(s.clone()).is_ok() || SnakeId::try_from(s.to_owned()).is_ok())
    {
        value.serialize(serializer)
    } else {
        Err(serde::ser::Error::custom(
            "expected kebab-case or snake_case",
        ))
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = toml::Value::deserialize(deserializer)?;
    let list: Vec<String> = Vec::deserialize(value).map_err(serde::de::Error::custom)?;
    if list
        .iter()
        .all(|s| KebabId::try_from(s.clone()).is_ok() || SnakeId::try_from(s.to_owned()).is_ok())
    {
        Ok(list)
    } else {
        Err(serde::de::Error::custom(
            "expected kebab-case or snake_case",
        ))
    }
}
