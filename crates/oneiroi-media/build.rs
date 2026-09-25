fn main() {
    println!("cargo:rerun-if-changed=src/capture_macos.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/capture_macos.m")
            .flag("-fobjc-arc")
            .compile("oneiroi_capture_discovery");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
