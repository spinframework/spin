use helper::ensure_ok;
use helper::http_trigger_bindings::fermyon::spin2_0_0::mqtt::{self, Qos};
use std::env;

const MQTT_ADDRESS_ENV: &str = "MQTT_ADDRESS";
const MQTT_USERNAME_ENV: &str = "MQTT_USERNAME";
const MQTT_PASSWORD_ENV: &str = "MQTT_PASSWORD";
const MQTT_KEEP_ALIVE_INTERVAL_ENV: &str = "MQTT_KEEP_ALIVE_INTERVAL";

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        let address = ensure_ok!(env::var(MQTT_ADDRESS_ENV));
        let username = ensure_ok!(env::var(MQTT_USERNAME_ENV));
        let password = ensure_ok!(env::var(MQTT_PASSWORD_ENV));
        let keep_alive_interval =
            ensure_ok!(ensure_ok!(env::var(MQTT_KEEP_ALIVE_INTERVAL_ENV)).parse::<u64>());

        let connection = ensure_ok!(mqtt::Connection::open(
            &address,
            &username,
            &password,
            keep_alive_interval
        ));

        ensure_ok!(connection.publish("telemetry-topic", b"Eureka!", Qos::AtLeastOnce));

        Ok(())
    }
}
