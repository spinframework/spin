use std::{collections::HashMap, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=components");
    println!("cargo:rerun-if-changed=helper");
    println!("cargo:rerun-if-changed=adapters");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR env variable not set"));
    let packages = std::fs::read_dir("components")
        .expect("could not read components directory")
        .filter_map(|e| {
            let dir = e.ok()?;
            let file_type = dir.file_type().ok()?;
            file_type.is_dir().then_some(dir)
        })
        .map(|e| {
            e.file_name()
                .into_string()
                .expect("file name is not valid utf8")
        })
        .collect::<Vec<_>>();

    let mut generated_code = String::new();
    let mut name_to_path = HashMap::new();
    for package in packages {
        let crate_path = PathBuf::from("components").join(&package);
        let manifest_path = crate_path.join("Cargo.toml");
        if !manifest_path.exists() {
            eprintln!("No Cargo.toml in {crate_path:?}; skipping");
            continue;
        }
        let manifest = cargo_toml::Manifest::from_path(&manifest_path).unwrap_or_else(|_| {
            panic!("failed to read and parse Cargo manifest {manifest_path:?}")
        });

        eprintln!("Building test component {:?}", manifest.package().name());

        // Build the test component
        let mut cargo = Command::new("cargo");
        cargo
            .current_dir(crate_path)
            .arg("build")
            .arg("--target=wasm32-wasip1")
            .env("RUSTFLAGS", rustflags())
            .env("CARGO_TARGET_DIR", &out_dir)
            // If RUSTFLAGS was set it'll be passed to this build script through
            // this variable but since we're cross-compiling to wasm we almost
            // surely don't want this to get picked up, so remove this from the
            // destination flags.
            .env_remove("CARGO_ENCODED_RUSTFLAGS");
        eprintln!("running: {cargo:?}");
        let status = cargo.status().expect("`cargo build` failed");
        assert!(status.success(), "{status:?}");
        let const_name = to_shouty_snake_case(&package);
        let package_name = manifest.package.expect("manifest has no package").name;
        let binary_name = package_name.replace(['-', '.'], "_");
        let mut wasm_path = out_dir
            .join("wasm32-wasip1")
            .join("debug")
            .join(format!("{binary_name}.wasm"));

        let adapter_version = package.split('v').next_back().and_then(|v| match v {
            // Only allow these versions through
            "0.2.0-rc-2023-11-10" | "0.2.0" => Some(v),
            _ => None,
        });

        if let Some(adapter_version) = adapter_version {
            let module_bytes = std::fs::read(&wasm_path).expect("failed to read wasm binary");
            let adapter_bytes = std::fs::read(format!("adapters/{adapter_version}.reactor.wasm"))
                .expect("failed to read adapter wasm binary");
            let new_bytes = wit_component::ComponentEncoder::default()
                .validate(true)
                .module(&module_bytes)
                .expect("failed to set wasm module")
                .adapter("wasi_snapshot_preview1", &adapter_bytes)
                .expect("failed to apply adapter")
                .encode()
                .expect("failed to encode component");
            wasm_path = wasm_path.with_extension("adapted.wasm");
            std::fs::write(&wasm_path, new_bytes).expect("failed to write new wasm binary");
        }

        // Generate const with the wasm binary path
        generated_code += &format!("pub const {const_name}: &str = {wasm_path:?};\n",);
        name_to_path.insert(package, wasm_path);
    }

    // Generate helper function to map package name to binary path
    generated_code.push_str("pub fn path(name: &str) -> Option<&'static str> {\n");
    for (name, path) in name_to_path {
        generated_code.push_str(&format!("    if name == {name:?} {{\n"));
        generated_code.push_str(&format!("        return Some({path:?});\n"));
        generated_code.push_str("    }\n");
    }
    generated_code.push_str("None\n}");
    std::fs::write(out_dir.join("gen.rs"), generated_code).expect("failed to write gen.rs");
}

fn to_shouty_snake_case(package: &str) -> String {
    package.to_uppercase().replace(['-', '.'], "_")
}

fn rustflags() -> &'static str {
    match option_env!("RUSTFLAGS") {
        // If we're in CI which is denying warnings then deny warnings to code
        // built here too to keep the tree warning-free.
        Some(s) if s.contains("-D warnings") => "-D warnings",
        _ => "",
    }
}
