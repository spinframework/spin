use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use anyhow::{anyhow, Context};
use indexmap::IndexMap;
use itertools::Itertools;
use regex::Regex;

use crate::{
    constraints::StringConstraints,
    reader::{
        RawCondition, RawConditional, RawExtraOutput, RawParameter, RawTemplateManifest,
        RawTemplateManifestV1, RawTemplateVariant,
    },
    run::{Run, RunOptions},
    store::TemplateLayout,
};

/// A Spin template.
#[derive(Debug)]
pub struct Template {
    id: String,
    tags: HashSet<String>,
    description: Option<String>,
    installed_from: InstalledFrom,
    trigger: TemplateTriggerCompatibility,
    variants: HashMap<TemplateVariantKind, TemplateVariant>,
    parameters: Vec<TemplateParameter>,
    extra_outputs: Vec<ExtraOutputAction>,
    snippets_dir: Option<PathBuf>,
    partials_dir: Option<PathBuf>,
    content_dir: Option<PathBuf>, // TODO: maybe always need a spin.toml file in there?
}

#[derive(Debug)]
enum InstalledFrom {
    Git(String),
    Directory(String),
    RemoteTar(String),
    Unknown,
}

#[derive(Debug, Eq, PartialEq, Hash)]
enum TemplateVariantKind {
    NewApplication,
    AddComponent,
}

/// The variant mode in which a template should be run.
#[derive(Clone, Debug)]
pub enum TemplateVariantInfo {
    /// Create a new application from the template.
    NewApplication,
    /// Create a new component in an existing application from the template.
    AddComponent {
        /// The manifest to which the component will be added.
        manifest_path: PathBuf,
    },
}

impl TemplateVariantInfo {
    fn kind(&self) -> TemplateVariantKind {
        match self {
            Self::NewApplication => TemplateVariantKind::NewApplication,
            Self::AddComponent { .. } => TemplateVariantKind::AddComponent,
        }
    }

    /// A human-readable description of the variant.
    pub fn description(&self) -> &'static str {
        match self {
            Self::NewApplication => "new application",
            Self::AddComponent { .. } => "add component",
        }
    }

    /// The noun that should be used for the variant in a prompt
    pub fn prompt_noun(&self) -> &'static str {
        match self {
            Self::NewApplication => "application",
            Self::AddComponent { .. } => "component",
        }
    }

    /// The noun that should be used for the variant in a prompt,
    /// qualified with the appropriate a/an article for English
    pub fn articled_noun(&self) -> &'static str {
        match self {
            Self::NewApplication => "an application",
            Self::AddComponent { .. } => "a component",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TemplateVariant {
    skip_files: Vec<String>,
    skip_parameters: Vec<String>,
    snippets: HashMap<String, String>,
    conditions: Vec<Conditional>,
}

#[derive(Clone, Debug)]
pub(crate) struct Conditional {
    condition: Condition,
    skip_files: Vec<String>,
    skip_parameters: Vec<String>,
    skip_snippets: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum Condition {
    ManifestEntryExists(Vec<String>),
    #[cfg(test)]
    Always(bool),
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) enum TemplateTriggerCompatibility {
    Any,
    Only(String),
}

#[derive(Clone, Debug)]
pub(crate) enum TemplateParameterDataType {
    String(StringConstraints),
}

#[derive(Debug)]
pub(crate) struct TemplateParameter {
    id: String,
    data_type: TemplateParameterDataType, // TODO: possibly abstract to a ValidationCriteria type?
    prompt: String,
    default_value: Option<String>,
}

pub(crate) enum ExtraOutputAction {
    CreateDirectory(
        String,
        std::sync::Arc<liquid::Template>,
        crate::reader::CreateLocation,
    ),
}

impl std::fmt::Debug for ExtraOutputAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateDirectory(orig, ..) => {
                f.debug_tuple("CreateDirectory").field(orig).finish()
            }
        }
    }
}

