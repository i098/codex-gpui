//! TeX math as Unicode text: the original typesets `$…$` and `$$…$$` with KaTeX; here the
//! common commands become their symbols and scripts become Unicode super- and subscripts.
//! shortcut: no 2-D layout (fractions are `a/b`, matrices flat); add a math layout engine
//! when answers need stacked notation.

use std::iter::Peekable;
use std::str::Chars;

/// `tex` as text; `\\` line breaks stay `\n`.
pub(super) fn text(tex: &str) -> String {
    let mut chars = tex.chars().peekable();
    let out = group(&mut chars);
    // TeX ignores source spacing; keep one space where the author wrote any.
    let mut collapsed = String::with_capacity(out.len());
    for c in out.trim().chars() {
        if c == ' ' && collapsed.ends_with([' ', '\n']) {
            continue;
        }
        if c == '\n' && collapsed.ends_with(' ') {
            collapsed.pop();
        }
        collapsed.push(c);
    }
    collapsed
}

/// Atoms up to the closing `}` of the current group (consumed) or the end.
fn group(chars: &mut Peekable<Chars>) -> String {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '}' => break,
            '{' => out.push_str(&group(chars)),
            '\\' => out.push_str(&command(chars)),
            '^' => out.push_str(&script(chars, true)),
            '_' => out.push_str(&script(chars, false)),
            '\'' => out.push('′'),
            '-' => out.push('−'),
            '*' => out.push('∗'),
            '~' | '&' | '\t' | '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The next argument: a group, a command, or one character.
fn argument(chars: &mut Peekable<Chars>) -> String {
    while chars.next_if(|c| c.is_whitespace()).is_some() {}
    match chars.next() {
        Some('{') => group(chars),
        Some('\\') => command(chars),
        Some(c) => c.to_string(),
        None => String::new(),
    }
}

/// The raw text of the next `{…}` argument, for `\text`.
fn raw(chars: &mut Peekable<Chars>) -> String {
    while chars.next_if(|c| c.is_whitespace()).is_some() {}
    if chars.next_if_eq(&'{').is_none() {
        return chars.next().map(String::from).unwrap_or_default();
    }
    let mut out = String::new();
    let mut depth = 0;
    for c in chars.by_ref() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => break,
            '}' => depth -= 1,
            _ => out.push(c),
        }
    }
    out
}

/// `a` as a fraction or root operand: parenthesized unless it is one symbol or a number.
fn operand(a: &str) -> String {
    if a.chars().count() <= 1 || a.chars().all(|c| c.is_alphanumeric() || c == '.') {
        a.to_owned()
    } else {
        format!("({a})")
    }
}

fn script(chars: &mut Peekable<Chars>, sup: bool) -> String {
    let body = argument(chars);
    let mapped: Option<String> = body
        .chars()
        .map(|c| if sup { superscript(c) } else { subscript(c) })
        .collect();
    match mapped {
        Some(mapped) if !body.is_empty() => mapped,
        _ if body.chars().count() == 1 => format!("{}{body}", if sup { '^' } else { '_' }),
        _ => format!("{}({body})", if sup { '^' } else { '_' }),
    }
}

