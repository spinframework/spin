use anyhow::Context;
use spin_common::data_dir::data_dir;
use std::path::{Path, PathBuf};

use crate::directory::subdirectories;

pub(crate) struct TemplateStore {
    root: PathBuf,
}

lazy_static::lazy_static! {
    static ref UNSAFE_CHARACTERS: regex::Regex = regex::Regex::new("[^-_a-zA-Z0-9]").expect("Invalid identifier regex");
}

impl TemplateStore {
    pub(crate) fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_owned(),
        }
    }

    pub(crate) fn try_default() -> anyhow::Result<Self> {
        Ok(Self::new(data_dir()?.join("templates")))
    }

    pub(crate) fn get_directory(&self, id: impl AsRef<str>) -> PathBuf {
        self.root.join(Self::relative_dir(id.as_ref()))
    }

    pub(crate) fn get_layout(&self, id: impl AsRef<str>) -> Option<TemplateLayout> {
        let template_dir = self.get_directory(id);
        if template_dir.exists() {
            Some(TemplateLayout::new(&template_dir))
        } else {
            None
        }
    }

    pub(crate) async fn list_layouts(&self) -> anyhow::Result<Vec<TemplateLayout>> {
        if !self.root.exists() {
            return Ok(vec![]);
        }

        let template_dirs = subdirectories(&self.root).with_context(|| {
            format!(
                "Failed to read template directories from {}",
                self.root.display()
            )
        })?;

        Ok(template_dirs.iter().map(TemplateLayout::new).collect())
    }

    fn relative_dir(id: &str) -> impl AsRef<Path> {
        // Using the SHA could generate quite long directory names, which could be a problem on Windows
        // if the template filenames are also long. Longer term, consider an alternative approach where
        // we use an index or something for disambiguation, and/or disambiguating only if a clash is
        // detected, etc.
        let id_sha256 = spin_common::sha256::hex_digest_from_bytes(id);
        format!("{}_{}", UNSAFE_CHARACTERS.replace_all(id, "_"), id_sha256)
    }
}

pub(crate) struct TemplateLayout {
    template_dir: PathBuf,
}

const METADATA_DIR_NAME: &str = "metadata";
const CONTENT_DIR_NAME: &str = "content";
const SNIPPETS_DIR_NAME: &str = "snippets";
const PARTIALS_DIR_NAME: &str = "partials";

const MANIFEST_FILE_NAME: &str = "spin-template.toml";

const INSTALLATION_RECORD_FILE_NAME: &str = ".install.toml";

impl TemplateLayout {
    pub fn new(template_dir: impl AsRef<Path>) -> Self {
        Self {
            template_dir: template_dir.as_ref().to_owned(),
        }
    }

    pub fn metadata_dir(&self) -> PathBuf {
        self.template_dir.join(METADATA_DIR_NAME)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.metadata_dir().join(MANIFEST_FILE_NAME)
    }

    pub fn content_dir(&self) -> PathBuf {
        self.template_dir.join(CONTENT_DIR_NAME)
    }

    pub fn snippets_dir(&self) -> PathBuf {
        self.metadata_dir().join(SNIPPETS_DIR_NAME)
    }

    pub fn partials_dir(&self) -> PathBuf {
        self.metadata_dir().join(PARTIALS_DIR_NAME)
    }

    pub fn installation_record_file(&self) -> PathBuf {
        self.template_dir.join(INSTALLATION_RECORD_FILE_NAME)
    }
}
