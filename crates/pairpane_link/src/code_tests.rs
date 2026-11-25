//! The code: what it is made of, how it is shown and how typed input is read.

use std::collections::HashSet;

use pretty_assertions::assert_eq;

use super::{ALPHABET, CODE_LEN, PairingCode};
use crate::error::Error;

#[test]
fn a_generated_code_has_eight_characters_of_the_alphabet() {
    for _ in 0..200 {
        let code = PairingCode::generate().unwrap();

        assert_eq!(code.as_bytes().len(), CODE_LEN);
        assert!(code.as_bytes().iter().all(|byte| ALPHABET.contains(byte)));
    }
}

#[test]
fn the_alphabet_is_crockfords_without_i_l_o_and_u() {
    assert_eq!(ALPHABET.len(), 32);
    assert_eq!(ALPHABET.iter().copied().collect::<HashSet<_>>().len(), 32);
    for letter in *b"ILOU" {
        assert!(!ALPHABET.contains(&letter), "{}", char::from(letter));
    }
}

#[test]
fn generated_codes_differ_and_use_many_characters() {
    let codes: Vec<PairingCode> = (0..64).map(|_| PairingCode::generate().unwrap()).collect();
    let distinct: HashSet<[u8; CODE_LEN]> = codes.iter().map(|code| *code.as_bytes()).collect();
    let characters: HashSet<u8> = codes.iter().flat_map(|code| code.as_bytes().iter().copied()).collect();

    assert_eq!(distinct.len(), 64);
    // 512 draws from 32 characters leave every one of them unseen with a chance of about 10^-7 each.
    assert!(characters.len() > 28, "{} different characters", characters.len());
}

#[test]
fn a_code_is_shown_as_two_groups_of_four() {
    let code = PairingCode::parse("0123abcd").unwrap();

    assert_eq!(code.to_string(), "0123-ABCD");
}

#[test]
fn input_is_read_without_regard_to_case_dash_and_spaces() {
    let expected = PairingCode::parse("7K2M9QXZ").unwrap();

    for input in [
        "7K2M-9QXZ",
        "7k2m-9qxz",
        " 7k2m 9qxz ",
        "7K2M9QXZ",
        "7-K-2-M-9-Q-X-Z",
        "7k2m\t-\n9qxz",
    ] {
        assert!(PairingCode::parse(input).unwrap() == expected, "{input:?}");
    }
}

#[test]
fn crockfords_look_alike_letters_read_as_the_digits() {
    let code = PairingCode::parse("O0I1L1AB").unwrap();

    assert_eq!(code.to_string(), "0011-11AB");
}

#[test]
fn a_wrong_length_or_character_is_refused() {
    for input in [
        "",
        "7K2M",
        "7K2M-9QX",
        "7K2M-9QXZZ",
        "7K2M-9QX!",
        "7K2M-9QXU",
        "7K2M-9QXÄ",
        "7K2M-9QX\u{0}",
    ] {
        assert!(
            matches!(PairingCode::parse(input), Err(Error::InvalidCode)),
            "{input:?}"
        );
    }
}

#[test]
fn a_shown_code_reads_back_as_the_same_code() {
    let code = PairingCode::generate().unwrap();

    assert!(PairingCode::parse(&code.to_string()).unwrap() == code);
}

#[test]
fn debug_does_not_print_the_code() {
    let code = PairingCode::parse("7K2M-9QXZ").unwrap();

    assert!(!format!("{code:?}").contains("7K2M"));
}
