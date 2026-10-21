use super::Kind;
use std::ops::Range;

fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut result: Vec<Range<usize>> = Vec::new();
    let mut previous = None;
    for (offset, ch) in text.char_indices() {
        let kind = if ch.is_ascii_alphanumeric()
            || ch == '_'
            || matches!(ch as u32,
            0xad | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2c6 | 0x2c8..=0x2d7 | 0x2de..=0x2ff | 0x1e00..=0x1eff)
        {
            0
        } else if ch.is_whitespace() {
            1
        } else {
            2
        };
        if kind != 2 && previous == Some(kind) {
            result.last_mut().unwrap().end = offset + ch.len_utf8();
        } else {
            result.push(offset..offset + ch.len_utf8());
        }
        previous = Some(kind);
    }
    result
}

struct Run {
    kind: Kind,
    old: Range<usize>,
    new: Range<usize>,
}

fn push(runs: &mut Vec<Run>, kind: Kind, old: Range<usize>, new: Range<usize>) {
    if let Some(last) = runs.last_mut()
        && last.kind == kind
    {
        last.old.end = old.end;
        last.new.end = new.end;
    } else {
        runs.push(Run { kind, old, new });
    }
}

fn marks(runs: &[Run], text: &str, side: Kind) -> Vec<Range<usize>> {
    let mut marks: Vec<Range<usize>> = Vec::new();
    let mut changing = false;
    for (index, run) in runs.iter().enumerate() {
        let range = if side == Kind::Removed {
            &run.old
        } else {
            &run.new
        };
        if run.kind == side {
            if changing {
                marks.last_mut().unwrap().end = range.end;
            } else {
                marks.push(range.clone());
            }
            changing = true;
        } else if run.kind == Kind::Context {
            // The original word-alt mode joins a nonterminal one-character equal run.
            if changing && index + 1 < runs.len() && text[range.clone()].encode_utf16().count() == 1
            {
                marks.last_mut().unwrap().end = range.end;
            } else {
                changing = false;
            }
        }
    }
    marks
}

pub(super) fn changes(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old == new
        || old.encode_utf16().take(1001).count() > 1000
        || new.encode_utf16().take(1001).count() > 1000
    {
        return (Vec::new(), Vec::new());
    }
    let a = tokens(old);
    let b = tokens(new);
    let equal = |i: usize, j: usize| old[a[i].clone()] == new[b[j].clone()];
    let mut start = 0;
    while start < a.len().min(b.len()) && equal(start, start) {
        start += 1;
    }
    let (mut end_a, mut end_b) = (a.len(), b.len());
    while end_a > start && end_b > start && equal(end_a - 1, end_b - 1) {
        end_a -= 1;
        end_b -= 1;
    }
    let (n, m) = (end_a - start, end_b - start);
    let width = m + 1;
    let mut lengths = vec![0u16; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i * width + j] = if equal(start + i, start + j) {
                1 + lengths[(i + 1) * width + j + 1]
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let old_at = |i: usize| a.get(i).map_or(old.len(), |range| range.start);
    let new_at = |i: usize| b.get(i).map_or(new.len(), |range| range.start);
    let mut runs = Vec::new();
    if start > 0 {
        push(&mut runs, Kind::Context, 0..old_at(start), 0..new_at(start));
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && equal(start + i, start + j) {
            push(
                &mut runs,
                Kind::Context,
                a[start + i].clone(),
                b[start + j].clone(),
            );
            i += 1;
            j += 1;
        } else if i < n && (j == m || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1]) {
            push(
                &mut runs,
                Kind::Removed,
                a[start + i].clone(),
                new_at(start + j)..new_at(start + j),
            );
            i += 1;
        } else {
            push(
                &mut runs,
                Kind::Added,
                old_at(start + i)..old_at(start + i),
                b[start + j].clone(),
            );
            j += 1;
        }
    }
    if end_a < a.len() || end_b < b.len() {
        push(
            &mut runs,
            Kind::Context,
            old_at(end_a)..old.len(),
            new_at(end_b)..new.len(),
        );
    }
    (
        marks(&runs, old, Kind::Removed),
        marks(&runs, new, Kind::Added),
    )
}

#[cfg(test)]
mod tests {
    use super::changes;

    #[test]
    fn replacements_keep_equal_words_between_separate_changes() {
        let old = "let first = old; let second = before;";
        let new = "let first = new; let second = after;";
        let (removed, added) = changes(old, new);
        assert_eq!(
            removed.iter().map(|r| &old[r.clone()]).collect::<Vec<_>>(),
            ["old", "before"]
        );
        assert_eq!(
            added.iter().map(|r| &new[r.clone()]).collect::<Vec<_>>(),
            ["new", "after"]
        );
    }

    #[test]
    fn insertions_and_deletions_keep_utf8_boundaries() {
        assert_eq!(
            changes("α = value", "α = new_value + value"),
            (vec![], vec![5..17])
        );
        assert_eq!(changes("αβ", "αxβ"), (vec![], vec![2..3]));
        assert_eq!(changes("same", "same"), (vec![], vec![]));
        assert_eq!(changes("", "🙂"), (vec![], vec![0..4]));
        assert_eq!(changes("🙂", ""), (vec![0..4], vec![]));
    }

    #[test]
    fn word_alt_joins_single_character_gaps_and_caps_long_lines() {
        assert_eq!(changes("a+x", "b+y"), (vec![0..3], vec![0..3]));
        assert_eq!(
            changes(&"a".repeat(1001), &"b".repeat(1001)),
            (vec![], vec![])
        );
    }
}
