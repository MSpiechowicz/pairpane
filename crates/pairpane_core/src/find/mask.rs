//! Wildcard masks for names: the same rules as the file lists' mask filter (`ui/src/lib/panes/mask.ts`), so a mask
//! means the same thing in a pane and in a search.

/// A `;`-separated list of wildcard masks. `*` is any run of characters (empty too), `?` exactly one; everything else
/// is literal, a regular expression character included. Empty parts are ignored, and `*.*` fits every name, with or
/// without a dot.
#[derive(Debug, Clone)]
pub(crate) struct Mask {
    masks: Vec<Vec<char>>,
    everything: bool,
    case_sensitive: bool,
}

/// The parts of `text` that are not blank.
pub(crate) fn parts(text: &str) -> impl Iterator<Item = &str> {
    text.split(';').map(str::trim).filter(|part| !part.is_empty())
}

impl Mask {
    /// The masks of `text`; names are compared with their case when `case_sensitive`, else without it.
    pub(crate) fn new(text: &str, case_sensitive: bool) -> Self {
        let masks: Vec<Vec<char>> = parts(text).map(|part| fold(part, case_sensitive)).collect();
        let everything = masks.iter().any(|mask| mask.iter().eq(&['*', '.', '*']));

        Self {
            masks,
            everything,
            case_sensitive,
        }
    }

    /// Whether `name` fits one of the masks; no masks fit nothing.
    pub(crate) fn fits(&self, name: &str) -> bool {
        if self.everything {
            return true;
        }

        let name = fold(name, self.case_sensitive);

        self.masks.iter().any(|mask| fits(mask, &name))
    }
}

fn fold(text: &str, case_sensitive: bool) -> Vec<char> {
    if case_sensitive {
        text.chars().collect()
    } else {
        text.to_lowercase().chars().collect()
    }
}

/// Whether `pattern` fits all of `name`. One pass with a single fallback point: when a `*` has to swallow one more
/// character, the last `*` is retried and earlier ones never are, so the work stays linear in the product of the two
/// lengths at worst and a pathological mask cannot stall a search.
fn fits(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<usize> = None;
    let mut resume = 0;

    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = n;
            p += 1;
        } else if let Some(at) = star {
            resume += 1;
            n = resume;
            p = at + 1;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }

    p == pattern.len()
}
