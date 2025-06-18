use anyhow::Context as _;
use spin_factors::{App, AppComponent};
use spin_locked_app::MetadataKey;
use spin_outbound_networking_config::allowed_hosts::parse_service_chaining_target;

const ALLOWED_HOSTS_KEY: MetadataKey<Vec<String>> = MetadataKey::new("allowed_outbound_hosts");
const ALLOWED_HTTP_KEY: MetadataKey<Vec<String>> = MetadataKey::new("allowed_http_hosts");

/// Get the raw values of the `allowed_outbound_hosts` locked app metadata key.
///
/// This has support for converting the old `allowed_http_hosts` key to the new `allowed_outbound_hosts` key.
pub fn allowed_outbound_hosts(component: &AppComponent) -> anyhow::Result<Vec<String>> {
    let mut allowed_hosts = component
        .get_metadata(ALLOWED_HOSTS_KEY)
        .with_context(|| {
            format!(
                "locked app metadata was malformed for key {}",
                ALLOWED_HOSTS_KEY.as_ref()
            )
        })?
        .unwrap_or_default();
    let allowed_http = component
        .get_metadata(ALLOWED_HTTP_KEY)
        .map(|h| h.unwrap_or_default())
        .unwrap_or_default();
    let converted =
        spin_manifest::compat::convert_allowed_http_to_allowed_hosts(&allowed_http, false)
            .unwrap_or_default();
    allowed_hosts.extend(converted);
    Ok(allowed_hosts)
}

/// Validates that all service chaining of an app will be satisfied by the
/// supplied subset of components.
///
/// This does a best effort look up of components that are
/// allowed to be accessed through service chaining and will error early if a
/// component is configured to to chain to another component that is not
/// retained. All wildcard service chaining is disallowed and all templated URLs
/// are ignored.
pub fn validate_service_chaining_for_components(
    app: &App,
    retained_components: &[&str],
) -> anyhow::Result<()> {
    app
        .triggers().try_for_each(|t| {
            let Ok(component) = t.component() else  { return Ok(()) };
            if retained_components.contains(&component.id()) {
            let allowed_hosts = allowed_outbound_hosts(&component).context("failed to get allowed hosts")?;
            for host in allowed_hosts {
                // Templated URLs are not yet resolved at this point, so ignore unresolvable URIs
                if let Ok(uri) = host.parse::<http::Uri>() {
                    if let Some(chaining_target) = parse_service_chaining_target(&uri) {
                        if !retained_components.contains(&chaining_target.as_ref()) {
                            if chaining_target == "*" {
                                return  Err(anyhow::anyhow!("Selected component '{}' cannot use wildcard service chaining: allowed_outbound_hosts = [\"http://*.spin.internal\"]", component.id()));
                            }
                            return  Err(anyhow::anyhow!(
                                "Selected component '{}' cannot use service chaining to unselected component: allowed_outbound_hosts = [\"http://{}.spin.internal\"]",
                                component.id(), chaining_target
                            ));
                        }
                    }
                }
            }
        }
        anyhow::Ok(())
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validate_service_chaining_for_components_fails() {
        let manifest = toml::toml! {
            spin_manifest_version = 2

            [application]
            name = "test-app"

            [[trigger.test-trigger]]
            component = "empty"

            [component.empty]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["http://another.spin.internal"]

            [[trigger.another-trigger]]
            component = "another"

            [component.another]
            source = "does-not-exist.wasm"

            [[trigger.third-trigger]]
            component = "third"

            [component.third]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["http://*.spin.internal"]
        };
        let locked_app = spin_factors_test::build_locked_app(&manifest)
            .await
            .expect("could not build locked app");
        let app = App::new("unused", locked_app);
        let Err(e) = validate_service_chaining_for_components(&app, &["empty"]) else {
            panic!("Expected service chaining to non-retained component error");
        };
        assert_eq!(
            e.to_string(),
            "Selected component 'empty' cannot use service chaining to unselected component: allowed_outbound_hosts = [\"http://another.spin.internal\"]"
        );
        let Err(e) = validate_service_chaining_for_components(&app, &["third", "another"]) else {
            panic!("Expected wildcard service chaining error");
        };
        assert_eq!(
            e.to_string(),
            "Selected component 'third' cannot use wildcard service chaining: allowed_outbound_hosts = [\"http://*.spin.internal\"]"
        );
        assert!(validate_service_chaining_for_components(&app, &["another"]).is_ok());
    }

    #[tokio::test]
    async fn validate_service_chaining_for_components_with_templated_host_passes() {
        let manifest = toml::toml! {
            spin_manifest_version = 2

            [application]
            name = "test-app"

            [variables]
            host = { default = "test" }

            [[trigger.test-trigger]]
            component = "empty"

            [component.empty]
            source = "does-not-exist.wasm"

            [[trigger.another-trigger]]
            component = "another"

            [component.another]
            source = "does-not-exist.wasm"

            [[trigger.third-trigger]]
            component = "third"

            [component.third]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["http://{{ host }}.spin.internal"]
        };
        let locked_app = spin_factors_test::build_locked_app(&manifest)
            .await
            .expect("could not build locked app");
        let app = App::new("unused", locked_app);
        assert!(validate_service_chaining_for_components(&app, &["empty", "third"]).is_ok());
    }
}
