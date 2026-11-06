//! uniffi's bindings generator, built at this crate's uniffi version; `apple/build-rust.sh` runs it to write the Swift
//! bindings from the built library.

fn main() {
    uniffi::uniffi_bindgen_main()
}
