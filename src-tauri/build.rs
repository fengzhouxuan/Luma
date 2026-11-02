fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Vendor the official MIT SDKs; no runtime DLL lookup or download.
        cc::Build::new()
            .file("vendor/everything-sdk2/src/Everything.c")
            .warnings(false)
            .compile("everything_sdk2");
        cc::Build::new()
            .file("vendor/everything-sdk3/src/Everything3.c")
            .define("EVERYTHING3_LIB", None)
            .warnings(false)
            .compile("everything_sdk3");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rerun-if-changed=vendor");
    }
    tauri_build::build()
}
