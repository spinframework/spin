#![allow(missing_docs)]
#![allow(non_camel_case_types)] // bindgen emits Host_Pre and Host_Indices

pub use async_trait::async_trait;

wasmtime::component::bindgen!({
    inline: r#"
    package fermyon:runtime;
    world host {
        include fermyon:spin/host;
        include fermyon:spin/platform@2.0.0;
        include fermyon:spin/platform@3.0.0;
        include spin:up/platform@3.2.0;
        include spin:up/platform@3.4.0;
        include spin:up/platform@3.6.0;
        include wasi:keyvalue/imports@0.2.0-draft2;
    }
    "#,
    path: "../../wit",
    imports: { default: async | trappable },
    exports: { default: async },
    // The following is a roundabout way of saying "the host implementations for these interfaces don't trap"
    trappable_error_type: {
        "fermyon:spin/config.error" => v1::config::Error,
        "fermyon:spin/http-types.http-error" => v1::http_types::HttpError,
        "fermyon:spin/llm@2.0.0.error" => v2::llm::Error,
        "fermyon:spin/llm.error" => v1::llm::Error,
        "fermyon:spin/mqtt@2.0.0.error" => v2::mqtt::Error,
        "fermyon:spin/mysql.mysql-error" => v1::mysql::MysqlError,
        "fermyon:spin/postgres.pg-error" => v1::postgres::PgError,
        "fermyon:spin/rdbms-types@2.0.0.error" => v2::rdbms_types::Error,
        "fermyon:spin/redis-types.error" => v1::redis_types::Error,
        "fermyon:spin/redis@2.0.0.error" => v2::redis::Error,
        "fermyon:spin/sqlite@2.0.0.error" => v2::sqlite::Error,
        "fermyon:spin/sqlite.error" => v1::sqlite::Error,
        "fermyon:spin/variables@2.0.0.error" => v2::variables::Error,
        "spin:postgres/postgres@3.0.0.error" => spin::postgres3_0_0::postgres::Error,
        "spin:postgres/postgres@4.0.0.error" => spin::postgres4_0_0::postgres::Error,
        "spin:sqlite/sqlite.error" => spin::sqlite::sqlite::Error,
        "wasi:config/store@0.2.0-draft-2024-09-27.error" => wasi::config::store::Error,
        "wasi:keyvalue/store.error" => wasi::keyvalue::store::Error,
        "wasi:keyvalue/atomics.cas-error" => wasi::keyvalue::atomics::CasError,
    },
});

pub use fermyon::spin as v1;
pub use fermyon::spin2_0_0 as v2;

mod conversions;
pub mod wasi_otel;

/// Maximum allowed size of a host-buffered database query result, HTTP request
/// or response body, or similar.
///
/// If and when Spin encounters content larger than this
// TODO: make this configurable
pub const MAX_HOST_BUFFERED_BYTES: usize = 128 << 20;

impl spin::sqlite::sqlite::Value {
    pub fn memory_size(&self) -> usize {
        match self {
            Self::Null | Self::Integer(_) | Self::Real(_) => std::mem::size_of::<Self>(),
            Self::Text(t) => std::mem::size_of::<Self>() + t.len(),
            Self::Blob(b) => std::mem::size_of::<Self>() + b.len(),
        }
    }
}

impl spin::postgres4_0_0::postgres::DbValue {
    pub fn memory_size(&self) -> usize {
        match self {
            Self::DbNull
            | Self::Boolean(_)
            | Self::Int8(_)
            | Self::Int16(_)
            | Self::Int32(_)
            | Self::Int64(_)
            | Self::Floating32(_)
            | Self::Floating64(_)
            | Self::Date(_)
            | Self::Time(_)
            | Self::Datetime(_)
            | Self::Timestamp(_)
            | Self::RangeInt32(_)
            | Self::RangeInt64(_)
            | Self::RangeDecimal(_)
            | Self::Interval(_) => std::mem::size_of::<Self>(),
            Self::ArrayInt32(v) => {
                std::mem::size_of::<Self>() + (v.len() * std::mem::size_of::<Option<i32>>())
            }
            Self::ArrayInt64(v) => {
                std::mem::size_of::<Self>() + (v.len() * std::mem::size_of::<Option<i64>>())
            }
            Self::ArrayDecimal(v) | Self::ArrayStr(v) => {
                std::mem::size_of::<Self>()
                    + v.iter()
                        .map(|v| {
                            std::mem::size_of::<Option<String>>()
                                + v.as_ref().map(|s| s.len()).unwrap_or(0)
                        })
                        .sum::<usize>()
            }
            Self::Str(s) | Self::Uuid(s) | Self::Decimal(s) => {
                std::mem::size_of::<Self>() + s.len()
            }
            Self::Jsonb(b) | Self::Binary(b) | Self::Unsupported(b) => {
                std::mem::size_of::<Self>() + b.len()
            }
        }
    }
}

impl v2::rdbms_types::DbValue {
    pub fn memory_size(&self) -> usize {
        match self {
            Self::DbNull
            | Self::Unsupported
            | Self::Boolean(_)
            | Self::Int8(_)
            | Self::Int16(_)
            | Self::Int32(_)
            | Self::Int64(_)
            | Self::Uint8(_)
            | Self::Uint16(_)
            | Self::Uint32(_)
            | Self::Uint64(_)
            | Self::Floating32(_)
            | Self::Floating64(_) => std::mem::size_of::<Self>(),
            Self::Str(s) => std::mem::size_of::<Self>() + s.len(),
            Self::Binary(b) => std::mem::size_of::<Self>() + b.len(),
        }
    }
}
