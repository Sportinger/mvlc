fn main() {
    write_placeholder_shader("ui.vert.spv");
    write_placeholder_shader("ui.frag.spv");

    if std::env::var("CARGO_FEATURE_LIBPLACEBO").is_ok() {
        println!("cargo:rerun-if-changed=src/libplacebo_stub/pl_stub.c");
        println!("cargo:rerun-if-changed=src/libplacebo_stub/pl_stub.h");
        cc::Build::new()
            .file("src/libplacebo_stub/pl_stub.c")
            .compile("libplacebo_stub");
    }
}

fn write_placeholder_shader(output: &str) {
    // ponytail: native Vulkan UI shaders are absent in this branch; default WGPU UI does not use them.
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = std::path::Path::new(&out_dir).join(output);
    std::fs::write(&out_path, [])
        .unwrap_or_else(|err| panic!("Failed to write SPIR-V {}: {err}", out_path.display()));
}
