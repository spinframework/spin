use super::from_json;
use crate::wasi;
use base64::Engine;
use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use serde::de::{SeqAccess, Visitor};
use serde::{de, Deserialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

pub fn parse_wasi_log_record(
    wasi_log_record: wasi::otel::logs::LogRecord,
) -> (
    opentelemetry_sdk::logs::SdkLogRecord,
    opentelemetry::InstrumentationScope,
) {
    let log_provider = {
        let mut provider = SdkLoggerProvider::builder();
        if let Some(resource) = wasi_log_record.resource {
            let otel_resource = resource.into();
            provider = provider.with_resource(otel_resource);
        }
        provider.build()
    };

    let logger = log_provider.logger("spin-logger");

    // Parse LogRecord
    let mut otel_log_record = logger.create_log_record();
    if let Some(body) = wasi_log_record.body {
        let owned: OwnedAnyValue = from_json(&body);
        let otel_body: opentelemetry::logs::AnyValue = owned.into();
        otel_log_record.set_body(otel_body);
    }
    if let Some(name) = wasi_log_record.event_name {
        otel_log_record.set_event_name(Box::leak(name.into_boxed_str()));
    }
    if let Some(timestamp) = wasi_log_record.observed_timestamp {
        otel_log_record.set_observed_timestamp(timestamp.into());
    }
    if let Some(number) = wasi_log_record.severity_number {
        otel_log_record.set_severity_number(severity_from_u8(number));
    }
    if let Some(text) = wasi_log_record.severity_text {
        otel_log_record.set_severity_text(Box::leak(text.into_boxed_str()));
    }
    if let Some(timestamp) = wasi_log_record.timestamp {
        otel_log_record.set_timestamp(timestamp.into());
    }
    if let Some(trace_id) = wasi_log_record.trace_id {
        if let Some(span_id) = wasi_log_record.span_id {
            // Both the span ID and trace ID are required values to set trace context.
            otel_log_record.set_trace_context(
                opentelemetry::TraceId::from_hex(&trace_id).expect("Failed to parse trace ID"),
                opentelemetry::SpanId::from_hex(&span_id).expect("Failed to parse span ID"),
                wasi_log_record.trace_flags.map(Into::into),
            );
        }
    }

    // Parse InstrumentationScope
    let otel_scope = if let Some(wasi_scope) = wasi_log_record.instrumentation_scope {
        let attrs: Vec<opentelemetry::KeyValue> = wasi_scope
            .attributes
            .iter()
            .map(|e| {
                let kv: opentelemetry::KeyValue = e.into();
                kv
            })
            .collect();
        let mut scope = opentelemetry::InstrumentationScope::builder(Cow::Owned(wasi_scope.name))
            .with_attributes(attrs);
        if let Some(url) = wasi_scope.schema_url {
            scope = scope.with_schema_url(Cow::Owned(url));
        }
        if let Some(version) = wasi_scope.version {
            scope = scope.with_version(version);
        }
        scope.build()
    } else {
        opentelemetry::InstrumentationScope::default()
    };

    (otel_log_record, otel_scope)
}

fn severity_from_u8(n: u8) -> opentelemetry::logs::Severity {
    use opentelemetry::logs::Severity::*;
    match n {
        0 => {
            // In the spec, there is a brief mention of SeverityNumber=0 used to represent an unspecified severity;
            // however, this version of OpenTelemetry Rust doesn't implement it.
            // See https://opentelemetry.io/docs/specs/otel/logs/data-model/#comparing-severity
            unimplemented!()
        }
        1 => Trace,
        2 => Trace2,
        3 => Trace3,
        4 => Trace4,
        5 => Debug,
        6 => Debug2,
        7 => Debug3,
        8 => Debug4,
        9 => Info,
        10 => Info2,
        11 => Info3,
        12 => Info4,
        13 => Warn,
        14 => Warn2,
        15 => Warn3,
        16 => Warn4,
        17 => Error,
        18 => Error2,
        19 => Error3,
        20 => Error4,
        21 => Fatal,
        22 => Fatal2,
        23 => Fatal3,
        24 => Fatal4,
        num => panic!("{num} is not a valid severity number"),
    }
}

#[derive(Clone)]
enum OwnedAnyValue {
    Int(i64),
    Double(f64),
    String(String),
    Boolean(bool),
    Bytes(Vec<u8>),
    ListAny(Vec<OwnedAnyValue>),
    Map(HashMap<String, OwnedAnyValue>),
}

impl From<OwnedAnyValue> for opentelemetry::logs::AnyValue {
    fn from(value: OwnedAnyValue) -> Self {
        use opentelemetry::logs::AnyValue;
        match value {
            OwnedAnyValue::Boolean(v) => AnyValue::Boolean(v),
            OwnedAnyValue::Bytes(v) => AnyValue::Bytes(Box::new(v.to_vec())),
            OwnedAnyValue::Double(v) => AnyValue::Double(v),
            OwnedAnyValue::Int(v) => AnyValue::Int(v),
            OwnedAnyValue::String(v) => AnyValue::String(v.clone().into()),
            OwnedAnyValue::ListAny(v) => {
                AnyValue::ListAny(Box::new(v.iter().map(|e| e.clone().into()).collect()))
            }
            OwnedAnyValue::Map(v) => AnyValue::Map(Box::new(
                v.iter()
                    .map(|(k, v)| (k.clone().into(), v.clone().into()))
                    .collect(),
            )),
        }
    }
}

impl<'de> Deserialize<'de> for OwnedAnyValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AnyValueVisitor;

        impl<'de> Visitor<'de> for AnyValueVisitor {
            type Value = OwnedAnyValue;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a boolean, number, string, or array")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedAnyValue::Boolean(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedAnyValue::Int(value))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OwnedAnyValue::Double(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if let Some(stripped) = value.strip_prefix("data:application/octet-stream;base64,")
                {
                    // Handle byte array
                    base64::engine::general_purpose::STANDARD
                        .decode(stripped)
                        .map(OwnedAnyValue::Bytes)
                        .map_err(|e| de::Error::custom(e))
                } else {
                    // Handle String
                    Ok(OwnedAnyValue::String(value.to_owned()))
                }
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut result = HashMap::new();
                // Recursively deserialize each key-value pair.
                while let Some((key, value)) = map.next_entry::<String, OwnedAnyValue>()? {
                    result.insert(key, value);
                }

                Ok(OwnedAnyValue::Map(result))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut elements = Vec::new();

                // Recursively deserialize each element.
                while let Some(elem) = seq.next_element::<OwnedAnyValue>()? {
                    elements.push(elem);
                }

                Ok(OwnedAnyValue::ListAny(elements))
            }
        }

        deserializer.deserialize_any(AnyValueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_json_to_otel_log_any_value() {
        let test_json = "{\"key1\": false, \"key2\": 123.456, \"key3\": 41, \"key4\": \"data:application/octet-stream;base64,SGVsbG8sIHdvcmxkIQ==\", \"key5\": \"This is a string\", \"key6\": [1, 2, 3], \"key7\": {\"nestedkey1\": \"Hello, from within!\"}}";
        let expected: serde_json::Value = serde_json::json!({
            "key1": false,
            "key2": 123.456,
            "key3": 41,
            //'Hello, world!' encoded to base64
            "key4": "data:application/octet-stream;base64,SGVsbG8sIHdvcmxkIQ==",
            "key5": "This is a string",
            "key6": [1, 2, 3],
            "key7": {
                "nestedkey1": "Hello, from within!"
            }
        });
        let actual: serde_json::Value = from_json(test_json);
        assert_eq!(expected, actual);
    }
}
