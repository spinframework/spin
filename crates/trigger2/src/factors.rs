use std::path::PathBuf;

use spin_factor_key_value::{DefaultLabelResolver, KeyValueFactor};
use spin_factor_wasi::{spin::SpinFilesMounter, WasiFactor};
use spin_factors::RuntimeFactors;
use spin_runtime_config::TomlRuntimeConfigSource;

#[derive(RuntimeFactors)]
pub struct TriggerFactors {
    pub wasi: WasiFactor,
    pub key_value: KeyValueFactor,
}

impl TriggerFactors {
    pub fn new(
        working_dir: impl Into<PathBuf>,
        allow_transient_writes: bool,
        default_key_value_label_resolver: impl DefaultLabelResolver + 'static,
    ) -> Self {
        let files_mounter = SpinFilesMounter::new(working_dir, allow_transient_writes);
        Self {
            wasi: WasiFactor::new(files_mounter),
            key_value: KeyValueFactor::new(default_key_value_label_resolver),
        }
    }
}

impl TryFrom<TomlRuntimeConfigSource<'_>> for TriggerFactorsRuntimeConfig {
    type Error = anyhow::Error;

    fn try_from(value: TomlRuntimeConfigSource<'_>) -> Result<Self, Self::Error> {
        Self::from_source(value)
    }
}
