use heck::*;
use proc_macro::TokenStream;
use std::{env, path::PathBuf};

/// This macro generates the `#[test]` functions for the runtime tests.
#[proc_macro]
pub fn codegen_runtime_tests(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input);
    let ignores = ignores(input);
    let mut tests = Vec::new();
    let tests_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/runtime-tests/tests");
    let tests_path_string = tests_path
        .to_str()
        .expect("CARGO_MANIFEST_DIR is not valid utf8")
        .to_owned();
    for entry in std::fs::read_dir(tests_path).expect("failed to read tests directory") {
        let entry = entry.expect("error reading test directory entry");
        let test = entry.path();

        if entry.file_type().unwrap().is_dir() {
            let requires_services = entry.path().join("services").exists();

            let name = test.file_stem().unwrap().to_str().unwrap();
            let ignore = if ignores.contains(&name.to_string()) {
                quote::quote!(#[ignore])
            } else {
                quote::quote!()
            };
            let ident = quote::format_ident!("{}", name.to_snake_case());
            let feature_attribute = if requires_services {
                quote::quote!(#[cfg(feature = "extern-dependencies-tests")])
            } else {
                quote::quote!()
            };
            // Generate the following code:
            // ```rust
            // #[test]
            // fn outbound_mysql() {
            //     run("outbound-mysql")
            // }
            // ```
            tests.push(quote::quote! {
                #[test]
                #ignore
                #feature_attribute
                fn #ident() {
                    run(::std::path::PathBuf::from(#tests_path_string).join(#name))
                }
            });
        }
    }
    (quote::quote!(#(#tests)*)).into()
}

fn ignores(input: syn::FieldValue) -> Vec<String> {
    let syn::Member::Named(n) = input.member else {
        panic!("codegen_runtime_tests!() requires a named field");
    };
    if n != "ignore" {
        panic!("codegen_runtime_tests!() only supports the `ignore` field");
    }
    let syn::Expr::Array(a) = input.expr else {
        panic!("codegen_runtime_tests!() requires an array of strings");
    };
    a.elems
        .iter()
        .map(|e| {
            if let syn::Expr::Lit(l) = e
                && let syn::Lit::Str(s) = &l.lit
            {
                return s.value();
            }
            panic!("codegen_runtime_tests!() requires an array of strings");
        })
        .collect()
}
