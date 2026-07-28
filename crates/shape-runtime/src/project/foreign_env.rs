//! Declared, locked, content-addressed foreign environments (ADR-019 §4 / #198).
//!
//! A `fn python` or `fn typescript` body runs against an environment: an
//! interpreter, a set of installed packages, and — once POLY-FOREIGN-CHECK
//! lands — a checker that decides whether the body is well typed. Before this
//! module, none of that was declared: the Python extension sniffed the host for
//! a virtualenv and used whatever it found, so the same source could mean
//! different things on two machines and the compiler's content hash claimed a
//! determinism the environment did not provide.
//!
//! Everything here derives from **declared, locked inputs** — the
//! `[foreign.<language>]` table in `shape.toml` and the lockfile it names. No
//! function in this module inspects the host to decide what the environment
//! *is*. Host inspection has exactly one legal role, and it lives behind
//! [`ForeignEnvironmentDigest::check_provided`]: asking whether the host can
//! provide the environment that was declared. A host that cannot provide it
//! fails pre-entry with `[C0936]`; it never produces a different digest.
//!
//! # Canonicalization
//!
//! Both digests are domain-separated, length-framed SHA-256 over a canonical
//! pre-image, and every map in a pre-image is a `BTreeMap`, so the pre-image is
//! a function of the *content* and not of the order the author happened to
//! write the entries in. Reformatting a lockfile — reordering its tables,
//! changing its whitespace — does not move a digest; changing a version does.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Version of the environment-digest pre-image scheme.
///
/// Part of every pre-image: bumping it moves every digest, which is the point —
/// a scheme change must not leave old digests looking valid.
pub const FOREIGN_ENVIRONMENT_SCHEME_VERSION: u16 = 1;

/// The only lockfile document version this build understands.
///
/// A lockfile from the future is refused rather than read with the fields this
/// build happens to recognise: a half-understood lock is a silently different
/// environment.
pub const FOREIGN_LOCKFILE_VERSION: u32 = 1;

const DOMAIN_ENVIRONMENT: &str = "shape.foreign.environment";
const DOMAIN_LOCKFILE: &str = "shape.foreign.lockfile";

// ============================================================================
// Manifest surface — `[foreign.<language>]`
// ============================================================================

/// One language's declared environment, from `[foreign.<language>]`.
///
/// ```toml
/// [foreign.python]
/// runtime = "cpython"
/// version = "3.11.7"
/// root = ".venv"
/// lockfile = "python.lock"
///
/// [foreign.python.checker]
/// name = "pyright"
/// version = "1.1.350"
/// settings = { strict = true, pythonVersion = "3.11" }
/// ```
///
/// `runtime` and `version` are the interpreter identity the artifact is built
/// against. They are declared rather than detected for the reason the whole
/// module exists: a detected value is a property of the machine that compiled,
/// and content addressing needs a property of the source.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ForeignEnvironmentSection {
    /// Interpreter/runtime identity, e.g. `"cpython"`, `"deno"`.
    pub runtime: String,
    /// Interpreter/runtime version, e.g. `"3.11.7"`.
    pub version: String,
    /// Lockfile path relative to the project root. Defaults to
    /// `<language>.lock`.
    #[serde(default)]
    pub lockfile: Option<String>,
    /// The environment root the host must provide, relative to the project root
    /// — a virtualenv directory for Python, a vendored-module directory for
    /// TypeScript.
    ///
    /// Declared, so the extension is told where the environment is instead of
    /// looking for one. Absent means the base interpreter with no added search
    /// path, which is a real declaration and not a fallback.
    #[serde(default)]
    pub root: Option<String>,
    /// The pinned foreign checker (ADR-019 §1). Absent until
    /// POLY-FOREIGN-CHECK declares one; present, it is part of the digest, so
    /// a checker upgrade is a reviewed manifest change and never an ambient
    /// build break.
    #[serde(default)]
    pub checker: Option<ForeignCheckerPin>,
}

/// The pinned foreign type checker for one language (ADR-019 §1).
///
/// Identity, version, and a settings digest. ADR-019 is explicit that tracking
/// alone is not enough: identical source and lockfile must produce the same
/// verdict on every host, and a checker's strictness setting changes the
/// verdict as surely as its version does.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ForeignCheckerPin {
    /// Checker identity, e.g. `"pyright"`, `"tsc"`.
    pub name: String,
    /// Checker version, e.g. `"1.1.350"`.
    pub version: String,
    /// Checker settings that change its verdict — strictness level, language
    /// version target. Free-form because each checker's settings are its own;
    /// canonicalized by type-tagged rendering so `true` and `"true"` are
    /// different settings.
    #[serde(default)]
    pub settings: BTreeMap<String, toml::Value>,
}

// ============================================================================
// Lockfile
// ============================================================================

/// A per-language lockfile: the resolved set, written down.
///
/// TOML, like `shape.toml`, and canonical on the way out (see
/// [`ForeignLockfile::to_canonical_toml`]) so a regenerated lock that resolved
/// to the same thing is byte-identical.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ForeignLockfile {
    /// [`FOREIGN_LOCKFILE_VERSION`] this document was written against.
    pub version: u32,
    /// The language this lock resolves for. Checked against the manifest
    /// section that named it, so a mis-pointed `lockfile = ` is an error and
    /// not a silently wrong environment.
    pub language: String,
    /// Resolved packages, keyed by package name (Python).
    #[serde(default)]
    pub packages: BTreeMap<String, LockedPackage>,
    /// Resolved modules, keyed by the import specifier a foreign body writes
    /// (TypeScript).
    #[serde(default)]
    pub modules: BTreeMap<String, LockedModule>,
}

