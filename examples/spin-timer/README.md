# Extending Spin with a trigger and application

## Manually
To test:

* `cargo build --release`
* Copy: `cp ./target/release/trigger-timer .`
* `tar czvf trigger-timer.tar.gz trigger-timer`
* Update the plugin manifest (`trigger-timer.json`):
  * Get the SHA: `shasum -a 256 trigger-timer.tar.gz` and copy it into the `sha256` field
  * Update `os` and `arch` with values for your OS/Arch
  * Update the URL too, to reflect the directory where the tar file is
* `spin plugin install --file ./trigger-timer.json --yes`

Then you should be able to `spin build --up` the [guest](./app-example/).

## Pluginify

To test:

* create a `spin-pluginify.toml` file as follows:
```toml
name= "trigger-timer"
description= "Run Spin components at timed intervals"
homepage= "https://github.com/spinframework/spin/tree/main/examples/spin-timer"
version= "0.1.0"
spin_compatibility= ">=2.0"
license= "Apache-2.0"
package= "./target/release/trigger-timer"
```
* `cargo build --release`
* If pluginify plugin is not already installed run `spin plugin install pluginify`.
* `spin pluginify --install`

Then you should be able to `spin build --up` the [guest](./app-example/).
