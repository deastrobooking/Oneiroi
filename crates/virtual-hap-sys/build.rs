fn main() {
    println!("cargo:rerun-if-changed=vendor/hap/hap.c");
    println!("cargo:rerun-if-changed=vendor/hap/hap.h");
    println!("cargo:rerun-if-changed=src/snappy-c.h");

    cc::Build::new()
        .file("vendor/hap/hap.c")
        .include("vendor/hap")
        .include("src")
        // Keep warnings enabled for our Rust wrapper, but do not make
        // upstream's signedness warnings appear on every workspace build.
        .warnings(false)
        .compile("hap");
}
