use helper::{bail, ensure_matches, ensure_ok};

use helper::http_trigger_bindings::wasi::keyvalue::atomics as wasi_atomics;
use helper::http_trigger_bindings::wasi::keyvalue::batch as wasi_batch;
use helper::http_trigger_bindings::wasi::keyvalue::store::{Error, KeyResponse, open};

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        ensure_matches!(open("forbidden"), Err(Error::AccessDenied));

        let store = ensure_ok!(open("default"));

        // Ensure nothing set in `bar` key
        ensure_ok!(store.delete("bar"));
        ensure_matches!(store.exists("bar"), Ok(false));
        ensure_matches!(store.get("bar"), Ok(None));
        ensure_matches!(keys(&store.list_keys(None)), Ok(&[]));

        // Set `bar` key
        ensure_ok!(store.set("bar", b"baz"));
        ensure_matches!(store.exists("bar"), Ok(true));
        ensure_matches!(store.get("bar"), Ok(Some(v)) if v == b"baz");
        ensure_matches!(keys(&store.list_keys(None)), Ok([bar]) if bar == "bar");
        ensure_matches!(keys(&store.list_keys(Some("0"))), Err(Error::Other(_))); // "list_keys: cursor not supported"

        // Override `bar` key
        ensure_ok!(store.set("bar", b"wow"));
        ensure_matches!(store.exists("bar"), Ok(true));
        ensure_matches!(store.get("bar"), Ok(Some(wow)) if wow == b"wow");
        ensure_matches!(keys(&store.list_keys(None)), Ok([bar]) if bar == "bar");

        // Set another key
        ensure_ok!(store.set("qux", b"yay"));
        ensure_matches!(keys(&store.list_keys(None)), Ok(c) if c.len() == 2 && c.contains(&"bar".into()) && c.contains(&"qux".into()));

        // Delete everything
        ensure_ok!(store.delete("bar"));
        ensure_ok!(store.delete("bar"));
        ensure_ok!(store.delete("qux"));
        ensure_matches!(store.exists("bar"), Ok(false));
        ensure_matches!(store.get("qux"), Ok(None));
        ensure_matches!(keys(&store.list_keys(None)), Ok(&[]));

        ensure_ok!(wasi_batch::set_many(
            &store,
            &[
                ("bar".to_string(), b"bin".to_vec()),
                ("baz".to_string(), b"buzz".to_vec())
            ]
        ));
        ensure_ok!(wasi_batch::get_many(
            &store,
            &["bar".to_string(), "baz".to_string()]
        ));
        ensure_ok!(wasi_batch::delete_many(
            &store,
            &["bar".to_string(), "baz".to_string()]
        ));
        ensure_matches!(wasi_atomics::increment(&store, "counter", 10), Ok(v) if v == 10);
        ensure_matches!(wasi_atomics::increment(&store, "counter", 5), Ok(v) if v == 15);

        // successful compare and swap
        ensure_ok!(store.set("bar", b"wow"));
        let cas = ensure_ok!(wasi_atomics::Cas::new(&store, "bar"));
        ensure_matches!(cas.current(), Ok(Some(v)) if v == b"wow".to_vec());
        ensure_ok!(wasi_atomics::swap(cas, b"swapped"));
        ensure_matches!(store.get("bar"), Ok(Some(v)) if v == b"swapped");
        ensure_ok!(store.delete("bar"));

        // Insert 256 copies of a 1MB string, which exceeds the 128MB query
        // result limit we impose in `factor-key-value`:
        let big_text = "y".repeat(1 << 20);
        for i in 0..256 {
            ensure_ok!(store.set(&i.to_string(), big_text.as_bytes()));
        }

        // This should exceed the 128MB query result limit:
        match wasi_batch::get_many(&store, &(0..256).map(|i| i.to_string()).collect::<Vec<_>>()) {
            Ok(_) => bail!("large get_many should not have succeeded",),
            Err(Error::Other(s)) if s.contains("query result exceeds limit") => {}
            Err(e) => bail!("unexpected error: {e}",),
        }

        Ok(())
    }
}

fn keys<E>(res: &Result<KeyResponse, E>) -> Result<&[String], &E> {
    res.as_ref().map(|kr| kr.keys.as_slice())
}
