use anyhow::anyhow;
use lazy_static::lazy_static;
use std::{collections::HashMap, path::PathBuf};

use crate::writer::{TemplateOutput, TemplateOutputs};

// A template that has been evaluated and parsed, with all the values
// it needs to render.
pub(crate) struct TemplateRenderer {
    pub render_operations: Vec<RenderOperation>,
    pub parameter_values: HashMap<String, String>,
}

pub(crate) enum TemplateContent {
    Template(liquid::Template),
    Binary(Vec<u8>),
}

pub(crate) enum RenderOperation {
    AppendToml(PathBuf, TemplateContent),
    MergeToml(PathBuf, MergeTarget, TemplateContent), // file to merge into, table to merge into, content to merge
    WriteFile(TemplateablePath, TemplateContent),
    CreateDirectory(PathBuf, std::sync::Arc<liquid::Template>),
}

pub(crate) enum TemplateablePath {
    Plain(PathBuf),
    Template(liquid::Template),
}

impl TemplateablePath {
    fn render(self, globals: &liquid::Object) -> anyhow::Result<PathBuf> {
        Ok(match self {
            TemplateablePath::Plain(path) => path,
            TemplateablePath::Template(template) => PathBuf::from(template.render(globals)?),
        })
    }
}

pub(crate) enum MergeTarget {
    Application(&'static str),
}

impl TemplateRenderer {
    pub(crate) fn render(self) -> anyhow::Result<TemplateOutputs> {
        let globals = self.renderer_globals();

        let outputs = self
            .render_operations
            .into_iter()
            .map(|so| so.render(&globals))
            .collect::<anyhow::Result<Vec<_>>>()?;

        if outputs.is_empty() {
            return Err(anyhow!("Nothing to create"));
        }

        Ok(TemplateOutputs::new(outputs))
    }

    fn renderer_globals(&self) -> liquid::Object {
        let mut object = liquid::Object::new();

        for (k, v) in &self.parameter_values {
            object.insert(
                k.to_owned().into(),
                liquid_core::Value::Scalar(v.to_owned().into()),
            );
        }

        object
    }
}

impl RenderOperation {
    fn render(self, globals: &liquid::Object) -> anyhow::Result<TemplateOutput> {
        match self {
            Self::WriteFile(path, content) => {
                let rendered = content.render(globals)?;
                let path = path.render(globals)?;
                Ok(TemplateOutput::WriteFile(path, rendered))
            }
            Self::AppendToml(path, content) => {
                let rendered = content.render(globals)?;
                let rendered_text = String::from_utf8(rendered)?;
                Ok(TemplateOutput::AppendToml(path, rendered_text))
            }
            Self::MergeToml(path, target, content) => {
                let rendered = content.render(globals)?;
                let rendered_text = String::from_utf8(rendered)?;
                let MergeTarget::Application(target_table) = target;
                Ok(TemplateOutput::MergeToml(path, target_table, rendered_text))
            }
            Self::CreateDirectory(path, template) => {
                let rendered = template.render(globals)?;
                let path = path.join(rendered); // TODO: should we validate that `rendered` was relative?`
                Ok(TemplateOutput::CreateDirectory(path))
            }
        }
    }
}

impl TemplateContent {
    pub(crate) fn infer_from_bytes(
        raw: Vec<u8>,
        parser: &liquid::Parser,
    ) -> anyhow::Result<TemplateContent> {
        match string_from_bytes(&raw) {
            None => Ok(TemplateContent::Binary(raw)),
            Some(s) => {
                match parser.parse(&s) {
                    Ok(t) => Ok(TemplateContent::Template(t)),
                    Err(e) => match understand_liquid_error(e) {
                        TemplateParseFailure::Other(_e) => {
                            // TODO: emit a warning?
                            Ok(TemplateContent::Binary(raw))
                        }
                        TemplateParseFailure::UnknownFilter(id) => {
                            Err(anyhow!("internal error in template: unknown filter '{id}'"))
                        }
                    },
                }
            }
        }
    }

    fn render(self, globals: &liquid::Object) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Template(t) => {
                let text = t.render(globals)?;
                Ok(text.bytes().collect())
            }
            Self::Binary(v) => Ok(v),
        }
    }
}

// TODO: this doesn't truly belong in a module that claims to be about
// rendering but the only thing that uses it is the TemplateContent ctor
fn string_from_bytes(bytes: &[u8]) -> Option<String> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s.to_owned()),
        Err(_) => None, // TODO: try other encodings!
    }
}

enum TemplateParseFailure {
    UnknownFilter(String),
    Other(liquid::Error),
}

lazy_static! {
    static ref UNKNOWN_FILTER: regex::Regex =
        regex::Regex::new("requested filter=(\\S+)").expect("Invalid unknown filter regex");
}

fn understand_liquid_error(e: liquid::Error) -> TemplateParseFailure {
    let err_str = e.to_string();

    // They should use typed errors like we, er, don't
    match err_str.lines().next() {
        None => TemplateParseFailure::Other(e),
        Some("liquid: Unknown filter") => match UNKNOWN_FILTER.captures(&err_str) {
            None => TemplateParseFailure::Other(e),
            Some(captures) => match captures.get(1) {
                None => TemplateParseFailure::Other(e),
                Some(id) => TemplateParseFailure::UnknownFilter(id.as_str().to_owned()),
            },
        },
        _ => TemplateParseFailure::Other(e),
    }
}
