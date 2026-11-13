//! EraBasic Language Server - Main entry point.

mod analysis;
mod atomic64;
mod docs;
mod document;
mod fxhash;
mod lsp_ast;
mod lsp_parser;
mod parser;
mod providers;
mod server;
mod util;
mod workspace;

use crate::server::EraBasicLanguageServer;
use tokio::io::{stdin, stdout};
use tower_lsp::{LspService, Server};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Starting EraBasic Language Server");

    // Create the LSP service
    let (service, socket) = LspService::new(EraBasicLanguageServer::new);

    // Run the server over stdio using tokio
    Server::new(stdin(), stdout(), socket).serve(service).await;

    tracing::info!("EraBasic Language Server stopped");
}

#[cfg(test)]
mod test_lsp_parser {
    use crate::lsp_parser::parse_file;
    use crate::lsp_ast::SourceFile;

    #[test]
    fn test_parser_basic() {
        let source = SourceFile {
            path: "test.erb".to_string(),
            text: r#"@TEST
#DIM A
IF A >= "B"
ENDIF
ENDFUNC
"#.to_string(),
        };

        let program = parse_file(source);
        println!("Files: {:?}", program.files);
        println!("Functions: {:?}", program.functions.keys().collect::<Vec<_>>());
        println!("Global dims: {:?}", program.global_dims.iter().map(|d| &d.name).collect::<Vec<_>>());
        println!("Errors: {:?}", program.errors);
        println!("Warnings: {:?}", program.warnings);

        for (name, funcs) in &program.functions {
            for func in funcs {
                println!("Function: {} (line {})", name, func.line);
                println!("  Params: {:?}", func.params.iter().map(|p| &p.target.name).collect::<Vec<_>>());
                println!("  Body code count: {}", func.body.code.len());
                for (i, instr) in func.body.code.iter().enumerate() {
                    println!("  [{}] {:?}", i, instr);
                }
                println!("  Lines: {:?}", func.body.lines);
            }
        }

        assert!(!program.functions.is_empty());
    }
}
