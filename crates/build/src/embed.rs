//! Embedding a component manifest into its built Wasm artifact.
//!
//! On `spin build`, a standalone component manifest (`component.toml`) is
//! serialized to JSON and stored in a Wasm custom section of the built
//! component, so that the component's metadata travels with the artifact (for
//! example when it is published to a registry).

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use spin_manifest::schema::component::ComponentManifest;

/// The name of the Wasm custom section that holds the component manifest JSON.
pub const COMPONENT_MANIFEST_SECTION: &str = "spin-component-manifest";

/// The 8-byte Wasm preamble: the `\0asm` magic followed by a 4-byte version.
const WASM_PREAMBLE_LEN: usize = 8;
const WASM_MAGIC: &[u8] = b"\0asm";

/// Serializes `manifest` to JSON and writes it into a custom section named
/// [`COMPONENT_MANIFEST_SECTION`] in the Wasm binary at `wasm_path`.
///
/// Any existing top-level custom section with that name is replaced, so
/// repeated builds do not accumulate duplicate sections. Works for both core
/// Wasm modules and components (nested sections are left untouched).
pub fn embed_component_manifest(manifest: &ComponentManifest, wasm_path: &Path) -> Result<()> {
    let json = serde_json::to_vec(manifest)
        .context("failed to serialize the component manifest to JSON")?;

    let wasm = std::fs::read(wasm_path)
        .with_context(|| format!("failed to read Wasm file {}", wasm_path.display()))?;

    let updated = write_custom_section(&wasm, COMPONENT_MANIFEST_SECTION, &json)
        .with_context(|| format!("failed to embed the component manifest into {}", wasm_path.display()))?;

    std::fs::write(wasm_path, updated)
        .with_context(|| format!("failed to write Wasm file {}", wasm_path.display()))?;

    Ok(())
}

/// Returns a copy of `wasm` with a top-level custom section named `name`
/// containing `data`. Any existing top-level custom section with the same name
/// is removed first, so calling this repeatedly is idempotent.
fn write_custom_section(wasm: &[u8], name: &str, data: &[u8]) -> Result<Vec<u8>> {
    if !wasm.starts_with(WASM_MAGIC) {
        bail!("not a Wasm binary (missing \\0asm magic)");
    }

    let mut out = strip_top_level_custom_section(wasm, name)?;
    append_custom_section(&mut out, name, data);
    Ok(out)
}

/// Appends a well-formed custom section (`id 0`) carrying `name` and `data`.
fn append_custom_section(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    // Section body: the section name (a Wasm `name`: leb128 length + bytes)
    // immediately followed by the payload bytes.
    let mut body = Vec::with_capacity(5 + name.len() + data.len());
    write_u32_leb128(name.len() as u32, &mut body);
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(data);

    out.push(0x00); // custom section id
    write_u32_leb128(body.len() as u32, out);
    out.extend_from_slice(&body);
}

/// Walks the top-level section sequence of a Wasm binary (module or component)
/// and returns a copy with any top-level custom section named `name` removed.
/// The top level of both modules and components is a flat sequence of
/// `id, size, body` sections, so this does not need to understand the nested
/// structure of components.
fn strip_top_level_custom_section(wasm: &[u8], name: &str) -> Result<Vec<u8>> {
    if wasm.len() < WASM_PREAMBLE_LEN {
        bail!("Wasm binary is too short");
    }

    let mut out = Vec::with_capacity(wasm.len());
    out.extend_from_slice(&wasm[..WASM_PREAMBLE_LEN]);

    let mut i = WASM_PREAMBLE_LEN;
    while i < wasm.len() {
        let section_start = i;
        let id = wasm[i];
        i += 1;

        let (size, size_len) = read_u32_leb128(&wasm[i..]).context("malformed section size")?;
        i += size_len;

        let body_start = i;
        let body_end = body_start
            .checked_add(size as usize)
            .filter(|end| *end <= wasm.len())
            .context("section extends past end of Wasm binary")?;

        let is_target = id == 0 && custom_section_has_name(&wasm[body_start..body_end], name)?;
        if !is_target {
            out.extend_from_slice(&wasm[section_start..body_end]);
        }

        i = body_end;
    }

    Ok(out)
}

