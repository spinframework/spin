use std::str::from_utf8;

const KV_KEY: &str = "message";

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "spin:up/redis-trigger@4.0.0",
    generate_all,
});

struct Guest;

impl exports::spin::redis::inbound_redis::Guest for Guest {
    async fn handle_message(message: Vec<u8>) -> Result<(), spin::redis::redis::Error> {
        // Do some async stuff to prove it works
        let kv = spin::key_value::key_value::Store::open("default".to_string()).await.expect("should have had access to default KV store");
        kv.set(KV_KEY.to_string(), message.clone()).await.expect("should have set KV entry");
        let message = kv.get(KV_KEY.to_string()).await.expect("should have read KV entry").expect("KV entry should have existed");

        println!("Got message: '{}'", from_utf8(&message).unwrap_or("<MESSAGE NOT UTF8>"));
        Ok(())
    }
}

export!(Guest);