fn command(chars: &mut Peekable<Chars>) -> String {
    let mut name = String::new();
    while let Some(c) = chars.next_if(|c| c.is_ascii_alphabetic()) {
        name.push(c);
    }
    if name.is_empty() {
        return match chars.next() {
            Some('\\') => "\n".into(),
            Some(',' | ':' | ';' | ' ') => " ".into(),
            Some('!') | None => String::new(),
            Some('|') => "‖".into(),
            Some(c) => c.to_string(),
        };
    }
    match name.as_str() {
        "frac" | "dfrac" | "tfrac" | "cfrac" => {
            let (a, b) = (argument(chars), argument(chars));
            format!("{}/{}", operand(&a), operand(&b))
        }
        "binom" => {
            let (a, b) = (argument(chars), argument(chars));
            format!("C({a}, {b})")
        }
        "sqrt" => {
            let index = chars.next_if_eq(&'[').map(|_| {
                let index: String = chars.by_ref().take_while(|&c| c != ']').collect();
                index
            });
            let root = match index.as_deref() {
                None | Some("2") => "√",
                Some("3") => "∛",
                Some("4") => "∜",
                Some(n) => return format!("{n}√{}", operand(&argument(chars))),
            };
            format!("{root}{}", operand(&argument(chars)))
        }
        "text" | "textrm" | "textnormal" | "textbf" | "textit" | "mbox" | "operatorname" => {
            raw(chars)
        }
        "mathrm" | "mathit" | "mathbf" | "mathsf" | "mathtt" | "boldsymbol" | "bm" | "mathcal"
        | "displaystyle" | "textstyle" | "limits" | "nolimits" => String::new(),
        "mathbb" => argument(chars).chars().map(double_struck).collect(),
        "left" | "right" | "bigl" | "bigr" | "Bigl" | "Bigr" | "big" | "Big" | "bigg" | "Bigg" => {
            // `\left.` is an invisible delimiter.
            chars.next_if_eq(&'.');
            String::new()
        }
        "quad" | "qquad" => "  ".into(),
        "begin" | "end" => {
            raw(chars);
            String::new()
        }
        "hat" | "bar" | "vec" | "tilde" | "dot" | "overline" => {
            let mark = match name.as_str() {
                "hat" => '\u{302}',
                "bar" | "overline" => '\u{305}',
                "vec" => '\u{20D7}',
                "tilde" => '\u{303}',
                _ => '\u{307}',
            };
            let base = argument(chars);
            if base.chars().count() == 1 {
                format!("{base}{mark}")
            } else {
                base
            }
        }
        name => match symbol(name) {
            Some(symbol) => symbol.into(),
            None if OPERATORS.contains(&name) => name.into(),
            None => format!("\\{name}"),
        },
    }
}

const OPERATORS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "log", "ln", "lg", "exp", "lim", "liminf", "limsup", "max", "min", "sup", "inf", "det", "dim",
    "deg", "gcd", "arg", "ker", "Pr", "mod",
];

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ϵ",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "ϕ",
        "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "cdot" => "·",
        "times" => "×",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "∙",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "ll" => "≪",
        "gg" => "≫",
        "approx" => "≈",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "equiv" => "≡",
        "propto" => "∝",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "coprod" => "∐",
        "int" => "∫",
        "iint" => "∬",
        "iiint" => "∭",
        "oint" => "∮",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "emptyset" | "varnothing" => "∅",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" | "iff" => "⇔",
        "mapsto" => "↦",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "ldots" | "dots" | "dotsc" => "…",
        "cdots" | "dotsb" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "prime" => "′",
        "degree" => "°",
        "angle" => "∠",
        "perp" => "⊥",
        "parallel" => "∥",
        "mid" => "|",
        "vert" => "|",
        "Vert" => "‖",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lbrace" => "{",
        "rbrace" => "}",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "therefore" => "∴",
        "because" => "∵",
        _ => return None,
    })
}

fn double_struck(c: char) -> char {
    match c {
        'C' => 'ℂ',
        'H' => 'ℍ',
        'N' => 'ℕ',
        'P' => 'ℙ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'Z' => 'ℤ',
        c => c,
    }
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '−' | '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'T' => 'ᵀ',
        '′' => '′',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '−' | '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::text;

    #[test]
    fn scripts_use_unicode_forms_and_fall_back_to_carets() {
        assert_eq!(text("x^2 + y_{i+1}"), "x² + yᵢ₊₁");
        assert_eq!(text("e^{i\\pi} = -1"), "e^(iπ) = −1");
        assert_eq!(text("2^{\\alpha}"), "2^α");
        assert_eq!(text("f'(x)"), "f′(x)");
    }

    #[test]
    fn fractions_and_roots_parenthesize_compound_operands() {
        assert_eq!(text("\\frac{a+b}{2}"), "(a+b)/2");
        assert_eq!(text("\\frac12"), "1/2");
        assert_eq!(text("\\sqrt{x^2+1}"), "√(x²+1)");
        assert_eq!(text("\\sqrt[3]{8}"), "∛8");
    }

    #[test]
    fn commands_map_to_symbols_and_unknown_ones_stay_visible() {
        assert_eq!(
            text("\\sum_{i=1}^{n} \\alpha_i \\leq \\infty"),
            "∑ᵢ₌₁ⁿ αᵢ ≤ ∞"
        );
        assert_eq!(text("\\mathbb{R}^n \\to \\mathbf{x}"), "ℝⁿ → x");
        assert_eq!(text("\\text{if } x \\neq 0"), "if x ≠ 0");
        assert_eq!(text("\\left( a \\right."), "( a");
        assert_eq!(text("\\foo{x}"), "\\foox");
        assert_eq!(text("a \\\\ b"), "a\nb");
    }
}
