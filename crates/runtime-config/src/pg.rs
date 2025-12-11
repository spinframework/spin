use std::path::PathBuf;

use serde::Deserialize;
use spin_factor_outbound_pg::runtime_config::RuntimeConfig;
use spin_factors::runtime_config::toml::GetTomlValue;

pub struct PgConfigResolver {
    pub(crate) base_dir: Option<PathBuf>, // must have a value if any certs, but we need to deref it lazily
}

impl PgConfigResolver {
    pub fn runtime_config_from_toml(
        &self,
        table: &impl GetTomlValue,
    ) -> anyhow::Result<RuntimeConfig> {
        let Some(table) = table.get("postgres").and_then(|t| t.as_table()) else {
            return Ok(Default::default());
        };

        let table: RuntimeConfigTable = RuntimeConfigTable::deserialize(table.clone())?;

        let certificate_paths = table
            .root_certificates
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        let has_relative = certificate_paths.iter().any(|p| p.is_relative());

        let certificate_paths = match (has_relative, self.base_dir.as_ref()) {
            (false, _) => certificate_paths,
            (true, None) => anyhow::bail!("the runtime config file contains relative certificate paths, but we could not determine the runtime config directory for them to be relative to"),
            (true, Some(base)) => certificate_paths.into_iter().map(|p| base.join(p)).collect::<Vec<_>>(),
        };

        let certificates = certificate_paths
            .iter()
            .map(std::fs::read)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(RuntimeConfig { certificates })
    }
}

#[derive(Deserialize)]
struct RuntimeConfigTable {
    #[serde(default)]
    root_certificates: Vec<String>,
}
