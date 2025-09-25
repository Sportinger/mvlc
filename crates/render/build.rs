use std::path::Path;

fn main() {
    compile_shader("shaders/ui.vert", "ui.vert.spv");
    compile_shader("shaders/ui.frag", "ui.frag.spv");

    if std::env::var("CARGO_FEATURE_LIBPLACEBO").is_ok() {
        println!("cargo:rerun-if-changed=src/libplacebo_stub/pl_stub.c");
        println!("cargo:rerun-if-changed=src/libplacebo_stub/pl_stub.h");
        cc::Build::new()
            .file("src/libplacebo_stub/pl_stub.c")
            .compile("libplacebo_stub");
    }
}

fn compile_shader(path: &str, output: &str) {
    println!("cargo:rerun-if-changed={path}");

    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("Failed to read shader {path}: {err}"));

    let kind = match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "vert" => shaderc::ShaderKind::Vertex,
        "frag" => shaderc::ShaderKind::Fragment,
        other => panic!("Unsupported shader extension: {other}"),
    };

    let compiler = shaderc::Compiler::new().expect("Failed to create shader compiler");
    let artifact = compiler
        .compile_into_spirv(&source, kind, path, "main", None)
        .unwrap_or_else(|err| panic!("Shader compilation failed for {path}: {err}"));

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = Path::new(&out_dir).join(output);
    std::fs::write(&out_path, artifact.as_binary_u8())
        .unwrap_or_else(|err| panic!("Failed to write SPIR-V {}: {err}", out_path.display()));
}