/// Whether the given custom section body begins with the name `name`.
fn custom_section_has_name(body: &[u8], name: &str) -> Result<bool> {
    let (name_len, name_len_bytes) =
        read_u32_leb128(body).context("malformed custom section name")?;
    let start = name_len_bytes;
    let end = start
        .checked_add(name_len as usize)
        .filter(|end| *end <= body.len())
        .context("custom section name extends past section")?;
    Ok(&body[start..end] == name.as_bytes())
}

fn write_u32_leb128(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Reads an unsigned LEB128 `u32` from the start of `bytes`, returning the value
/// and the number of bytes consumed.
fn read_u32_leb128(bytes: &[u8]) -> Result<(u32, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for (idx, &byte) in bytes.iter().enumerate() {
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            let value = u32::try_from(result).map_err(|_| anyhow!("LEB128 value overflows u32"))?;
            return Ok((value, idx + 1));
        }
        shift += 7;
        if shift >= 35 {
            bail!("LEB128 value is too long for a u32");
        }
    }
    bail!("unexpected end of LEB128 value")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_module() -> Vec<u8> {
        // `\0asm` magic + version 1.
        vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
    }

    /// Returns the payloads of every top-level custom section named `name`.
    fn custom_sections<'a>(wasm: &'a [u8], name: &str) -> Vec<&'a [u8]> {
        let mut found = Vec::new();
        let mut i = WASM_PREAMBLE_LEN;
        while i < wasm.len() {
            let id = wasm[i];
            i += 1;
            let (size, size_len) = read_u32_leb128(&wasm[i..]).unwrap();
            i += size_len;
            let body = &wasm[i..i + size as usize];
            if id == 0 {
                let (name_len, name_len_bytes) = read_u32_leb128(body).unwrap();
                let start = name_len_bytes;
                let end = start + name_len as usize;
                if &body[start..end] == name.as_bytes() {
                    found.push(&body[end..]);
                }
            }
            i += size as usize;
        }
        found
    }

    #[test]
    fn writes_a_custom_section() {
        let out = write_custom_section(&minimal_module(), COMPONENT_MANIFEST_SECTION, b"{\"a\":1}")
            .unwrap();
        assert_eq!(
            custom_sections(&out, COMPONENT_MANIFEST_SECTION),
            vec![&b"{\"a\":1}"[..]]
        );
    }

    #[test]
    fn replacing_is_idempotent() {
        let once =
            write_custom_section(&minimal_module(), COMPONENT_MANIFEST_SECTION, b"first").unwrap();
        let twice = write_custom_section(&once, COMPONENT_MANIFEST_SECTION, b"second").unwrap();

        // The old section is replaced, not duplicated.
        assert_eq!(
            custom_sections(&twice, COMPONENT_MANIFEST_SECTION),
            vec![&b"second"[..]]
        );
    }

    #[test]
    fn round_trips_a_component_manifest_as_json() {
        let manifest: ComponentManifest = toml::from_str(
            r#"
component_manifest_version = 1
[component]
name = "demo"
source = "demo.wasm"
version = "0.1.0"
[build]
command = "echo build"
"#,
        )
        .unwrap();

        let json = serde_json::to_vec(&manifest).unwrap();
        let out = write_custom_section(&minimal_module(), COMPONENT_MANIFEST_SECTION, &json).unwrap();

        let payload = custom_sections(&out, COMPONENT_MANIFEST_SECTION)[0];
        let decoded: ComponentManifest = serde_json::from_slice(payload).unwrap();
        assert_eq!(decoded.component.name, "demo");
        assert_eq!(decoded.component.source, "demo.wasm");
    }

    #[test]
    fn rejects_non_wasm() {
        assert!(write_custom_section(b"not wasm", COMPONENT_MANIFEST_SECTION, b"x").is_err());
    }
}
