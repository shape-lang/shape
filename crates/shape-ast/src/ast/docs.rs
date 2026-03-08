use super::span::Span;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocTagKind {
    Module,
    TypeParam,
    Param,
    Returns,
    Throws,
    Deprecated,
    Requires,
    Since,
    See,
    Link,
    Note,
    Example,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocLink {
    pub target: String,
    #[serde(default)]
    pub target_span: Span,
    pub label: Option<String>,
    #[serde(default)]
    pub label_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocTag {
    pub kind: DocTagKind,
    #[serde(default)]
    pub span: Span,
    #[serde(default)]
    pub kind_span: Span,
    pub name: Option<String>,
    #[serde(default)]
    pub name_span: Option<Span>,
    pub body: String,
    #[serde(default)]
    pub body_span: Option<Span>,
    pub link: Option<DocLink>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocComment {
    #[serde(default)]
    pub span: Span,
    pub summary: String,
    pub body: String,
    pub tags: Vec<DocTag>,
}

impl DocComment {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty() && self.body.is_empty() && self.tags.is_empty()
    }

    pub fn param_doc(&self, name: &str) -> Option<&str> {
        self.tags.iter().find_map(|tag| match &tag.kind {
            DocTagKind::Param if tag.name.as_deref() == Some(name) => Some(tag.body.as_str()),
            _ => None,
        })
    }

    pub fn type_param_doc(&self, name: &str) -> Option<&str> {
        self.tags.iter().find_map(|tag| match &tag.kind {
            DocTagKind::TypeParam if tag.name.as_deref() == Some(name) => {
                Some(tag.body.as_str())
            }
            _ => None,
        })
    }

    pub fn returns_doc(&self) -> Option<&str> {
        self.tags.iter().find_map(|tag| match tag.kind {
            DocTagKind::Returns => Some(tag.body.as_str()),
            _ => None,
        })
    }

    pub fn deprecated_doc(&self) -> Option<&str> {
        self.tags.iter().find_map(|tag| match tag.kind {
            DocTagKind::Deprecated => Some(tag.body.as_str()),
            _ => None,
        })
    }

    pub fn example_doc(&self) -> Option<&str> {
        self.tags.iter().find_map(|tag| match tag.kind {
            DocTagKind::Example => Some(tag.body.as_str()),
            _ => None,
        })
    }

    pub fn since_doc(&self) -> Option<&str> {
        self.tags.iter().find_map(|tag| match tag.kind {
            DocTagKind::Since => Some(tag.body.as_str()),
            _ => None,
        })
    }

    pub fn to_markdown(&self) -> String {
        let mut sections = Vec::new();
        if !self.body.is_empty() {
            sections.push(self.body.clone());
        } else if !self.summary.is_empty() {
            sections.push(self.summary.clone());
        }

        let type_params: Vec<_> = self
            .tags
            .iter()
            .filter(|tag| matches!(tag.kind, DocTagKind::TypeParam))
            .collect();
        if !type_params.is_empty() {
            sections.push(render_named_section("Type Parameters", &type_params));
        }

        let params: Vec<_> = self
            .tags
            .iter()
            .filter(|tag| matches!(tag.kind, DocTagKind::Param))
            .collect();
        if !params.is_empty() {
            sections.push(render_named_section("Parameters", &params));
        }

        if let Some(returns) = self.returns_doc() {
            sections.push(format!("**Returns**\n{}", returns));
        }

        if let Some(deprecated) = self.deprecated_doc() {
            sections.push(format!("**Deprecated**\n{}", deprecated));
        }

        if let Some(since) = self.since_doc() {
            sections.push(format!("**Since**\n{}", since));
        }

        let notes: Vec<_> = self
            .tags
            .iter()
            .filter(|tag| matches!(tag.kind, DocTagKind::Note))
            .map(|tag| tag.body.as_str())
            .filter(|body| !body.trim().is_empty())
            .collect();
        if !notes.is_empty() {
            sections.push(format!(
                "**Notes**\n{}",
                notes
                    .into_iter()
                    .map(|body| format!("- {}", body))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        let related: Vec<_> = self
            .tags
            .iter()
            .filter_map(|tag| match &tag.kind {
                DocTagKind::See | DocTagKind::Link => tag.link.as_ref(),
                _ => None,
            })
            .map(|link| match &link.label {
                Some(label) => format!("- `{}` ({})", link.target, label),
                None => format!("- `{}`", link.target),
            })
            .collect();
        if !related.is_empty() {
            sections.push(format!("**See Also**\n{}", related.join("\n")));
        }

        if let Some(example) = self.example_doc() {
            sections.push(format!("**Example**\n```shape\n{}\n```", example));
        }

        sections
            .into_iter()
            .filter(|section| !section.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn render_named_section(title: &str, tags: &[&DocTag]) -> String {
    let lines = tags
        .iter()
        .map(|tag| {
            let name = tag.name.as_deref().unwrap_or("_");
            format!("- `{}`: {}", name, tag.body)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("**{}**\n{}", title, lines)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocTargetKind {
    Module,
    Function,
    Annotation,
    ForeignFunction,
    BuiltinFunction,
    BuiltinType,
    TypeParam,
    TypeAlias,
    Struct,
    StructField,
    Interface,
    InterfaceProperty,
    InterfaceMethod,
    InterfaceIndexSignature,
    Trait,
    TraitMethod,
    TraitAssociatedType,
    Enum,
    EnumVariant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocTarget {
    pub kind: DocTargetKind,
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEntry {
    pub target: DocTarget,
    pub comment: DocComment,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramDocs {
    pub entries: Vec<DocEntry>,
}

impl ProgramDocs {
    pub fn entry_for_path(&self, path: &str) -> Option<&DocEntry> {
        self.entries.iter().find(|entry| entry.target.path == path)
    }

    pub fn entry_for_span(&self, span: Span) -> Option<&DocEntry> {
        self.entries
            .iter()
            .find(|entry| entry.target.span == span && !span.is_dummy())
    }

    pub fn comment_for_path(&self, path: &str) -> Option<&DocComment> {
        self.entry_for_path(path).map(|entry| &entry.comment)
    }

    pub fn comment_for_span(&self, span: Span) -> Option<&DocComment> {
        self.entry_for_span(span).map(|entry| &entry.comment)
    }
}
