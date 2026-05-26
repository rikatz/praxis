// SPDX-License-Identifier: MIT

//! Build script for the Praxis server binary.
//!
//! When the `dynamic-modules` feature is enabled on Unix, passes
//! `--export-dynamic` to the linker so that `#[no_mangle]` callback
//! symbols from the dynamic modules crate are visible to `.so` modules
//! loaded via `dlopen`.

fn main() {
    #[cfg(all(feature = "dynamic-modules", target_family = "unix"))]
    println!("cargo:rustc-link-arg=-Wl,--export-dynamic");
}