/// One resolved package.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub version: String,
    /// Content hash of the distribution, e.g. `"sha256-..."`.
    #[serde(default)]
    pub integrity: Option<String>,
    /// Where it came from, e.g. an index URL.
    #[serde(default)]
    pub source: Option<String>,
}

/// One resolved module: a specifier bound to a vendored file.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedModule {
    /// Path to the vendored source, relative to the environment root.
    pub path: String,
    /// Content hash of the vendored source, e.g. `"sha256-<hex>"`. Verified at
    /// load when present.
    #[serde(default)]
    pub integrity: Option<String>,
}

impl ForeignLockfile {
    /// Parse a lockfile document.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Serialize canonically: sorted keys, stable shape.
    ///
    /// The determinism rule for this program (#205) applies to lockfile
    /// serializations as much as to digests — two runs that resolved the same
    /// set must write the same bytes, or every regeneration is a spurious diff.
    /// `BTreeMap` gives the ordering; `toml::to_string_pretty` gives the shape.
    pub fn to_canonical_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// The lock hash: a canonical digest of the resolved content.
    ///
    /// Over the *parsed* document, not the file bytes. A lockfile that was
    /// reformatted or had its tables reordered resolves to the same
    /// environment, and a digest that moved for it would make every
    /// content-addressed artifact hostage to a formatter.
    pub fn lock_hash(&self) -> [u8; 32] {
        let mut w = DigestWriter::new(DOMAIN_LOCKFILE);
        w.u32(self.version);
        w.str(&self.language);
        w.u32(self.packages.len() as u32);
        for (name, pkg) in &self.packages {
            w.str(name);
            w.str(&pkg.version);
            w.opt_str(pkg.integrity.as_deref());
            w.opt_str(pkg.source.as_deref());
        }
        w.u32(self.modules.len() as u32);
        for (specifier, module) in &self.modules {
            w.str(specifier);
            w.str(&module.path);
            w.opt_str(module.integrity.as_deref());
        }
        w.finish()
    }
}

// ============================================================================
// The digest
// ============================================================================

/// The content-addressed identity of one language's declared environment
/// (ADR-019 §4).
///
/// Carries both the digest and the facts it was derived from: the digest is
/// what joins a content hash, and the facts are what a mismatch diagnostic has
/// to be able to say. A digest a user cannot explain is a digest they cannot
/// fix.
///
/// # Tracked build input
///
/// ADR-013 §4 requires an external input to record provider identity, the
/// normalized request, and a public content digest.
/// [`Self::tracked_input_identity`] and [`Self::public_digest`] are those two
/// halves. The `ComptimeHost` that consumes them is POLY-FOREIGN-CHECK's
/// (#138 → #197) — this type is shaped to plug into it, and nothing here
/// depends on it existing.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignEnvironmentDigest {
    language: String,
    runtime_id: String,
    runtime_version: String,
    root: Option<String>,
    lock_hash: [u8; 32],
    checker: Option<ForeignCheckerPin>,
    lockfile: ForeignLockfile,
    digest: [u8; 32],
}

impl ForeignEnvironmentDigest {
    /// Derive the digest from a declared section and its parsed lockfile.
    ///
    /// Pure: same inputs, same digest, on every host. Nothing is read from the
    /// filesystem or the environment here.
    pub fn derive(
        language: &str,
        section: &ForeignEnvironmentSection,
        lockfile: ForeignLockfile,
    ) -> Self {
        let lock_hash = lockfile.lock_hash();
        let mut w = DigestWriter::new(DOMAIN_ENVIRONMENT);
        w.str(language);
        w.str(&section.runtime);
        w.str(&section.version);
        // The resolved set enters through `lock_hash` rather than being
        // written again beside it. One authority for one fact: a pre-image that
        // carried both would leave "which one decides?" open the first time
        // they could disagree.
        w.bytes(&lock_hash);
        match &section.checker {
            None => w.u8(0),
            Some(pin) => {
                w.u8(1);
                w.str(&pin.name);
                w.str(&pin.version);
                w.u32(pin.settings.len() as u32);
                for (key, value) in &pin.settings {
                    w.str(key);
                    w.toml_value(value);
                }
            }
        }
        let digest = w.finish();
        Self {
            language: language.to_string(),
            runtime_id: section.runtime.clone(),
            runtime_version: section.version.clone(),
            root: section.root.clone(),
            lock_hash,
            checker: section.checker.clone(),
            lockfile,
            digest,
        }
    }

    /// The 32-byte digest — the value that joins a content hash and a portable
    /// artifact's foreign-dependency manifest.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// Public content digest, in the ADR-013 §4 sense.
    pub fn public_digest(&self) -> [u8; 32] {
        self.digest
    }

    /// Tracked-input identity, in the ADR-013 §4 sense: stable, sortable, and
    /// carrying no secret.
    pub fn tracked_input_identity(&self) -> String {
        format!("{DOMAIN_ENVIRONMENT}/{}", self.language)
    }

    /// Short rendering for diagnostics. Presentation only.
    pub fn short_hex(&self) -> String {
        hex::encode(&self.digest[..8])
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.digest)
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn runtime_version(&self) -> &str {
        &self.runtime_version
    }

