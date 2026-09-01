use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc_mangler::MangleOptions;
use oxc_minifier::{CompressOptions, Minifier, MinifierOptions};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::env;
use std::path::Path;

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let src = Path::new(&manifest).join("src").join("admin.js");
    println!("cargo:rerun-if-changed={}", src.display());

    let source = std::fs::read_to_string(&src).expect("read admin.js");
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&src).unwrap_or_default();
    let parsed = Parser::new(&allocator, &source, source_type).parse();
    assert!(
        parsed.diagnostics.is_empty(),
        "admin.js parse errors: {:#?}",
        parsed.diagnostics
    );
    let mut program = parsed.program;

    let options = MinifierOptions {
        mangle: Some(MangleOptions::default()),
        mangle_properties: None,
        compress: Some(CompressOptions::smallest()),
    };
    let ret = Minifier::new(options).minify(&allocator, &mut program);

    let code = Codegen::new()
        .with_options(CodegenOptions {
            minify: true,
            comments: CommentOptions::disabled(),
            ..CodegenOptions::default()
        })
        .with_scoping(ret.scoping)
        .build(&program)
        .code;

    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("admin.min.js");
    std::fs::write(&out, code).expect("write admin.min.js");
}
