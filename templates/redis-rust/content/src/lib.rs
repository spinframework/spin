use anyhow::Result;
use spin_sdk::redis_subscriber;
use std::str::from_utf8;

/// A simple Spin Redis component.
#[redis_subscriber]
async fn on_message(message: Vec<u8>) -> Result<()> {
    println!("{}", from_utf8(&message)?);
    // Implement me ...
    Ok(())
}
