use spin_common::ui::quoted_path;
use spin_factors::anyhow::{self, bail, Context as _};
use std::{collections::HashMap, path::PathBuf, str::FromStr};

#[derive(Clone, Debug)]
pub enum VariableSource {
    /// The value of the given variable name is the given string
    Literal(String, String),
    /// The value of the given variable name is the content of the given file (as a string)
    FileContent(String, PathBuf),
    /// The file contains a map of variable names to (string) values
    JsonFile(PathBuf),
    /// The file contains a map of variable names to (string) values
    TomlFile(PathBuf),
}

impl VariableSource {
    pub fn get_variables(&self) -> anyhow::Result<HashMap<String, String>> {
        match self {
            VariableSource::Literal(key, val) => Ok([(key.to_string(), val.to_string())].into()),
            VariableSource::FileContent(key, path) => {
                let val = std::fs::read_to_string(path)
                    .with_context(|| format!("Failed to read {}.", quoted_path(path)))?;
                Ok([(key.to_string(), val)].into())
            }
            VariableSource::JsonFile(path) => {
                let json_bytes = std::fs::read(path)
                    .with_context(|| format!("Failed to read {}.", quoted_path(path)))?;
                let json_vars: HashMap<String, String> = serde_json::from_slice(&json_bytes)
                    .with_context(|| format!("Failed to parse JSON from {}.", quoted_path(path)))?;
                Ok(json_vars)
            }
            VariableSource::TomlFile(path) => {
                let toml_str = std::fs::read_to_string(path)
                    .with_context(|| format!("Failed to read {}.", quoted_path(path)))?;
                let toml_vars: HashMap<String, String> = toml::from_str(&toml_str)
                    .with_context(|| format!("Failed to parse TOML from {}.", quoted_path(path)))?;
                Ok(toml_vars)
            }
        }
    }
}

impl FromStr for VariableSource {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(path) = s.strip_prefix('@') {
            let path = PathBuf::from(path);
            match path.extension().and_then(|s| s.to_str()) {
                Some("json") => Ok(VariableSource::JsonFile(path)),
                Some("toml") => Ok(VariableSource::TomlFile(path)),
                _ => bail!("variable files must end in .json or .toml"),
            }
        } else if let Some((key, val)) = s.split_once('=') {
            if let Some(path) = val.strip_prefix('@') {
                Ok(VariableSource::FileContent(
                    key.to_string(),
                    PathBuf::from(path),
                ))
            } else {
                Ok(VariableSource::Literal(key.to_string(), val.to_string()))
            }
        } else {
            bail!("variables must be in the form 'key=value' or '@file'")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn source_from_str() {
        match "k=v".parse() {
            Ok(VariableSource::Literal(key, val)) => {
                assert_eq!(key, "k");
                assert_eq!(val, "v");
            }
            Ok(other) => panic!("wrong variant {other:?}"),
            Err(err) => panic!("{err:?}"),
        }
        match "k=@v.txt".parse() {
            Ok(VariableSource::FileContent(key, path)) => {
                assert_eq!(key, "k");
                assert_eq!(path, PathBuf::from("v.txt"));
            }
            Ok(other) => panic!("wrong variant {other:?}"),
            Err(err) => panic!("{err:?}"),
        }
        match "@file.json".parse() {
            Ok(VariableSource::JsonFile(_)) => {}
            Ok(other) => panic!("wrong variant {other:?}"),
            Err(err) => panic!("{err:?}"),
        }
        match "@file.toml".parse() {
            Ok(VariableSource::TomlFile(_)) => {}
            Ok(other) => panic!("wrong variant {other:?}"),
            Err(err) => panic!("{err:?}"),
        }
    }

    #[test]
    fn source_from_str_errors() {
        assert!(VariableSource::from_str("nope").is_err());
        assert!(VariableSource::from_str("@whatami").is_err());
        assert!(VariableSource::from_str("@wrong.kind").is_err());
    }

    #[test]
    fn literal_get_variables() {
        let vars = VariableSource::Literal("k".to_string(), "v".to_string())
            .get_variables()
            .unwrap();
        assert_eq!(vars["k"], "v");
    }

    #[test]
    fn file_content_get_variables() {
        let mut file = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        file.write_all(br#"sausage time!"#).unwrap();
        let path = file.into_temp_path();
        let vars = VariableSource::FileContent("k".to_string(), path.to_path_buf())
            .get_variables()
            .unwrap();
        assert_eq!(vars["k"], "sausage time!");
    }

    #[test]
    fn json_get_variables() {
        let mut json_file = tempfile::NamedTempFile::with_suffix(".json").unwrap();
        json_file.write_all(br#"{"k": "v"}"#).unwrap();
        let json_path = json_file.into_temp_path();
        let vars = VariableSource::JsonFile(json_path.to_path_buf())
            .get_variables()
            .unwrap();
        assert_eq!(vars["k"], "v");
    }

    #[test]
    fn toml() {
        let mut toml_file = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
        toml_file.write_all(br#"k = "v""#).unwrap();
        let toml_path = toml_file.into_temp_path();
        let vars = VariableSource::TomlFile(toml_path.to_path_buf())
            .get_variables()
            .unwrap();
        assert_eq!(vars["k"], "v");
    }
}
