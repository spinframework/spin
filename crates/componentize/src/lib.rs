#![deny(warnings)]

use {
    anyhow::{anyhow, Context, Result},
    module_info::ModuleInfo,
    std::{borrow::Cow, collections::HashSet},
    wasm_encoder::reencode::{Reencode, RoundtripReencoder},
    wasm_encoder::{CustomSection, ExportSection, ImportSection, Module, RawSection},
    wasmparser::{Encoding, Parser, Payload},
    wit_component::{metadata, ComponentEncoder},
};

pub mod bugs;

#[cfg(test)]
mod abi_conformance;
mod module_info;

const SPIN_ADAPTER: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/wasm32-unknown-unknown/release/wasi_snapshot_preview1_spin.wasm"
));
const PREVIEW1_ADAPTER: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/wasm32-unknown-unknown/release/wasi_snapshot_preview1_upstream.wasm"
));

const COMMAND_ADAPTER: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/wasm32-unknown-unknown/release/wasi_snapshot_preview1_command.wasm"
));

static ADAPTER_NAME: &str = "wasi_snapshot_preview1";
static CUSTOM_SECTION_NAME: &str = "component-type:reactor";
static WORLD_NAME: &str = "reactor";

static EXPORT_INTERFACES: &[(&str, &str)] = &[
    ("handle-redis-message", "inbound-redis"),
    ("handle-http-request", "inbound-http"),
];

pub fn componentize_if_necessary(module_or_component: &[u8]) -> Result<Cow<[u8]>> {
    for payload in Parser::new(0).parse_all(module_or_component) {
        if let Payload::Version { encoding, .. } = payload.context("unable to parse binary")? {
            return match encoding {
                Encoding::Component => Ok(Cow::Borrowed(module_or_component)),
                Encoding::Module => componentize(module_or_component).map(Cow::Owned),
            };
        }
    }
    Err(anyhow!("unable to determine wasm binary encoding"))
}

pub fn componentize(module: &[u8]) -> Result<Vec<u8>> {
    let module_info = ModuleInfo::from_module(module)?;
    match WitBindgenVersion::detect(&module_info)? {
        WitBindgenVersion::V0_2OrNone => componentize_old_module(module, &module_info),
        WitBindgenVersion::GreaterThanV0_4 => componentize_new_bindgen(module),
        WitBindgenVersion::Other(other) => Err(anyhow::anyhow!(
            "cannot adapt modules created with wit-bindgen version {other}"
        )),
    }
}

/// In order to properly componentize modules, we need to know which
/// version of wit-bindgen was used
#[derive(Debug)]
enum WitBindgenVersion {
    GreaterThanV0_4,
    V0_2OrNone,
    Other(String),
}

impl WitBindgenVersion {
    fn detect(module_info: &ModuleInfo) -> Result<Self> {
        if let Some(processors) = module_info.bindgen_processors() {
            let bindgen_version = processors
                .iter()
                .find_map(|(key, value)| key.starts_with("wit-bindgen").then_some(value.as_str()));
            if let Some(v) = bindgen_version {
                let mut parts = v.split('.');
                let Some(major) = parts.next().and_then(|p| p.parse::<u8>().ok()) else {
                    return Ok(Self::Other(v.to_owned()));
                };
                let Some(minor) = parts.next().and_then(|p| p.parse::<u8>().ok()) else {
                    return Ok(Self::Other(v.to_owned()));
                };
                if (major == 0 && minor < 5) || major >= 1 {
                    return Ok(Self::Other(v.to_owned()));
                }
                // Either there should be no patch version or nothing after patch
                if parts.next().is_none() || parts.next().is_none() {
                    return Ok(Self::GreaterThanV0_4);
                } else {
                    return Ok(Self::Other(v.to_owned()));
                }
            }
        }
        Ok(Self::V0_2OrNone)
    }
}

/// Modules produced with wit-bindgen 0.5 and newer only need wasi preview 1 to preview 2 adapter
pub fn componentize_new_bindgen(module: &[u8]) -> Result<Vec<u8>> {
    ComponentEncoder::default()
        .validate(true)
        .module(module)?
        .adapter("wasi_snapshot_preview1", PREVIEW1_ADAPTER)?
        .encode()
}

/// Modules *not* produced with wit-bindgen >= 0.5 could be old wit-bindgen or no wit-bindgen
pub fn componentize_old_module(module: &[u8], module_info: &ModuleInfo) -> Result<Vec<u8>> {
    // If the module has a _start export and doesn't obviously use wit-bindgen
    // it is likely an old p1 command module.
    if module_info.has_start_export && !module_info.probably_uses_wit_bindgen() {
        bugs::WasiLibc377Bug::check(module_info)?;
        componentize_command(module)
    } else {
        componentize_old_bindgen(module)
    }
}

