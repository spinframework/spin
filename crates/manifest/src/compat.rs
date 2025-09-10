//! Compatibility for old manifest versions.

mod allowed_http_hosts;

use crate::{
    error::Error,
    schema::{v1, v2},
};
use allowed_http_hosts::{parse_allowed_http_hosts, AllowedHttpHosts};

/// Converts a V1 app manifest to V2.
pub fn v1_to_v2_app(manifest: v1::AppManifestV1) -> Result<v2::AppManifest, Error> {
    let trigger_type = manifest.trigger.trigger_type.clone();
    let trigger_global_configs = [(trigger_type.clone(), manifest.trigger.config)]
        .into_iter()
        .collect();

    let application = v2::AppDetails {
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        authors: manifest.authors,
        targets: Default::default(),
        trigger_global_configs,
        tool: Default::default(),
    };

    let app_variables = manifest
        .variables
        .into_iter()
        .map(|(key, var)| Ok((id_from_string(key)?, var)))
        .collect::<Result<_, Error>>()?;

    let mut triggers = v2::Map::<String, Vec<v2::Trigger>>::default();
    let mut components = v2::Map::default();
    for component in manifest.components {
        let component_id = component_id_from_string(component.id)?;

        let variables = component
            .config
            .into_iter()
            .map(|(key, var)| Ok((id_from_string(key)?, var)))
            .collect::<Result<_, Error>>()?;

        let ai_models = component
            .ai_models
            .into_iter()
            .map(id_from_string)
            .collect::<Result<_, Error>>()?;
        let allowed_http = convert_allowed_http_to_allowed_hosts(
            &component.allowed_http_hosts,
            component.allowed_outbound_hosts.is_none(),
        )
        .map_err(Error::ValidationError)?;
        let allowed_outbound_hosts = match component.allowed_outbound_hosts {
            Some(mut hs) => {
                hs.extend(allowed_http);
                hs
            }
            None => allowed_http,
        };
        components.insert(
            component_id.clone(),
            #[allow(deprecated)]
            v2::Component {
                source: component.source,
                description: component.description,
                variables,
                environment: component.environment,
                files: component.files,
                exclude_files: component.exclude_files,
                key_value_stores: component.key_value_stores,
                sqlite_databases: component.sqlite_databases,
                ai_models,
                build: component.build,
                tool: Default::default(),
                allowed_outbound_hosts,
                allowed_http_hosts: Vec::new(),
                dependencies_inherit_configuration: false,
                dependencies: Default::default(),
            },
        );
        triggers
            .entry(trigger_type.clone())
            .or_default()
            .push(v2::Trigger {
                id: format!("trigger-{component_id}"),
                component: Some(v2::ComponentSpec::Reference(component_id)),
                components: Default::default(),
                config: component.trigger,
            });
    }
    Ok(v2::AppManifest {
        spin_manifest_version: Default::default(),
        application,
        variables: app_variables,
        triggers,
        components,
    })
}

/// Converts the old `allowed_http_hosts` field to the new `allowed_outbound_hosts` field.
///
/// If `allow_database_access` is `true`, the function will also allow access to all redis,
/// mysql, and postgres databases as this was the default before `allowed_outbound_hosts` was introduced.
pub fn convert_allowed_http_to_allowed_hosts(
    allowed_http_hosts: &[impl AsRef<str>],
    allow_database_access: bool,
) -> anyhow::Result<Vec<String>> {
    let http_hosts = parse_allowed_http_hosts(allowed_http_hosts)?;
    let mut outbound_hosts = if allow_database_access {
        vec![
            "redis://*:*".into(),
            "mysql://*:*".into(),
            "postgres://*:*".into(),
        ]
    } else {
        Vec::new()
    };
    match http_hosts {
        AllowedHttpHosts::AllowAll => outbound_hosts.extend([
            "http://*:*".into(),
            "https://*:*".into(),
            "http://self".into(),
        ]),
        AllowedHttpHosts::AllowSpecific(specific) => {
            outbound_hosts.extend(specific.into_iter().flat_map(|s| {
                if s.domain == "self" {
                    vec!["http://self".into()]
                } else {
                    let port = s.port.map(|p| format!(":{p}")).unwrap_or_default();
                    vec![
                        format!("http://{}{}", s.domain, port),
                        format!("https://{}{}", s.domain, port),
                    ]
                }
            }))
        }
    };
    Ok(outbound_hosts)
}

fn component_id_from_string(id: String) -> Result<v2::KebabId, Error> {
    // If it's already valid, do nothing
    if let Ok(id) = id.clone().try_into() {
        return Ok(id);
    }
    // Fix two likely problems; under_scores and mixedCase
    let id = id.replace('_', "-").to_lowercase();
    id.clone()
        .try_into()
        .map_err(|err: String| Error::InvalidID { id, reason: err })
}

fn id_from_string<const DELIM: char, const LOWER: bool>(
    id: String,
) -> Result<spin_serde::id::Id<DELIM, LOWER>, Error> {
    id.clone()
        .try_into()
        .map_err(|err: String| Error::InvalidID { id, reason: err })
}
