//! The wildcard patterns of `known_hosts`: `*` for any run of characters and `?`
//! for one, compared without regard to case (host names have none).

/// Whether `text` matches `pattern`, without regard to case.
pub(crate) fn matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();

    wild(&pattern, &text)
}

fn wild(pattern: &[char], text: &[char]) -> bool {
    let mut p = 0;
    let mut t = 0;
    // Where the last `*` was in the pattern, and how much of the text it has taken so far.
    let mut star: Option<(usize, usize)> = None;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_at, taken)) = star {
            p = star_at + 1;
            t = taken + 1;
            star = Some((star_at, taken + 1));
        } else {
            return false;
        }
    }

    pattern[p..].iter().all(|&c| c == '*')
}

/// Whether `text` is selected by a list of patterns where `!` before one excludes: some pattern without it matches and
/// none with it does.
pub(crate) fn selected<'a>(patterns: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    let mut any = false;

    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if matches(negated, text) {
                return false;
            }
        } else if matches(pattern, text) {
            any = true;
        }
    }

    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_runs_and_single_characters() {
        let cases = [
            ("*", "anything", true),
            ("*.example.com", "files.example.com", true),
            ("*.example.com", "example.com", false),
            ("host?", "host1", true),
            ("host?", "host", false),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
            ("Files.Example", "files.example", true),
            ("", "", true),
            ("", "x", false),
        ];

        for (pattern, text, expected) in cases {
            assert_eq!(matches(pattern, text), expected, "{pattern} against {text}");
        }
    }

    #[test]
    fn a_negated_pattern_excludes() {
        assert!(selected(["*.example.com", "!secret.example.com"], "files.example.com"));
        assert!(!selected(
            ["*.example.com", "!secret.example.com"],
            "secret.example.com"
        ));
        assert!(!selected(["!only-a-negation"], "other"));
    }
}
