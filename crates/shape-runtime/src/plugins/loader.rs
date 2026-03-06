//! Plugin Loader
//!
//! Handles dynamic loading of plugin shared libraries using libloading.

use std::collections::HashMap;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use libloading::{Library, Symbol};

use shape_abi_v1::{
    ABI_VERSION, CAPABILITY_DATA_SOURCE, CAPABILITY_LANGUAGE_RUNTIME, CAPABILITY_MODULE,
    CAPABILITY_OUTPUT_SINK, CapabilityKind, CapabilityManifest, DataSourceVTable, GetAbiVersionFn,
    GetCapabilityManifestFn, GetCapabilityVTableFn, GetClaimedSectionsFn, GetPluginInfoFn,
    LanguageRuntimeVTable, ModuleVTable, OutputSinkVTable, PluginType, SectionsManifest,
};

use shape_ast::error::{Result, ShapeError};

/// A TOML section claimed by a loaded plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedSection {
    /// Section name (e.g., "native-dependencies")
    pub name: String,
    /// Whether this section is required (error if missing)
    pub required: bool,
}

/// One declared capability exposed by a loaded plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCapability {
    /// Capability family.
    pub kind: CapabilityKind,
    /// Contract name (e.g., `shape.datasource`).
    pub contract: String,
    /// Contract version (e.g., `1`).
    pub version: String,
    /// Reserved capability flags.
    pub flags: u64,
}

/// Information about a loaded plugin
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin type
    pub plugin_type: PluginType,
    /// Plugin description
    pub description: String,
    /// Self-declared capability contracts.
    pub capabilities: Vec<PluginCapability>,
    /// TOML sections claimed by this plugin.
    pub claimed_sections: Vec<ClaimedSection>,
}

impl LoadedPlugin {
    /// Returns true if the plugin declares at least one capability with `kind`.
    pub fn has_capability_kind(&self, kind: CapabilityKind) -> bool {
        self.capabilities.iter().any(|cap| cap.kind == kind)
    }

    /// Returns the names of all claimed sections.
    pub fn claimed_section_names(&self) -> Vec<&str> {
        self.claimed_sections
            .iter()
            .map(|s| s.name.as_str())
            .collect()
    }
}

/// Plugin Loader
///
/// Manages dynamic loading and unloading of Shape plugins.
/// Keeps loaded libraries in memory to prevent unloading while in use.
pub struct PluginLoader {
    /// Loaded libraries (kept alive to prevent unloading)
    loaded_libraries: HashMap<String, Library>,
}

impl PluginLoader {
    /// Create a new plugin loader
    pub fn new() -> Self {
        Self {
            loaded_libraries: HashMap::new(),
        }
    }

