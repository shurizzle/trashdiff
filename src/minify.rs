use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc_mangler::MangleOptions;
use oxc_minifier::{CompressOptions, Minifier, MinifierOptions};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Minify an inline JS snippet the same way `build.rs` minifies `admin.js`.
/// When the snippet does not parse, the original (trimmed) source is returned
/// as a fallback instead of failing.
pub fn minify_js(source: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path("custom.js").unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return source.trim().to_string();
    }
    let mut program = parsed.program;

    let options = MinifierOptions {
        mangle: Some(MangleOptions::default()),
        mangle_properties: None,
        compress: Some(CompressOptions::smallest()),
    };
    let ret = Minifier::new(options).minify(&allocator, &mut program);

    Codegen::new()
        .with_options(CodegenOptions {
            minify: true,
            comments: CommentOptions::disabled(),
            ..CodegenOptions::default()
        })
        .with_scoping(ret.scoping)
        .build(&program)
        .code
}
