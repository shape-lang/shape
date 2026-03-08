use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::cli_args::ExecutionModeArg;
use crate::commands::ProviderOptions;

/// Wire protocol request types
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WireRequest {
    /// Execute Shape code and return the result
    #[serde(rename = "execute")]
    Execute { code: String },
    /// Validate Shape code (parse + type-check) without executing
    #[serde(rename = "validate")]
    Validate { code: String },
    /// Get version information
    #[serde(rename = "version")]
    Version,
}

/// Wire protocol response types
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WireResponse {
    #[serde(rename = "result")]
    Result {
        success: bool,
        output: Option<String>,
        error: Option<String>,
        diagnostics: Vec<Diagnostic>,
    },
    #[serde(rename = "version")]
    Version {
        shape_version: String,
        wire_protocol: u32,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct Diagnostic {
    severity: String, // "error", "warning", "info"
    message: String,
    line: Option<u32>,
    column: Option<u32>,
}

pub async fn run_wire_serve(
    address: String,
    _mode: ExecutionModeArg,
    _extensions: Vec<std::path::PathBuf>,
    _provider_opts: &ProviderOptions,
) -> Result<()> {
    let addr: SocketAddr = address.parse()?;
    let listener = TcpListener::bind(addr).await?;
    eprintln!("Shape wire-serve listening on {}", addr);

    loop {
        let (mut socket, peer) = listener.accept().await?;
        eprintln!("Connection from {}", peer);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(&mut socket).await {
                eprintln!("Connection error: {}", e);
            }
        });
    }
}

async fn handle_connection(socket: &mut tokio::net::TcpStream) -> Result<()> {
    loop {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        match socket.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let msg_len = u32::from_be_bytes(len_buf) as usize;

        // Read framed payload
        let mut payload = vec![0u8; msg_len];
        socket.read_exact(&mut payload).await?;

        // Decode framing (flags byte + optional zstd compression)
        let decompressed = shape_wire::transport::framing::decode_framed(&payload)
            .map_err(|e| anyhow::anyhow!("framing decode error: {}", e))?;

        // Decode request from MessagePack
        let request: WireRequest = shape_wire::decode_message(&decompressed)
            .map_err(|e| anyhow::anyhow!("request decode error: {}", e))?;

        // Handle request
        let response = handle_request(request).await;

        // Encode response as MessagePack
        let response_bytes = shape_wire::encode_message(&response)
            .map_err(|e| anyhow::anyhow!("response encode error: {}", e))?;

        // Apply framing (flags byte + optional zstd compression)
        let framed = shape_wire::transport::framing::encode_framed(&response_bytes);

        // Write length-prefixed framed response
        let len = framed.len() as u32;
        socket.write_all(&len.to_be_bytes()).await?;
        socket.write_all(&framed).await?;
        socket.flush().await?;
    }
}

async fn handle_request(request: WireRequest) -> WireResponse {
    match request {
        WireRequest::Execute { code } => match execute_shape_code(&code) {
            Ok(output) => WireResponse::Result {
                success: true,
                output: Some(output),
                error: None,
                diagnostics: vec![],
            },
            Err(e) => WireResponse::Result {
                success: false,
                output: None,
                error: Some(e.to_string()),
                diagnostics: vec![],
            },
        },
        WireRequest::Validate { code } => match validate_shape_code(&code) {
            Ok(diagnostics) => WireResponse::Result {
                success: diagnostics.iter().all(|d| d.severity != "error"),
                output: None,
                error: None,
                diagnostics,
            },
            Err(e) => WireResponse::Result {
                success: false,
                output: None,
                error: Some(e.to_string()),
                diagnostics: vec![],
            },
        },
        WireRequest::Version => WireResponse::Version {
            shape_version: env!("CARGO_PKG_VERSION").to_string(),
            wire_protocol: shape_wire::WIRE_PROTOCOL_V1,
        },
    }
}

fn execute_shape_code(code: &str) -> Result<String> {
    use std::io::Write;
    use std::process::Command;

    let mut temp = tempfile::NamedTempFile::new()?;
    write!(temp, "{}", code)?;
    let temp_path = temp.path().to_owned();

    let output = Command::new("shape").arg("run").arg(&temp_path).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(anyhow::anyhow!(
            "{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn validate_shape_code(code: &str) -> Result<Vec<Diagnostic>> {
    match shape_ast::parse_program(code) {
        Ok(_program) => Ok(vec![]),
        Err(e) => Ok(vec![Diagnostic {
            severity: "error".to_string(),
            message: e.to_string(),
            line: None,
            column: None,
        }]),
    }
}
