#![allow(dead_code)]

use assert_cmd::Command;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub fn shape_binary() -> PathBuf {
    assert_cmd::cargo::cargo_bin!("shape").to_path_buf()
}

pub fn shape_cmd() -> Command {
    Command::new(shape_binary())
}

pub fn process_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn lock_process() -> MutexGuard<'static, ()> {
    process_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct IsolatedEnv {
    xdg_data: PathBuf,
    shape_config: PathBuf,
    _dir: TempDir,
}

impl IsolatedEnv {
    pub fn new(prefix: &str) -> Self {
        let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        let xdg_data = dir.path().join("xdg-data");
        let shape_config = dir.path().join("shape-config");
        std::fs::create_dir_all(&xdg_data).unwrap();
        std::fs::create_dir_all(&shape_config).unwrap();
        Self {
            xdg_data,
            shape_config,
            _dir: dir,
        }
    }

    pub fn apply_assert_cmd(&self, cmd: &mut Command) {
        cmd.env("XDG_DATA_HOME", &self.xdg_data)
            .env("SHAPE_CONFIG_DIR", &self.shape_config);
    }

    pub fn apply_std_cmd(&self, cmd: &mut StdCommand) {
        cmd.env("XDG_DATA_HOME", &self.xdg_data)
            .env("SHAPE_CONFIG_DIR", &self.shape_config);
    }

    pub fn snapshot_store(&self, name: &str) -> PathBuf {
        self._dir.path().join(name)
    }
}

pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn write_script(dir: &Path, name: &str, program: &str) -> PathBuf {
    let script = dir.join(name);
    let mut f = std::fs::File::create(&script).unwrap();
    f.write_all(program.as_bytes()).unwrap();
    script
}

pub fn run_shape_program(program: &str, mode: &str, env: &IsolatedEnv) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(dir.path(), "program.shape", program);
    let mut cmd = shape_cmd();
    cmd.args(["run", "--mode", mode])
        .arg(&script)
        .timeout(Duration::from_secs(120));
    env.apply_assert_cmd(&mut cmd);
    let output = cmd.output().unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn run_shape_program_with_snapshot_store(
    program: &str,
    mode: &str,
    env: &IsolatedEnv,
    snapshot_store: &Path,
) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(dir.path(), "program.shape", program);
    let mut cmd = shape_cmd();
    cmd.arg("--snapshot-store")
        .arg(snapshot_store)
        .args(["run", "--mode", mode])
        .arg(&script)
        .timeout(Duration::from_secs(120));
    env.apply_assert_cmd(&mut cmd);
    let output = cmd.output().unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn assert_success(run: &Run, context: &str) {
    assert_eq!(
        run.code,
        Some(0),
        "{context} should exit 0; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

pub fn combined(run: &Run) -> String {
    format!("{}{}", run.stdout, run.stderr)
}

pub fn marker_value(output: &str, marker: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once(marker)?;
        Some(value.trim().to_string())
    })
}

pub fn assert_hexish(value: &str, context: &str) {
    assert!(
        value.len() >= 16 && value.chars().all(|c| c.is_ascii_hexdigit()),
        "{context} should be a snapshot/content hash, got {value:?}"
    );
}

pub struct ServeNode {
    child: std::process::Child,
    pub addr: String,
    tls_addr: Option<String>,
    stderr_path: PathBuf,
    _cfg: TempDir,
    _extdir: Option<TempDir>,
}

impl ServeNode {
    pub fn tls_addr(&self) -> &str {
        self.tls_addr
            .as_deref()
            .expect("serve node was not started with TLS")
    }
}

impl Drop for ServeNode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn start_serve(
    sandbox: &str,
    extension_so: Option<&Path>,
    ffi_languages: &[&str],
) -> ServeNode {
    start_serve_inner(sandbox, extension_so, ffi_languages, None, false, 4)
}

pub fn start_serve_with_max_concurrent(sandbox: &str, max_concurrent: usize) -> ServeNode {
    start_serve_inner(sandbox, None, &[], None, false, max_concurrent)
}

pub fn start_tls_serve_with_max_concurrent(sandbox: &str, max_concurrent: usize) -> ServeNode {
    start_serve_inner(sandbox, None, &[], None, true, max_concurrent)
}

