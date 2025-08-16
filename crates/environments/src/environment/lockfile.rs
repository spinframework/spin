use std::collections::HashMap;

use super::is_versioned;

const DIGEST_TTL_HOURS: i64 = 24;

/// Serialisation format for the lockfile: registry -> env|pkg -> { name -> digest }
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TargetEnvironmentLockfile(HashMap<String, Digests>);

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Digests {
    env: HashMap<String, ExpirableDigest>,
    package: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum ExpirableDigest {
    Forever(String),
    Expiring {
        digest: String,
        correct_at: chrono::DateTime<chrono::Utc>,
    },
}

impl TargetEnvironmentLockfile {
    pub fn env_digest(&self, registry: &str, env_id: &str) -> Option<&str> {
        self.0
            .get(registry)
            .and_then(|ds| ds.env.get(env_id))
            .and_then(|s| s.current())
    }

    pub fn set_env_digest(&mut self, registry: &str, env_id: &str, digest: &str) {
        // If the environment is versioned, we assume it will not change (that is, any changes will
        // be reflected as a new version).  If the environment is *not* versioned, it represents
        // a hosted service which may change over time: allow the cached definition to expire every day or
        // so that we do not use a definition that is out of sync with the actual service.
        let expirable_digest = if is_versioned(env_id) {
            ExpirableDigest::forever(digest)
        } else {
            ExpirableDigest::expiring(digest)
        };

        match self.0.get_mut(registry) {
            Some(ds) => {
                ds.env.insert(env_id.to_string(), expirable_digest);
            }
            None => {
                let map = vec![(env_id.to_string(), expirable_digest)]
                    .into_iter()
                    .collect();
                let ds = Digests {
                    env: map,
                    package: Default::default(),
                };
                self.0.insert(registry.to_string(), ds);
            }
        }
    }

    pub fn package_digest(
        &self,
        registry: &str,
        package: &wit_parser::PackageName,
    ) -> Option<&str> {
        self.0
            .get(registry)
            .and_then(|ds| ds.package.get(&package.to_string()))
            .map(|s| s.as_str())
    }

    pub fn set_package_digest(
        &mut self,
        registry: &str,
        package: &wit_parser::PackageName,
        digest: &str,
    ) {
        match self.0.get_mut(registry) {
            Some(ds) => {
                ds.package.insert(package.to_string(), digest.to_string());
            }
            None => {
                let map = vec![(package.to_string(), digest.to_string())]
                    .into_iter()
                    .collect();
                let ds = Digests {
                    env: Default::default(),
                    package: map,
                };
                self.0.insert(registry.to_string(), ds);
            }
        }
    }
}

impl ExpirableDigest {
    fn current(&self) -> Option<&str> {
        match self {
            Self::Forever(digest) => Some(digest),
            Self::Expiring { digest, correct_at } => {
                let now = chrono::Utc::now();
                let time_since = now - correct_at;
                if time_since.abs().num_hours() > DIGEST_TTL_HOURS {
                    None
                } else {
                    Some(digest)
                }
            }
        }
    }

    fn forever(digest: &str) -> Self {
        Self::Forever(digest.to_string())
    }

    fn expiring(digest: &str) -> Self {
        Self::Expiring {
            digest: digest.to_string(),
            correct_at: chrono::Utc::now(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const DUMMY_REG: &str = "reggy-mc-regface";

    #[test]
    fn versioned_envs_have_no_expiry() {
        const TEST_ENV: &str = "my-env:1.0";
        const TEST_DIGEST: &str = "12345";

        let mut lockfile = TargetEnvironmentLockfile::default();
        lockfile.set_env_digest(DUMMY_REG, TEST_ENV, TEST_DIGEST);

        let json = serde_json::to_value(&lockfile).unwrap();

        let saved_digest = json
            .get(DUMMY_REG)
            .and_then(|j| j.get("env"))
            .and_then(|j| j.get(TEST_ENV))
            .expect("should have had recorded a digest");
        let saved_digest = saved_digest
            .as_str()
            .expect("saved digest should have been a string");
        assert_eq!(TEST_DIGEST, saved_digest);
    }

    #[test]
    fn unversioned_envs_expire() {
        const TEST_ENV: &str = "my-env";
        const TEST_DIGEST: &str = "12345";

        let mut lockfile = TargetEnvironmentLockfile::default();
        lockfile.set_env_digest(DUMMY_REG, TEST_ENV, TEST_DIGEST);

        let json = serde_json::to_value(&lockfile).unwrap();

        let saved_digest = json
            .get(DUMMY_REG)
            .and_then(|j| j.get("env"))
            .and_then(|j| j.get(TEST_ENV))
            .expect("should have recorded a digest");
        let saved_digest = saved_digest
            .as_object()
            .expect("saved digest should have been an object");
        assert_eq!(TEST_DIGEST, saved_digest.get("digest").unwrap());
        assert!(saved_digest
            .get("correct_at")
            .is_some_and(|v| v.is_string()));
    }

    #[test]
    fn expired_env_digests_are_not_returned() {
        const TEST_ENV: &str = "my-env";
        const TEST_DIGEST: &str = "12345";

        let mut lockfile = TargetEnvironmentLockfile::default();
        lockfile.set_env_digest(DUMMY_REG, TEST_ENV, TEST_DIGEST);
        assert_eq!(
            TEST_DIGEST,
            lockfile
                .env_digest(DUMMY_REG, TEST_ENV)
                .expect("should have returned env digest")
        );

        // Pass this legit lockfile through JSON and massage the digest date to be old. NEARLY AS OLD AS ME
        let mut json = serde_json::to_value(&lockfile).unwrap();
        let digest = json
            .get_mut(DUMMY_REG)
            .and_then(|j| j.get_mut("env"))
            .and_then(|j| j.get_mut(TEST_ENV))
            .expect("should have recorded a digest");
        let digest = digest
            .as_object_mut()
            .expect("saved digest should have been an object");
        digest.insert(
            "correct_at".to_string(),
            serde_json::to_value("1969-12-31T01:01:01.001001001Z").unwrap(),
        );
        let stale_lockfile: TargetEnvironmentLockfile = serde_json::from_value(json).unwrap();

        // It should not give us the potentially stale digest
        assert!(stale_lockfile.env_digest(DUMMY_REG, TEST_ENV).is_none());
    }
}
