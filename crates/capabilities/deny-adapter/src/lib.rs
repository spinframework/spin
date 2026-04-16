wit_bindgen::generate!({
    path: "../../../wit",
    generate_all,
    inline: r#"
    package spin:deny;

    world adapter {
        export wasi:http/outgoing-handler@0.2.6;
        export wasi:http/client@0.3.0-rc-2026-03-15;
        export spin:key-value/key-value@3.0.0;
        export spin:mqtt/mqtt@3.0.0;
        export spin:postgres/postgres@3.0.0;
        export spin:postgres/postgres@4.2.0;
        export spin:redis/redis@3.0.0;
        export spin:sqlite/sqlite@3.1.0;
        export spin:variables/variables@3.0.0;
        export wasi:config/store@0.2.0-draft-2024-09-27;
        export fermyon:spin/config;
        export fermyon:spin/http;
        export fermyon:spin/key-value;
        export fermyon:spin/llm;
        export fermyon:spin/mysql;
        export fermyon:spin/postgres;
        export fermyon:spin/redis;
        export fermyon:spin/sqlite;
        export wasi:cli/environment@0.2.6;
        export wasi:filesystem/preopens@0.2.6;
        export wasi:sockets/udp@0.2.6;
        export wasi:sockets/udp-create-socket@0.2.6;
        export wasi:sockets/tcp@0.2.6;
        export wasi:sockets/tcp-create-socket@0.2.6;
        export wasi:sockets/ip-name-lookup@0.2.6;
        export wasi:cli/environment@0.3.0-rc-2026-03-15;
        export wasi:filesystem/preopens@0.3.0-rc-2026-03-15;
        export wasi:sockets/ip-name-lookup@0.3.0-rc-2026-03-15;
        export fermyon:spin/llm@2.0.0;
        export fermyon:spin/redis@2.0.0;
        export fermyon:spin/mqtt@2.0.0;
        export fermyon:spin/postgres@2.0.0;
        export fermyon:spin/mysql@2.0.0;
        export fermyon:spin/sqlite@2.0.0;
        export fermyon:spin/key-value@2.0.0;
        export fermyon:spin/variables@2.0.0;
        export wasi:keyvalue/store@0.2.0-draft2;
    }
    "#,
});

fn format_deny_error(s: &str) -> String {
    format!("{s:?} is not permitted")
}

struct Adapter;
export!(Adapter);

