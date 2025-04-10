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
        include wasi:keyvalue/imports@0.2.0-draft2;
    }
    "#,
    path: "../../wit",
    async: true,
    // The following is a roundabout way of saying "the host implementations for these interfaces don't trap"
    trappable_error_type: {
        "fermyon:spin/config/error" => v1::config::Error,
        "fermyon:spin/http-types/http-error" => v1::http_types::HttpError,
        "fermyon:spin/llm@2.0.0/error" => v2::llm::Error,
        "fermyon:spin/llm/error" => v1::llm::Error,
        "fermyon:spin/mqtt@2.0.0/error" => v2::mqtt::Error,
        "fermyon:spin/mysql/mysql-error" => v1::mysql::MysqlError,
        "fermyon:spin/postgres/pg-error" => v1::postgres::PgError,
        "fermyon:spin/rdbms-types@2.0.0/error" => v2::rdbms_types::Error,
        "fermyon:spin/redis-types/error" => v1::redis_types::Error,
        "fermyon:spin/redis@2.0.0/error" => v2::redis::Error,
        "fermyon:spin/sqlite@2.0.0/error" => v2::sqlite::Error,
        "fermyon:spin/sqlite/error" => v1::sqlite::Error,
        "fermyon:spin/variables@2.0.0/error" => v2::variables::Error,
        "spin:postgres/postgres/error" => spin::postgres::postgres::Error,
        "spin:sqlite/sqlite/error" => spin::sqlite::sqlite::Error,
        "wasi:config/store@0.2.0-draft-2024-09-27/error" => wasi::config::store::Error,
        "wasi:keyvalue/store/error" => wasi::keyvalue::store::Error,
        "wasi:keyvalue/atomics/cas-error" => wasi::keyvalue::atomics::CasError,
    },
    trappable_imports: true,
});

pub use fermyon::spin as v1;
pub use fermyon::spin2_0_0 as v2;

mod conversions;
