use super::Kind;
use std::ops::Range;

const MAX_LINE_UNITS: usize = 1000;

fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut result: Vec<Range<usize>> = Vec::new();
    let mut previous = None;
    let mut chars = text.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        let mut end = offset + ch.len_utf8();
        let kind = if ch == '\r' && chars.peek().is_some_and(|(_, ch)| *ch == '\n') {
            end = chars.next().unwrap().0 + 1;
            2
        } else if ch.is_ascii_alphanumeric()
            || ch == '_'
            || matches!(ch as u32,
                0xad | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2c6 |
                0x2c8..=0x2d7 | 0x2de..=0x2ff | 0x1e00..=0x1eff)
        {
            0
        } else if matches!(
            ch,
            '\t' | '\u{b}' | '\u{c}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
                    | '\u{feff}'
        ) {
            1
        } else {
            2
        };
        if kind != 2 && previous == Some(kind) {
            result.last_mut().unwrap().end = end;
        } else {
            result.push(offset..end);
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
            if changing && index + 1 < runs.len() {
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

// Each trace row has d + 1 diagonals; the input limit keeps positions inside i16.
fn previous(
    trace: &[i16],
    distance: usize,
    slot: usize,
    old_len: usize,
    new_len: usize,
) -> Option<(Kind, i32, i32)> {
    let base = (distance - 1) * distance / 2;
    let diagonal = slot as i32 * 2 - distance as i32;
    let remove = slot
        .checked_sub(1)
        .map(|index| i32::from(trace[base + index]))
        .filter(|old| *old != i32::from(i16::MIN) && *old + 1 < old_len as i32);
    let add = (slot < distance)
        .then(|| i32::from(trace[base + slot]))
        .filter(|old| {
            *old != i32::from(i16::MIN) && *old - diagonal >= 0 && *old - diagonal < new_len as i32
        });
    if let Some(old) = add.filter(|old| remove.is_none_or(|removed| removed < *old)) {
        Some((Kind::Added, old, old - diagonal - 1))
    } else {
        remove.map(|old| (Kind::Removed, old, old - diagonal + 1))
    }
}

/// Returns original word-alt highlights as UTF-8 byte ranges for two line contents.
/// Final LF or CRLF terminators are ignored.
/// Equal lines and lines over 1,000 UTF-16 units return no highlights.
///
/// ```
/// let (removed, added) = ui::screens::diff::panel::word::changes("red blue", "red green");
/// assert_eq!(removed, vec![4..8]);
/// assert_eq!(added, vec![4..9]);
/// ```
pub fn changes(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let old = old
        .strip_suffix("\r\n")
        .or_else(|| old.strip_suffix('\n'))
        .unwrap_or(old);
    let new = new
        .strip_suffix("\r\n")
        .or_else(|| new.strip_suffix('\n'))
        .unwrap_or(new);
    if old.encode_utf16().take(MAX_LINE_UNITS + 1).count() > MAX_LINE_UNITS
        || new.encode_utf16().take(MAX_LINE_UNITS + 1).count() > MAX_LINE_UNITS
        || old == new
    {
        return (Vec::new(), Vec::new());
    }
    let a = tokens(old);
    let b = tokens(new);
    let equal = |i: i32, j: i32| old[a[i as usize].clone()] == new[b[j as usize].clone()];
    let mut first = -1i32;
    while first + 1 < a.len().min(b.len()) as i32 && equal(first + 1, first + 1) {
        first += 1;
    }
    let mut trace = vec![first as i16];
    let mut distance = 0;
    let distance = 'search: loop {
        distance += 1;
        assert!(
            distance <= a.len() + b.len(),
            "word edit path must reach both line ends"
        );
        let base = distance * (distance + 1) / 2;
        trace.resize(base + distance + 1, i16::MIN);
        for slot in 0..=distance {
            let Some((kind, mut i, mut j)) = previous(&trace, distance, slot, a.len(), b.len())
            else {
                continue;
            };
            i += i32::from(kind == Kind::Removed);
            j += i32::from(kind == Kind::Added);
            while i + 1 < a.len() as i32 && j + 1 < b.len() as i32 && equal(i + 1, j + 1) {
                i += 1;
                j += 1;
            }
            trace[base + slot] = i as i16;
            if i + 1 == a.len() as i32 && j + 1 == b.len() as i32 {
                break 'search distance;
            }
        }
    };
    let mut edits = Vec::new();
    let (mut i, mut j) = (a.len() as i32 - 1, b.len() as i32 - 1);
    for distance in (1..=distance).rev() {
        let slot = ((i - j + distance as i32) / 2) as usize;
        let (kind, old, new) = previous(&trace, distance, slot, a.len(), b.len()).unwrap();
        let common = i - old - i32::from(kind == Kind::Removed);
        if common > 0 {
            edits.push((Kind::Context, common as usize));
        }
        edits.push((kind, 1));
        (i, j) = (old, new);
    }
    if first >= 0 {
        edits.push((Kind::Context, first as usize + 1));
    }
    let old_at = |i: usize| a.get(i).map_or(old.len(), |range| range.start);
    let new_at = |i: usize| b.get(i).map_or(new.len(), |range| range.start);
    let mut runs = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (kind, count) in edits.into_iter().rev() {
        let next_i = i + if kind == Kind::Added { 0 } else { count };
        let next_j = j + if kind == Kind::Removed { 0 } else { count };
        push(
            &mut runs,
            kind,
            old_at(i)..old_at(next_i),
            new_at(j)..new_at(next_j),
        );
        (i, j) = (next_i, next_j);
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
        assert_eq!(
            changes("  β", " β β"),
            (std::iter::once(0..4).collect::<Vec<_>>(), vec![0..3, 3..6])
        );
    }

    #[test]
    fn insertions_and_deletions_keep_utf8_boundaries() {
        assert_eq!(
            changes("α = value", "α = new_value + value"),
            (vec![], std::iter::once(5..17).collect::<Vec<_>>())
        );
        assert_eq!(
            changes("αβ", "αxβ"),
            (vec![], std::iter::once(2..3).collect::<Vec<_>>())
        );
        assert_eq!(changes("same", "same"), (vec![], vec![]));
        assert_eq!(
            changes("", "🙂"),
            (vec![], std::iter::once(0..4).collect::<Vec<_>>())
        );
        assert_eq!(
            changes("🙂", ""),
            (std::iter::once(0..4).collect::<Vec<_>>(), vec![])
        );
        assert_eq!(changes("same\r\n", "same\n"), (vec![], vec![]));
        assert_eq!(
            changes("old\r\n", "new\n"),
            (
                std::iter::once(0..3).collect::<Vec<_>>(),
                std::iter::once(0..3).collect::<Vec<_>>()
            )
        );
    }

    #[test]
    fn word_alt_joins_single_character_gaps_and_caps_long_lines() {
        assert_eq!(
            changes("a+x", "b+y"),
            (std::iter::once(0..3).collect::<Vec<_>>(), vec![0..2, 2..3])
        );
        assert_eq!(
            changes(&"(".repeat(1000), &")".repeat(1000)),
            (
                std::iter::once(0..1000).collect::<Vec<_>>(),
                std::iter::once(0..1000).collect::<Vec<_>>()
            ),
        );
        assert_eq!(changes(&"🙂".repeat(501), "new"), (vec![], vec![]),);
        assert_eq!(
            changes(&"a".repeat(1001), &"b".repeat(1001)),
            (vec![], vec![])
        );
    }
}
