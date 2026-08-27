use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<T, S>(vec: &Vec<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    if vec.len() == 1 {
        vec[0].serialize(serializer)
    } else {
        vec.serialize(serializer)
    }
}

pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let value = toml::Value::deserialize(deserializer)?;
    // NOTE: We explicitly check for array first rather than trying T::deserialize
    // first, because toml's serde impl will treat an array as a sequence of fields
    // to be assigned to struct members (e.g. Component), producing nonsensical results.
    if let Some(arr) = value.as_array() {
        arr.iter()
            .map(|v| T::deserialize(v.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::de::Error::custom)
    } else {
        T::deserialize(value)
            .map(|v| vec![v])
            .map_err(serde::de::Error::custom)
    }
}