pub fn start_tls_serve(
    sandbox: &str,
    extension_so: Option<&Path>,
    ffi_languages: &[&str],
) -> ServeNode {
    start_serve_inner(sandbox, extension_so, ffi_languages, None, true, 4)
}

pub fn start_tls_serve_with_snapshot_store(
    sandbox: &str,
    extension_so: Option<&Path>,
    ffi_languages: &[&str],
    snapshot_store: &Path,
) -> ServeNode {
    start_serve_inner(
        sandbox,
        extension_so,
        ffi_languages,
        Some(snapshot_store),
        true,
        4,
    )
}

pub fn start_serve_with_snapshot_store(
    sandbox: &str,
    extension_so: Option<&Path>,
    ffi_languages: &[&str],
    snapshot_store: &Path,
) -> ServeNode {
    start_serve_inner(
        sandbox,
        extension_so,
        ffi_languages,
        Some(snapshot_store),
        false,
        4,
    )
}

fn start_serve_inner(
    sandbox: &str,
    extension_so: Option<&Path>,
    ffi_languages: &[&str],
    snapshot_store: Option<&Path>,
    tls: bool,
    max_concurrent: usize,
) -> ServeNode {
    let cfg = tempfile::tempdir().unwrap();
    let extdir = extension_so.map(|so| {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(so.file_name().unwrap());
        #[cfg(unix)]
        std::os::unix::fs::symlink(so, &dest).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(so, &dest).unwrap();
        dir
    });
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let addr = format!("127.0.0.1:{port}");
    let stderr_path = cfg.path().join("serve.stderr");
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let tls_args = if tls {
        let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_path = cfg.path().join("cert.pem");
        let key_path = cfg.path().join("key.pem");
        std::fs::write(&cert_path, generated.cert.pem()).unwrap();
        std::fs::write(&key_path, generated.key_pair.serialize_pem()).unwrap();
        let tls_addr = format!(
            "shape+tls://{}?ca={}&server_name=localhost",
            addr,
            cert_path.display()
        );
        Some((cert_path, key_path, tls_addr))
    } else {
        None
    };

    let mut cmd = StdCommand::new(shape_binary());
    let max_concurrent_arg = max_concurrent.to_string();
    if let Some(store) = snapshot_store {
        cmd.arg("--snapshot-store").arg(store);
    }
    cmd.args(["serve", "--address", &addr, "--sandbox", sandbox])
        .args(["--max-concurrent", &max_concurrent_arg])
        .env("SHAPE_CONFIG_DIR", cfg.path())
        .env("XDG_DATA_HOME", cfg.path().join("xdg-data"))
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    if let Some((cert_path, key_path, _)) = tls_args.as_ref() {
        cmd.arg("--tls-cert")
            .arg(cert_path)
            .arg("--tls-key")
            .arg(key_path);
    }
    if let Some(dir) = extdir.as_ref() {
        cmd.arg("--extension-dir").arg(dir.path());
    }
    if !ffi_languages.is_empty() {
        cmd.arg("--ffi-languages").arg(ffi_languages.join(","));
    }
    let child = cmd.spawn().unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if TcpStream::connect(&addr).is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "`shape serve` did not become ready at {addr}; stderr={}",
            std::fs::read_to_string(&stderr_path).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    ServeNode {
        child,
        addr,
        tls_addr: tls_args.map(|(_, _, tls_addr)| tls_addr),
        stderr_path,
        _cfg: cfg,
        _extdir: extdir,
    }
}

const REQUIRE_FFI_EXT_ENV: &str = "SHAPE_REQUIRE_FFI_EXT";

pub fn language_ext_so(language: &str) -> Option<PathBuf> {
    let file = language_ext_so_file(language);
    language_ext_search_dirs()
        .into_iter()
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
}

pub fn require_language_ext_so(language: &str) -> Option<PathBuf> {
    if let Some(so) = language_ext_so(language) {
        return Some(so);
    }

    if ffi_extensions_required() {
        let file = language_ext_so_file(language);
        let searched = language_ext_search_dirs()
            .into_iter()
            .map(|dir| format!("  {}", dir.join(&file).display()))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "{REQUIRE_FFI_EXT_ENV}=1 requires {file} for distributed {language} composition \
             tests, but it was not found.\n\
             Build the {language} FFI extension or set SHAPE_FFI_EXT_DIR to a directory \
             containing {file}.\n\
             Searched:\n{searched}"
        );
    }

    None
}

