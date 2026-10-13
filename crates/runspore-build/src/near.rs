//! The closest name to a typo, for fixes that rename it.

/// Edit distance with adjacent swaps (optimal string alignment), over characters.
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The candidate closest to `word` when it is close enough to be a typo of it: at
/// most a third of its length away (at least 1), ignoring case. Ties go to the
/// first candidate.
pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let lower = word.to_lowercase();
    let limit = (word.chars().count() / 3).max(1);
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        if candidate == word {
            continue;
        }
        let d = distance(&lower, &candidate.to_lowercase());
        if d <= limit && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, candidate));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_typos_and_nothing_else() {
        assert_eq!(closest("efect", ["effect", "kind"]), Some("effect"));
        assert_eq!(closest("uplaod", ["upload", "dump"]), Some("upload"));
        assert_eq!(closest("Kind", ["kind"]), Some("kind"));
        assert_eq!(closest("banana", ["effect", "kind"]), None);
        assert_eq!(distance("ab", "ba"), 1);
    }
}
