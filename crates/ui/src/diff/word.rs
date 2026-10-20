use std::ops::Range;

fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut result: Vec<Range<usize>> = Vec::new();
    let mut previous = None;
    for (offset, ch) in text.char_indices() {
        let kind = if ch.is_alphanumeric() || ch == '_' {
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

fn append(ranges: &mut Vec<Range<usize>>, range: &Range<usize>) {
    if let Some(last) = ranges.last_mut()
        && last.end == range.start
    {
        last.end = range.end;
    } else {
        ranges.push(range.clone());
    }
}

pub(super) fn changes(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old == new {
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
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    // ponytail: cap the LCS table at 2 MB; longer replacements use one span per side.
    if (n + 1).saturating_mul(m + 1) > 1_000_000 {
        for range in &a[start..end_a] {
            append(&mut removed, range);
        }
        for range in &b[start..end_b] {
            append(&mut added, range);
        }
        return (removed, added);
    }
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
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && equal(start + i, start + j) {
            i += 1;
            j += 1;
        } else if i < n && (j == m || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1]) {
            append(&mut removed, &a[start + i]);
            i += 1;
        } else {
            append(&mut added, &b[start + j]);
            j += 1;
        }
    }
    (removed, added)
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
        let (removed, added) = changes("α = value", "α = new_value + value");
        assert!(removed.is_empty());
        assert_eq!(added, vec![5..17]);
        assert_eq!(changes("same", "same"), (vec![], vec![]));
        assert_eq!(changes("", "🙂"), (vec![], vec![0..4]));
        assert_eq!(changes("🙂", ""), (vec![0..4], vec![]));
    }
}