fn language_ext_so_file(language: &str) -> String {
    format!("libshape_ext_{language}.so")
}

fn language_ext_search_dirs() -> Vec<PathBuf> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("SHAPE_FFI_EXT_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    dirs.push(workspace.join("extensions"));
    dirs.push(workspace.join("target").join("debug"));
    dirs.push(workspace.join("target").join("release"));
    dirs
}

fn ffi_extensions_required() -> bool {
    let Some(value) = std::env::var_os(REQUIRE_FFI_EXT_ENV) else {
        return false;
    };
    let value = value.to_string_lossy();
    let value = value.trim();
    !(value.is_empty()
        || value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no"))
}

pub fn assert_serve_logged_foreign_stub(node: &ServeNode, language: &str) {
    let stderr = std::fs::read_to_string(&node.stderr_path).unwrap_or_default();
    let landed = stderr.lines().any(|line| {
        let Some((_, rest)) = line.split_once("blobs=") else {
            return false;
        };
        let blob_count = rest
            .split_whitespace()
            .next()
            .and_then(|count| count.parse::<usize>().ok());
        blob_count.is_some_and(|count| count >= 2) && line.contains("foreign_entries=1")
    });
    assert!(
        landed,
        "{language} remote transfer must land on the serve node with the foreign stub; stderr:\n{stderr}"
    );
}

pub fn serve_stderr(node: &ServeNode) -> String {
    std::fs::read_to_string(&node.stderr_path).unwrap_or_default()
}

pub struct WireClient {
    stream: TcpStream,
}

impl WireClient {
    pub fn connect(addr: &str) -> Self {
        let stream = TcpStream::connect(addr)
            .unwrap_or_else(|e| panic!("failed to connect to shape serve at {addr}: {e}"));
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        Self { stream }
    }

    pub fn roundtrip(
        &mut self,
        msg: &shape_vm::remote::WireMessage,
    ) -> shape_vm::remote::WireMessage {
        let payload = shape_wire::encode_message(msg).expect("encode wire message");
        let framed = shape_wire::transport::framing::encode_framed(&payload);
        self.stream
            .write_all(&(framed.len() as u32).to_be_bytes())
            .expect("write wire frame length");
        self.stream
            .write_all(&framed)
            .expect("write wire frame payload");
        self.stream.flush().expect("flush wire frame");

        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .expect("read response frame length");
        let len = u32::from_be_bytes(len_buf) as usize;
        assert!(
            len <= 256 * 1024 * 1024,
            "response frame too large: {len} bytes"
        );
        let mut buf = vec![0u8; len];
        self.stream
            .read_exact(&mut buf)
            .expect("read response frame payload");
        let response =
            shape_wire::transport::framing::decode_framed(&buf).expect("decode response frame");
        shape_wire::decode_message(&response).expect("decode wire response")
    }
}

pub fn assert_tls_remote_call_user_surface() {
    let server = start_tls_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-tls-user-e2e-");

    let program = r#"use std::core::remote

fn mul(a: int, b: int) -> int {
    a * b
}

match remote::call("__TLS_ADDR__", mul, 6, 7) {
    Ok(value) => print(f"TLS_OK={value}")
    Err(e) => print(f"TLS_ERR={e}")
}

match remote::call("__PLAIN_ADDR__", mul, 6, 7) {
    Ok(value) => print(f"PLAIN_OK={value}")
    Err(e) => print(f"PLAIN_ERR={e}")
}
"#
    .replace("__TLS_ADDR__", server.tls_addr())
    .replace("__PLAIN_ADDR__", &server.addr);

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call TLS client");
    assert!(
        run.stdout.contains("TLS_OK=42"),
        "trusted TLS remote::call should succeed; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("PLAIN_ERR=") && !run.stdout.contains("PLAIN_OK="),
        "plaintext remote::call must fail against a TLS serve node; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}
