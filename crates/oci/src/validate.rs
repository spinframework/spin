use anyhow::{bail, Context, Result};
use spin_common::{ui::quoted_path, url::parse_file_url};
use spin_locked_app::locked::{LockedComponent, LockedComponentSource};

/// Validate that all Spin components specify valid wasm binaries in both the `source`
/// field and for each dependency.
pub async fn ensure_wasms(component: &LockedComponent) -> Result<()> {
    // Ensure that the component source is a valid wasm binary.
    let bytes = read_component_source(&component.source).await?;
    if !is_wasm_binary(&bytes) {
        bail!(
            "Component {} source is not a valid .wasm file",
            component.id,
        );
    }

    // Ensure that each dependency is a valid wasm binary.
    for (dep_name, dep) in &component.dependencies {
        let bytes = read_component_source(&dep.source).await?;
        if !is_wasm_binary(&bytes) {
            bail!(
                "dependency {} for component {} is not a valid .wasm file",
                dep_name,
                component.id,
            );
        }
    }
    Ok(())
}

fn is_wasm_binary(bytes: &[u8]) -> bool {
    wasmparser::Parser::is_component(bytes)
        || wasmparser::Parser::is_core_wasm(bytes)
        || wat::parse_bytes(bytes).is_ok()
}

async fn read_component_source(source: &LockedComponentSource) -> Result<Vec<u8>> {
    let source = source
        .content
        .source
        .as_ref()
        .context("LockedComponentSource missing source field")?;

    let path = parse_file_url(source)?;

    let bytes: Vec<u8> = tokio::fs::read(&path).await.with_context(|| {
        format!(
            "failed to read component source from disk at path {}",
            quoted_path(&path)
        )
    })?;
    Ok(bytes)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::from_json;
    use spin_locked_app::locked::LockedComponent;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn ensures_valid_wasm_binaries() {
        let working_dir = tempfile::tempdir().unwrap();

        macro_rules! make_locked {
            ($source:literal, $($dep_name:literal=$dep_path:literal),*) => {
                from_json!({
                    "id": "jiggs",
                    "source": {
                        "content_type": "application/wasm",
                        "source": format!("file://{}", working_dir.path().join($source).to_str().unwrap()),
                        "digest": "digest",
                    },
                    "dependencies": {
                        $(
                            $dep_name: {
                                "source": {
                                    "content_type": "application/wasm",
                                    "source": format!("file://{}", working_dir.path().join($dep_path).to_str().unwrap()),
                                    "digest": "digest",
                                },
                            }
                        ),*
                    }
                })
            };
        }

        let make_file = async |name, content| {
            let path = working_dir.path().join(name);

            let mut file = tokio::fs::File::create(path)
                .await
                .expect("should create file");
            file.write_all(content)
                .await
                .expect("should write file contents");
        };

        // valid component source using WAT
        make_file("component.wat", b"(component)").await;
        // valid module source using WAT
        make_file("module.wat", b"(module)").await;
        // valid component source
        make_file("component.wasm", b"\x00\x61\x73\x6D\x0D\x00\x01\x00").await;
        // valid core module source
        make_file("module.wasm", b"\x00\x61\x73\x6D\x01\x00\x00\x00").await;
        // invalid wasm binary
        make_file("invalid.wasm", b"not a wasm file").await;

        #[derive(Clone)]
        struct TestCase {
            name: &'static str,
            locked_component: LockedComponent,
            valid: bool,
        }

        let tests: Vec<TestCase> = vec![
            TestCase {
                name: "Valid Spin component with component WAT",
                locked_component: make_locked!("component.wat",),
                valid: true,
            },
            TestCase {
                name: "Valid Spin component with module WAT",
                locked_component: make_locked!("module.wat",),
                valid: true,
            },
            TestCase {
                name: "Valid Spin component with wasm component",
                locked_component: make_locked!("component.wasm",),
                valid: true,
            },
            TestCase {
                name: "Valid Spin component with wasm core module",
                locked_component: make_locked!("module.wasm",),
                valid: true,
            },
            TestCase {
                name: "Valid Spin component with wasm dependency",
                locked_component: make_locked!("component.wasm", "test:comp2" = "component.wasm"),
                valid: true,
            },
            TestCase {
                name: "Invalid Spin component with invalid wasm binary",
                locked_component: make_locked!("invalid.wasm",),
                valid: false,
            },
            TestCase {
                name: "Valid Spin component with invalid wasm dependency",
                locked_component: make_locked!("component.wasm", "test:comp2" = "invalid.wasm"),
                valid: false,
            },
        ];

        for tc in tests {
            let result = ensure_wasms(&tc.locked_component).await;
            if tc.valid {
                assert!(result.is_ok(), "Test failed: {}", tc.name);
            } else {
                assert!(result.is_err(), "Test should have failed: {}", tc.name);
            }
        }
    }
}
