//! The single constructor for generated closure structural paths.

use crate::ast::GeneratedNodePath;

pub(super) struct GeneratedClosurePathCursor {
    parent: GeneratedNodePath,
    next_index: u32,
}

pub(super) struct GeneratedClosurePath {
    segment: String,
    path: GeneratedNodePath,
}

impl GeneratedClosurePathCursor {
    pub(super) fn new(parent: GeneratedNodePath) -> Self {
        Self {
            parent,
            next_index: 0,
        }
    }

    pub(super) fn from_rendered(parent: &[String]) -> Self {
        let parent = GeneratedNodePath::try_from_rendered_segments(parent.iter().cloned())
            .unwrap_or_else(|error| {
                panic!("compiler supplied an invalid generated declaration path: {error}")
            });
        Self::new(parent)
    }

    pub(super) fn next_closure(&mut self) -> GeneratedClosurePath {
        let index = self.next_index;
        self.next_index = self
            .next_index
            .checked_add(1)
            .expect("generated closure sibling index overflow");
        let segment = format!("closure:{index}");
        let path = self.parent.child(segment.clone());
        GeneratedClosurePath { segment, path }
    }
}

impl GeneratedClosurePath {
    pub(super) fn segment(&self) -> &str {
        &self.segment
    }

    pub(super) fn path(&self) -> &GeneratedNodePath {
        &self.path
    }

    pub(super) fn rendered(&self) -> Vec<String> {
        self.path.segments().to_vec()
    }

    pub(super) fn nested_cursor(&self) -> GeneratedClosurePathCursor {
        GeneratedClosurePathCursor::new(self.path.clone())
    }
}
