//! One context-registered compiler per LSP navigation request.

use shape_ast::ast::Program;

use crate::generated_symbols::{
    compile_for_generated_capture_queries, program_may_generate_symbols,
};
use crate::module_cache::ModuleCache;

pub(crate) struct CaptureQueryContext<'request> {
    pub(crate) file_path: Option<&'request std::path::Path>,
    pub(crate) module_cache: Option<&'request ModuleCache>,
    pub(crate) workspace_root: Option<&'request std::path::Path>,
}

impl CaptureQueryContext<'_> {
    pub(crate) const fn unavailable() -> Self {
        Self {
            file_path: None,
            module_cache: None,
            workspace_root: None,
        }
    }
}

pub(crate) enum GeneratedQuerySession {
    NotNeeded,
    Unavailable,
    Ready(shape_vm::BytecodeCompiler),
}

impl GeneratedQuerySession {
    pub(crate) fn new(program: &Program, text: &str, context: CaptureQueryContext<'_>) -> Self {
        if !program_may_generate_symbols(program) {
            return Self::NotNeeded;
        }
        match compile_for_generated_capture_queries(
            program,
            text,
            context.file_path,
            context.module_cache,
            context.workspace_root,
        ) {
            Some(compiler) => Self::Ready(compiler),
            None => Self::Unavailable,
        }
    }

    pub(crate) fn compiler(&self) -> Option<&shape_vm::BytecodeCompiler> {
        match self {
            Self::Ready(compiler) => Some(compiler),
            Self::NotNeeded | Self::Unavailable => None,
        }
    }
}
