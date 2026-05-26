// SPDX-License-Identifier: MIT

//! Build script: optionally regenerates Envoy ABI bindings via `bindgen`.
//!
//! By default, the crate uses pre-generated bindings checked into the
//! repository at `src/abi_generated.rs`. To regenerate from the
//! vendored `abi/abi.h`, build with `--features regenerate-abi`
//! (requires `libclang`).

fn main() {
    #[cfg(feature = "regenerate-abi")]
    regenerate();
}

#[cfg(feature = "regenerate-abi")]
fn regenerate() {
    println!("cargo:rerun-if-changed=abi/abi.h");

    let bindings = bindgen::Builder::default()
        .header("abi/abi.h")
        .allowlist_type("envoy_dynamic_module_.*")
        .allowlist_function("envoy_dynamic_module_.*")
        .allowlist_var("envoy_dynamic_module_.*")
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        .derive_eq(true)
        .derive_hash(true)
        .derive_partialeq(true)
        .generate()
        .expect("bindgen: failed to generate bindings from abi/abi.h");

    let out_path = std::path::Path::new("src/abi_generated.rs");
    bindings
        .write_to_file(out_path)
        .expect("bindgen: failed to write src/abi_generated.rs");
}
