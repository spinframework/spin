//! ID (de)serialization

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An ID is a non-empty string containing one or more component model
/// `word`s separated by a delimiter char.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(into = "String", try_from = "String")]
pub struct Id<const DELIM: char, const LOWER: bool>(String);

impl<const DELIM: char, const LOWER: bool> std::fmt::Display for Id<DELIM, LOWER> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<const DELIM: char, const LOWER: bool> AsRef<str> for Id<DELIM, LOWER> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<const DELIM: char, const LOWER: bool> From<Id<DELIM, LOWER>> for String {
    fn from(value: Id<DELIM, LOWER>) -> Self {
        value.0
    }
}

impl<const DELIM: char, const LOWER: bool> TryFrom<String> for Id<DELIM, LOWER> {
    type Error = String;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        if id.is_empty() {
            return Err("empty".into());
        }
        // Special-case common "wrong separator" errors
        if let Some(wrong) = wrong_delim::<DELIM>() {
            if id.contains(wrong) {
                return Err(format!(
                    "words must be separated with {DELIM:?}, not {wrong:?}"
                ));
            }
        }
        for word in id.split(DELIM) {
            if word.is_empty() {
                return Err(format!("{DELIM:?}-separated words must not be empty"));
            }
            let mut chars = word.chars();
            let first = chars.next().unwrap();
            if !first.is_ascii_alphabetic() {
                return Err(format!(
                    "{DELIM:?}-separated words must start with an ASCII letter; got {first:?}"
                ));
            }
            let word_is_uppercase = first.is_ascii_uppercase();
            for ch in chars {
                if ch.is_ascii_digit() {
                } else if !ch.is_ascii_alphanumeric() {
                    return Err(format!(
                        "{DELIM:?}-separated words may only contain alphanumeric ASCII; got {ch:?}"
                    ));
                } else if ch.is_ascii_uppercase() != word_is_uppercase {
                    return Err(format!("{DELIM:?}-separated words must be all lowercase or all UPPERCASE; got {word:?}"));
                }
            }
            if LOWER && word_is_uppercase {
                return Err(format!(
                    "Lower-case identifiers must be all lowercase; got {id:?}"
                ));
            }
        }
        Ok(Self(id))
    }
}

const fn wrong_delim<const DELIM: char>() -> Option<char> {
    match DELIM {
        '_' => Some('-'),
        '-' => Some('_'),
        _ => None,
    }
}
