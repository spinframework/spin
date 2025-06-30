use std::collections::HashMap;

use anyhow::Result;
use tokio::process::Command;

use crate::git::UnderstandGitResult;

#[derive(Debug, Default)]
pub(crate) struct Authors {
    pub author: String,
    pub username: String,
}

type GitConfig = HashMap<String, String>;

/// Heavily adapted from cargo, portions (c) 2020 Cargo Developers
///
/// cf. <https://github.com/rust-lang/cargo/blob/2d5c2381e4e50484bf281fc1bfe19743aa9eb37a/src/cargo/ops/cargo_new.rs#L769-L851>
pub(crate) async fn get_authors() -> Result<Authors> {
    fn get_environment_variable(variables: &[&str]) -> Option<String> {
        variables
            .iter()
            .filter_map(|var| std::env::var(var).ok())
            .next()
    }

    async fn discover_author() -> Result<(String, Option<String>)> {
        let git_config = find_real_git_config().await;

        let name_variables = ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"];
        let backup_name_variables = ["USER", "USERNAME", "NAME"];
        let name = get_environment_variable(&name_variables)
            .or_else(|| git_config.get("user.name").map(|s| s.to_owned()))
            .or_else(|| get_environment_variable(&backup_name_variables));

        let name = match name {
            Some(name) => name,
            None => {
                let username_var = if cfg!(windows) { "USERNAME" } else { "USER" };
                anyhow::bail!(
                    "could not determine the current user, please set ${}",
                    username_var
                )
            }
        };
        let email_variables = ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL", "EMAIL"];
        let email = get_environment_variable(&email_variables[0..3])
            .or_else(|| git_config.get("user.email").map(|s| s.to_owned()))
            .or_else(|| get_environment_variable(&email_variables[3..]));

        let name = name.trim().to_string();
        let email = email.map(|s| {
            let mut s = s.trim();

            // In some cases emails will already have <> remove them since they
            // are already added when needed.
            if s.starts_with('<') && s.ends_with('>') {
                s = &s[1..s.len() - 1];
            }

            s.to_string()
        });

        Ok((name, email))
    }

    async fn find_real_git_config() -> GitConfig {
        find_real_git_config_inner().await.unwrap_or_default()
    }

    async fn find_real_git_config_inner() -> Option<GitConfig> {
        Command::new("git")
            .arg("config")
            .arg("--list")
            .output()
            .await
            .understand_git_result()
            .ok()
            .and_then(|stdout| try_parse_git_config(&stdout))
    }

    let author = match discover_author().await? {
        (name, Some(email)) => Authors {
            author: format!("{name} <{email}>"),
            username: name,
        },
        (name, None) => Authors {
            author: name.clone(),
            username: name,
        },
    };

    Ok(author)
}

fn try_parse_git_config(stdout: &[u8]) -> Option<GitConfig> {
    std::str::from_utf8(stdout).ok().map(parse_git_config_text)
}

fn parse_git_config_text(text: &str) -> GitConfig {
    text.lines().filter_map(try_parse_git_config_line).collect()
}

fn try_parse_git_config_line(line: &str) -> Option<(String, String)> {
    line.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
}
