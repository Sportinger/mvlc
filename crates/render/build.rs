fn main() {
    if std::env::var("CARGO_FEATURE_LIBPLACEBO").is_ok() {
        println!("cargo:rerun-if-changed=src/libplacebo_stub/pl_stub.c");
        println!("cargo:rerun-if-changed=src/libplacebo_stub/pl_stub.h");
        cc::Build::new()
            .file("src/libplacebo_stub/pl_stub.c")
            .compile("libplacebo_stub");
    }
}
