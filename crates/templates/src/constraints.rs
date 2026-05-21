use regex::Regex;

#[derive(Clone, Debug)]
pub(crate) struct StringConstraints {
    pub regex: Option<Regex>,
    pub allowed_values: Option<Vec<String>>,
}

impl StringConstraints {
    pub fn validate(&self, text: String) -> anyhow::Result<String> {
        if let Some(regex) = self.regex.as_ref()
            && !regex.is_match(&text)
        {
            anyhow::bail!("Input '{}' does not match pattern '{}'", text, regex);
        }
        if let Some(allowed_values) = self.allowed_values.as_ref()
            && !allowed_values.contains(&text)
        {
            anyhow::bail!(
                "Input '{}' is not one of the allowed values ({})",
                text,
                allowed_values.join(", ")
            );
        }
        Ok(text)
    }
}