    /// Load a plugin from a shared library file
    ///
    /// # Arguments
    /// * `path` - Path to the shared library (.so, .dll, .dylib)
    ///
    /// # Returns
    /// Information about the loaded plugin
    ///
    /// # Safety
    /// Loading plugins executes arbitrary code. Only load from trusted sources.
    pub fn load(&mut self, path: &Path) -> Result<LoadedPlugin> {
        // Load the library
        let lib =
            load_library_with_python_fallback(path).map_err(|e| ShapeError::RuntimeError {
                message: format!("Failed to load plugin library '{}': {}", path.display(), e),
                location: None,
            })?;

        // Check ABI version if available
        if let Ok(get_version) = unsafe { lib.get::<GetAbiVersionFn>(b"shape_abi_version") } {
            let version = unsafe { get_version() };
            if version != ABI_VERSION {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "Plugin ABI version mismatch: expected {}, got {}",
                        ABI_VERSION, version
                    ),
                    location: None,
                });
            }
        }

        // Get plugin info
        let get_info: Symbol<GetPluginInfoFn> = unsafe {
            lib.get(b"shape_plugin_info")
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("Plugin missing 'shape_plugin_info' export: {}", e),
                    location: None,
                })?
        };

        let info_ptr = unsafe { get_info() };
        if info_ptr.is_null() {
            return Err(ShapeError::RuntimeError {
                message: "Plugin returned null PluginInfo".to_string(),
                location: None,
            });
        }

        let info = unsafe { &*info_ptr };

        // Extract info strings
        let name = read_c_string(info.name, "PluginInfo.name")?;
        let version = read_c_string(info.version, "PluginInfo.version")?;
        let description = read_c_string(info.description, "PluginInfo.description")?;

        let capabilities = self.load_capabilities(&lib)?;

        // Load optional section claims
        let claimed_sections = if let Ok(get_sections) =
            unsafe { lib.get::<GetClaimedSectionsFn>(b"shape_claimed_sections") }
        {
            let manifest_ptr = unsafe { get_sections() };
            if manifest_ptr.is_null() {
                vec![]
            } else {
                let manifest = unsafe { &*manifest_ptr };
                parse_sections_manifest(manifest)?
            }
        } else {
            vec![] // Optional — no section claims
        };

        // Store the library
        self.loaded_libraries.insert(name.clone(), lib);

        Ok(LoadedPlugin {
            name,
            version,
            plugin_type: info.plugin_type,
            description,
            capabilities,
            claimed_sections,
        })
    }

    fn load_capabilities(&self, lib: &Library) -> Result<Vec<PluginCapability>> {
        let get_manifest =
            unsafe { lib.get::<GetCapabilityManifestFn>(b"shape_capability_manifest") }.map_err(
                |e| ShapeError::RuntimeError {
                    message: format!(
                        "Plugin missing required 'shape_capability_manifest' export: {}",
                        e
                    ),
                    location: None,
                },
            )?;

        let manifest_ptr = unsafe { get_manifest() };
        if manifest_ptr.is_null() {
            return Err(ShapeError::RuntimeError {
                message: "Plugin returned null CapabilityManifest".to_string(),
                location: None,
            });
        }
        let manifest = unsafe { &*manifest_ptr };
        parse_capability_manifest(manifest)
    }

    /// Get the data source vtable for a loaded plugin
    ///
    /// # Arguments
    /// * `name` - Name of the loaded plugin
    ///
    /// # Returns
    /// The DataSourceVTable if plugin exists and is a data source
    pub fn get_data_source_vtable(&self, name: &str) -> Result<&'static DataSourceVTable> {
        let lib = self
            .loaded_libraries
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Plugin '{}' not loaded", name),
                location: None,
            })?;

        if let Some(vtable_ptr) = try_capability_vtable(lib, CAPABILITY_DATA_SOURCE)? {
            // SAFETY: vtable pointer is provided by the loaded module and expected static.
            return Ok(unsafe { &*(vtable_ptr as *const DataSourceVTable) });
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' does not provide capability vtable for '{}'",
                name, CAPABILITY_DATA_SOURCE
            ),
            location: None,
        })
    }

    /// Get the output sink vtable for a loaded plugin
    ///
    /// # Arguments
    /// * `name` - Name of the loaded plugin
    ///
    /// # Returns
    /// The OutputSinkVTable if plugin exists and is an output sink
    pub fn get_output_sink_vtable(&self, name: &str) -> Result<&'static OutputSinkVTable> {
        let lib = self
            .loaded_libraries
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Plugin '{}' not loaded", name),
                location: None,
            })?;

        if let Some(vtable_ptr) = try_capability_vtable(lib, CAPABILITY_OUTPUT_SINK)? {
            // SAFETY: vtable pointer is provided by the loaded module and expected static.
            return Ok(unsafe { &*(vtable_ptr as *const OutputSinkVTable) });
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' does not provide capability vtable for '{}'",
                name, CAPABILITY_OUTPUT_SINK
            ),
            location: None,
        })
    }

    /// Get the base module vtable for a loaded plugin.
    pub fn get_module_vtable(&self, name: &str) -> Result<&'static ModuleVTable> {
        let lib = self
            .loaded_libraries
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Plugin '{}' not loaded", name),
                location: None,
            })?;

        if let Some(vtable_ptr) = try_capability_vtable(lib, CAPABILITY_MODULE)? {
            // SAFETY: vtable pointer is provided by the loaded module and expected static.
            return Ok(unsafe { &*(vtable_ptr as *const ModuleVTable) });
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' does not provide capability vtable for '{}'",
                name, CAPABILITY_MODULE
            ),
            location: None,
        })
    }

    /// Get the language runtime vtable for a loaded plugin.
    pub fn get_language_runtime_vtable(
        &self,
        name: &str,
    ) -> Result<&'static LanguageRuntimeVTable> {
        let lib = self
            .loaded_libraries
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Plugin '{}' not loaded", name),
                location: None,
            })?;

        if let Some(vtable_ptr) = try_capability_vtable(lib, CAPABILITY_LANGUAGE_RUNTIME)? {
            return Ok(unsafe { &*(vtable_ptr as *const LanguageRuntimeVTable) });
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' does not provide capability vtable for '{}'",
                name, CAPABILITY_LANGUAGE_RUNTIME
            ),
            location: None,
        })
    }

    /// Unload a plugin
    ///
    /// Note: The library is actually unloaded when dropped. This removes it
    /// from the loader's tracking.
    pub fn unload(&mut self, name: &str) -> bool {
        self.loaded_libraries.remove(name).is_some()
    }

    /// List all loaded plugins
    pub fn loaded_plugins(&self) -> Vec<&str> {
        self.loaded_libraries.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a plugin is loaded
    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded_libraries.contains_key(name)
    }

    /// Load a data source plugin and return a ready-to-use wrapper
    ///
    /// This is a convenience method that combines loading the library,
    /// getting the vtable, and creating the PluginDataSource wrapper.
    ///
    /// # Arguments
    /// * `path` - Path to the shared library
    /// * `config` - Configuration value for the plugin
    ///
    /// # Returns
    /// Ready-to-use PluginDataSource wrapper
    pub fn load_data_source(
        &mut self,
        path: &Path,
        config: &serde_json::Value,
    ) -> Result<super::PluginDataSource> {
        // Load the library and get info
        let info = self.load(path)?;
        let name = info.name.clone();

        if !info.has_capability_kind(CapabilityKind::DataSource) {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Plugin '{}' does not declare data source capability",
                    info.name
                ),
                location: None,
            });
        }

        // Get the vtable
        let vtable = self.get_data_source_vtable(&name)?;

        // Create and return the wrapper
        super::PluginDataSource::new(name, vtable, config)
    }
}