impl Template {
    pub(crate) fn load_from(layout: &TemplateLayout) -> anyhow::Result<Self> {
        let manifest_path = layout.manifest_path();

        let manifest_text = std::fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "Failed to read template manifest file {}",
                manifest_path.display()
            )
        })?;
        let raw = crate::reader::parse_manifest_toml(manifest_text).with_context(|| {
            format!(
                "Manifest file {} is not a valid manifest",
                manifest_path.display()
            )
        })?;

        validate_manifest(&raw)?;

        let content_dir = if layout.content_dir().exists() {
            Some(layout.content_dir())
        } else {
            None
        };

        let snippets_dir = if layout.snippets_dir().exists() {
            Some(layout.snippets_dir())
        } else {
            None
        };

        let partials_dir = if layout.partials_dir().exists() {
            Some(layout.partials_dir())
        } else {
            None
        };

        let installed_from = read_install_record(layout);

        let template = match raw {
            RawTemplateManifest::V1(raw) => Self {
                id: raw.id.clone(),
                tags: raw.tags.map(Self::normalize_tags).unwrap_or_default(),
                description: raw.description.clone(),
                installed_from,
                trigger: Self::parse_trigger_type(raw.trigger_type, layout),
                variants: Self::parse_template_variants(raw.new_application, raw.add_component),
                parameters: Self::parse_parameters(&raw.parameters)?,
                extra_outputs: Self::parse_extra_outputs(&raw.outputs)?,
                snippets_dir,
                partials_dir,
                content_dir,
            },
        };
        Ok(template)
    }

    /// The ID of the template. This is used to identify the template
    /// on the Spin command line.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns true if the templates matches the provided set of tags.
    pub fn matches_all_tags(&self, match_set: &[String]) -> bool {
        match_set
            .iter()
            .all(|tag| self.tags().contains(&tag.to_lowercase()))
    }

    /// The set of tags associated with the template, provided by the
    /// template author.
    pub fn tags(&self) -> &HashSet<String> {
        &self.tags
    }

    /// A human-readable description of the template, provided by the
    /// template author.
    pub fn description(&self) -> &Option<String> {
        &self.description
    }

    /// A human-readable description of the template, provided by the
    /// template author, or an empty string if no description was
    /// provided.
    pub fn description_or_empty(&self) -> &str {
        match &self.description {
            Some(s) => s,
            None => "",
        }
    }

    /// The Git repository from which the template was installed, if
    /// it was installed from Git; otherwise None.
    pub fn source_repo(&self) -> Option<&str> {
        // TODO: this is kind of specialised - should we do the discarding of
        // non-Git sources at the application layer?
        match &self.installed_from {
            InstalledFrom::Git(url) => Some(url),
            _ => None,
        }
    }

    /// A human-readable description of where the template was installed
    /// from.
    pub fn installed_from_or_empty(&self) -> &str {
        match &self.installed_from {
            InstalledFrom::Git(repo) => repo,
            InstalledFrom::Directory(path) => path,
            InstalledFrom::RemoteTar(url) => url,
            InstalledFrom::Unknown => "",
        }
    }

    // TODO: we should resolve this once at the start of Run and then use that forever
    fn variant(&self, variant_info: &TemplateVariantInfo) -> Option<TemplateVariant> {
        let kind = variant_info.kind();
        self.variants
            .get(&kind)
            .map(|vt| vt.resolve_conditions(variant_info))
    }

    pub(crate) fn parameters(
        &self,
        variant_kind: &TemplateVariantInfo,
    ) -> impl Iterator<Item = &TemplateParameter> {
        let variant = self.variant(variant_kind).unwrap(); // TODO: for now
        self.parameters
            .iter()
            .filter(move |p| !variant.skip_parameter(p))
    }

    pub(crate) fn parameter(&self, name: impl AsRef<str>) -> Option<&TemplateParameter> {
        self.parameters.iter().find(|p| p.id == name.as_ref())
    }

    pub(crate) fn extra_outputs(&self) -> &[ExtraOutputAction] {
        &self.extra_outputs
    }

    pub(crate) fn content_dir(&self) -> &Option<PathBuf> {
        &self.content_dir
    }

    pub(crate) fn snippets_dir(&self) -> &Option<PathBuf> {
        &self.snippets_dir
    }

    pub(crate) fn partials_dir(&self) -> &Option<PathBuf> {
        &self.partials_dir
    }

    /// Checks if the template supports the specified variant mode.
    pub fn supports_variant(&self, variant: &TemplateVariantInfo) -> bool {
        self.variants.contains_key(&variant.kind())
    }

    pub(crate) fn snippets(&self, variant_kind: &TemplateVariantInfo) -> HashMap<String, String> {
        let variant = self.variant(variant_kind).unwrap(); // TODO: for now
        variant.snippets
    }

    /// Creates a runner for the template, governed by the given options. Call
    /// the relevant associated function of the `Run` to execute the template
    /// as appropriate to your application (e.g. `interactive()` to prompt the user
    /// for values and interact with the user at the console).
    pub fn run(self, options: RunOptions) -> Run {
        Run::new(self, options)
    }

    fn normalize_tags(tags: HashSet<String>) -> HashSet<String> {
        tags.into_iter().map(|tag| tag.to_lowercase()).collect()
    }

    fn parse_trigger_type(
        raw: Option<String>,
        layout: &TemplateLayout,
    ) -> TemplateTriggerCompatibility {
        match raw {
            None => Self::infer_trigger_type(layout),
            Some(t) => TemplateTriggerCompatibility::Only(t),
        }
    }

    fn infer_trigger_type(layout: &TemplateLayout) -> TemplateTriggerCompatibility {
        match crate::app_info::AppInfo::from_layout(layout) {
            Some(Ok(app_info)) => match app_info.trigger_type() {
                None => TemplateTriggerCompatibility::Any,
                Some(t) => TemplateTriggerCompatibility::Only(t.to_owned()),
            },
            _ => TemplateTriggerCompatibility::Any, // Fail forgiving
        }
    }

    fn parse_template_variants(
        new_application: Option<RawTemplateVariant>,
        add_component: Option<RawTemplateVariant>,
    ) -> HashMap<TemplateVariantKind, TemplateVariant> {
        let mut variants = HashMap::default();
        if let Some(vt) = Self::get_variant(new_application, true) {
            variants.insert(TemplateVariantKind::NewApplication, vt);
        }
        if let Some(vt) = Self::get_variant(add_component, false) {
            variants.insert(TemplateVariantKind::AddComponent, vt);
        }
        variants
    }

    fn get_variant(
        raw: Option<RawTemplateVariant>,
        default_supported: bool,
    ) -> Option<TemplateVariant> {
        match raw {
            None => {
                if default_supported {
                    Some(Default::default())
                } else {
                    None
                }
            }
            Some(rv) => {
                if rv.supported.unwrap_or(true) {
                    Some(Self::parse_template_variant(rv))
                } else {
                    None
                }
            }
        }
    }

    fn parse_template_variant(raw: RawTemplateVariant) -> TemplateVariant {
        TemplateVariant {
            skip_files: raw.skip_files.unwrap_or_default(),
            skip_parameters: raw.skip_parameters.unwrap_or_default(),
            snippets: raw.snippets.unwrap_or_default(),
            conditions: raw
                .conditions
                .unwrap_or_default()
                .into_values()
                .map(Self::parse_conditional)
                .collect(),
        }
    }

    fn parse_conditional(conditional: RawConditional) -> Conditional {
        Conditional {
            condition: Self::parse_condition(conditional.condition),
            skip_files: conditional.skip_files.unwrap_or_default(),
            skip_parameters: conditional.skip_parameters.unwrap_or_default(),
            skip_snippets: conditional.skip_snippets.unwrap_or_default(),
        }
    }

    fn parse_condition(condition: RawCondition) -> Condition {
        match condition {
            RawCondition::ManifestEntryExists(path) => {
                Condition::ManifestEntryExists(path.split('.').map(|s| s.to_string()).collect_vec())
            }
        }
    }

    fn parse_parameters(
        raw: &Option<IndexMap<String, RawParameter>>,
    ) -> anyhow::Result<Vec<TemplateParameter>> {
        match raw {
            None => Ok(vec![]),
            Some(parameters) => parameters
                .iter()
                .map(|(k, v)| TemplateParameter::from_raw(k, v))
                .collect(),
        }
    }

    fn parse_extra_outputs(
        raw: &Option<IndexMap<String, RawExtraOutput>>,
    ) -> anyhow::Result<Vec<ExtraOutputAction>> {
        match raw {
            None => Ok(vec![]),
            Some(parameters) => parameters
                .iter()
                .map(|(k, v)| ExtraOutputAction::from_raw(k, v))
                .collect(),
        }
    }

    pub(crate) fn included_files(
        &self,
        base: &std::path::Path,
        all_files: Vec<PathBuf>,
        variant_kind: &TemplateVariantInfo,
    ) -> Vec<PathBuf> {
        let variant = self.variant(variant_kind).unwrap(); // TODO: for now
        all_files
            .into_iter()
            .filter(|path| !variant.skip_file(base, path))
            .collect()
    }

    pub(crate) fn check_compatible_trigger(&self, app_trigger: Option<&str>) -> anyhow::Result<()> {
        // The application we are merging into might not have a trigger yet, in which case
        // we're good to go.
        let Some(app_trigger) = app_trigger else {
            return Ok(());
        };
        match &self.trigger {
            TemplateTriggerCompatibility::Any => Ok(()),
            TemplateTriggerCompatibility::Only(t) => {
                if app_trigger == t {
                    Ok(())
                } else {
                    Err(anyhow!("Component trigger type '{t}' does not match application trigger type '{app_trigger}'"))
                }
            }
        }
    }

    pub(crate) fn check_compatible_manifest_format(
        &self,
        manifest_format: u32,
    ) -> anyhow::Result<()> {
        let Some(content_dir) = &self.content_dir else {
            return Ok(());
        };
        let manifest_tpl = content_dir.join("spin.toml");
        if !manifest_tpl.is_file() {
            return Ok(());
        }

        // We can't load the manifest template because it's not valid TOML until
        // substituted, so GO BIG or at least GO CRUDE.
        let Ok(manifest_tpl_str) = std::fs::read_to_string(&manifest_tpl) else {
            return Ok(());
        };
        let is_v1_tpl = manifest_tpl_str.contains("spin_manifest_version = \"1\"");
        let is_v2_tpl = manifest_tpl_str.contains("spin_manifest_version = 2");

        // If we have not positively identified a format, err on the side of forgiveness
        let positively_identified = is_v1_tpl ^ is_v2_tpl; // exactly one should be true
        if !positively_identified {
            return Ok(());
        }

        let compatible = (is_v1_tpl && manifest_format == 1) || (is_v2_tpl && manifest_format == 2);

        if compatible {
            Ok(())
        } else {
            Err(anyhow!(
                "This template is for a different version of the Spin manifest"
            ))
        }
    }
}

