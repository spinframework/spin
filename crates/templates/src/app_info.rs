// Information about the application manifest that is of
// interest to the template system.  spin_loader does too
// much processing to fit our needs here.

use std::path::Path;

pub(crate) struct AppInfo {
    manifest_format: u32,
}

impl AppInfo {
    pub fn from_file(manifest_path: &Path) -> Option<anyhow::Result<AppInfo>> {
        if manifest_path.exists() {
            Some(Self::from_existent_file(manifest_path))
        } else {
            None
        }
    }

    fn from_existent_file(manifest_path: &Path) -> anyhow::Result<Self> {
        // In the add-component case this is the target application's spin.toml,
        // which is always fully rendered (valid TOML). The component being added
        // may be templated, but that is handled by the renderer, not here.
        let manifest_str = std::fs::read_to_string(manifest_path)?;
        Self::from_manifest_text(&manifest_str)
    }

    fn from_manifest_text(manifest_str: &str) -> anyhow::Result<Self> {
        let manifest_version = spin_manifest::ManifestVersion::detect(manifest_str)?;
        let manifest_format = match manifest_version {
            spin_manifest::ManifestVersion::V1 => 1,
            spin_manifest::ManifestVersion::V2 => 2,
        };
        Ok(Self { manifest_format })
    }

    pub fn manifest_format(&self) -> u32 {
        self.manifest_format
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_detect_v1_manifest_format() {
        let manifest = r#"spin_manifest_version = "1"
        name = "test"
        version = "1.2.3"
        trigger = { type = "http" }

        [[component]]
        id = "test"
        source = "test.wasm"
        [component.trigger]
        route = "/"
        "#;

        let info = AppInfo::from_manifest_text(manifest).unwrap();
        assert_eq!(1, info.manifest_format);
    }

    #[test]
    fn can_detect_v2_manifest_format() {
        let manifest = r#"spin_manifest_version = 2
        name = "test"
        version = "1.2.3"

        [[trigger.http]]
        route = "/"
        component = "test"

        [component.test]
        source = "test.wasm"
        "#;

        let info = AppInfo::from_manifest_text(manifest).unwrap();
        assert_eq!(2, info.manifest_format);
    }
}
