use super::{FullText, Hunk, Kind};
use tree_sitter::{Language, Node, Parser, Point};

const MAX_IMPORT_BYTES: usize = 1_000_000;

pub(super) struct ImportFilter {
    before: Vec<bool>,
    after: Vec<bool>,
    pub(super) hidden: Vec<bool>,
}

impl ImportFilter {
    pub(super) fn new(path: &str, full: &FullText) -> Self {
        Self {
            before: import_rows(path, &full.before),
            after: import_rows(path, &full.after),
            hidden: Vec::new(),
        }
    }

    fn imports_only(&self, hunk: &Hunk) -> bool {
        let mut found = false;
        for line in &hunk.lines {
            let (rows, number) = match line.kind {
                Kind::Context => continue,
                Kind::Added => (&self.after, line.number),
                Kind::Removed => (&self.before, line.old_number),
            };
            if number
                .checked_sub(1)
                .and_then(|index| rows.get(index as usize))
                == Some(&true)
            {
                found = true;
            } else if !line.text.trim().is_empty() {
                return false;
            }
        }
        found
    }

    pub(super) fn update(&mut self, hunks: &[Hunk], keep: impl Fn(&Hunk) -> bool) {
        self.hidden.clear();
        for hunk in hunks {
            let hidden = self.imports_only(hunk) && !keep(hunk);
            self.hidden.push(hidden);
        }
        if self.hidden.iter().any(|hidden| *hidden)
            && hunks
                .iter()
                .zip(&self.hidden)
                .all(|(hunk, hidden)| *hidden || hunk.lines.is_empty())
        {
            self.hidden.fill(true);
        }
    }
}

fn language(path: &str) -> Option<Language> {
    let extension = path.rsplit('.').next()?;
    Some(match () {
        _ if ["js", "jsx", "mjs", "cjs"]
            .iter()
            .any(|name| extension.eq_ignore_ascii_case(name)) =>
        {
            tree_sitter_javascript::LANGUAGE.into()
        }
        _ if ["ts", "mts", "cts"]
            .iter()
            .any(|name| extension.eq_ignore_ascii_case(name)) =>
        {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        }
        _ if extension.eq_ignore_ascii_case("tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ if ["py", "pyi"]
            .iter()
            .any(|name| extension.eq_ignore_ascii_case(name)) =>
        {
            tree_sitter_python::LANGUAGE.into()
        }
        _ if extension.eq_ignore_ascii_case("rs") => tree_sitter_rust::LANGUAGE.into(),
        _ if extension.eq_ignore_ascii_case("go") => tree_sitter_go::LANGUAGE.into(),
        _ if extension.eq_ignore_ascii_case("java") => tree_sitter_java::LANGUAGE.into(),
        _ => return None,
    })
}

fn import_node(node: Node<'_>) -> bool {
    match node.kind() {
        "import_statement"
        | "import_from_statement"
        | "future_import_statement"
        | "import_declaration" => true,
        "use_declaration" | "extern_crate_declaration" => !node
            .named_child(0)
            .is_some_and(|child| child.kind() == "visibility_modifier"),
        _ => false,
    }
}

fn import_rows(path: &str, lines: &[String]) -> Vec<bool> {
    if lines.iter().map(String::len).sum::<usize>() > MAX_IMPORT_BYTES {
        return Vec::new();
    }
    let Some(language) = language(path) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse_with_options(
        &mut |_, point: Point| {
            lines
                .get(point.row)
                .and_then(|line| line.as_bytes().get(point.column..))
                .unwrap_or_default()
        },
        None,
        None,
    ) else {
        return Vec::new();
    };
    let mut rows = vec![false; lines.len()];
    let mut cursor = tree.walk();
    for node in tree.root_node().named_children(&mut cursor) {
        if !import_node(node) || node.has_error() {
            continue;
        }
        let start = node.start_position();
        let mut end = node.end_position();
        if end.column == 0 && end.row > start.row {
            end.row -= 1;
            end.column = lines[end.row].len();
        }
        let mut next = node.next_named_sibling();
        while let Some(comment) = next {
            if !matches!(comment.kind(), "comment" | "line_comment" | "block_comment")
                || comment.start_position().row != end.row
                || comment.end_position().row != end.row
                || !lines[end.row][end.column..comment.start_position().column]
                    .trim()
                    .is_empty()
            {
                break;
            }
            end = comment.end_position();
            next = comment.next_named_sibling();
        }
        if !lines[start.row][..start.column].trim().is_empty()
            || !lines[end.row][end.column..].trim().is_empty()
        {
            continue;
        }
        rows[start.row..=end.row].fill(true);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(path: &str, text: &str) -> Vec<bool> {
        import_rows(
            path,
            &text
                .split_inclusive('\n')
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn imports_cover_supported_languages_without_hiding_other_statements() {
        for (path, text) in [
            ("a.js", "import { x } from './x';\nconst y = x;\n"),
            ("a.ts", "import type { X } from './x';\nconst y = 1;\n"),
            ("a.tsx", "import { X } from './x';\nconst y = <X />;\n"),
            ("a.py", "from x import y\nprint(y)\n"),
            ("a.rs", "use std::io;\nfn main() {}\n"),
            ("A.java", "import java.util.List;\nclass A {}\n"),
        ] {
            assert_eq!(rows(path, text), [true, false], "{path}");
        }
        assert_eq!(
            rows("a.go", "package main\nimport \"fmt\"\nfunc main() {}\n"),
            [false, true, false]
        );
        assert_eq!(
            rows(
                "a.rs",
                "pub use std::io;\nextern crate std;\nfn f() { use std::fs; }\n"
            ),
            [false, true, false]
        );
        assert_eq!(
            rows(
                "a.js",
                "const s = \"import x from 'x'\";\nimport('x');\nimport x from 'x'; call();\n"
            ),
            [false, false, false]
        );
        assert_eq!(
            rows(
                "a.py",
                "from x import (\n a,\n b,\n) # kept comment\nprint(a)\n"
            ),
            [true, true, true, true, false]
        );
    }
}
