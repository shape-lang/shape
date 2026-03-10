use crate::doc_links::{render_doc_link_target, resolve_doc_link};
use crate::module_cache::ModuleCache;
use shape_ast::ast::{DocComment, DocTag, DocTagKind, Program};
use std::path::Path;

pub fn render_doc_comment(
    program: &Program,
    comment: &DocComment,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> String {
    let mut sections = Vec::new();

    if !comment.body.is_empty() {
        sections.push(comment.body.clone());
    } else if !comment.summary.is_empty() {
        sections.push(comment.summary.clone());
    }

    push_named_section(
        &mut sections,
        "Type Parameters",
        comment
            .tags
            .iter()
            .filter(|tag| matches!(tag.kind, DocTagKind::TypeParam))
            .collect(),
    );
    push_named_section(
        &mut sections,
        "Parameters",
        comment
            .tags
            .iter()
            .filter(|tag| matches!(tag.kind, DocTagKind::Param))
            .collect(),
    );
    push_singleton_section(
        &mut sections,
        "Returns",
        tag_body(comment, DocTagKind::Returns),
    );
    push_singleton_section(
        &mut sections,
        "Deprecated",
        tag_body(comment, DocTagKind::Deprecated),
    );
    push_singleton_section(&mut sections, "Since", tag_body(comment, DocTagKind::Since));

    let notes = comment
        .tags
        .iter()
        .filter(|tag| matches!(tag.kind, DocTagKind::Note))
        .map(|tag| tag.body.trim())
        .filter(|body| !body.is_empty())
        .map(|body| format!("- {body}"))
        .collect::<Vec<_>>();
    if !notes.is_empty() {
        sections.push(format!("**Notes**\n{}", notes.join("\n")));
    }

    let related = comment
        .tags
        .iter()
        .filter(|tag| matches!(tag.kind, DocTagKind::See | DocTagKind::Link))
        .filter_map(|tag| {
            let link = tag.link.as_ref()?;
            let resolved = resolve_doc_link(
                program,
                &link.target,
                module_cache,
                current_file,
                workspace_root,
            );
            let rendered =
                render_doc_link_target(&link.target, link.label.as_deref(), resolved.as_ref());
            Some(format!("- {rendered}"))
        })
        .collect::<Vec<_>>();
    if !related.is_empty() {
        sections.push(format!("**See Also**\n{}", related.join("\n")));
    }

    let examples = comment
        .tags
        .iter()
        .filter(|tag| matches!(tag.kind, DocTagKind::Example))
        .map(|tag| tag.body.trim())
        .filter(|body| !body.is_empty())
        .map(|body| format!("```shape\n{body}\n```"))
        .collect::<Vec<_>>();
    if !examples.is_empty() {
        sections.push(format!("**Examples**\n{}", examples.join("\n\n")));
    }

    sections
        .into_iter()
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn push_named_section(sections: &mut Vec<String>, title: &str, tags: Vec<&DocTag>) {
    if tags.is_empty() {
        return;
    }
    let lines = tags
        .into_iter()
        .map(|tag| {
            let name = tag.name.as_deref().unwrap_or("_");
            format!("- `{name}`: {}", tag.body)
        })
        .collect::<Vec<_>>()
        .join("\n");
    sections.push(format!("**{title}**\n{lines}"));
}

fn push_singleton_section(sections: &mut Vec<String>, title: &str, body: Option<&str>) {
    let Some(body) = body.filter(|body| !body.trim().is_empty()) else {
        return;
    };
    sections.push(format!("**{title}**\n{body}"));
}

fn tag_body(comment: &DocComment, kind: DocTagKind) -> Option<&str> {
    comment.tags.iter().find_map(|tag| {
        if tag.kind == kind {
            Some(tag.body.as_str())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn renders_multiple_examples() {
        let program = parse_program(
            "/// Summary\n/// @example\n/// one()\n/// @example\n/// two()\nfn sample() {}\n",
        )
        .expect("program");
        let comment = program.docs.comment_for_path("sample").expect("doc");
        let markdown = render_doc_comment(&program, comment, None, None, None);
        assert!(markdown.contains("one()"));
        assert!(markdown.contains("two()"));
    }
}
