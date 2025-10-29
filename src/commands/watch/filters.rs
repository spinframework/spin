use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use async_trait::async_trait;
use spin_common::ui::quoted_path;
use spin_manifest::schema::v2;
use watchexec::filter::Filterer;

#[derive(Debug)]
pub(crate) struct AnyFilterer(Arc<dyn Filterer>);

impl Filterer for AnyFilterer {
    fn check_event(
        &self,
        event: &watchexec_events::Event,
        priority: watchexec_events::Priority,
    ) -> Result<bool, watchexec::error::RuntimeError> {
        let is_mutating = event.tags.iter().any(is_mutating);
        if !is_mutating {
            return Ok(false);
        }
        self.0.check_event(event, priority)
    }
}

/// The new globset filterer sends us *ALL* file events, including
/// non-mutating accesses from things like, say, GUI file explorers.
/// If we don't filter it down then we have a storm of gazillions
/// of events every few milliseconds, which would kick off a rebuild
/// every few milliseconds. ASK ME HOW I KNOW.
fn is_mutating(tag: &watchexec_events::Tag) -> bool {
    use watchexec_events::{filekind::FileEventKind, Tag};
    matches!(
        tag,
        Tag::FileEventKind(
            FileEventKind::Create(_) | FileEventKind::Modify(_) | FileEventKind::Remove(_)
        )
    )
}

#[async_trait]
pub(crate) trait FilterFactory: Send + Sync {
    async fn build_filter(
        &self,
        manifest_file: &Path,
        manifest_dir: &Path,
        manifest: &v2::AppManifest,
    ) -> anyhow::Result<AnyFilterer>;
}

pub(crate) struct ArtifactFilterFactory {
    pub skip_build: bool,
    pub skip_assets: bool,
}

pub(crate) struct BuildFilterFactory;
pub(crate) struct ManifestFilterFactory;

#[async_trait]
impl FilterFactory for ArtifactFilterFactory {
    async fn build_filter(
        &self,
        manifest_file: &Path,
        manifest_dir: &Path,
        manifest: &v2::AppManifest,
    ) -> anyhow::Result<AnyFilterer> {
        let manifest_glob = if self.skip_build {
            vec![manifest_path_to_watch(manifest_file)?]
        } else {
            vec![] // In this case, manifest changes trigger a rebuild, which will poke the uppificator anyway
        };
        let wasm_globs = manifest
            .components
            .values()
            .filter_map(|c| match &c.source {
                v2::ComponentSource::Local(path) => Some(path.clone()),
                _ => None,
            });
        let asset_globs = match self.skip_assets {
            true => {
                tracing::debug!("Skipping asset globs from being watched");
                vec![]
            }
            false => manifest
                .components
                .values()
                .flat_map(|c| c.files.iter())
                .filter_map(globbify)
                .collect::<Vec<_>>(),
        };

        let artifact_globs = manifest_glob
            .into_iter()
            .chain(wasm_globs)
            .chain(asset_globs)
            .collect::<Vec<_>>();

        let filterer = globset_filter(manifest_dir, artifact_globs).await?;

        Ok(AnyFilterer(Arc::new(filterer)))
    }
}

fn globbify(files_mount: &v2::WasiFilesMount) -> Option<String> {
    match files_mount {
        v2::WasiFilesMount::Placement { source, .. } => {
            Path::new(source).join("**/*").to_str().map(String::from)
        }
        v2::WasiFilesMount::Pattern(pattern) => Some(pattern.clone()),
    }
}

#[async_trait]
impl FilterFactory for BuildFilterFactory {
    async fn build_filter(
        &self,
        manifest_file: &Path,
        manifest_dir: &Path,
        manifest: &v2::AppManifest,
    ) -> anyhow::Result<AnyFilterer> {
        let mut filterers: Vec<Box<dyn Filterer>> =
            Vec::with_capacity(manifest.components.len() + 1);

        let manifest_globs = vec![manifest_path_to_watch(manifest_file)?];
        let manifest_filterer = globset_filter(manifest_dir, manifest_globs).await?;

        filterers.push(Box::new(manifest_filterer));

        for (cid, c) in &manifest.components {
            if let Some(build_globs) = create_source_globs(cid.as_ref(), c) {
                let build_filterer = globset_filter(manifest_dir, build_globs).await?;
                filterers.push(Box::new(build_filterer));
            }
        }

        let filterer = CompositeFilterer { filterers };

        Ok(AnyFilterer(Arc::new(filterer)))
    }
}