impl exports::wasi::http0_2_6::outgoing_handler::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn handle(
        request: exports::wasi::http0_2_6::outgoing_handler::OutgoingRequest,
        options: Option<exports::wasi::http0_2_6::outgoing_handler::RequestOptions>,
    ) -> Result<
        exports::wasi::http0_2_6::outgoing_handler::FutureIncomingResponse,
        exports::wasi::http0_2_6::outgoing_handler::ErrorCode,
    > {
        Err(
            exports::wasi::http0_2_6::outgoing_handler::ErrorCode::InternalError(Some(
                format_deny_error("wasi:http/outgoing-handler"),
            )),
        )
    }
}
impl exports::wasi::http0_3_0_rc_2026_03_15::client::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn send(
        request: exports::wasi::http0_3_0_rc_2026_03_15::client::Request,
    ) -> Result<
        exports::wasi::http0_3_0_rc_2026_03_15::client::Response,
        exports::wasi::http0_3_0_rc_2026_03_15::client::ErrorCode,
    > {
        Err(
            exports::wasi::http0_3_0_rc_2026_03_15::client::ErrorCode::InternalError(Some(
                format_deny_error("wasi:http/client"),
            )),
        )
    }
}
impl exports::spin::key_value::key_value::GuestStore for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn open(
        label: _rt::String,
    ) -> Result<
        exports::spin::key_value::key_value::Store,
        exports::spin::key_value::key_value::Error,
    > {
        Err(exports::spin::key_value::key_value::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn get(
        &self,
        key: _rt::String,
    ) -> Result<Option<_rt::Vec<u8>>, exports::spin::key_value::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn set(
        &self,
        key: _rt::String,
        value: _rt::Vec<u8>,
    ) -> Result<(), exports::spin::key_value::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn delete(
        &self,
        key: _rt::String,
    ) -> Result<(), exports::spin::key_value::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn exists(
        &self,
        key: _rt::String,
    ) -> Result<bool, exports::spin::key_value::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn get_keys(
        &self,
    ) -> (
        wit_bindgen::rt::async_support::StreamReader<_rt::String>,
        wit_bindgen::rt::async_support::FutureReader<
            Result<(), exports::spin::key_value::key_value::Error>,
        >,
    ) {
        unreachable!()
    }
}
impl exports::spin::key_value::key_value::Guest for Adapter {
    type Store = Adapter;
}
impl exports::spin::mqtt::mqtt::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn open(
        address: _rt::String,
        username: _rt::String,
        password: _rt::String,
        keep_alive_interval_in_secs: u64,
    ) -> Result<exports::spin::mqtt::mqtt::Connection, exports::spin::mqtt::mqtt::Error> {
        Err(exports::spin::mqtt::mqtt::Error::Other(format_deny_error(
            "spin:mqtt/mqtt",
        )))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn publish(
        &self,
        topic: _rt::String,
        payload: exports::spin::mqtt::mqtt::Payload,
        qos: exports::spin::mqtt::mqtt::Qos,
    ) -> Result<(), exports::spin::mqtt::mqtt::Error> {
        unreachable!()
    }
}
impl exports::spin::mqtt::mqtt::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::spin::postgres3_0_0::postgres::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        address: _rt::String,
    ) -> Result<
        exports::spin::postgres3_0_0::postgres::Connection,
        exports::spin::postgres3_0_0::postgres::Error,
    > {
        Err(exports::spin::postgres3_0_0::postgres::Error::Other(
            format_deny_error("spin:postgres/postgres"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn query(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::spin::postgres3_0_0::postgres::ParameterValue>,
    ) -> Result<
        exports::spin::postgres3_0_0::postgres::RowSet,
        exports::spin::postgres3_0_0::postgres::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::spin::postgres3_0_0::postgres::ParameterValue>,
    ) -> Result<u64, exports::spin::postgres3_0_0::postgres::Error> {
        unreachable!()
    }
}
impl exports::spin::postgres3_0_0::postgres::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::spin::postgres4_2_0::postgres::GuestConnectionBuilder for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn new(address: _rt::String) -> Self {
        Adapter {}
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_ca_root(
        &self,
        certificate: _rt::String,
    ) -> Result<(), exports::spin::postgres4_2_0::postgres::Error> {
        Ok(())
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn build(
        &self,
    ) -> Result<
        exports::spin::postgres4_2_0::postgres::Connection,
        exports::spin::postgres4_2_0::postgres::Error,
    > {
        Err(exports::spin::postgres4_2_0::postgres::Error::Other(
            format_deny_error("spin:postgres/postgres"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn build_async(
        &self,
    ) -> Result<
        exports::spin::postgres4_2_0::postgres::Connection,
        exports::spin::postgres4_2_0::postgres::Error,
    > {
        Err(exports::spin::postgres4_2_0::postgres::Error::Other(
            format_deny_error("spin:postgres/postgres"),
        ))
    }
}
impl exports::spin::postgres4_2_0::postgres::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        address: _rt::String,
    ) -> Result<
        exports::spin::postgres4_2_0::postgres::Connection,
        exports::spin::postgres4_2_0::postgres::Error,
    > {
        Err(exports::spin::postgres4_2_0::postgres::Error::Other(
            format_deny_error("spin:postgres/postgres"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn open_async(
        address: _rt::String,
    ) -> Result<
        exports::spin::postgres4_2_0::postgres::Connection,
        exports::spin::postgres4_2_0::postgres::Error,
    > {
        Err(exports::spin::postgres4_2_0::postgres::Error::Other(
            format_deny_error("spin:postgres/postgres"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn query(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::spin::postgres4_2_0::postgres::ParameterValue>,
    ) -> Result<
        exports::spin::postgres4_2_0::postgres::RowSet,
        exports::spin::postgres4_2_0::postgres::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn query_async(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::spin::postgres4_2_0::postgres::ParameterValue>,
    ) -> Result<
        (
            _rt::Vec<exports::spin::postgres4_2_0::postgres::Column>,
            wit_bindgen::rt::async_support::StreamReader<
                exports::spin::postgres4_2_0::postgres::Row,
            >,
            wit_bindgen::rt::async_support::FutureReader<
                Result<(), exports::spin::postgres4_2_0::postgres::Error>,
            >,
        ),
        exports::spin::postgres4_2_0::postgres::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::spin::postgres4_2_0::postgres::ParameterValue>,
    ) -> Result<u64, exports::spin::postgres4_2_0::postgres::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn execute_async(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::spin::postgres4_2_0::postgres::ParameterValue>,
    ) -> Result<u64, exports::spin::postgres4_2_0::postgres::Error> {
        unreachable!()
    }
}
impl exports::spin::postgres4_2_0::postgres::Guest for Adapter {
    type ConnectionBuilder = Adapter;
    type Connection = Adapter;
}
impl exports::spin::redis::redis::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn open(
        address: _rt::String,
    ) -> Result<exports::spin::redis::redis::Connection, exports::spin::redis::redis::Error> {
        Err(exports::spin::redis::redis::Error::Other(
            format_deny_error("spin:redis/redis"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn publish(
        &self,
        channel: _rt::String,
        payload: exports::spin::redis::redis::Payload,
    ) -> Result<(), exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn get(
        &self,
        key: _rt::String,
    ) -> Result<Option<exports::spin::redis::redis::Payload>, exports::spin::redis::redis::Error>
    {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn set(
        &self,
        key: _rt::String,
        value: exports::spin::redis::redis::Payload,
    ) -> Result<(), exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn incr(&self, key: _rt::String) -> Result<i64, exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn del(
        &self,
        keys: _rt::Vec<_rt::String>,
    ) -> Result<u32, exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn sadd(
        &self,
        key: _rt::String,
        values: _rt::Vec<_rt::String>,
    ) -> Result<u32, exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn smembers(
        &self,
        key: _rt::String,
    ) -> Result<_rt::Vec<_rt::String>, exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn srem(
        &self,
        key: _rt::String,
        values: _rt::Vec<_rt::String>,
    ) -> Result<u32, exports::spin::redis::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn execute(
        &self,
        command: _rt::String,
        arguments: _rt::Vec<exports::spin::redis::redis::RedisParameter>,
    ) -> Result<
        _rt::Vec<exports::spin::redis::redis::RedisResult>,
        exports::spin::redis::redis::Error,
    > {
        unreachable!()
    }
}
impl exports::spin::redis::redis::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::spin::sqlite3_1_0::sqlite::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        database: _rt::String,
    ) -> Result<
        exports::spin::sqlite3_1_0::sqlite::Connection,
        exports::spin::sqlite3_1_0::sqlite::Error,
    > {
        Err(exports::spin::sqlite3_1_0::sqlite::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn open_async(
        database: _rt::String,
    ) -> Result<
        exports::spin::sqlite3_1_0::sqlite::Connection,
        exports::spin::sqlite3_1_0::sqlite::Error,
    > {
        Err(exports::spin::sqlite3_1_0::sqlite::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        statement: _rt::String,
        parameters: _rt::Vec<exports::spin::sqlite3_1_0::sqlite::Value>,
    ) -> Result<
        exports::spin::sqlite3_1_0::sqlite::QueryResult,
        exports::spin::sqlite3_1_0::sqlite::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn execute_async(
        &self,
        statement: _rt::String,
        parameters: _rt::Vec<exports::spin::sqlite3_1_0::sqlite::Value>,
    ) -> Result<
        (
            _rt::Vec<_rt::String>,
            wit_bindgen::rt::async_support::StreamReader<
                exports::spin::sqlite3_1_0::sqlite::RowResult,
            >,
            wit_bindgen::rt::async_support::FutureReader<
                Result<(), exports::spin::sqlite3_1_0::sqlite::Error>,
            >,
        ),
        exports::spin::sqlite3_1_0::sqlite::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn last_insert_rowid(&self) -> i64 {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn last_insert_rowid_async(&self) -> i64 {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn changes(&self) -> u64 {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn changes_async(&self) -> u64 {
        unreachable!()
    }
}
impl exports::spin::sqlite3_1_0::sqlite::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::spin::variables::variables::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn get(
        name: _rt::String,
    ) -> Result<_rt::String, exports::spin::variables::variables::Error> {
        Err(exports::spin::variables::variables::Error::Undefined(name))
    }
}
impl exports::wasi::config::store::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(key: _rt::String) -> Result<Option<_rt::String>, exports::wasi::config::store::Error> {
        Err(exports::wasi::config::store::Error::Io(format_deny_error(
            "wasi:config/store",
        )))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_all() -> Result<_rt::Vec<(_rt::String, _rt::String)>, exports::wasi::config::store::Error>
    {
        Err(exports::wasi::config::store::Error::Io(format_deny_error(
            "wasi:config/store",
        )))
    }
}
impl exports::fermyon::spin::config::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_config(key: _rt::String) -> Result<_rt::String, exports::fermyon::spin::config::Error> {
        Err(exports::fermyon::spin::config::Error::Other(
            format_deny_error("fermyon:spin/config"),
        ))
    }
}
impl exports::fermyon::spin::http::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn send_request(
        req: exports::fermyon::spin::http::Request,
    ) -> Result<exports::fermyon::spin::http::Response, exports::fermyon::spin::http::HttpError>
    {
        Err(exports::fermyon::spin::http::HttpError::DestinationNotAllowed)
    }
}
impl exports::fermyon::spin::key_value::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        name: _rt::String,
    ) -> Result<exports::fermyon::spin::key_value::Store, exports::fermyon::spin::key_value::Error>
    {
        Err(exports::fermyon::spin::key_value::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(
        store: exports::fermyon::spin::key_value::Store,
        key: _rt::String,
    ) -> Result<_rt::Vec<u8>, exports::fermyon::spin::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set(
        store: exports::fermyon::spin::key_value::Store,
        key: _rt::String,
        value: _rt::Vec<u8>,
    ) -> Result<(), exports::fermyon::spin::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn delete(
        store: exports::fermyon::spin::key_value::Store,
        key: _rt::String,
    ) -> Result<(), exports::fermyon::spin::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn exists(
        store: exports::fermyon::spin::key_value::Store,
        key: _rt::String,
    ) -> Result<bool, exports::fermyon::spin::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_keys(
        store: exports::fermyon::spin::key_value::Store,
    ) -> Result<_rt::Vec<_rt::String>, exports::fermyon::spin::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn close(store: exports::fermyon::spin::key_value::Store) -> () {
        unreachable!()
    }
}
impl exports::fermyon::spin::llm::Guest for Adapter {
    #[allow(unused_variables)]
    /// Perform inferencing using the provided model and prompt with the given optional params
    #[allow(async_fn_in_trait)]
    fn infer(
        model: exports::fermyon::spin::llm::InferencingModel,
        prompt: _rt::String,
        params: Option<exports::fermyon::spin::llm::InferencingParams>,
    ) -> Result<exports::fermyon::spin::llm::InferencingResult, exports::fermyon::spin::llm::Error>
    {
        Err(exports::fermyon::spin::llm::Error::ModelNotSupported)
    }
    #[allow(unused_variables)]
    /// Generate embeddings for the supplied list of text
    #[allow(async_fn_in_trait)]
    fn generate_embeddings(
        model: exports::fermyon::spin::llm::EmbeddingModel,
        text: _rt::Vec<_rt::String>,
    ) -> Result<exports::fermyon::spin::llm::EmbeddingsResult, exports::fermyon::spin::llm::Error>
    {
        Err(exports::fermyon::spin::llm::Error::ModelNotSupported)
    }
}
impl exports::fermyon::spin::mysql::Guest for Adapter {
    #[allow(unused_variables)]
    /// query the database: select
    #[allow(async_fn_in_trait)]
    fn query(
        address: _rt::String,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin::mysql::ParameterValue>,
    ) -> Result<exports::fermyon::spin::mysql::RowSet, exports::fermyon::spin::mysql::MysqlError>
    {
        Err(exports::fermyon::spin::mysql::MysqlError::OtherError(
            format_deny_error("fermyon:spin/mysql"),
        ))
    }
    #[allow(unused_variables)]
    /// execute command to the database: insert, update, delete
    #[allow(async_fn_in_trait)]
    fn execute(
        address: _rt::String,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin::mysql::ParameterValue>,
    ) -> Result<(), exports::fermyon::spin::mysql::MysqlError> {
        Err(exports::fermyon::spin::mysql::MysqlError::OtherError(
            format_deny_error("fermyon:spin/mysql"),
        ))
    }
}
impl exports::fermyon::spin::postgres::Guest for Adapter {
    #[allow(unused_variables)]
    /// query the database: select
    #[allow(async_fn_in_trait)]
    fn query(
        address: _rt::String,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin::postgres::ParameterValue>,
    ) -> Result<exports::fermyon::spin::postgres::RowSet, exports::fermyon::spin::postgres::PgError>
    {
        Err(exports::fermyon::spin::postgres::PgError::OtherError(
            format_deny_error("fermyon:spin/postgres"),
        ))
    }
    #[allow(unused_variables)]
    /// execute command to the database: insert, update, delete
    #[allow(async_fn_in_trait)]
    fn execute(
        address: _rt::String,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin::postgres::ParameterValue>,
    ) -> Result<u64, exports::fermyon::spin::postgres::PgError> {
        Err(exports::fermyon::spin::postgres::PgError::OtherError(
            format_deny_error("fermyon:spin/postgres"),
        ))
    }
}
impl exports::fermyon::spin::redis::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn publish(
        address: _rt::String,
        channel: _rt::String,
        payload: exports::fermyon::spin::redis::Payload,
    ) -> Result<(), exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(
        address: _rt::String,
        key: _rt::String,
    ) -> Result<exports::fermyon::spin::redis::Payload, exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set(
        address: _rt::String,
        key: _rt::String,
        value: exports::fermyon::spin::redis::Payload,
    ) -> Result<(), exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn incr(
        address: _rt::String,
        key: _rt::String,
    ) -> Result<i64, exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn del(
        address: _rt::String,
        keys: _rt::Vec<_rt::String>,
    ) -> Result<i64, exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn sadd(
        address: _rt::String,
        key: _rt::String,
        values: _rt::Vec<_rt::String>,
    ) -> Result<i64, exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn smembers(
        address: _rt::String,
        key: _rt::String,
    ) -> Result<_rt::Vec<_rt::String>, exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn srem(
        address: _rt::String,
        key: _rt::String,
        values: _rt::Vec<_rt::String>,
    ) -> Result<i64, exports::fermyon::spin::redis::Error> {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        address: _rt::String,
        command: _rt::String,
        arguments: _rt::Vec<exports::fermyon::spin::redis::RedisParameter>,
    ) -> Result<
        _rt::Vec<exports::fermyon::spin::redis::RedisResult>,
        exports::fermyon::spin::redis::Error,
    > {
        Err(exports::fermyon::spin::redis::Error::Error)
    }
}
impl exports::fermyon::spin::sqlite::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        database: _rt::String,
    ) -> Result<exports::fermyon::spin::sqlite::Connection, exports::fermyon::spin::sqlite::Error>
    {
        Err(exports::fermyon::spin::sqlite::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        conn: exports::fermyon::spin::sqlite::Connection,
        statement: _rt::String,
        parameters: _rt::Vec<exports::fermyon::spin::sqlite::Value>,
    ) -> Result<exports::fermyon::spin::sqlite::QueryResult, exports::fermyon::spin::sqlite::Error>
    {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn close(conn: exports::fermyon::spin::sqlite::Connection) -> () {
        unreachable!()
    }
}
impl exports::wasi::cli0_2_6::environment::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_environment() -> _rt::Vec<(_rt::String, _rt::String)> {
        Vec::new()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_arguments() -> _rt::Vec<_rt::String> {
        Vec::new()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn initial_cwd() -> Option<_rt::String> {
        None
    }
}
impl exports::wasi::filesystem0_2_6::preopens::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_directories() -> _rt::Vec<(
        exports::wasi::filesystem0_2_6::preopens::Descriptor,
        _rt::String,
    )> {
        Vec::new()
    }
}
impl exports::wasi::sockets0_2_6::udp::GuestUdpSocket for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn start_bind(
        &self,
        network: &exports::wasi::sockets0_2_6::udp::Network,
        local_address: exports::wasi::sockets0_2_6::udp::IpSocketAddress,
    ) -> Result<(), exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn finish_bind(&self) -> Result<(), exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn stream(
        &self,
        remote_address: Option<exports::wasi::sockets0_2_6::udp::IpSocketAddress>,
    ) -> Result<
        (
            exports::wasi::sockets0_2_6::udp::IncomingDatagramStream,
            exports::wasi::sockets0_2_6::udp::OutgoingDatagramStream,
        ),
        exports::wasi::sockets0_2_6::udp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn local_address(
        &self,
    ) -> Result<
        exports::wasi::sockets0_2_6::udp::IpSocketAddress,
        exports::wasi::sockets0_2_6::udp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn remote_address(
        &self,
    ) -> Result<
        exports::wasi::sockets0_2_6::udp::IpSocketAddress,
        exports::wasi::sockets0_2_6::udp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn address_family(&self) -> exports::wasi::sockets0_2_6::udp::IpAddressFamily {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn unicast_hop_limit(&self) -> Result<u8, exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_unicast_hop_limit(
        &self,
        value: u8,
    ) -> Result<(), exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn receive_buffer_size(&self) -> Result<u64, exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_receive_buffer_size(
        &self,
        value: u64,
    ) -> Result<(), exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn send_buffer_size(&self) -> Result<u64, exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_send_buffer_size(
        &self,
        value: u64,
    ) -> Result<(), exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn subscribe(&self) -> exports::wasi::sockets0_2_6::udp::Pollable {
        unreachable!()
    }
}
impl exports::wasi::sockets0_2_6::udp::GuestIncomingDatagramStream for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn receive(
        &self,
        max_results: u64,
    ) -> Result<
        _rt::Vec<exports::wasi::sockets0_2_6::udp::IncomingDatagram>,
        exports::wasi::sockets0_2_6::udp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn subscribe(&self) -> exports::wasi::sockets0_2_6::udp::Pollable {
        unreachable!()
    }
}
impl exports::wasi::sockets0_2_6::udp::GuestOutgoingDatagramStream for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn check_send(&self) -> Result<u64, exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn send(
        &self,
        datagrams: _rt::Vec<exports::wasi::sockets0_2_6::udp::OutgoingDatagram>,
    ) -> Result<u64, exports::wasi::sockets0_2_6::udp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn subscribe(&self) -> exports::wasi::sockets0_2_6::udp::Pollable {
        unreachable!()
    }
}
impl exports::wasi::sockets0_2_6::udp::Guest for Adapter {
    type UdpSocket = Adapter;
    type IncomingDatagramStream = Adapter;
    type OutgoingDatagramStream = Adapter;
}
impl exports::wasi::sockets0_2_6::udp_create_socket::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn create_udp_socket(
        address_family: exports::wasi::sockets0_2_6::udp_create_socket::IpAddressFamily,
    ) -> Result<
        exports::wasi::sockets0_2_6::udp_create_socket::UdpSocket,
        exports::wasi::sockets0_2_6::udp_create_socket::ErrorCode,
    > {
        Err(exports::wasi::sockets0_2_6::udp_create_socket::ErrorCode::AccessDenied)
    }
}
impl exports::wasi::sockets0_2_6::tcp::GuestTcpSocket for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn start_bind(
        &self,
        network: &exports::wasi::sockets0_2_6::tcp::Network,
        local_address: exports::wasi::sockets0_2_6::tcp::IpSocketAddress,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn finish_bind(&self) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn start_connect(
        &self,
        network: &exports::wasi::sockets0_2_6::tcp::Network,
        remote_address: exports::wasi::sockets0_2_6::tcp::IpSocketAddress,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn finish_connect(
        &self,
    ) -> Result<
        (
            exports::wasi::sockets0_2_6::tcp::InputStream,
            exports::wasi::sockets0_2_6::tcp::OutputStream,
        ),
        exports::wasi::sockets0_2_6::tcp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn start_listen(&self) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn finish_listen(&self) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn accept(
        &self,
    ) -> Result<
        (
            exports::wasi::sockets0_2_6::tcp::TcpSocket,
            exports::wasi::sockets0_2_6::tcp::InputStream,
            exports::wasi::sockets0_2_6::tcp::OutputStream,
        ),
        exports::wasi::sockets0_2_6::tcp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn local_address(
        &self,
    ) -> Result<
        exports::wasi::sockets0_2_6::tcp::IpSocketAddress,
        exports::wasi::sockets0_2_6::tcp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn remote_address(
        &self,
    ) -> Result<
        exports::wasi::sockets0_2_6::tcp::IpSocketAddress,
        exports::wasi::sockets0_2_6::tcp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn is_listening(&self) -> bool {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn address_family(&self) -> exports::wasi::sockets0_2_6::tcp::IpAddressFamily {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_listen_backlog_size(
        &self,
        value: u64,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn keep_alive_enabled(&self) -> Result<bool, exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_keep_alive_enabled(
        &self,
        value: bool,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn keep_alive_idle_time(
        &self,
    ) -> Result<
        exports::wasi::sockets0_2_6::tcp::Duration,
        exports::wasi::sockets0_2_6::tcp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_keep_alive_idle_time(
        &self,
        value: exports::wasi::sockets0_2_6::tcp::Duration,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn keep_alive_interval(
        &self,
    ) -> Result<
        exports::wasi::sockets0_2_6::tcp::Duration,
        exports::wasi::sockets0_2_6::tcp::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_keep_alive_interval(
        &self,
        value: exports::wasi::sockets0_2_6::tcp::Duration,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn keep_alive_count(&self) -> Result<u32, exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_keep_alive_count(
        &self,
        value: u32,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn hop_limit(&self) -> Result<u8, exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_hop_limit(&self, value: u8) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn receive_buffer_size(&self) -> Result<u64, exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_receive_buffer_size(
        &self,
        value: u64,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn send_buffer_size(&self) -> Result<u64, exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set_send_buffer_size(
        &self,
        value: u64,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn subscribe(&self) -> exports::wasi::sockets0_2_6::tcp::Pollable {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn shutdown(
        &self,
        shutdown_type: exports::wasi::sockets0_2_6::tcp::ShutdownType,
    ) -> Result<(), exports::wasi::sockets0_2_6::tcp::ErrorCode> {
        unreachable!()
    }
}
impl exports::wasi::sockets0_2_6::tcp::Guest for Adapter {
    type TcpSocket = Adapter;
}
impl exports::wasi::sockets0_2_6::tcp_create_socket::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn create_tcp_socket(
        address_family: exports::wasi::sockets0_2_6::tcp_create_socket::IpAddressFamily,
    ) -> Result<
        exports::wasi::sockets0_2_6::tcp_create_socket::TcpSocket,
        exports::wasi::sockets0_2_6::tcp_create_socket::ErrorCode,
    > {
        Err(exports::wasi::sockets0_2_6::tcp_create_socket::ErrorCode::AccessDenied)
    }
}
impl exports::wasi::sockets0_2_6::ip_name_lookup::GuestResolveAddressStream for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn resolve_next_address(
        &self,
    ) -> Result<
        Option<exports::wasi::sockets0_2_6::ip_name_lookup::IpAddress>,
        exports::wasi::sockets0_2_6::ip_name_lookup::ErrorCode,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn subscribe(&self) -> exports::wasi::sockets0_2_6::ip_name_lookup::Pollable {
        unreachable!()
    }
}
impl exports::wasi::sockets0_2_6::ip_name_lookup::Guest for Adapter {
    type ResolveAddressStream = Adapter;
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn resolve_addresses(
        network: &exports::wasi::sockets0_2_6::ip_name_lookup::Network,
        name: _rt::String,
    ) -> Result<
        exports::wasi::sockets0_2_6::ip_name_lookup::ResolveAddressStream,
        exports::wasi::sockets0_2_6::ip_name_lookup::ErrorCode,
    > {
        Err(exports::wasi::sockets0_2_6::ip_name_lookup::ErrorCode::AccessDenied)
    }
}
impl exports::wasi::cli0_3_0_rc_2026_03_15::environment::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_environment() -> _rt::Vec<(_rt::String, _rt::String)> {
        Vec::new()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_arguments() -> _rt::Vec<_rt::String> {
        Vec::new()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_initial_cwd() -> Option<_rt::String> {
        None
    }
}
impl exports::wasi::filesystem0_3_0_rc_2026_03_15::preopens::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_directories() -> _rt::Vec<(
        exports::wasi::filesystem0_3_0_rc_2026_03_15::preopens::Descriptor,
        _rt::String,
    )> {
        Vec::new()
    }
}
impl exports::wasi::sockets0_3_0_rc_2026_03_15::ip_name_lookup::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    async fn resolve_addresses(
        name: _rt::String,
    ) -> Result<
        _rt::Vec<exports::wasi::sockets0_3_0_rc_2026_03_15::ip_name_lookup::IpAddress>,
        exports::wasi::sockets0_3_0_rc_2026_03_15::ip_name_lookup::ErrorCode,
    > {
        Err(exports::wasi::sockets0_3_0_rc_2026_03_15::ip_name_lookup::ErrorCode::AccessDenied)
    }
}
impl exports::fermyon::spin2_0_0::llm::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn infer(
        model: exports::fermyon::spin2_0_0::llm::InferencingModel,
        prompt: _rt::String,
        params: Option<exports::fermyon::spin2_0_0::llm::InferencingParams>,
    ) -> Result<
        exports::fermyon::spin2_0_0::llm::InferencingResult,
        exports::fermyon::spin2_0_0::llm::Error,
    > {
        Err(exports::fermyon::spin2_0_0::llm::Error::ModelNotSupported)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn generate_embeddings(
        model: exports::fermyon::spin2_0_0::llm::EmbeddingModel,
        text: _rt::Vec<_rt::String>,
    ) -> Result<
        exports::fermyon::spin2_0_0::llm::EmbeddingsResult,
        exports::fermyon::spin2_0_0::llm::Error,
    > {
        Err(exports::fermyon::spin2_0_0::llm::Error::ModelNotSupported)
    }
}
impl exports::fermyon::spin2_0_0::redis::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        address: _rt::String,
    ) -> Result<
        exports::fermyon::spin2_0_0::redis::Connection,
        exports::fermyon::spin2_0_0::redis::Error,
    > {
        Err(exports::fermyon::spin2_0_0::redis::Error::Other(
            format_deny_error("fermyon:spin/redis"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn publish(
        &self,
        channel: _rt::String,
        payload: exports::fermyon::spin2_0_0::redis::Payload,
    ) -> Result<(), exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(
        &self,
        key: _rt::String,
    ) -> Result<
        Option<exports::fermyon::spin2_0_0::redis::Payload>,
        exports::fermyon::spin2_0_0::redis::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set(
        &self,
        key: _rt::String,
        value: exports::fermyon::spin2_0_0::redis::Payload,
    ) -> Result<(), exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn incr(&self, key: _rt::String) -> Result<i64, exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn del(
        &self,
        keys: _rt::Vec<_rt::String>,
    ) -> Result<u32, exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn sadd(
        &self,
        key: _rt::String,
        values: _rt::Vec<_rt::String>,
    ) -> Result<u32, exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn smembers(
        &self,
        key: _rt::String,
    ) -> Result<_rt::Vec<_rt::String>, exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn srem(
        &self,
        key: _rt::String,
        values: _rt::Vec<_rt::String>,
    ) -> Result<u32, exports::fermyon::spin2_0_0::redis::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        command: _rt::String,
        arguments: _rt::Vec<exports::fermyon::spin2_0_0::redis::RedisParameter>,
    ) -> Result<
        _rt::Vec<exports::fermyon::spin2_0_0::redis::RedisResult>,
        exports::fermyon::spin2_0_0::redis::Error,
    > {
        unreachable!()
    }
}
impl exports::fermyon::spin2_0_0::redis::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::fermyon::spin2_0_0::mqtt::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        address: _rt::String,
        username: _rt::String,
        password: _rt::String,
        keep_alive_interval_in_secs: u64,
    ) -> Result<
        exports::fermyon::spin2_0_0::mqtt::Connection,
        exports::fermyon::spin2_0_0::mqtt::Error,
    > {
        Err(exports::fermyon::spin2_0_0::mqtt::Error::Other(
            format_deny_error("fermyon:spin/mqtt"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn publish(
        &self,
        topic: _rt::String,
        payload: exports::fermyon::spin2_0_0::mqtt::Payload,
        qos: exports::fermyon::spin2_0_0::mqtt::Qos,
    ) -> Result<(), exports::fermyon::spin2_0_0::mqtt::Error> {
        unreachable!()
    }
}
impl exports::fermyon::spin2_0_0::mqtt::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::fermyon::spin2_0_0::postgres::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        address: _rt::String,
    ) -> Result<
        exports::fermyon::spin2_0_0::postgres::Connection,
        exports::fermyon::spin2_0_0::postgres::Error,
    > {
        Err(exports::fermyon::spin2_0_0::postgres::Error::Other(
            format_deny_error("fermyon:spin/postgres"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn query(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin2_0_0::postgres::ParameterValue>,
    ) -> Result<
        exports::fermyon::spin2_0_0::postgres::RowSet,
        exports::fermyon::spin2_0_0::postgres::Error,
    > {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin2_0_0::postgres::ParameterValue>,
    ) -> Result<u64, exports::fermyon::spin2_0_0::postgres::Error> {
        unreachable!()
    }
}
impl exports::fermyon::spin2_0_0::postgres::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::fermyon::spin2_0_0::mysql::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        address: _rt::String,
    ) -> Result<
        exports::fermyon::spin2_0_0::mysql::Connection,
        exports::fermyon::spin2_0_0::mysql::Error,
    > {
        Err(exports::fermyon::spin2_0_0::mysql::Error::Other(
            format_deny_error("fermyon:spin/mysql"),
        ))
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn query(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin2_0_0::mysql::ParameterValue>,
    ) -> Result<exports::fermyon::spin2_0_0::mysql::RowSet, exports::fermyon::spin2_0_0::mysql::Error>
    {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        statement: _rt::String,
        params: _rt::Vec<exports::fermyon::spin2_0_0::mysql::ParameterValue>,
    ) -> Result<(), exports::fermyon::spin2_0_0::mysql::Error> {
        unreachable!()
    }
}
impl exports::fermyon::spin2_0_0::mysql::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::fermyon::spin2_0_0::sqlite::GuestConnection for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        database: _rt::String,
    ) -> Result<
        exports::fermyon::spin2_0_0::sqlite::Connection,
        exports::fermyon::spin2_0_0::sqlite::Error,
    > {
        Err(exports::fermyon::spin2_0_0::sqlite::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn execute(
        &self,
        statement: _rt::String,
        parameters: _rt::Vec<exports::fermyon::spin2_0_0::sqlite::Value>,
    ) -> Result<
        exports::fermyon::spin2_0_0::sqlite::QueryResult,
        exports::fermyon::spin2_0_0::sqlite::Error,
    > {
        unreachable!()
    }
}
impl exports::fermyon::spin2_0_0::sqlite::Guest for Adapter {
    type Connection = Adapter;
}
impl exports::fermyon::spin2_0_0::key_value::GuestStore for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        label: _rt::String,
    ) -> Result<
        exports::fermyon::spin2_0_0::key_value::Store,
        exports::fermyon::spin2_0_0::key_value::Error,
    > {
        Err(exports::fermyon::spin2_0_0::key_value::Error::AccessDenied)
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(
        &self,
        key: _rt::String,
    ) -> Result<Option<_rt::Vec<u8>>, exports::fermyon::spin2_0_0::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set(
        &self,
        key: _rt::String,
        value: _rt::Vec<u8>,
    ) -> Result<(), exports::fermyon::spin2_0_0::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn delete(
        &self,
        key: _rt::String,
    ) -> Result<(), exports::fermyon::spin2_0_0::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn exists(
        &self,
        key: _rt::String,
    ) -> Result<bool, exports::fermyon::spin2_0_0::key_value::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get_keys(
        &self,
    ) -> Result<_rt::Vec<_rt::String>, exports::fermyon::spin2_0_0::key_value::Error> {
        unreachable!()
    }
}
impl exports::fermyon::spin2_0_0::key_value::Guest for Adapter {
    type Store = Adapter;
}
impl exports::fermyon::spin2_0_0::variables::Guest for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(
        name: _rt::String,
    ) -> Result<_rt::String, exports::fermyon::spin2_0_0::variables::Error> {
        Err(exports::fermyon::spin2_0_0::variables::Error::Undefined(
            name,
        ))
    }
}
impl exports::wasi::keyvalue::store::GuestBucket for Adapter {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn get(
        &self,
        key: _rt::String,
    ) -> Result<Option<_rt::Vec<u8>>, exports::wasi::keyvalue::store::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn set(
        &self,
        key: _rt::String,
        value: _rt::Vec<u8>,
    ) -> Result<(), exports::wasi::keyvalue::store::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn delete(&self, key: _rt::String) -> Result<(), exports::wasi::keyvalue::store::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn exists(&self, key: _rt::String) -> Result<bool, exports::wasi::keyvalue::store::Error> {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn list_keys(
        &self,
        cursor: Option<_rt::String>,
    ) -> Result<exports::wasi::keyvalue::store::KeyResponse, exports::wasi::keyvalue::store::Error>
    {
        unreachable!()
    }
}
impl exports::wasi::keyvalue::store::Guest for Adapter {
    type Bucket = Adapter;
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn open(
        identifier: _rt::String,
    ) -> Result<exports::wasi::keyvalue::store::Bucket, exports::wasi::keyvalue::store::Error> {
        Err(exports::wasi::keyvalue::store::Error::AccessDenied)
    }
}
