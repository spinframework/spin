use std::{collections::HashMap, path::Path};

use crate::{
    cancellable::Cancellable,
    template::{TemplateParameter, TemplateParameterDataType},
    Run,
};

use anyhow::anyhow;
// use console::style;
use dialoguer::{Confirm, Input, Select};

pub(crate) trait InteractionStrategy {
    fn allow_generate_into(&self, target_dir: &Path) -> Cancellable<(), anyhow::Error>;
    fn populate_parameters(
        &self,
        run: &Run,
    ) -> Cancellable<HashMap<String, String>, anyhow::Error> {
        let mut values = HashMap::new();
        for parameter in run.template.parameters(&run.options.variant) {
            match self.populate_parameter(run, parameter) {
                Cancellable::Ok(value) => {
                    values.insert(parameter.id().to_owned(), value);
                }
                Cancellable::Cancelled => return Cancellable::Cancelled,
                Cancellable::Err(e) => return Cancellable::Err(e),
            }
        }
        Cancellable::Ok(values)
    }
    fn populate_parameter(
        &self,
        run: &Run,
        parameter: &TemplateParameter,
    ) -> Cancellable<String, anyhow::Error>;
}

pub(crate) struct Interactive;
pub(crate) struct Silent;

impl InteractionStrategy for Interactive {
    fn allow_generate_into(&self, target_dir: &Path) -> Cancellable<(), anyhow::Error> {
        if !is_directory_empty(target_dir) {
            let prompt = format!(
                "Directory '{}' already contains other files. Generate into it anyway?",
                target_dir.display()
            );
            match crate::interaction::confirm(&prompt) {
                Ok(true) => Cancellable::Ok(()),
                Ok(false) => Cancellable::Cancelled,
                Err(e) => Cancellable::Err(e.into()),
            }
        } else {
            Cancellable::Ok(())
        }
    }

    fn populate_parameter(
        &self,
        run: &Run,
        parameter: &TemplateParameter,
    ) -> Cancellable<String, anyhow::Error> {
        match run.options.values.get(parameter.id()) {
            Some(s) => Cancellable::Ok(s.clone()),
            None => match (run.options.accept_defaults, parameter.default_value()) {
                (true, Some(v)) => Cancellable::Ok(v.to_string()),
                _ => match crate::interaction::prompt_parameter(parameter) {
                    Some(v) => Cancellable::Ok(v),
                    None => Cancellable::Cancelled,
                },
            },
        }
    }
}

impl InteractionStrategy for Silent {
    fn allow_generate_into(&self, target_dir: &Path) -> Cancellable<(), anyhow::Error> {
        if is_directory_empty(target_dir) {
            Cancellable::Ok(())
        } else {
            let err = anyhow!(
                "Can't generate into {} as it already contains other files",
                target_dir.display()
            );
            Cancellable::Err(err)
        }
    }

    fn populate_parameter(
        &self,
        run: &Run,
        parameter: &TemplateParameter,
    ) -> Cancellable<String, anyhow::Error> {
        match run.options.values.get(parameter.id()) {
            Some(s) => Cancellable::Ok(s.clone()),
            None => match (run.options.accept_defaults, parameter.default_value()) {
                (true, Some(v)) => Cancellable::Ok(v.to_string()),
                _ => Cancellable::Err(anyhow!("Parameter '{}' not provided", parameter.id())),
            },
        }
    }
}

pub(crate) fn confirm(text: &str) -> dialoguer::Result<bool> {
    Confirm::new().with_prompt(text).interact()
}

pub(crate) fn prompt_parameter(parameter: &TemplateParameter) -> Option<String> {
    let prompt = parameter.prompt();
    let default_value = parameter.default_value();

    loop {
        let input = match parameter.data_type() {
            TemplateParameterDataType::String(constraints) => match &constraints.allowed_values {
                Some(allowed_values) => ask_choice(prompt, default_value, allowed_values),
                None => ask_free_text(prompt, default_value),
            },
        };

        match input {
            Ok(text) => match parameter.validate_value(text) {
                Ok(text) => return Some(text),
                Err(e) => {
                    println!("Invalid value: {e}");
                }
            },
            Err(e) => {
                println!("Invalid value: {e}");
            }
        }
    }
}

fn ask_free_text(prompt: &str, default_value: &Option<String>) -> anyhow::Result<String> {
    let mut input = Input::<String>::new();
    input = input.with_prompt(prompt);
    if let Some(s) = default_value {
        input = input.default(s.to_owned());
    }
    let result = input.interact_text()?;
    Ok(result)
}

fn ask_choice(
    prompt: &str,
    default_value: &Option<String>,
    allowed_values: &[String],
) -> anyhow::Result<String> {
    let mut select = Select::new().with_prompt(prompt).items(allowed_values);
    if let Some(s) = default_value {
        if let Some(default_index) = allowed_values.iter().position(|item| item == s) {
            select = select.default(default_index);
        }
    }
    let selected_index = select.interact()?;
    Ok(allowed_values[selected_index].clone())
}

fn is_directory_empty(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    if !path.is_dir() {
        return false;
    }
    match path.read_dir() {
        Err(_) => false,
        Ok(mut read_dir) => read_dir.next().is_none(),
    }
}
