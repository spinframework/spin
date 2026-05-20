use std::path::PathBuf;

use spin_manifest::schema::v2::TargetEnvironmentRef;

use crate::opt_value::OptionalValueFlag;

pub(crate) fn parse_env(env_id: &str) -> TargetEnvironmentRef {
    if env_id.starts_with("https:") || env_id.starts_with("http:") {
        TargetEnvironmentRef::Http {
            url: env_id.to_string(),
        }
    } else if let Some(path) = env_id.strip_prefix("file:") {
        TargetEnvironmentRef::File {
            path: PathBuf::from(path),
        }
    } else {
        TargetEnvironmentRef::Catalogue(env_id.to_string())
    }
}

pub(crate) async fn env_def_from(
    opt_value: OptionalValueFlag,
) -> anyhow::Result<Option<(String, spin_environments::EnvironmentDefinition)>> {
    let env_id = match opt_value {
        OptionalValueFlag::NotPresent => None,
        OptionalValueFlag::PresentButNoValue => {
            let mut targets = spin_common::paths::search_upwards_for_manifest()
                .and_then(|(path, _)| std::fs::read_to_string(&path).ok())
                .and_then(|text| {
                    toml::from_str::<spin_manifest::schema::v2::AppManifest>(&text).ok()
                })
                .map(|m| m.application.targets)
                .unwrap_or_default();
            match targets.len() {
                1 => Some(targets.remove(0)),
                _ => anyhow::bail!("expected single environment ref, got multiple"),
            }
        }
        OptionalValueFlag::Present(env_id) => Some(parse_env(&env_id)),
    };

    match env_id {
        Some(env_id) => {
            let loaded_env_def =
                spin_environments::load_environment_def(&env_id, &std::env::current_dir()?).await?;
            Ok(Some((loaded_env_def.name, loaded_env_def.env_def)))
        }
        None => Ok(None),
    }
}
