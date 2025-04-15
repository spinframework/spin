// Information about the application manifest that is of
// interest to the template system.  spin_loader does too
// much processing to fit our needs here.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::ensure;
use serde::Deserialize;
use spin_manifest::schema::v1;

use crate::store::TemplateLayout;

pub(crate) struct AppInfo {
    manifest_format: u32,
    trigger_type: Option<String>, // None = v2 template does not contain any triggers yet
}

impl AppInfo {
    pub fn from_layout(layout: &TemplateLayout) -> Option<anyhow::Result<AppInfo>> {
        Self::layout_manifest_path(layout)
            .map(|manifest_path| Self::from_existent_template(&manifest_path))
    }

    pub fn from_file(manifest_path: &Path) -> Option<anyhow::Result<AppInfo>> {
        if manifest_path.exists() {
            Some(Self::from_existent_file(manifest_path))
        } else {
            None
        }
    }

    fn layout_manifest_path(layout: &TemplateLayout) -> Option<PathBuf> {
        let manifest_path = layout.content_dir().join("spin.toml");
        if manifest_path.exists() {
            Some(manifest_path)
        } else {
            None
        }
    }

    fn from_existent_file(manifest_path: &Path) -> anyhow::Result<Self> {
        let manifest_str = std::fs::read_to_string(manifest_path)?;
        Self::from_manifest_text(&manifest_str)
    }

    fn from_manifest_text(manifest_str: &str) -> anyhow::Result<Self> {
        let manifest_version = spin_manifest::ManifestVersion::detect(manifest_str)?;
        let manifest_format = match manifest_version {
            spin_manifest::ManifestVersion::V1 => 1,
            spin_manifest::ManifestVersion::V2 => 2,
        };
        let trigger_type = match manifest_version {
            spin_manifest::ManifestVersion::V1 => Some(
                toml::from_str::<ManifestV1TriggerProbe>(manifest_str)?
                    .trigger
                    .trigger_type,
            ),
            spin_manifest::ManifestVersion::V2 => {
                let triggers = toml::from_str::<ManifestV2TriggerProbe>(manifest_str)?
                    .trigger
                    .unwrap_or_default();
                let type_count = triggers.len();
                ensure!(
                    type_count <= 1,
                    "only 1 trigger type currently supported; got {type_count}"
                );
                triggers.into_iter().next().map(|t| t.0)
            }
        };
        Ok(Self {
            manifest_format,
            trigger_type,
        })
    }

    fn from_existent_template(manifest_path: &Path) -> anyhow::Result<Self> {
        // This has to be cruder, because (with the v2 style of component) a template
        // is no longer valid TOML, so `from_existent_file` fails at manifest
        // version inference.
        let read_to_string = std::fs::read_to_string(manifest_path)?;
        let manifest_tpl_str = read_to_string;

        Self::from_template_text(&manifest_tpl_str)
    }

    fn from_template_text(manifest_tpl_str: &str) -> anyhow::Result<Self> {
        // TODO: investigate using a TOML parser or regex to be more accurate
        let is_v1_tpl = manifest_tpl_str.contains("spin_manifest_version = \"1\"");
        let is_v2_tpl = manifest_tpl_str.contains("spin_manifest_version = 2");
        if is_v1_tpl {
            // V1 manifest templates are valid TOML
            return Self::from_manifest_text(manifest_tpl_str);
        }
        if !is_v2_tpl {
            // The system will default to being permissive in this case
            anyhow::bail!("Unsure of template manifest version");
        }

        Self::from_v2_template_text(manifest_tpl_str)
    }

    fn from_v2_template_text(manifest_tpl_str: &str) -> anyhow::Result<Self> {
        let trigger_types: HashSet<_> = manifest_tpl_str
            .lines()
            .filter_map(infer_trigger_type_from_raw_line)
            .collect();
        let type_count = trigger_types.len();
        ensure!(
            type_count <= 1,
            "only 1 trigger type currently supported; got {type_count}"
        );
        let trigger_type = trigger_types.into_iter().next();

        Ok(Self {
            manifest_format: 2,
            trigger_type,
        })
    }

