//! Line numbers of keys in a JSON text, so a settings error can name its line.

/// The line of the key at `path` (object keys from the root), or of its nearest
/// ancestor present in the text; 1 when none is.
pub fn line_near(text: &str, path: &[&str]) -> usize {
    (1..=path.len())
        .rev()
        .find_map(|n| line_of(text, &path[..n]))
        .unwrap_or(1)
}

/// The line of the key at `path`, when the text holds it. Assumes the text is JSON.
fn line_of(text: &str, path: &[&str]) -> Option<usize> {
    // One frame per open container: the current key of an object, `None` for an array.
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut expect_key = false;
    let mut line = 1;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            '{' => {
                stack.push(Some(String::new()));
                expect_key = true;
            }
            '[' => {
                stack.push(None);
                expect_key = false;
            }
            '}' | ']' => {
                stack.pop();
                expect_key = false;
            }
            ',' => expect_key = matches!(stack.last(), Some(Some(_))),
            ':' => expect_key = false,
            '"' => {
                let start = line;
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                s.push(escaped);
                            }
                        }
                        '"' => break,
                        c => {
                            line += usize::from(c == '\n');
                            s.push(c);
                        }
                    }
                }
                if expect_key {
                    if let Some(Some(key)) = stack.last_mut() {
                        *key = s;
                    }
                    let here = stack.len() == path.len()
                        && stack
                            .iter()
                            .zip(path)
                            .all(|(frame, want)| frame.as_deref() == Some(*want));
                    if here {
                        return Some(start);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::line_near;

    #[test]
    fn finds_nested_keys_and_falls_back_to_the_parent() {
        let text = "{\n  \"nodes\": {\n    \"a\": {\"x\": [1, {\"b\": 2}]},\n    \"b\": {}\n  }\n}";
        assert_eq!(line_near(text, &["nodes"]), 2);
        assert_eq!(line_near(text, &["nodes", "b"]), 4);
        assert_eq!(line_near(text, &["nodes", "a", "x"]), 3);
        assert_eq!(line_near(text, &["nodes", "a", "missing"]), 3);
        assert_eq!(line_near(text, &["other"]), 1);
    }
}
