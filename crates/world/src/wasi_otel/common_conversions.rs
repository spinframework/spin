use crate::wasi::{self, clocks0_2_0::wall_clock};
use serde::{
    de::{self, SeqAccess, Visitor},
    Deserialize,
};
use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

impl From<wasi::otel::types::KeyValue> for opentelemetry::KeyValue {
    fn from(kv: wasi::otel::types::KeyValue) -> Self {
        let owned: OwnedValue = from_json(&kv.value);
        let value: opentelemetry::Value = owned.into();
        opentelemetry::KeyValue::new(kv.key, value)
    }
}

impl From<&wasi::otel::types::KeyValue> for opentelemetry::KeyValue {
    fn from(kv: &wasi::otel::types::KeyValue) -> Self {
        let owned: OwnedValue = from_json(&kv.value);
        let value: opentelemetry::Value = owned.into();
        opentelemetry::KeyValue::new(kv.key.to_owned(), value)
    }
}

impl From<OwnedValue> for opentelemetry::Value {
    fn from(value: OwnedValue) -> Self {
        match value {
            OwnedValue::String(s) => opentelemetry::Value::String(s.into()),
            OwnedValue::Bool(v) => opentelemetry::Value::Bool(v),
            OwnedValue::F64(v) => opentelemetry::Value::F64(v),
            OwnedValue::I64(v) => opentelemetry::Value::I64(v),
            OwnedValue::Array(arr) => opentelemetry::Value::Array(match arr {
                OwnedArray::Bool(v) => opentelemetry::Array::Bool(v),
                OwnedArray::F64(v) => opentelemetry::Array::F64(v),
                OwnedArray::I64(v) => opentelemetry::Array::I64(v),
                OwnedArray::String(v) => opentelemetry::Array::String(
                    v.iter()
                        .map(|e| opentelemetry::StringValue::from(e.to_owned()))
                        .collect(),
                ),
            }),
        }
    }
}

enum OwnedValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    Array(OwnedArray),
}

enum OwnedArray {
    Bool(Vec<bool>),
    I64(Vec<i64>),
    F64(Vec<f64>),
    String(Vec<String>),
}

impl<'de> Deserialize<'de> for OwnedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ValueVisitor;

        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = OwnedValue;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a boolean, number, string, or array")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedValue::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedValue::I64(value))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedValue::F64(value))
            }

            /// u64 isn't an option in the OpenTelemetry Rust SDK; however, Serde may interpret a JSON number as u64.
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                i64::try_from(value)
                    .map(|v| Ok(OwnedValue::I64(v)))
                    .map_err(|_| de::Error::custom("Integer too large for i64"))?
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedValue::String(value.to_owned()))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut elements = Vec::new();

                // Determine the type by looking at the first element
                if let Some(first) = seq.next_element::<serde_json::Value>()? {
                    elements.push(first);

                    // Collect remaining elements
                    while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                        elements.push(elem);
                    }

                    if elements.is_empty() {
                        return Ok(OwnedValue::Array(OwnedArray::Bool(vec![])));
                    }

                    match &elements[0] {
                        serde_json::Value::Bool(_) => {
                            let bools: Result<Vec<bool>, _> = elements
                                .iter()
                                .map(|v| {
                                    v.as_bool()
                                        .ok_or_else(|| de::Error::custom("Mixed types in array"))
                                })
                                .collect();
                            Ok(OwnedValue::Array(OwnedArray::Bool(bools?)))
                        }
                        serde_json::Value::Number(n) if n.is_i64() => {
                            let ints: Result<Vec<i64>, _> = elements
                                .iter()
                                .map(|v| {
                                    v.as_i64()
                                        .ok_or_else(|| de::Error::custom("Mixed types in array"))
                                })
                                .collect();
                            Ok(OwnedValue::Array(OwnedArray::I64(ints?)))
                        }
                        serde_json::Value::Number(n) if n.is_f64() => {
                            let nums: Result<Vec<f64>, _> = elements
                                .iter()
                                .map(|v| {
                                    v.as_f64()
                                        .ok_or_else(|| de::Error::custom("Mixed types in array"))
                                })
                                .collect();
                            Ok(OwnedValue::Array(OwnedArray::F64(nums?)))
                        }
                        serde_json::Value::String(_) => {
                            let strings: Result<Vec<String>, _> = elements
                                .iter()
                                .map(|v| match v.as_str() {
                                    Some(s) => Ok(s.to_string()),
                                    None => Err(de::Error::custom("Mixed types in array")),
                                })
                                .collect();
                            Ok(OwnedValue::Array(OwnedArray::String(strings?)))
                        }
                        _ => Err(de::Error::custom("Unsupported array element type")),
                    }
                } else {
                    // Empty array using bool as the default
                    Ok(OwnedValue::Array(OwnedArray::Bool(vec![])))
                }
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

// Deserialize a JSON string to a Serde-serializable struct
pub(crate) fn from_json<T: for<'de> Deserialize<'de>>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|e| {
        panic!(
            "Failed to deserialize JSON to {}\
             \n Input: {}\
             \n Error: {}",
            std::any::type_name::<T>(),
            json,
            e
        )
    })
}

impl From<wasi::otel::types::InstrumentationScope> for opentelemetry::InstrumentationScope {
    fn from(value: wasi::otel::types::InstrumentationScope) -> Self {
        let builder =
            Self::builder(value.name).with_attributes(value.attributes.into_iter().map(Into::into));
        match (value.version, value.schema_url) {
            (Some(version), Some(schema_url)) => builder
                .with_version(version)
                .with_schema_url(schema_url)
                .build(),
            (Some(version), None) => builder.with_version(version).build(),
            (None, Some(schema_url)) => builder.with_schema_url(schema_url).build(),
            (None, None) => builder.build(),
        }
    }
}

impl From<wall_clock::Datetime> for SystemTime {
    fn from(timestamp: wall_clock::Datetime) -> Self {
        UNIX_EPOCH
            + Duration::from_secs(timestamp.seconds)
            + Duration::from_nanos(timestamp.nanoseconds as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! compare_json_and_literal {
        ($json:expr, $literal:expr) => {{
            let left: serde_json::Value = from_json($json);
            let right: serde_json::Value = serde_json::json!($literal);
            assert_eq!(left, right);
        }};
    }

    #[test]
    fn deserialize_from_json_to_otel_value() {
        compare_json_and_literal!("false", false);
        compare_json_and_literal!("[false,true,true]", vec![false, true, true]);
        compare_json_and_literal!("6", 6);
        compare_json_and_literal!("[1,2,3,4]", vec![1, 2, 3, 4]);
        compare_json_and_literal!("-6", -6);
        compare_json_and_literal!("[-1,-2,-3,-4]", vec![-1, -2, -3, -4]);
        compare_json_and_literal!("123.456", 123.456);
        compare_json_and_literal!("[1.0,2.1,3.2,4.3]", vec![1.0, 2.1, 3.2, 4.3]);
        compare_json_and_literal!("\"test\"", "test");
        compare_json_and_literal!(
            "[\"Hello, world!\",\"Goodnight, moon.\"]",
            vec!["Hello, world!", "Goodnight, moon."]
        );
    }
}