    pub fn lock_hash(&self) -> [u8; 32] {
        self.lock_hash
    }

    pub fn checker(&self) -> Option<&ForeignCheckerPin> {
        self.checker.as_ref()
    }

    pub fn lockfile(&self) -> &ForeignLockfile {
        &self.lockfile
    }

    /// The declared environment root, relative to the project root.
    pub fn declared_root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    /// Ask whether this host can provide the declared environment.
    ///
    /// The one place host inspection is legal, and it decides nothing about
    /// *what* the environment is — only whether the declared one is here. A
    /// host that cannot provide it fails; it never silently becomes a different
    /// environment. That asymmetry is the whole point of ADR-019 §4.
    ///
    /// Returns the resolved search paths the extension should add, in declared
    /// order.
    pub fn check_provided(
        &self,
        project_root: &Path,
    ) -> Result<Vec<PathBuf>, ForeignEnvironmentError> {
        let Some(root) = &self.root else {
            return Ok(Vec::new());
        };
        let root_path = project_root.join(root);
        if !root_path.is_dir() {
            return Err(ForeignEnvironmentError::RootMissing {
                language: self.language.clone(),
                path: root_path,
            });
        }
        let mut paths = Vec::new();
        for relative in self.declared_search_paths() {
            let candidate = root_path.join(&relative);
            if !candidate.is_dir() {
                return Err(ForeignEnvironmentError::SearchPathMissing {
                    language: self.language.clone(),
                    path: candidate,
                });
            }
            paths.push(candidate);
        }
        if paths.is_empty() {
            paths.push(root_path);
        }
        Ok(paths)
    }

