//! Regression test: constructing an `mqtts://` client must not depend on a
//! rustls `CryptoProvider` having been installed by a Spin entrypoint.
//!
//! This lives in its own test binary so that it runs in a fresh process where
//! no provider has been installed yet, like an embedder that assembles the
//! factors directly. Both rustls provider features (`ring` and `aws_lc_rs`)
//! are compiled into this crate's dependency graph, so without an explicit
//! install rustls cannot pick one and panics.
//!
//! Like `spin up` with an `mqtts://` address, this relies on rumqttc loading
//! the platform's root certificates, so it needs a system trust store.

use std::time::Duration;

use spin_factor_outbound_mqtt::NetworkedMqttClient;

#[test]
fn creating_mqtts_client_in_fresh_process_does_not_panic() {
    NetworkedMqttClient::create(
        "mqtts://localhost:8883?client_id=test".to_string(),
        "user".to_string(),
        "pass".to_string(),
        Duration::from_secs(30),
    )
    .expect("mqtts client construction should succeed");
}
