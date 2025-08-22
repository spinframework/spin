use helper::{ensure_eq, ensure_ok};

use helper::http_trigger_bindings::wasi::blobstore::blobstore::get_container;
use helper::http_trigger_bindings::wasi::blobstore::types::{IncomingValue, OutgoingValue};

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        let container = ensure_ok!(get_container(&"default".into()));

        let blob_name = "my-blob".to_string();

        let outgoing = OutgoingValue::new_outgoing_value();
        ensure_ok!(container.write_data(&blob_name, &outgoing));

        let stm = ensure_ok!(outgoing.outgoing_value_write_body().map_err(|_| "failed to write outgoing body"));

        ensure_ok!(stm.blocking_write_and_flush(b"Hello world"));

        ensure_ok!(OutgoingValue::finish(outgoing));

        let incoming = ensure_ok!(container.get_data(&blob_name, 0, u64::MAX));
        let content = ensure_ok!(IncomingValue::incoming_value_consume_sync(incoming));
        ensure_eq!(content, b"Hello world");

        let incoming = ensure_ok!(container.get_data(&blob_name, 0, 4));
        let content = ensure_ok!(IncomingValue::incoming_value_consume_sync(incoming));
        ensure_eq!(content, b"Hello");

        let incoming = ensure_ok!(container.get_data(&blob_name, 2, 4));
        let content = ensure_ok!(IncomingValue::incoming_value_consume_sync(incoming));
        ensure_eq!(content, b"llo");

        Ok(())
    }
}
