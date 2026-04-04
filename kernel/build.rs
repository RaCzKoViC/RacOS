fn main() {
    println!("cargo:rerun-if-changed=linker.ld");

    // Use our custom linker script
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let linker_script = format!("{}/linker.ld", manifest_dir);
    println!("cargo:rustc-link-arg=-T");
    println!("cargo:rustc-link-arg={}", linker_script);
}
