use std::collections::HashMap;

/// Serialisation format for the lockfile: registry -> env|pkg -> { name -> digest }
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TargetEnvironmentLockfile(HashMap<String, Digests>);

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Digests {
    package: HashMap<String, String>,
}

impl TargetEnvironmentLockfile {
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
                let ds = Digests { package: map };
                self.0.insert(registry.to_string(), ds);
            }
        }
    }
}
