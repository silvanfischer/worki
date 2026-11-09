fn main() {
    // The icons are embedded at compile time, but tauri-build doesn't watch them.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
