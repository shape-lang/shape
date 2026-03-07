//! Shape Language Server binary
//!
//! Runs the LSP server over stdin/stdout for editor integration.

use shape_lsp::ShapeLanguageServer;
use tower_lsp_server::{LspService, Server};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("shape-lsp {VERSION}");
        return;
    }

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr) // Log to stderr to not interfere with LSP protocol on stdout
        .init();

    tracing::info!("Starting Shape Language Server v{VERSION}");

    // Create the LSP service
    let (service, socket) = LspService::new(ShapeLanguageServer::new);

    // Run the server over stdin/stdout
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;

    tracing::info!("Shape Language Server stopped");
}