    pub fn manifest_format(&self) -> u32 {
        self.manifest_format
    }

    pub fn trigger_type(&self) -> Option<&str> {
        self.trigger_type.as_deref()
    }
}

lazy_static::lazy_static! {
    static ref EXTRACT_TRIGGER: regex::Regex =
        regex::Regex::new(r"^\s*\[\[trigger\.(?<trigger>[a-zA-Z0-9-]+)").expect("Invalid unknown filter regex");
}

fn infer_trigger_type_from_raw_line(line: &str) -> Option<String> {
    EXTRACT_TRIGGER
        .captures(line)
        .map(|c| c["trigger"].to_owned())
}

#[derive(Deserialize)]
struct ManifestV1TriggerProbe {
    // `trigger = { type = "<type>", ...}`
    trigger: v1::AppTriggerV1,
}

#[derive(Deserialize)]
struct ManifestV2TriggerProbe {
    /// `[trigger.<type>]` - empty will not have a trigger table in v2
    trigger: Option<toml::value::Table>,
}

// an irrelevant change to cause tests to run in ci

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_extract_triggers() {
        assert_eq!(
            "http",
            infer_trigger_type_from_raw_line("[[trigger.http]]").unwrap()
        );
        assert_eq!(
            "http",
            infer_trigger_type_from_raw_line("  [[trigger.http]]").unwrap()
        );
        assert_eq!(
            "fie",
            infer_trigger_type_from_raw_line("  [[trigger.fie]]").unwrap()
        );
        assert_eq!(
            "x-y",
            infer_trigger_type_from_raw_line("  [[trigger.x-y]]").unwrap()
        );

        assert_eq!(None, infer_trigger_type_from_raw_line("# [[trigger.http]]"));
        assert_eq!(None, infer_trigger_type_from_raw_line("trigger. But,"));
        assert_eq!(None, infer_trigger_type_from_raw_line("[[trigger.  snerk"));
    }

    #[test]
    fn can_read_app_info_from_template_v1() {
        let tpl = r#"spin_manifest_version = "1"
        name = "{{ thingy }}"
        version = "1.2.3"
        trigger = { type = "triggy", arg = "{{ another-thingy }}" }

        [[component]]
        id = "{{ thingy | kebab_case }}"
        source = "path/to/{{ thingy | snake_case }}.wasm"
        [component.trigger]
        spork = "{{ utensil }}"
        "#;

        let info = AppInfo::from_template_text(tpl).unwrap();
        assert_eq!(1, info.manifest_format);
        assert_eq!("triggy", info.trigger_type.unwrap());
    }

    #[test]
    fn can_read_app_info_from_template_v2() {
        let tpl = r#"spin_manifest_version = 2
        name = "{{ thingy }}"
        version = "1.2.3"

        [application.trigger.triggy]
        arg = "{{ another-thingy }}"

        [[trigger.triggy]]
        spork = "{{ utensil }}"
        component = "{{ thingy | kebab_case }}"

        [component.{{ thingy | kebab_case }}]
        source = "path/to/{{ thingy | snake_case }}.wasm"
        "#;

        let info = AppInfo::from_template_text(tpl).unwrap();
        assert_eq!(2, info.manifest_format);
        assert_eq!("triggy", info.trigger_type.unwrap());
    }

    #[test]
    fn can_read_app_info_from_triggerless_template_v2() {
        let tpl = r#"spin_manifest_version = 2
        name = "{{ thingy }}"
        version = "1.2.3"
        "#;

        let info = AppInfo::from_template_text(tpl).unwrap();
        assert_eq!(2, info.manifest_format);
        assert_eq!(None, info.trigger_type);
    }
}