fn create_source_globs(cid: &str, c: &v2::Component) -> Option<Vec<String>> {
    let build = c.build.as_ref()?;
    if build.watch.is_empty() {
        eprintln!(
            "You haven't configured what to watch for the component: '{cid}'. Learn how to configure Spin watch at https://developer.fermyon.com/common/cli-reference#watch"
        );
        return None;
    };
    let globs = build
        .workdir
        .as_deref()
        .map(|workdir| {
            build
                .watch
                .iter()
                .map(|w| concatenate_glob_friendly(workdir, w))
                .collect()
        })
        .unwrap_or_else(|| build.watch.clone());
    if globs.is_empty() {
        // watchexec misinterprets empty list as "match all"
        None
    } else {
        Some(globs)
    }
}

/// Using Path::join on Windows correctly joins with a backslash. But the watchexec glob
/// engines hates backslashes. So we must always use forward slashes in glob expressions,
/// and in workdirs, and for concatenating the two. (We could use Path::join on Unix but
/// the signature ends up a bit different because to_str returns an Option, so...)
fn concatenate_glob_friendly(workdir: &str, watch_glob: &str) -> String {
    if PathBuf::from(watch_glob).is_absolute() {
        watch_glob.to_string()
    } else if workdir.ends_with('/') {
        format!("{workdir}{watch_glob}")
    } else {
        format!("{workdir}/{watch_glob}")
    }
}

#[async_trait]
impl FilterFactory for ManifestFilterFactory {
    async fn build_filter(
        &self,
        manifest_file: &Path,
        manifest_dir: &Path,
        _: &v2::AppManifest,
    ) -> anyhow::Result<AnyFilterer> {
        let manifest_glob = manifest_path_to_watch(manifest_file)?;

        let filterer = globset_filter(manifest_dir, [manifest_glob]).await?;

        Ok(AnyFilterer(Arc::new(filterer)))
    }
}

async fn globset_filter(
    manifest_dir: &Path,
    globs: impl IntoIterator<Item = String>,
) -> anyhow::Result<watchexec_filterer_globset::GlobsetFilterer> {
    let filterer = watchexec_filterer_globset::GlobsetFilterer::new(
        manifest_dir,
        globs.into_iter().map(|s| (s, None)),
        standard_ignores(),
        [],
        [],
        [],
    )
    .await?;

    Ok(filterer)
}

// Although manifest dir must be absolute, and most things are safer with abs
// file paths, the manifest _path_ for the watchers must be relative to manifest dir
fn manifest_path_to_watch(path: &Path) -> anyhow::Result<String> {
    let rel_path = path
        .file_name()
        .with_context(|| format!("resolved manifest {} has no filename", quoted_path(path)))?;
    Ok(rel_path.to_string_lossy().to_string())
}

fn standard_ignores() -> Vec<(String, Option<PathBuf>)> {
    [
        "**/*.swp", // Vim creates swap files during editing
    ]
    .into_iter()
    .map(|pat| (pat.to_owned(), None))
    .collect()
}

#[derive(Debug)]
struct CompositeFilterer {
    filterers: Vec<Box<dyn watchexec::filter::Filterer>>,
}

impl watchexec::filter::Filterer for CompositeFilterer {
    fn check_event(
        &self,
        event: &watchexec_events::Event,
        priority: watchexec_events::Priority,
    ) -> Result<bool, watchexec::error::RuntimeError> {
        // We are interested in a change if _any_ component is interested in it
        for f in &self.filterers {
            if f.check_event(event, priority)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joining_workdir_always_uses_forward_slash() {
        assert_eq!(
            "work/src/**/*.rs",
            concatenate_glob_friendly("work", "src/**/*.rs")
        );
        assert_eq!(
            "work/src/**/*.rs",
            concatenate_glob_friendly("work/", "src/**/*.rs")
        );

        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            "/global/src/**/*.rs",
            concatenate_glob_friendly("work", "/global/src/**/*.rs")
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            "D:/global/src/**/*.rs",
            concatenate_glob_friendly("work", "D:/global/src/**/*.rs")
        );
    }
}
