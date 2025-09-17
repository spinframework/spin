use helper::{ensure_eq, ensure_matches, ensure_ok, ensure_some};

const REDIS_ADDRESS_ENV: &str = "REDIS_ADDRESS";

use helper::http_trigger_bindings::fermyon::spin2_0_0::redis;

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        let address = ensure_ok!(std::env::var(REDIS_ADDRESS_ENV));
        let connection = ensure_ok!(redis::Connection::open(&address));

        ensure_ok!(connection.set("spin-example-get-set", b"Eureka!"));

        let payload = ensure_some!(ensure_ok!(connection.get("spin-example-get-set")));

        ensure_eq!(String::from_utf8_lossy(&payload), "Eureka!");

        ensure_ok!(connection.set("spin-example-incr", b"0"));

        let int_value = ensure_ok!(connection.incr("spin-example-incr"));

        ensure_eq!(int_value, 1);

        let keys = vec!["spin-example-get-set".into(), "spin-example-incr".into()];

        let del_keys = ensure_ok!(connection.del(&keys));

        ensure_eq!(del_keys, 2);

        ensure_ok!(connection.execute(
            "set",
            &[
                redis::RedisParameter::Binary(b"spin-example".to_vec()),
                redis::RedisParameter::Binary(b"Eureka!".to_vec()),
            ],
        ));

        ensure_ok!(connection.execute(
            "append",
            &[
                redis::RedisParameter::Binary(b"spin-example".to_vec()),
                redis::RedisParameter::Binary(b" I've got it!".to_vec()),
            ],
        ));

        let values = ensure_ok!(connection.execute(
            "get",
            &[redis::RedisParameter::Binary(b"spin-example".to_vec())]
        ));

        ensure_matches!(
            values.as_slice(),
            &[redis::RedisResult::Binary(ref b)] if b == b"Eureka! I've got it!"
        );

        ensure_ok!(connection.execute(
            "set",
            &[
                redis::RedisParameter::Binary(b"int-key".to_vec()),
                redis::RedisParameter::Int64(0),
            ],
        ));

        let values = ensure_ok!(connection.execute(
            "incr",
            &[redis::RedisParameter::Binary(b"int-key".to_vec())]
        ));

        ensure_matches!(values.as_slice(), &[redis::RedisResult::Int64(1)]);

        let values = ensure_ok!(
            connection.execute("get", &[redis::RedisParameter::Binary(b"int-key".to_vec())])
        );

        ensure_matches!(
            values.as_slice(),
            &[redis::RedisResult::Binary(ref b)] if b == b"1"
        );

        ensure_ok!(connection.execute("del", &[redis::RedisParameter::Binary(b"foo".to_vec())]));

        ensure_ok!(connection.execute(
            "sadd",
            &[
                redis::RedisParameter::Binary(b"foo".to_vec()),
                redis::RedisParameter::Binary(b"bar".to_vec()),
                redis::RedisParameter::Binary(b"baz".to_vec()),
            ],
        ));

        let values = ensure_ok!(connection.execute(
            "smembers",
            &[redis::RedisParameter::Binary(b"foo".to_vec())],
        ));
        let mut values: Vec<_> = ensure_ok!(values
            .iter()
            .map(|v| match v {
                redis::RedisResult::Binary(v) => Ok(v.as_slice()),
                v => Err(format!("unexpected value: {v:?}")),
            })
            .collect());
        // Ensure the values are always in a deterministic order
        values.sort();

        ensure_matches!(values.as_slice(), &[b"bar", b"baz",]);

        ensure_ok!(connection.execute(
            "srem",
            &[
                redis::RedisParameter::Binary(b"foo".to_vec()),
                redis::RedisParameter::Binary(b"baz".to_vec()),
            ],
        ));

        let values = ensure_ok!(connection.execute(
            "smembers",
            &[redis::RedisParameter::Binary(b"foo".to_vec())]
        ));

        ensure_matches!(
            values.as_slice(),
            &[redis::RedisResult::Binary(ref bar)] if bar == b"bar"
        );

        Ok(())
    }
}