fn load_library_with_python_fallback(path: &Path) -> std::result::Result<Library, String> {
    let initial = unsafe { Library::new(path) };
    let initial_error = match initial {
        Ok(lib) => return Ok(lib),
        Err(err) => err,
    };
    let initial_msg = initial_error.to_string();

    if !should_try_python_fallback(&initial_msg) {
        return Err(initial_msg);
    }

    if !preload_python_shared_library() {
        return Err(initial_msg);
    }

    match unsafe { Library::new(path) } {
        Ok(lib) => Ok(lib),
        Err(retry_err) => Err(format!(
            "{} (retry after python preload failed: {})",
            initial_msg, retry_err
        )),
    }
}

fn should_try_python_fallback(error_message: &str) -> bool {
    let lowered = error_message.to_ascii_lowercase();
    lowered.contains("libpython") || lowered.contains("python.framework")
}

fn preload_python_shared_library() -> bool {
    let candidates = discover_python_shared_library_candidates();
    for candidate in candidates {
        match unsafe { Library::new(&candidate) } {
            Ok(lib) => {
                tracing::info!(
                    "preloaded python runtime library for extension loading fallback: {}",
                    candidate.display()
                );
                // Keep the library loaded for process lifetime.
                std::mem::forget(lib);
                return true;
            }
            Err(err) => {
                tracing::debug!(
                    "failed to preload python runtime candidate '{}': {}",
                    candidate.display(),
                    err
                );
            }
        }
    }
    false
}

