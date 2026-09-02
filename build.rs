use lightningcss::{printer::PrinterOptions, stylesheet::{MinifyOptions, ParserOptions, StyleSheet}};
use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc_mangler::MangleOptions;
use oxc_minifier::{CompressOptions, Minifier, MinifierOptions};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::{env, path::Path};

fn minify_css(src: &Path) {
    let source = std::fs::read_to_string(src).expect("read css");
    let mut stylesheet = StyleSheet::parse(
        &source,
        ParserOptions {
            filename: src.to_string_lossy().into_owned(),
            css_modules: None,
            source_index: 0,
            error_recovery: false,
            warnings: None,
            flags: Default::default(),
        },
    )
    .expect("parse css");
    stylesheet.minify(MinifyOptions::default()).expect("minify css");
    let code = stylesheet
        .to_css(PrinterOptions {
            minify: true,
            project_root: None,
            targets: Default::default(),
            analyze_dependencies: None,
            pseudo_classes: None,
        })
        .expect("print css")
        .code;

    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR"))
        .join(src.file_name().expect("css filename"))
        .with_extension("min.css");
    std::fs::write(&out, code).expect("write min css");
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    for file in ["admin.js", "style.css", "dark.css"] {
        println!("cargo:rerun-if-changed={}", Path::new(&manifest).join("src").join(file).display());
    }

    let src = Path::new(&manifest).join("src").join("admin.js");
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

    minify_css(&Path::new(&manifest).join("src").join("style.css"));
    minify_css(&Path::new(&manifest).join("src").join("dark.css"));
}