impl TemplateParameter {
    fn from_raw(id: &str, raw: &RawParameter) -> anyhow::Result<Self> {
        let data_type = TemplateParameterDataType::parse(raw)?;

        Ok(Self {
            id: id.to_owned(),
            data_type,
            prompt: raw.prompt.clone(),
            default_value: raw.default_value.clone(),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn data_type(&self) -> &TemplateParameterDataType {
        &self.data_type
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn default_value(&self) -> &Option<String> {
        &self.default_value
    }

    pub fn validate_value(&self, value: impl AsRef<str>) -> anyhow::Result<String> {
        self.data_type.validate_value(value.as_ref().to_owned())
    }
}

impl TemplateParameterDataType {
    fn parse(raw: &RawParameter) -> anyhow::Result<Self> {
        match &raw.data_type[..] {
            "string" => Ok(Self::String(parse_string_constraints(raw)?)),
            _ => Err(anyhow!("Unrecognised data type '{}'", raw.data_type)),
        }
    }

    fn validate_value(&self, value: String) -> anyhow::Result<String> {
        match self {
            TemplateParameterDataType::String(constraints) => constraints.validate(value),
        }
    }
}

impl ExtraOutputAction {
    fn from_raw(id: &str, raw: &RawExtraOutput) -> anyhow::Result<Self> {
        Ok(match raw {
            RawExtraOutput::CreateDir(create) => {
                let path_template =
                    liquid::Parser::new().parse(&create.path).with_context(|| {
                        format!("Template error: output {id} is not a valid template")
                    })?;
                Self::CreateDirectory(
                    create.path.clone(),
                    std::sync::Arc::new(path_template),
                    create.at.unwrap_or_default(),
                )
            }
        })
    }
}

impl TemplateVariant {
    pub(crate) fn skip_file(&self, base: &std::path::Path, path: &std::path::Path) -> bool {
        self.skip_files
            .iter()
            .map(|s| base.join(s))
            .any(|f| path == f)
    }

    pub(crate) fn skip_parameter(&self, parameter: &TemplateParameter) -> bool {
        self.skip_parameters.iter().any(|p| &parameter.id == p)
    }

    fn resolve_conditions(&self, variant_info: &TemplateVariantInfo) -> Self {
        let mut resolved = self.clone();
        for condition in &self.conditions {
            if condition.condition.is_true(variant_info) {
                resolved
                    .skip_files
                    .append(&mut condition.skip_files.clone());
                resolved
                    .skip_parameters
                    .append(&mut condition.skip_parameters.clone());
                resolved
                    .snippets
                    .retain(|id, _| !condition.skip_snippets.contains(id));
            }
        }
        resolved
    }
}

impl Condition {
    fn is_true(&self, variant_info: &TemplateVariantInfo) -> bool {
        match self {
            Self::ManifestEntryExists(path) => match variant_info {
                TemplateVariantInfo::NewApplication => false,
                TemplateVariantInfo::AddComponent { manifest_path } => {
                    let Ok(toml_text) = std::fs::read_to_string(manifest_path) else {
                        return false;
                    };
                    let Ok(table) = toml::from_str::<toml::Value>(&toml_text) else {
                        return false;
                    };
                    crate::toml::get_at(table, path).is_some()
                }
            },
            #[cfg(test)]
            Self::Always(b) => *b,
        }
    }
}

fn parse_string_constraints(raw: &RawParameter) -> anyhow::Result<StringConstraints> {
    let regex = raw.pattern.as_ref().map(|re| Regex::new(re)).transpose()?;

    Ok(StringConstraints {
        regex,
        allowed_values: raw.allowed_values.clone(),
    })
}

fn read_install_record(layout: &TemplateLayout) -> InstalledFrom {
    use crate::reader::{parse_installed_from, RawInstalledFrom};

    let installed_from_text = std::fs::read_to_string(layout.installation_record_file()).ok();
    match installed_from_text.and_then(parse_installed_from) {
        Some(RawInstalledFrom::Git { git }) => InstalledFrom::Git(git),
        Some(RawInstalledFrom::File { dir }) => InstalledFrom::Directory(dir),
        Some(RawInstalledFrom::RemoteTar { url }) => InstalledFrom::RemoteTar(url),
        None => InstalledFrom::Unknown,
    }
}

fn validate_manifest(raw: &RawTemplateManifest) -> anyhow::Result<()> {
    match raw {
        RawTemplateManifest::V1(raw) => validate_v1_manifest(raw),
    }
}

fn validate_v1_manifest(raw: &RawTemplateManifestV1) -> anyhow::Result<()> {
    if raw.custom_filters.is_some() {
        anyhow::bail!("Custom filters are not supported in this version of Spin. Please update your template.");
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    struct TempFile {
        _temp_dir: tempfile::TempDir,
        path: PathBuf,
    }

    impl TempFile {
        fn path(&self) -> PathBuf {
            self.path.clone()
        }
    }

    fn make_temp_manifest(content: &str) -> TempFile {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_file = temp_dir.path().join("spin.toml");
        std::fs::write(&temp_file, content).unwrap();
        TempFile {
            _temp_dir: temp_dir,
            path: temp_file,
        }
    }

    #[test]
    fn manifest_entry_exists_condition_is_false_for_new_app() {
        let condition = Template::parse_condition(RawCondition::ManifestEntryExists(
            "application.trigger.redis".to_owned(),
        ));
        assert!(!condition.is_true(&TemplateVariantInfo::NewApplication));
    }

    #[test]
    fn manifest_entry_exists_condition_is_false_if_not_present_in_existing_manifest() {
        let temp_file =
            make_temp_manifest("name = \"hello\"\n[application.trigger.http]\nbase = \"/\"");
        let condition = Template::parse_condition(RawCondition::ManifestEntryExists(
            "application.trigger.redis".to_owned(),
        ));
        assert!(!condition.is_true(&TemplateVariantInfo::AddComponent {
            manifest_path: temp_file.path()
        }));
    }

    #[test]
    fn manifest_entry_exists_condition_is_true_if_present_in_existing_manifest() {
        let temp_file = make_temp_manifest(
            "name = \"hello\"\n[application.trigger.redis]\nchannel = \"HELLO\"",
        );
        let condition = Template::parse_condition(RawCondition::ManifestEntryExists(
            "application.trigger.redis".to_owned(),
        ));
        assert!(condition.is_true(&TemplateVariantInfo::AddComponent {
            manifest_path: temp_file.path()
        }));
    }

    #[test]
    fn manifest_entry_exists_condition_is_false_if_path_does_not_exist() {
        let condition = Template::parse_condition(RawCondition::ManifestEntryExists(
            "application.trigger.redis".to_owned(),
        ));
        assert!(!condition.is_true(&TemplateVariantInfo::AddComponent {
            manifest_path: PathBuf::from("this/file/does/not.exist")
        }));
    }

    #[test]
    fn selected_variant_respects_target() {
        let add_component_vt = TemplateVariant {
            conditions: vec![Conditional {
                condition: Condition::Always(true),
                skip_files: vec!["test2".to_owned()],
                skip_parameters: vec!["p1".to_owned()],
                skip_snippets: vec!["s1".to_owned()],
            }],
            skip_files: vec!["test1".to_owned()],
            snippets: [
                ("s1".to_owned(), "s1val".to_owned()),
                ("s2".to_owned(), "s2val".to_owned()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let variants = [
            (
                TemplateVariantKind::NewApplication,
                TemplateVariant::default(),
            ),
            (TemplateVariantKind::AddComponent, add_component_vt),
        ]
        .into_iter()
        .collect();
        let template = Template {
            id: "test".to_owned(),
            tags: HashSet::new(),
            description: None,
            installed_from: InstalledFrom::Unknown,
            trigger: TemplateTriggerCompatibility::Any,
            variants,
            parameters: vec![],
            extra_outputs: vec![],
            snippets_dir: None,
            partials_dir: None,
            content_dir: None,
        };

        let variant_info = TemplateVariantInfo::NewApplication;
        let variant = template.variant(&variant_info).unwrap();
        assert!(variant.skip_files.is_empty());
        assert!(variant.skip_parameters.is_empty());
        assert!(variant.snippets.is_empty());

        let add_variant_info = TemplateVariantInfo::AddComponent {
            manifest_path: PathBuf::from("dummy"),
        };
        let add_variant = template.variant(&add_variant_info).unwrap();
        // the conditional skip_files and skip_parameters are added to the variant's skip lists
        assert_eq!(2, add_variant.skip_files.len());
        assert!(add_variant.skip_files.contains(&"test1".to_owned()));
        assert!(add_variant.skip_files.contains(&"test2".to_owned()));
        assert_eq!(1, add_variant.skip_parameters.len());
        assert!(add_variant.skip_parameters.contains(&"p1".to_owned()));
        // the conditional skip_snippets are *removed from* the variant's snippets list
        assert_eq!(1, add_variant.snippets.len());
        assert!(!add_variant.snippets.contains_key("s1"));
        assert!(add_variant.snippets.contains_key("s2"));
    }
}
