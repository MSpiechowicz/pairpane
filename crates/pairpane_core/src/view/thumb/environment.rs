//! The environment block of the thumbnail helper on Windows, built from `(name, value)` pairs. It has no Windows call in
//! it, so that the layout `CreateProcessW` takes (and has refused before: "the system could not find the environment
//! option", error 203) is tested on every system.
//!
//! The block is UTF-16 (the flag `CREATE_UNICODE_ENVIRONMENT`), `NAME=VALUE` and a NUL for each variable, sorted by name
//! without regard to case (the order the system keeps its own blocks in), and ends with one NUL more; a block with no
//! variable is two NULs.

#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "the helper is started in a container on Windows only; the tests run everywhere"
    )
)]

/// The block of the variables `entries`. An entry with no name, a name that holds `=` or a NUL, or a value that holds a NUL
/// is left out (the block could not tell it from two).
pub(super) fn block<N: AsRef<str>, V: AsRef<str>>(entries: &[(N, V)]) -> Vec<u16> {
    let mut usable: Vec<(String, &str, &str)> = entries
        .iter()
        .map(|(name, value)| (name.as_ref(), value.as_ref()))
        .filter(|(name, value)| !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0'))
        .map(|(name, value)| (name.to_uppercase(), name, value))
        .collect();
    usable.sort_by(|a, b| a.0.cmp(&b.0));

    let mut block: Vec<u16> = Vec::new();
    for (_, name, value) in usable {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }

    // The block ends with an empty string: one NUL after the last variable, and two NULs when there is none.
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }

    block
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::block;

    fn utf16(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn two_variables_are_name_equals_value_and_a_nul_each_and_one_nul_more() {
        let mut want = utf16("SystemRoot=C:\\Windows");
        want.push(0);
        want.extend(utf16("WINDIR=C:\\Windows"));
        want.extend([0, 0]);

        assert_eq!(block(&[("SystemRoot", "C:\\Windows"), ("WINDIR", "C:\\Windows")]), want);
    }

    #[test]
    fn a_block_with_no_variable_is_two_nuls() {
        assert_eq!(block::<&str, &str>(&[]), [0, 0]);
        // What cannot be a variable leaves nothing, and the block is still well formed.
        assert_eq!(block(&[("", "x"), ("A=B", "x"), ("N\0", "x"), ("C", "a\0b")]), [0, 0]);
    }

    #[test]
    fn a_missing_variable_is_just_not_there() {
        // A machine that has no `SystemRoot` in the environment of the app: the block is the one of the others.
        let mut want = utf16("WINDIR=C:\\Windows");
        want.extend([0, 0]);

        assert_eq!(block(&[("WINDIR", "C:\\Windows")]), want);
    }

    #[test]
    fn the_variables_are_sorted_by_name_without_regard_to_case() {
        let sorted = block(&[("windir", "3"), ("TEMP", "2"), ("SystemRoot", "1"), ("APPDATA", "0")]);

        let mut want = Vec::new();
        for entry in ["APPDATA=0", "SystemRoot=1", "TEMP=2", "windir=3"] {
            want.extend(utf16(entry));
            want.push(0);
        }
        want.push(0);

        assert_eq!(sorted, want);
    }

    #[test]
    fn text_outside_the_basic_plane_is_utf16_and_an_empty_value_is_kept() {
        let mut want = utf16("A=");
        want.push(0);
        want.extend(utf16("B=\u{1F4F7}"));
        want.extend([0, 0]);

        assert_eq!(block(&[("B", "\u{1F4F7}"), ("A", "")]), want);
        assert_eq!(utf16("\u{1F4F7}").len(), 2, "a surrogate pair");
    }
}
