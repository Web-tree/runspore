//! Line diff of two pretty-printed JSON texts, for mismatch reports.

/// Unchanged lines kept around each change.
const CONTEXT: usize = 3;
/// Above this many table cells the differing middle is shown as one block.
const MAX_CELLS: usize = 1_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Both,
    Expected,
    Actual,
}

/// Lines only in `expected` are prefixed `- `, lines only in `actual` `+ `, and
/// shared lines two spaces. Unchanged stretches away from any change collapse
/// to `  ...`. Empty when the texts are equal.
pub(crate) fn lines(expected: &str, actual: &str) -> String {
    if expected == actual {
        return String::new();
    }
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];

    let mut script: Vec<(Side, &str)> = old[..prefix].iter().map(|l| (Side::Both, *l)).collect();
    script.extend(middle(old_middle, new_middle));
    script.extend(old[old.len() - suffix..].iter().map(|l| (Side::Both, *l)));
    render(&script)
}

/// Longest-common-subsequence edit script.
fn middle<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Side, &'a str)> {
    let mut script = Vec::with_capacity(old.len() + new.len());
    if old.len().saturating_mul(new.len()) > MAX_CELLS {
        script.extend(old.iter().map(|l| (Side::Expected, *l)));
        script.extend(new.iter().map(|l| (Side::Actual, *l)));
        return script;
    }
    let width = new.len() + 1;
    // common[i * width + j] is the LCS length of old[i..] and new[j..].
    let mut common = vec![0u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i * width + j] = if old[i] == new[j] {
                common[(i + 1) * width + j + 1] + 1
            } else {
                common[(i + 1) * width + j].max(common[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            script.push((Side::Both, old[i]));
            i += 1;
            j += 1;
        } else if common[(i + 1) * width + j] >= common[i * width + j + 1] {
            script.push((Side::Expected, old[i]));
            i += 1;
        } else {
            script.push((Side::Actual, new[j]));
            j += 1;
        }
    }
    script.extend(old[i..].iter().map(|l| (Side::Expected, *l)));
    script.extend(new[j..].iter().map(|l| (Side::Actual, *l)));
    script
}

fn render(script: &[(Side, &str)]) -> String {
    let changed: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, (side, _))| *side != Side::Both)
        .map(|(index, _)| index)
        .collect();
    let mut out = String::new();
    let mut next_change = 0;
    let mut elided = false;
    for (index, (side, text)) in script.iter().enumerate() {
        while next_change < changed.len() && changed[next_change] + CONTEXT < index {
            next_change += 1;
        }
        let near = changed
            .get(next_change)
            .is_some_and(|&change| change <= index + CONTEXT);
        if !near {
            if !elided {
                out.push_str("  ...\n");
                elided = true;
            }
            continue;
        }
        elided = false;
        out.push_str(match side {
            Side::Both => "  ",
            Side::Expected => "- ",
            Side::Actual => "+ ",
        });
        out.push_str(text);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_have_no_diff() {
        assert_eq!(lines("a\nb", "a\nb"), "");
    }

    #[test]
    fn shows_changes_with_context_and_elides_the_rest() {
        let expected = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12";
        let actual = "1\n2\n3\n4\n5\nsix\n7\n8\n9\n10\n11\n12";
        assert_eq!(
            lines(expected, actual),
            "  ...\n  3\n  4\n  5\n- 6\n+ six\n  7\n  8\n  9\n  ...\n"
        );
    }

    #[test]
    fn handles_insertions_deletions_and_distant_changes() {
        let expected = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn";
        let actual = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no";
        assert_eq!(
            lines(expected, actual),
            "  a\n- b\n+ B\n  c\n  d\n  e\n  ...\n  l\n  m\n  n\n+ o\n"
        );
        assert_eq!(lines("a\nb\nc", "a\nc"), "  a\n- b\n  c\n");
    }

    #[test]
    fn oversized_middles_fall_back_to_a_block_replacement() {
        let old: Vec<String> = (0..1100).map(|n| format!("old{n}")).collect();
        let new: Vec<String> = (0..1100).map(|n| format!("new{n}")).collect();
        let out = lines(&old.join("\n"), &new.join("\n"));
        assert_eq!(out.lines().count(), 2200);
        assert!(out.starts_with("- old0\n") && out.ends_with("+ new1099\n"));
    }
}