    /// Search paths inside the declared root, derived from the DECLARED runtime
    /// version.
    ///
    /// Constructed, not discovered: `lib/python3.11/site-packages` follows from
    /// `runtime = "cpython"` and `version = "3.11.7"`. That is why deleting the
    /// sniffer costs nothing — the layout was always derivable from facts the
    /// manifest now states, and the sniffer's job was to guess them.
    fn declared_search_paths(&self) -> Vec<String> {
        match self.runtime_id.as_str() {
            "cpython" => match major_minor(&self.runtime_version) {
                Some(mm) => vec![format!("lib/python{mm}/site-packages")],
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

/// `"3.11.7"` → `"3.11"`. `None` when the version is not `major.minor[...]`.
fn major_minor(version: &str) -> Option<String> {
    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major.is_empty() || minor.is_empty() {
        return None;
    }
    if !major.bytes().all(|b| b.is_ascii_digit()) || !minor.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

// ============================================================================
// Failures
// ============================================================================

/// The structured pre-entry failure class for foreign environments
/// (`[C0936]`).
///
/// ADR-019 §4: a declared environment that cannot be provided is a failure
/// *before* any foreign code runs. Every variant here is refusal — there is no
/// variant that means "carried on with something else", because that variant is
/// exactly what this ticket deleted.
#[derive(Debug, Clone, PartialEq)]
pub enum ForeignEnvironmentError {
    /// A `fn <language>` body exists but no `[foreign.<language>]` declares its
    /// environment.
    Undeclared { language: String },
    /// The declared lockfile is not on disk.
    LockfileMissing { language: String, path: PathBuf },
    /// The lockfile is unreadable or malformed.
    LockfileUnreadable {
        language: String,
        path: PathBuf,
        detail: String,
    },
    /// The lockfile was written against a document version this build does not
    /// understand.
    LockfileVersionUnsupported {
        language: String,
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    /// `[foreign.python]` pointed at a lockfile that says it locks something
    /// else.
    LockfileLanguageMismatch {
        language: String,
        path: PathBuf,
        found: String,
    },
    /// The declared environment root is not on this host.
    RootMissing { language: String, path: PathBuf },
    /// The root is here but the search path the declared runtime version
    /// implies is not.
    SearchPathMissing { language: String, path: PathBuf },
}

impl ForeignEnvironmentError {
    pub fn code(&self) -> &'static str {
        "C0936"
    }

    /// The language whose environment failed.
    pub fn language(&self) -> &str {
        match self {
            Self::Undeclared { language }
            | Self::LockfileMissing { language, .. }
            | Self::LockfileUnreadable { language, .. }
            | Self::LockfileVersionUnsupported { language, .. }
            | Self::LockfileLanguageMismatch { language, .. }
            | Self::RootMissing { language, .. }
            | Self::SearchPathMissing { language, .. } => language,
        }
    }
}

impl std::fmt::Display for ForeignEnvironmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[C0936] ")?;
        match self {
            Self::Undeclared { language } => write!(
                f,
                "no environment is declared for `{language}`. A `fn {language}` body \
                 runs against an interpreter and a package set, and both must be \
                 written down: add a `[foreign.{language}]` table to shape.toml with \
                 `runtime`, `version`, and a lockfile."
            ),
            Self::LockfileMissing { language, path } => write!(
                f,
                "the `{language}` environment declares the lockfile `{}`, which is not \
                 on this host. The resolved package set is what makes the environment \
                 reproducible; without it there is nothing to run against.",
                path.display()
            ),
            Self::LockfileUnreadable {
                language,
                path,
                detail,
            } => write!(
                f,
                "the `{language}` lockfile `{}` could not be read: {detail}",
                path.display()
            ),
            Self::LockfileVersionUnsupported {
                language,
                path,
                found,
                supported,
            } => write!(
                f,
                "the `{language}` lockfile `{}` is version {found}; this build \
                 understands version {supported}. A lockfile is refused rather than \
                 read partially, because a half-understood lock is a different \
                 environment.",
                path.display()
            ),
            Self::LockfileLanguageMismatch {
                language,
                path,
                found,
            } => write!(
                f,
                "`[foreign.{language}]` points at `{}`, which locks `{found}`.",
                path.display()
            ),
            Self::RootMissing { language, path } => write!(
                f,
                "the `{language}` environment declares the root `{}`, which is not a \
                 directory on this host. This is a pre-entry failure: nothing falls \
                 back to whatever the host happens to have installed.",
                path.display()
            ),
            Self::SearchPathMissing { language, path } => write!(
                f,
                "the `{language}` environment root exists but `{}` — the search path \
                 its declared runtime version implies — does not. The declared \
                 version and the environment on disk disagree.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ForeignEnvironmentError {}

// ============================================================================
// Resolution
// ============================================================================

/// Resolve one language's environment from a project.
///
/// Reads the lockfile the manifest names and derives the digest. Does not check
/// whether the host can provide the environment — that is
/// [`ForeignEnvironmentDigest::check_provided`], deliberately separate so the
/// digest is a function of the source alone.
pub fn resolve_foreign_environment(
    project_root: &Path,
    foreign: &BTreeMap<String, ForeignEnvironmentSection>,
    language: &str,
) -> Result<ForeignEnvironmentDigest, ForeignEnvironmentError> {
    let section = foreign
        .get(language)
        .ok_or_else(|| ForeignEnvironmentError::Undeclared {
            language: language.to_string(),
        })?;
    let lock_relative = section
        .lockfile
        .clone()
        .unwrap_or_else(|| format!("{language}.lock"));
    let lock_path = project_root.join(&lock_relative);
    let text = std::fs::read_to_string(&lock_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ForeignEnvironmentError::LockfileMissing {
                language: language.to_string(),
                path: lock_path.clone(),
            }
        } else {
            ForeignEnvironmentError::LockfileUnreadable {
                language: language.to_string(),
                path: lock_path.clone(),
                detail: e.to_string(),
            }
        }
    })?;
    let lockfile = ForeignLockfile::parse(&text).map_err(|detail| {
        ForeignEnvironmentError::LockfileUnreadable {
            language: language.to_string(),
            path: lock_path.clone(),
            detail,
        }
    })?;
    if lockfile.version != FOREIGN_LOCKFILE_VERSION {
        return Err(ForeignEnvironmentError::LockfileVersionUnsupported {
            language: language.to_string(),
            path: lock_path,
            found: lockfile.version,
            supported: FOREIGN_LOCKFILE_VERSION,
        });
    }
    if lockfile.language != language {
        return Err(ForeignEnvironmentError::LockfileLanguageMismatch {
            language: language.to_string(),
            path: lock_path,
            found: lockfile.language.clone(),
        });
    }
    Ok(ForeignEnvironmentDigest::derive(
        language, section, lockfile,
    ))
}

// ============================================================================
// Binding: what the host resolved, for the two consumers that need it
// ============================================================================

/// The key under which an extension receives its declared environment at
/// `init`.
///
/// The host hands every extension the whole map and each picks out its own
/// language: an extension's language id is not known until after `init`
/// returns, so per-extension targeting is not available at the moment the
/// config is built.
pub const FOREIGN_ENVIRONMENTS_CONFIG_KEY: &str = "shape_foreign_environments";

/// One declared language's environment, as this host resolved it.
#[derive(Debug, Clone, PartialEq)]
pub enum ForeignEnvironmentBinding {
    /// Declared, resolved, and present on this host.
    Provided {
        digest: ForeignEnvironmentDigest,
        /// The declared environment root, resolved against the project root.
        /// `None` when the declaration named no root (the base interpreter).
        root: Option<PathBuf>,
        /// Absolute search paths for the extension to add, in declared order.
        search_paths: Vec<PathBuf>,
    },
    /// Declared and NOT providable. The rendered `[C0936]` message; the host
    /// refuses with it before any foreign code runs.
    Refused(String),
}

/// Resolve and provision-check every declared foreign environment.
///
/// Undeclared languages are simply absent from the result. That is deliberate
/// and is the boundary this slice draws: `[foreign.<lang>]` is how a project
/// *pins* an environment, and a project that pins none runs foreign bodies
/// against the base interpreter with no search path added — which is what
/// deleting the sniffer leaves, and is truthful. ADR-019 §4's stricter reading
/// (an undeclared environment refuses outright) is a release/remote admission
/// rule and belongs to #160 / #167, which own the artifact and admission
/// surfaces; [`ForeignEnvironmentError::Undeclared`] exists for them.
pub fn bind_declared_environments(
    project_root: &Path,
    foreign: &BTreeMap<String, ForeignEnvironmentSection>,
) -> BTreeMap<String, ForeignEnvironmentBinding> {
    let mut bindings = BTreeMap::new();
    for language in foreign.keys() {
        let binding = match resolve_foreign_environment(project_root, foreign, language) {
            Err(err) => ForeignEnvironmentBinding::Refused(err.to_string()),
            Ok(digest) => match digest.check_provided(project_root) {
                Err(err) => ForeignEnvironmentBinding::Refused(err.to_string()),
                Ok(search_paths) => ForeignEnvironmentBinding::Provided {
                    root: digest.declared_root().map(|r| project_root.join(r)),
                    digest,
                    search_paths,
                },
            },
        };
        bindings.insert(language.clone(), binding);
    }
    bindings
}

/// The `init` config an extension receives, carrying only the environments
/// this host actually provided.
///
/// A refused language contributes nothing: there is no half-environment to
/// activate, and the refusal is the host's to report — an extension that was
/// handed a broken environment and asked to complain about it would be the
/// silent-fallback shape wearing a different hat.
pub fn extension_environment_config(
    bindings: &BTreeMap<String, ForeignEnvironmentBinding>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (language, binding) in bindings {
        let ForeignEnvironmentBinding::Provided {
            digest,
            root,
            search_paths,
        } = binding
        else {
            continue;
        };
        map.insert(
            language.clone(),
            serde_json::json!({
                "runtime": digest.runtime_id(),
                "version": digest.runtime_version(),
                "digest": digest.to_hex(),
                "root": root.as_ref().map(|p| p.display().to_string()),
                "search_paths": search_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
            }),
        );
    }
    serde_json::Value::Object(map)
}

/// The per-language refusals, for the host's pre-entry gate.
pub fn environment_refusals(
    bindings: &BTreeMap<String, ForeignEnvironmentBinding>,
) -> BTreeMap<String, String> {
    bindings
        .iter()
        .filter_map(|(language, binding)| match binding {
            ForeignEnvironmentBinding::Refused(message) => {
                Some((language.clone(), message.clone()))
            }
            ForeignEnvironmentBinding::Provided { .. } => None,
        })
        .collect()
}

// ============================================================================
// Canonical pre-image writer
// ============================================================================

/// Domain-separated, length-framed SHA-256 pre-image writer.
///
/// Every field is written as a little-endian `u64` length followed by its
/// bytes, so no two distinct field sequences share a pre-image — `("ab", "c")`
/// and `("a", "bc")` are different inputs here, which they would not be under
/// plain concatenation.
///
/// The same shape as `shape_semantic_db::identity::DigestWriter`, deliberately
/// not shared with it: ADR-013's R16 stop line makes `shape-semantic-db` a leaf
/// on `shape-ast`, and adding a `shape-runtime` → `shape-semantic-db` edge to
/// borrow thirty lines of hashing would invert the dependency the stop line
/// exists to protect.
struct DigestWriter {
    hasher: Sha256,
}

impl DigestWriter {
    fn new(domain: &str) -> Self {
        let mut w = DigestWriter {
            hasher: Sha256::new(),
        };
        w.bytes(domain.as_bytes());
        w.u32(u32::from(FOREIGN_ENVIRONMENT_SCHEME_VERSION));
        w
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.hasher.update((bytes.len() as u64).to_le_bytes());
        self.hasher.update(bytes);
    }

    fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn opt_str(&mut self, value: Option<&str>) {
        match value {
            None => self.u8(0),
            Some(v) => {
                self.u8(1);
                self.str(v);
            }
        }
    }

    fn u8(&mut self, value: u8) {
        self.hasher.update([value]);
    }

    fn u32(&mut self, value: u32) {
        self.hasher.update(value.to_le_bytes());
    }

    /// Type-tagged canonical rendering of a checker setting.
    ///
    /// Tagged because `strict = true` and `strict = "true"` are different
    /// settings to every checker that reads them, and an untagged rendering
    /// would give them one digest.
    fn toml_value(&mut self, value: &toml::Value) {
        match value {
            toml::Value::String(s) => {
                self.u8(b's');
                self.str(s);
            }
            toml::Value::Integer(i) => {
                self.u8(b'i');
                self.hasher.update(i.to_le_bytes());
            }
            toml::Value::Float(f) => {
                self.u8(b'f');
                // Canonical bits, with the one NaN payload that would otherwise
                // let two equal-looking documents differ.
                let bits = if f.is_nan() { f64::NAN } else { *f };
                self.hasher.update(bits.to_le_bytes());
            }
            toml::Value::Boolean(b) => {
                self.u8(b'b');
                self.u8(*b as u8);
            }
            toml::Value::Datetime(dt) => {
                self.u8(b'd');
                self.str(&dt.to_string());
            }
            toml::Value::Array(items) => {
                self.u8(b'a');
                self.u32(items.len() as u32);
                for item in items {
                    self.toml_value(item);
                }
            }
            toml::Value::Table(table) => {
                self.u8(b't');
                self.u32(table.len() as u32);
                // `toml::value::Table` is a `Map` that preserves insertion order
                // unless the `preserve_order` feature is off; sort explicitly so
                // the pre-image cannot depend on which it is.
                let sorted: BTreeMap<&String, &toml::Value> = table.iter().collect();
                for (key, item) in sorted {
                    self.str(key);
                    self.toml_value(item);
                }
            }
        }
    }

    fn finish(self) -> [u8; 32] {
        let result = self.hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(runtime: &str, version: &str) -> ForeignEnvironmentSection {
        ForeignEnvironmentSection {
            runtime: runtime.to_string(),
            version: version.to_string(),
            lockfile: None,
            root: None,
            checker: None,
        }
    }

    fn lock(language: &str, packages: &[(&str, &str)]) -> ForeignLockfile {
        ForeignLockfile {
            version: FOREIGN_LOCKFILE_VERSION,
            language: language.to_string(),
            packages: packages
                .iter()
                .map(|(name, version)| {
                    (
                        name.to_string(),
                        LockedPackage {
                            version: version.to_string(),
                            integrity: None,
                            source: None,
                        },
                    )
                })
                .collect(),
            modules: BTreeMap::new(),
        }
    }

    // --- canonicalization ---------------------------------------------------

    #[test]
    fn lock_hash_is_order_independent() {
        let a = "version = 1\nlanguage = \"python\"\n\
                 [packages.numpy]\nversion = \"1.26.4\"\n\
                 [packages.attrs]\nversion = \"23.2.0\"\n";
        let b = "version = 1\nlanguage = \"python\"\n\
                 [packages.attrs]\nversion = \"23.2.0\"\n\
                 [packages.numpy]\nversion = \"1.26.4\"\n";
        let left = ForeignLockfile::parse(a).expect("a parses");
        let right = ForeignLockfile::parse(b).expect("b parses");
        assert_eq!(
            left.lock_hash(),
            right.lock_hash(),
            "the same resolved set written in a different order is the same environment"
        );
    }

    #[test]
    fn lock_hash_ignores_formatting() {
        // Non-vacuous against the obvious wrong implementation — hashing the
        // file bytes. Same resolved set; comments, key order, and inline-table
        // spelling all differ.
        let plain = "version = 1\nlanguage = \"python\"\n\
                     [packages.numpy]\nversion = \"1.26.4\"\n";
        let reformatted = "# regenerated by the resolver\n\n\
                           language = \"python\"\n\
                           version   =   1\n\n\
                           packages = { numpy = { version = \"1.26.4\" } }\n";
        assert_eq!(
            ForeignLockfile::parse(plain).expect("plain").lock_hash(),
            ForeignLockfile::parse(reformatted)
                .expect("reformatted")
                .lock_hash(),
            "a formatter must not be able to move a content-addressed artifact"
        );
    }

    #[test]
    fn lock_hash_moves_when_a_version_moves() {
        let before = lock("python", &[("numpy", "1.26.4")]);
        let after = lock("python", &[("numpy", "1.26.5")]);
        assert_ne!(before.lock_hash(), after.lock_hash());
    }

    #[test]
    fn lock_hash_distinguishes_field_boundaries() {
        // The length framing earns its keep here: unframed, ("ab","c") and
        // ("a","bc") concatenate identically.
        let left = lock("python", &[("ab", "c")]);
        let right = lock("python", &[("a", "bc")]);
        assert_ne!(left.lock_hash(), right.lock_hash());
    }

    #[test]
    fn canonical_serialization_round_trips_and_is_stable() {
        let original = lock("python", &[("numpy", "1.26.4"), ("attrs", "23.2.0")]);
        let text = original.to_canonical_toml().expect("serializes");
        let reparsed = ForeignLockfile::parse(&text).expect("round-trips");
        assert_eq!(original, reparsed);
        assert_eq!(
            text,
            reparsed.to_canonical_toml().expect("serializes again"),
            "a regenerated lock that resolved to the same set must be byte-identical"
        );
    }

    #[test]
    fn checker_settings_are_order_independent_but_type_sensitive() {
        let mut a = section("cpython", "3.11.7");
        let mut settings_a = BTreeMap::new();
        settings_a.insert("strict".to_string(), toml::Value::Boolean(true));
        settings_a.insert(
            "pythonVersion".to_string(),
            toml::Value::String("3.11".to_string()),
        );
        a.checker = Some(ForeignCheckerPin {
            name: "pyright".to_string(),
            version: "1.1.350".to_string(),
            settings: settings_a,
        });

        // Same settings, inserted in the other order.
        let mut b = a.clone();
        let mut settings_b = BTreeMap::new();
        settings_b.insert(
            "pythonVersion".to_string(),
            toml::Value::String("3.11".to_string()),
        );
        settings_b.insert("strict".to_string(), toml::Value::Boolean(true));
        b.checker.as_mut().unwrap().settings = settings_b;

        let lockfile = lock("python", &[]);
        let da = ForeignEnvironmentDigest::derive("python", &a, lockfile.clone());
        let db = ForeignEnvironmentDigest::derive("python", &b, lockfile.clone());
        assert_eq!(da.digest(), db.digest(), "insertion order is not content");

        // `true` is not `"true"`.
        let mut c = a.clone();
        c.checker.as_mut().unwrap().settings.insert(
            "strict".to_string(),
            toml::Value::String("true".to_string()),
        );
        let dc = ForeignEnvironmentDigest::derive("python", &c, lockfile);
        assert_ne!(
            da.digest(),
            dc.digest(),
            "a bool setting and a string setting are different settings"
        );
    }

    // --- what moves the digest ---------------------------------------------

    #[test]
    fn digest_moves_with_the_lockfile() {
        let sec = section("cpython", "3.11.7");
        let before = ForeignEnvironmentDigest::derive(
            "python",
            &sec,
            lock("python", &[("numpy", "1.26.4")]),
        );
        let after = ForeignEnvironmentDigest::derive(
            "python",
            &sec,
            lock("python", &[("numpy", "1.26.5")]),
        );
        assert_ne!(before.digest(), after.digest());
    }

    #[test]
    fn digest_moves_with_the_interpreter_version() {
        let lockfile = lock("python", &[]);
        let a = ForeignEnvironmentDigest::derive(
            "python",
            &section("cpython", "3.11.7"),
            lockfile.clone(),
        );
        let b = ForeignEnvironmentDigest::derive("python", &section("cpython", "3.12.1"), lockfile);
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn digest_moves_with_the_checker_pin() {
        let lockfile = lock("python", &[]);
        let plain = section("cpython", "3.11.7");
        let mut pinned = plain.clone();
        pinned.checker = Some(ForeignCheckerPin {
            name: "pyright".to_string(),
            version: "1.1.350".to_string(),
            settings: BTreeMap::new(),
        });
        let mut bumped = pinned.clone();
        bumped.checker.as_mut().unwrap().version = "1.1.351".to_string();

        let d_plain = ForeignEnvironmentDigest::derive("python", &plain, lockfile.clone());
        let d_pinned = ForeignEnvironmentDigest::derive("python", &pinned, lockfile.clone());
        let d_bumped = ForeignEnvironmentDigest::derive("python", &bumped, lockfile);
        assert_ne!(d_plain.digest(), d_pinned.digest());
        assert_ne!(
            d_pinned.digest(),
            d_bumped.digest(),
            "a checker upgrade is a reviewed environment change, not a no-op"
        );
    }

    #[test]
    fn digest_is_language_separated() {
        let sec = section("deno", "1.40.0");
        let a = ForeignEnvironmentDigest::derive("typescript", &sec, lock("typescript", &[]));
        let b = ForeignEnvironmentDigest::derive("python", &sec, lock("python", &[]));
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn digest_does_not_move_with_the_declared_root() {
        // The root says where the environment is on this host; the digest says
        // what the environment is. Two checkouts at different paths must
        // produce the same artifact.
        let mut a = section("cpython", "3.11.7");
        a.root = Some(".venv".to_string());
        let mut b = a.clone();
        b.root = Some("/opt/envs/app".to_string());
        let lockfile = lock("python", &[]);
        assert_eq!(
            ForeignEnvironmentDigest::derive("python", &a, lockfile.clone()).digest(),
            ForeignEnvironmentDigest::derive("python", &b, lockfile).digest()
        );
    }

    #[test]
    fn tracked_input_identity_names_the_language() {
        let d = ForeignEnvironmentDigest::derive(
            "python",
            &section("cpython", "3.11.7"),
            lock("python", &[]),
        );
        assert_eq!(
            d.tracked_input_identity(),
            "shape.foreign.environment/python"
        );
        assert_eq!(d.public_digest(), d.digest());
    }

    // --- resolution and refusal --------------------------------------------

    fn temp_project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, contents) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(path, contents).expect("write");
        }
        dir
    }

    fn foreign_map(
        section: ForeignEnvironmentSection,
    ) -> BTreeMap<String, ForeignEnvironmentSection> {
        let mut map = BTreeMap::new();
        map.insert("python".to_string(), section);
        map
    }

    #[test]
    fn resolves_from_the_default_lockfile_name() {
        let dir = temp_project(&[(
            "python.lock",
            "version = 1\nlanguage = \"python\"\n[packages.numpy]\nversion = \"1.26.4\"\n",
        )]);
        let resolved = resolve_foreign_environment(
            dir.path(),
            &foreign_map(section("cpython", "3.11.7")),
            "python",
        )
        .expect("resolves");
        assert_eq!(resolved.lockfile().packages.len(), 1);
        assert_eq!(resolved.runtime_id(), "cpython");
    }

    #[test]
    fn undeclared_language_is_a_structured_refusal() {
        let dir = temp_project(&[]);
        let err = resolve_foreign_environment(dir.path(), &BTreeMap::new(), "python")
            .expect_err("no declaration");
        assert!(matches!(err, ForeignEnvironmentError::Undeclared { .. }));
        let message = err.to_string();
        assert!(message.contains("[C0936]"), "got: {message}");
        assert!(message.contains("[foreign.python]"), "got: {message}");
    }

    #[test]
    fn missing_lockfile_is_a_structured_refusal() {
        let dir = temp_project(&[]);
        let err = resolve_foreign_environment(
            dir.path(),
            &foreign_map(section("cpython", "3.11.7")),
            "python",
        )
        .expect_err("no lockfile");
        assert!(matches!(
            err,
            ForeignEnvironmentError::LockfileMissing { .. }
        ));
        assert!(err.to_string().contains("[C0936]"));
    }

    #[test]
    fn a_future_lockfile_version_is_refused_not_read() {
        let dir = temp_project(&[("python.lock", "version = 99\nlanguage = \"python\"\n")]);
        let err = resolve_foreign_environment(
            dir.path(),
            &foreign_map(section("cpython", "3.11.7")),
            "python",
        )
        .expect_err("unsupported version");
        assert!(matches!(
            err,
            ForeignEnvironmentError::LockfileVersionUnsupported { found: 99, .. }
        ));
    }

    #[test]
    fn a_lockfile_for_another_language_is_refused() {
        let dir = temp_project(&[("python.lock", "version = 1\nlanguage = \"typescript\"\n")]);
        let err = resolve_foreign_environment(
            dir.path(),
            &foreign_map(section("cpython", "3.11.7")),
            "python",
        )
        .expect_err("mismatched language");
        assert!(matches!(
            err,
            ForeignEnvironmentError::LockfileLanguageMismatch { .. }
        ));
    }

    #[test]
    fn an_unknown_lockfile_key_is_refused() {
        let dir = temp_project(&[(
            "python.lock",
            "version = 1\nlanguage = \"python\"\npackagez = {}\n",
        )]);
        let err = resolve_foreign_environment(
            dir.path(),
            &foreign_map(section("cpython", "3.11.7")),
            "python",
        )
        .expect_err("typo is an error");
        assert!(matches!(
            err,
            ForeignEnvironmentError::LockfileUnreadable { .. }
        ));
    }

    // --- provision checking -------------------------------------------------

    #[test]
    fn a_missing_declared_root_fails_pre_entry() {
        let dir = temp_project(&[]);
        let mut sec = section("cpython", "3.11.7");
        sec.root = Some(".venv".to_string());
        let digest = ForeignEnvironmentDigest::derive("python", &sec, lock("python", &[]));
        let err = digest.check_provided(dir.path()).expect_err("no venv here");
        assert!(matches!(err, ForeignEnvironmentError::RootMissing { .. }));
        assert!(err.to_string().contains("[C0936]"));
    }

    #[test]
    fn a_provided_root_yields_the_declared_search_path() {
        let dir = temp_project(&[(".venv/lib/python3.11/site-packages/marker.txt", "")]);
        let mut sec = section("cpython", "3.11.7");
        sec.root = Some(".venv".to_string());
        let digest = ForeignEnvironmentDigest::derive("python", &sec, lock("python", &[]));
        let paths = digest.check_provided(dir.path()).expect("provided");
        assert_eq!(paths.len(), 1);
        assert!(
            paths[0].ends_with("lib/python3.11/site-packages"),
            "got: {paths:?}"
        );
    }

    #[test]
    fn a_root_that_does_not_match_the_declared_version_fails() {
        // The venv is 3.10; the manifest says 3.11. Nothing here searches for
        // "whichever python is in there" — that search was the sniffer.
        let dir = temp_project(&[(".venv/lib/python3.10/site-packages/marker.txt", "")]);
        let mut sec = section("cpython", "3.11.7");
        sec.root = Some(".venv".to_string());
        let digest = ForeignEnvironmentDigest::derive("python", &sec, lock("python", &[]));
        let err = digest.check_provided(dir.path()).expect_err("version skew");
        assert!(matches!(
            err,
            ForeignEnvironmentError::SearchPathMissing { .. }
        ));
    }

    #[test]
    fn no_declared_root_means_no_search_paths() {
        let dir = temp_project(&[]);
        let digest = ForeignEnvironmentDigest::derive(
            "python",
            &section("cpython", "3.11.7"),
            lock("python", &[]),
        );
        assert!(
            digest.check_provided(dir.path()).expect("ok").is_empty(),
            "absent root is a declaration of the base interpreter, not a licence to search"
        );
    }

    // --- binding -------------------------------------------------------------

    #[test]
    fn a_provided_environment_binds_and_reaches_the_extension_config() {
        let dir = temp_project(&[
            (
                "python.lock",
                "version = 1\nlanguage = \"python\"\n[packages.numpy]\nversion = \"1.26.4\"\n",
            ),
            (".venv/lib/python3.11/site-packages/marker.txt", ""),
        ]);
        let mut sec = section("cpython", "3.11.7");
        sec.root = Some(".venv".to_string());
        let bindings = bind_declared_environments(dir.path(), &foreign_map(sec));
        assert!(matches!(
            bindings.get("python"),
            Some(ForeignEnvironmentBinding::Provided { .. })
        ));
        assert!(environment_refusals(&bindings).is_empty());

        let config = extension_environment_config(&bindings);
        let python = config.get("python").expect("python is in the config");
        assert_eq!(python.get("runtime").unwrap(), "cpython");
        assert_eq!(python.get("version").unwrap(), "3.11.7");
        let paths = python.get("search_paths").unwrap().as_array().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(
            paths[0].as_str().unwrap().ends_with("site-packages"),
            "got: {paths:?}"
        );
    }

    #[test]
    fn a_refused_environment_reaches_the_gate_and_not_the_extension() {
        // Declared root absent: the extension must be told nothing (there is no
        // half-environment to activate) and the host must hold the refusal.
        let dir = temp_project(&[("python.lock", "version = 1\nlanguage = \"python\"\n")]);
        let mut sec = section("cpython", "3.11.7");
        sec.root = Some(".venv".to_string());
        let bindings = bind_declared_environments(dir.path(), &foreign_map(sec));

        let refusals = environment_refusals(&bindings);
        let message = refusals.get("python").expect("refused");
        assert!(message.contains("[C0936]"), "got: {message}");

        let config = extension_environment_config(&bindings);
        assert!(
            config.get("python").is_none(),
            "a refused environment must not be handed to the extension in any form"
        );
    }

    #[test]
    fn an_undeclared_language_binds_to_nothing() {
        let dir = temp_project(&[]);
        let bindings = bind_declared_environments(dir.path(), &BTreeMap::new());
        assert!(bindings.is_empty());
        assert!(environment_refusals(&bindings).is_empty());
    }

    #[test]
    fn major_minor_extraction() {
        assert_eq!(major_minor("3.11.7").as_deref(), Some("3.11"));
        assert_eq!(major_minor("3.11").as_deref(), Some("3.11"));
        assert_eq!(major_minor("3").as_deref(), None);
        assert_eq!(major_minor("3.x").as_deref(), None);
        assert_eq!(major_minor("").as_deref(), None);
    }
}