/// Modules produced with wit-bindgen 0.2 need more extensive adaption
pub fn componentize_old_bindgen(module: &[u8]) -> Result<Vec<u8>> {
    let (module, exports) = retarget_imports_and_get_exports(ADAPTER_NAME, module)?;
    let allowed = exports
        .into_iter()
        .filter_map(|export| {
            EXPORT_INTERFACES
                .iter()
                .find_map(|(k, v)| (*k == export).then_some(*v))
        })
        .collect::<HashSet<&str>>();

    let (adapter, mut bindgen) = metadata::decode(SPIN_ADAPTER)?;
    let adapter = adapter.expect(
        "adapter module was malformed, and did not contain a 'component-type' custom section",
    );

    let world = bindgen
        .resolve
        .worlds
        .iter()
        .find_map(|(k, v)| (v.name == WORLD_NAME).then_some(k))
        .ok_or_else(|| anyhow!("world not found: {WORLD_NAME}"))?;

    bindgen.resolve.worlds[world].exports.retain(|k, _| {
        let k = match &k {
            wit_parser::WorldKey::Name(n) => n,
            wit_parser::WorldKey::Interface(i) => match &bindgen.resolve.interfaces[*i].name {
                Some(n) => n,
                None => return true,
            },
        };
        allowed.contains(k.as_str())
    });

    let body = metadata::encode(
        &bindgen.resolve,
        world,
        wit_component::StringEncoding::UTF8,
        None,
    )?;

    let adapter = add_custom_section(CUSTOM_SECTION_NAME, &body, &adapter)?;

    ComponentEncoder::default()
        .validate(true)
        .module(&module)?
        .adapter(ADAPTER_NAME, &adapter)?
        .encode()
}

pub fn componentize_command(module: &[u8]) -> Result<Vec<u8>> {
    ComponentEncoder::default()
        .validate(true)
        .module(module)?
        .adapter(ADAPTER_NAME, COMMAND_ADAPTER)?
        .encode()
}

fn retarget_imports_and_get_exports(target: &str, module: &[u8]) -> Result<(Vec<u8>, Vec<String>)> {
    let mut result = Module::new();
    let mut exports_result = Vec::new();

    for payload in Parser::new(0).parse_all(module) {
        match payload? {
            Payload::ImportSection(reader) => {
                let mut imports = ImportSection::new();
                for import in reader {
                    let import = import?;
                    let (module, field) = if import.module == target {
                        (Cow::Borrowed(import.module), Cow::Borrowed(import.name))
                    } else {
                        (
                            Cow::Borrowed(target),
                            Cow::Owned(format!("{}:{}", import.module, import.name)),
                        )
                    };
                    let ty = RoundtripReencoder.entity_type(import.ty)?;
                    imports.import(&module, &field, ty);
                }
                result.section(&imports);
            }

            Payload::ExportSection(reader) => {
                let mut exports = ExportSection::new();
                for export in reader {
                    let export = export?;
                    exports_result.push(export.name.to_owned());
                    let kind = RoundtripReencoder.export_kind(export.kind)?;
                    exports.export(export.name, kind, export.index);
                }
                result.section(&exports);
            }

            payload => {
                if let Some((id, range)) = payload.as_section() {
                    result.section(&RawSection {
                        id,
                        data: &module[range],
                    });
                }
            }
        }
    }

    Ok((result.finish(), exports_result))
}

