fn main() {
    println!("cargo:rerun-if-changed=native/Updater.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/Updater.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("monoterm_updater");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
