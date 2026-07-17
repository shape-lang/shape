//! Typed provenance for the resolver that supplied a module artifact.

/// Resolver origin that actually won the module-loader priority chain.
///
/// Only [`super::ModuleLoader`] assigns non-`Direct` origins. Consumers may
/// inspect this value, but cannot mutate a loaded module's origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModuleArtifactOrigin {
    /// Parsed or constructed directly, outside the resolver chain.
    #[default]
    Direct,
    /// Loaded from an on-disk module path.
    Filesystem,
    /// Supplied by the extension resolver.
    Extension,
    /// Supplied by the package-bundle resolver.
    Bundle,
    /// Supplied by the embedded standard-library resolver.
    EmbeddedStdlib,
}
