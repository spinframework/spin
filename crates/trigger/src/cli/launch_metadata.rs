use clap::{Args, CommandFactory};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;

use crate::{cli::FactorsTriggerCommand, Trigger};

use super::RuntimeFactorsBuilder;

/// Contains information about the trigger flags (and potentially
/// in future configuration) that a consumer (such as `spin up`)
/// can query using `--launch-metadata-only`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LaunchMetadata {
    all_flags: Vec<LaunchFlag>,
}

// This assumes no triggers that want to participate in multi-trigger
// use positional arguments. This is a restriction we'll have to make
// anyway: suppose triggers A and B both take one positional arg, and
// the user writes `spin up 123 456` - which value would go to which trigger?
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct LaunchFlag {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    short: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    long: Option<String>,
}

impl LaunchMetadata {
    pub fn infer<T, B>() -> Self
    where
        T: Trigger<B::Factors>,
        T::GlobalConfig: Args,
        B: RuntimeFactorsBuilder,
    {
        let all_flags: Vec<_> = FactorsTriggerCommand::<T, B>::command()
            .get_arguments()
            .map(LaunchFlag::infer)
            .collect();

        LaunchMetadata { all_flags }
    }

    pub fn matches<'a>(&self, groups: &[Vec<&'a OsString>]) -> Vec<&'a OsString> {
        let mut matches = vec![];

        for group in groups {
            if group.is_empty() {
                continue;
            }
            if self.is_match(group[0]) {
                matches.extend(group);
            }
        }

        matches
    }

    fn is_match(&self, arg: &OsString) -> bool {
        self.all_flags.iter().any(|f| f.is_match(arg))
    }

    pub fn is_group_match(&self, group: &[&OsString]) -> bool {
        if group.is_empty() {
            false
        } else {
            self.all_flags.iter().any(|f| f.is_match(group[0]))
        }
    }
}

impl LaunchFlag {
    fn infer(arg: &clap::Arg) -> Self {
        Self {
            long: arg.get_long().map(|s| format!("--{s}")),
            short: arg.get_short().map(|ch| format!("-{ch}")),
        }
    }

    fn is_match(&self, candidate: &OsString) -> bool {
        let Some(s) = candidate.to_str() else {
            return false;
        };
        let candidate = Some(s.to_owned());

        candidate == self.long || candidate == self.short
    }
}
