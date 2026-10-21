use super::{Hunk, Kind};
use gpui::{HighlightStyle, Hsla, rgb};
use std::{ops::Range, sync::LazyLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color, StyleModifier, Theme as SyntaxTheme, ThemeItem, ThemeSettings},
    parsing::SyntaxSet,
};
use theme::{Appearance, Theme};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Token {
    Keyword,
    Name,
    String,
    Comment,
    Literal,
    Parameter,
    Punctuation,
    Operator,
}

impl Token {
    pub(super) fn style(self, theme: &Theme) -> HighlightStyle {
        let (dark, light) = match self {
            Self::Keyword => (0xf67576, 0xd53538),
            Self::Name => (0xb06dff, 0x751ed9),
            Self::String => (0x83d197, 0x3a843f),
            Self::Comment => (0xb9b9b9, 0x4f4f4f),
            Self::Literal => (0xf1a275, 0xac4f23),
            Self::Parameter => (0xfa994c, 0xbd5800),
            Self::Punctuation => (0x999999, 0x666666),
            Self::Operator => (0x6dcbf4, 0x0071ea),
        };
        let color: Hsla = rgb(if theme.appearance == Appearance::Dark {
            dark
        } else {
            light
        })
        .into();
        HighlightStyle {
            color: Some(color),
            ..Default::default()
        }
    }
}

pub(super) type Mark = (Range<usize>, Token);
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_no_newlines);
static TOKENS: [Token; 8] = [
    Token::Keyword,
    Token::Name,
    Token::String,
    Token::Comment,
    Token::Literal,
    Token::Parameter,
    Token::Punctuation,
    Token::Operator,
];
static THEME: LazyLock<SyntaxTheme> = LazyLock::new(|| {
    SyntaxTheme {
    settings: ThemeSettings {
        foreground: Some(Color { r: 0, g: 0, b: 0, a: 255 }),
        ..Default::default()
    },
    scopes: [
        ("keyword, storage", 1),
        ("entity.name.function, entity.name.class, entity.name.type, storage.type.function.python, keyword.declaration.function.python, storage.type.class.python", 2),
        ("string", 3),
        ("comment", 4),
        ("constant, support.function, support.type", 5),
        ("variable.parameter", 6),
        ("punctuation", 7),
        ("keyword.operator", 8),
    ].into_iter().map(|(scope, code)| ThemeItem {
        scope: scope.parse().expect("fixed syntax scopes"),
        style: StyleModifier {
            foreground: Some(Color { r: code, g: 0, b: 0, a: 255 }),
            ..Default::default()
        },
    }).collect(),
    ..Default::default()
}
});

fn line(highlighter: &mut HighlightLines<'_>, text: &str) -> Vec<Mark> {
    let Ok(tokens) = highlighter.highlight_line(text, &SYNTAXES) else {
        return Vec::new();
    };
    let mut offset = 0;
    tokens
        .into_iter()
        .filter_map(|(style, text)| {
            let start = offset;
            offset += text.len();
            style
                .foreground
                .r
                .checked_sub(1)
                .and_then(|index| TOKENS.get(usize::from(index)))
                .map(|token| (start..offset, *token))
        })
        .collect()
}

fn highlighter(path: &str, first: Option<&str>) -> HighlightLines<'static> {
    let extension = path.rsplit('.').next().unwrap_or(path);
    let syntax = SYNTAXES
        .find_syntax_by_extension(extension)
        .or_else(|| SYNTAXES.find_syntax_by_name(path))
        .or_else(|| first.and_then(|line| SYNTAXES.find_syntax_by_first_line(line)))
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    HighlightLines::new(syntax, &THEME)
}

pub(super) fn full_file(path: &str, contents: &str) -> Vec<Vec<Mark>> {
    let mut state = highlighter(path, super::controls::lines(contents).next());
    super::controls::lines(contents)
        .map(|text| line(&mut state, text))
        .collect()
}

pub(super) fn apply(path: &str, hunks: &mut [Hunk]) {
    let first = hunks
        .first()
        .and_then(|hunk| hunk.lines.first())
        .map(|line| line.text.as_ref());
    let mut old = highlighter(path, first);
    let mut new = highlighter(path, first);
    for hunk in hunks {
        for item in &mut hunk.lines {
            item.syntax = match item.kind {
                Kind::Removed => line(&mut old, &item.text),
                Kind::Added => line(&mut new, &item.text),
                Kind::Context => {
                    line(&mut old, &item.text);
                    line(&mut new, &item.text)
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::diff::panel::FileDiff;

    #[test]
    fn syntax_follows_the_file_language_and_keeps_multiline_state() {
        let rust = FileDiff::patch(
            "src/lib.rs",
            "@@ -0,0 +1,3 @@\n+/* start\n+still comment */\n+fn main() { let count = 12; }\n",
        );
        assert!(
            rust.hunks[0].lines[1]
                .syntax
                .iter()
                .any(|(_, token)| *token == Token::Comment)
        );
        let last = &rust.hunks[0].lines[2];
        assert!(last.syntax.iter().any(|(range, token)| *token == Token::Keyword && &last.text[range.clone()] == "fn"));
        assert!(last.syntax.iter().any(|(range, token)| *token == Token::Literal && &last.text[range.clone()] == "12"));
        let typescript = FileDiff::patch(
            "src/app.ts",
            "@@ -0,0 +1,2 @@\n+interface State { ready: boolean }\n+const value: State = { ready: true };\n",
        );
        let declaration = &typescript.hunks[0].lines[0];
        assert!(
            declaration
                .syntax
                .iter()
                .any(|(range, token)| *token == Token::Keyword
                    && &declaration.text[range.clone()] == "interface")
        );
        let value = &typescript.hunks[0].lines[1];
        assert!(
            value
                .syntax
                .iter()
                .any(|(range, token)| *token == Token::Literal
                    && &value.text[range.clone()] == "true")
        );
        let python = FileDiff::patch(
            "math.py",
            "@@ -0,0 +1,2 @@\n+def subtract(a, b):\n+    return a - b\n",
        );
        let definition = &python.hunks[0].lines[0];
        assert!(definition.syntax.iter().any(
            |(range, token)| *token == Token::Name && &definition.text[range.clone()] == "def"
        ));
        let text = FileDiff::patch("notes.txt", "@@ -0,0 +1 @@\n+def return class 12\n");
        assert!(text.hunks[0].lines[0].syntax.is_empty());
    }
}
