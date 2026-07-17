//! Compile avoidance and request-context routing for capture queries.

use shape_ast::ast::Program;
use shape_vm::compiler::GeneratedCaptureQuery;

#[cfg(test)]
use super::session::CaptureQueryContext;
use super::session::GeneratedQuerySession;

pub(crate) enum GeneratedCaptureLookup<T> {
    NotCapture,
    Unavailable,
    Found(T),
}

#[cfg(test)]
impl<T> GeneratedCaptureLookup<T> {
    pub(super) fn found(self, message: &str) -> T {
        match self {
            Self::Found(value) => value,
            Self::NotCapture => panic!("{message}: not a capture site"),
            Self::Unavailable => panic!("{message}: query unavailable"),
        }
    }

    pub(super) fn is_not_capture(&self) -> bool {
        matches!(self, Self::NotCapture)
    }
}

pub(super) enum CaptureAnalysis {
    NotNeeded,
    Unavailable,
    Ready(GeneratedCaptureQuery),
}

#[cfg(test)]
pub(super) fn analyze(
    program: &Program,
    text: &str,
    context: CaptureQueryContext<'_>,
) -> CaptureAnalysis {
    let session = GeneratedQuerySession::new(program, text, context);
    analyze_session(&session, program)
}

pub(super) fn analyze_session(
    session: &GeneratedQuerySession,
    program: &Program,
) -> CaptureAnalysis {
    match session {
        GeneratedQuerySession::NotNeeded => CaptureAnalysis::NotNeeded,
        GeneratedQuerySession::Unavailable => CaptureAnalysis::Unavailable,
        GeneratedQuerySession::Ready(compiler) => {
            CaptureAnalysis::Ready(compiler.generated_capture_query(program))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_source_short_circuits_before_compiler_query() {
        crate::generated_symbols::reset_generated_capture_compile_count();
        let source = "fn add(a: int, b: int) -> int { a + b }";
        let program = shape_ast::parse_program(source).expect("fixture parses");
        assert!(matches!(
            analyze(&program, source, CaptureQueryContext::unavailable()),
            CaptureAnalysis::NotNeeded,
        ));
        assert_eq!(
            crate::generated_symbols::generated_capture_compile_count(),
            0,
        );
    }

    #[test]
    fn generating_document_with_imports_refuses_without_diagnostic_context() {
        crate::generated_symbols::reset_generated_capture_compile_count();
        let source = "from support use { helper }\n@derive()\ntype Job { id: int }";
        let program = shape_ast::parse_program(source).expect("fixture parses");
        assert!(matches!(
            analyze(&program, source, CaptureQueryContext::unavailable()),
            CaptureAnalysis::Unavailable,
        ));
        assert_eq!(
            crate::generated_symbols::generated_capture_compile_count(),
            0,
        );
    }
}