fn discover_python_shared_library_candidates() -> Vec<PathBuf> {
    let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let script = r#"import os, sys, sysconfig
cands = []
libdir = sysconfig.get_config_var("LIBDIR")
ldlibrary = sysconfig.get_config_var("LDLIBRARY")
if libdir and ldlibrary:
    cands.append(os.path.join(libdir, ldlibrary))
if libdir:
    for name in ("libpython3.so", "libpython3.so.1.0", "libpython3.dylib"):
        cands.append(os.path.join(libdir, name))
for base in {sys.base_prefix, sys.prefix}:
    if not base:
        continue
    for rel in ("lib", "lib64"):
        d = os.path.join(base, rel)
        if ldlibrary:
            cands.append(os.path.join(d, ldlibrary))
seen = set()
for cand in cands:
    if not cand:
        continue
    real = os.path.realpath(cand)
    if real in seen:
        continue
    seen.add(real)
    if os.path.exists(real):
        print(real)
"#;

    let output = Command::new(&python).arg("-c").arg(script).output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

impl Drop for PluginLoader {
    fn drop(&mut self) {
        // Language runtime extensions (e.g. Python/pyo3) may register process-level
        // atexit handlers that reference code inside the loaded .so. If we dlclose
        // the library before those handlers run at process exit, the process segfaults.
        // Intentionally leak language runtime libraries so they remain mapped.
        for (_name, lib) in self.loaded_libraries.drain() {
            if let Ok(get_manifest) =
                unsafe { lib.get::<GetCapabilityManifestFn>(b"shape_capability_manifest") }
            {
                let manifest_ptr = unsafe { get_manifest() };
                if !manifest_ptr.is_null() {
                    let manifest = unsafe { &*manifest_ptr };
                    if let Ok(caps) = parse_capability_manifest(manifest) {
                        if caps
                            .iter()
                            .any(|c| c.kind == CapabilityKind::LanguageRuntime)
                        {
                            // Leak: keep the library mapped for the process lifetime.
                            std::mem::forget(lib);
                            continue;
                        }
                    }
                }
            }
            // Non-language-runtime libraries are dropped normally (dlclose).
            drop(lib);
        }
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

fn try_capability_vtable(lib: &Library, contract: &str) -> Result<Option<*const std::ffi::c_void>> {
    let get_vtable_fn = unsafe { lib.get::<GetCapabilityVTableFn>(b"shape_capability_vtable") };
    let Ok(get_vtable_fn) = get_vtable_fn else {
        return Ok(None);
    };

    let vtable_ptr = unsafe { get_vtable_fn(contract.as_ptr(), contract.len()) };
    if vtable_ptr.is_null() {
        return Ok(None);
    }
    Ok(Some(vtable_ptr))
}

fn parse_capability_manifest(manifest: &CapabilityManifest) -> Result<Vec<PluginCapability>> {
    if manifest.capabilities_len == 0 {
        return Err(ShapeError::RuntimeError {
            message: "CapabilityManifest must contain at least one capability".to_string(),
            location: None,
        });
    }
    if manifest.capabilities.is_null() {
        return Err(ShapeError::RuntimeError {
            message: "CapabilityManifest.capabilities is null".to_string(),
            location: None,
        });
    }

    let caps =
        unsafe { std::slice::from_raw_parts(manifest.capabilities, manifest.capabilities_len) };
    let mut parsed = Vec::with_capacity(caps.len());
    for cap in caps {
        parsed.push(PluginCapability {
            kind: cap.kind,
            contract: read_c_string(cap.contract, "CapabilityDescriptor.contract")?,
            version: read_c_string(cap.version, "CapabilityDescriptor.version")?,
            flags: cap.flags,
        });
    }
    Ok(parsed)
}

pub fn parse_sections_manifest(manifest: &SectionsManifest) -> Result<Vec<ClaimedSection>> {
    if manifest.sections_len == 0 {
        return Ok(vec![]);
    }
    if manifest.sections.is_null() {
        return Err(ShapeError::RuntimeError {
            message: "SectionsManifest.sections is null but sections_len > 0".to_string(),
            location: None,
        });
    }

    let claims = unsafe { std::slice::from_raw_parts(manifest.sections, manifest.sections_len) };
    let mut parsed = Vec::with_capacity(claims.len());
    for claim in claims {
        parsed.push(ClaimedSection {
            name: read_c_string(claim.name, "SectionClaim.name")?,
            required: claim.required,
        });
    }
    Ok(parsed)
}

fn read_c_string(ptr: *const std::ffi::c_char, field: &str) -> Result<String> {
    if ptr.is_null() {
        return Err(ShapeError::RuntimeError {
            message: format!("{} is null", field),
            location: None,
        });
    }

    Ok(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_abi_v1::{CAPABILITY_MODULE, CapabilityDescriptor};

    #[test]
    fn test_plugin_loader_new() {
        let loader = PluginLoader::new();
        assert!(loader.loaded_plugins().is_empty());
    }

    #[test]
    fn test_is_loaded_false() {
        let loader = PluginLoader::new();
        assert!(!loader.is_loaded("nonexistent"));
    }

    #[test]
    fn test_should_try_python_fallback_matches_libpython_errors() {
        assert!(should_try_python_fallback(
            "libpython3.13.so.1.0: cannot open shared object file"
        ));
        assert!(should_try_python_fallback(
            "Library not loaded: @rpath/Python.framework/Versions/3.12/Python"
        ));
        assert!(!should_try_python_fallback(
            "undefined symbol: sqlite3_open"
        ));
    }

    #[test]
    fn test_parse_capability_manifest() {
        static CAPS: [CapabilityDescriptor; 2] = [
            CapabilityDescriptor {
                kind: CapabilityKind::DataSource,
                contract: c"shape.datasource".as_ptr(),
                version: c"1".as_ptr(),
                flags: 0,
            },
            CapabilityDescriptor {
                kind: CapabilityKind::Compute,
                contract: c"shape.compute".as_ptr(),
                version: c"1".as_ptr(),
                flags: 42,
            },
        ];
        static MANIFEST: CapabilityManifest = CapabilityManifest {
            capabilities: CAPS.as_ptr(),
            capabilities_len: CAPS.len(),
        };

        let parsed = parse_capability_manifest(&MANIFEST).expect("manifest should parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].contract, "shape.datasource");
        assert_eq!(parsed[1].kind, CapabilityKind::Compute);
        assert_eq!(parsed[1].flags, 42);
    }

    #[test]
    fn test_parse_capability_manifest_rejects_empty() {
        static MANIFEST: CapabilityManifest = CapabilityManifest {
            capabilities: std::ptr::null(),
            capabilities_len: 0,
        };
        let result = parse_capability_manifest(&MANIFEST);
        assert!(result.is_err());
    }

    #[test]
    fn test_module_contract_constant_is_expected() {
        assert_eq!(CAPABILITY_MODULE, "shape.module");
    }

    #[test]
    fn test_parse_sections_manifest_valid() {
        use shape_abi_v1::SectionClaim as AbiSectionClaim;

        static CLAIMS: [AbiSectionClaim; 2] = [
            AbiSectionClaim {
                name: c"native-dependencies".as_ptr(),
                required: false,
            },
            AbiSectionClaim {
                name: c"custom-config".as_ptr(),
                required: true,
            },
        ];
        static MANIFEST: SectionsManifest = SectionsManifest {
            sections: CLAIMS.as_ptr(),
            sections_len: CLAIMS.len(),
        };

        let parsed = parse_sections_manifest(&MANIFEST).expect("should parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "native-dependencies");
        assert!(!parsed[0].required);
        assert_eq!(parsed[1].name, "custom-config");
        assert!(parsed[1].required);
    }

    #[test]
    fn test_parse_sections_manifest_empty() {
        static MANIFEST: SectionsManifest = SectionsManifest {
            sections: std::ptr::null(),
            sections_len: 0,
        };
        let parsed = parse_sections_manifest(&MANIFEST).expect("empty should parse");
        assert!(parsed.is_empty());
    }
}
