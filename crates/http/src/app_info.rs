use serde::{Deserialize, Serialize};
#[cfg(feature = "runtime")]
use spin_app::{APP_NAME_KEY, APP_VERSION_KEY, App, OCI_IMAGE_DIGEST_KEY};

#[derive(Debug, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oci_image_digest: Option<String>,
}

impl AppInfo {
    #[cfg(feature = "runtime")]
    pub fn new(app: &App) -> Self {
        let name = app
            .get_metadata(APP_NAME_KEY)
            .unwrap_or_default()
            .unwrap_or_default();
        let version = app.get_metadata(APP_VERSION_KEY).unwrap_or_default();
        let oci_image_digest = app.get_metadata(OCI_IMAGE_DIGEST_KEY).unwrap_or_default();
        Self {
            name,
            version,
            oci_image_digest,
        }
    }
}