fn add_custom_section(name: &str, data: &[u8], module: &[u8]) -> Result<Vec<u8>> {
    let mut result = Module::new();

    for payload in Parser::new(0).parse_all(module) {
        if let Some((id, range)) = payload?.as_section() {
            result.section(&RawSection {
                id,
                data: &module[range],
            });
        }
    }

    result.section(&CustomSection {
        name: Cow::Borrowed(name),
        data: Cow::Borrowed(data),
    });

    Ok(result.finish())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process};

    use anyhow::Context;
    use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

    use {
        super::abi_conformance::{
            InvocationStyle, KeyValueReport, LlmReport, MysqlReport, PostgresReport, RedisReport,
            Report, TestConfig, WasiReport,
        },
        anyhow::{anyhow, Result},
        tokio::fs,
        wasmtime::{
            component::{Component, Linker},
            Config, Engine, Store,
        },
        wasmtime_wasi::p2::{
            bindings::Command, pipe::MemoryInputPipe, IoView, WasiCtx, WasiCtxBuilder, WasiView,
        },
        wasmtime_wasi::ResourceTable,
    };

    async fn run_spin(module: &[u8]) -> Result<()> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);

        let engine = Engine::new(&config)?;

        let component = Component::new(
            &engine,
            crate::componentize(module).context("could not componentize")?,
        )
        .context("failed to instantiate componentized bytes")?;

        let report = super::abi_conformance::test(
            &component,
            &engine,
            TestConfig {
                invocation_style: InvocationStyle::InboundHttp,
            },
        )
        .await
        .context("abi conformance test failed")?;

        let expected = Report {
            inbound_http: Ok(()),
            inbound_redis: Ok(()),
            config: Ok(()),
            http: Ok(()),
            redis: RedisReport {
                publish: Ok(()),
                set: Ok(()),
                get: Ok(()),
                incr: Ok(()),
                del: Ok(()),
                sadd: Ok(()),
                srem: Ok(()),
                smembers: Ok(()),
                execute: Ok(()),
            },
            postgres: PostgresReport {
                execute: Ok(()),
                query: Ok(()),
            },
            mysql: MysqlReport {
                execute: Ok(()),
                query: Ok(()),
            },
            key_value: KeyValueReport {
                open: Ok(()),
                get: Ok(()),
                set: Ok(()),
                delete: Ok(()),
                exists: Ok(()),
                get_keys: Ok(()),
                close: Ok(()),
            },
            llm: LlmReport { infer: Ok(()) },
            wasi: WasiReport {
                env: Ok(()),
                epoch: Ok(()),
                random: Ok(()),
                stdio: Ok(()),
                read: Ok(()),
                readdir: Ok(()),
                stat: Ok(()),
            },
        };

        if report == expected {
            Ok(())
        } else {
            Err(anyhow!("{report:#?}"))
        }
    }

    async fn run_command(module: &[u8]) -> Result<()> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);

        let engine = Engine::new(&config)?;

        struct Wasi {
            ctx: WasiCtx,
            table: ResourceTable,
        }
        impl IoView for Wasi {
            fn table(&mut self) -> &mut ResourceTable {
                &mut self.table
            }
        }
        impl WasiView for Wasi {
            fn ctx(&mut self) -> &mut WasiCtx {
                &mut self.ctx
            }
        }

        let mut linker = Linker::<Wasi>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        let mut ctx = WasiCtxBuilder::new();
        let stdout = MemoryOutputPipe::new(1024);
        ctx.stdin(MemoryInputPipe::new("So rested he by the Tumtum tree"))
            .stdout(stdout.clone())
            .args(&["Jabberwocky"]);

        let table = ResourceTable::new();
        let wasi = Wasi {
            ctx: ctx.build(),
            table,
        };

        let mut store = Store::new(&engine, wasi);

        let component = Component::new(&engine, crate::componentize_command(module)?)?;

        let wasi = Command::instantiate_async(&mut store, &component, &linker).await?;

        wasi.wasi_cli_run()
            .call_run(&mut store)
            .await?
            .map_err(|()| anyhow!("command returned with failing exit status"))?;

        drop(store);

        let stdout = stdout.try_into_inner().unwrap().to_vec();

        assert_eq!(
            b"Jabberwocky\nSo rested he by the Tumtum tree" as &[_],
            &stdout
        );

        Ok(())
    }

    #[tokio::test]
    async fn rust_wit_bindgen_02() -> Result<()> {
        build_rust_test_case("rust-case-0.2");
        run_spin(
            &fs::read(concat!(
                env!("OUT_DIR"),
                "/wasm32-wasip1/release/rust_case_02.wasm"
            ))
            .await?,
        )
        .await
    }

    #[tokio::test]
    async fn rust_wit_bindgen_08() -> Result<()> {
        build_rust_test_case("rust-case-0.8");
        run_spin(
            &fs::read(concat!(
                env!("OUT_DIR"),
                "/wasm32-wasip1/release/rust_case_08.wasm"
            ))
            .await?,
        )
        .await
    }

    #[ignore]
    #[tokio::test]
    async fn go() -> Result<()> {
        let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let mut cmd = process::Command::new("tinygo");
        cmd.arg("build")
            .current_dir("tests/go-case")
            .arg("-target=wasip1")
            .arg("-gc=leaking")
            .arg("-buildmode=c-shared")
            .arg("-no-debug")
            .arg("-o")
            .arg(out_dir.join("go_case.wasm"))
            .arg("main.go");

        // If just skip this if TinyGo is not installed
        _ = cmd.status();
        run_spin(&fs::read(concat!(env!("OUT_DIR"), "/go_case.wasm")).await?).await
    }

    #[tokio::test]
    async fn rust_command() -> Result<()> {
        build_rust_test_case("rust-command");
        run_command(
            &fs::read(concat!(
                env!("OUT_DIR"),
                "/wasm32-wasip1/release/rust-command.wasm"
            ))
            .await?,
        )
        .await
    }

    fn build_rust_test_case(name: &str) {
        let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let mut cmd = process::Command::new("cargo");
        cmd.arg("build")
            .current_dir(format!("tests/{name}"))
            .arg("--release")
            .arg("--target=wasm32-wasip1")
            .env("CARGO_TARGET_DIR", out_dir);

        let status = cmd.status().unwrap();
        assert!(status.success());
    }
}
