//! The placeholders of a template: ranges of the name and the extension by characters, the counter, the date, the
//! brackets that stand for themselves, and what is refused.

use jiff::tz::TimeZone;
use pretty_assertions::assert_eq;

use super::template::{Context, Local, Template, split_name};

fn render_with(template: &str, name: &str, ext: &str, local: Option<Local>) -> String {
    let context = Context {
        name,
        ext,
        counter: "007",
        local,
    };

    Template::parse(template)
        .expect("a template")
        .render(&context, usize::MAX)
}

fn render(template: &str, name: &str, ext: &str) -> String {
    render_with(template, name, ext, None)
}

fn refused(template: &str) -> String {
    Template::parse(template).expect_err("refused")
}

#[test]
fn the_name_and_the_extension_are_written_whole() {
    assert_eq!(render("[N].[E]", "report", "txt"), "report.txt");
    assert_eq!(render("copy of [N] ([E])", "a", "b"), "copy of a (b)");
    assert_eq!(render("", "a", "b"), "");
}

#[test]
fn ranges_count_unicode_characters_from_one() {
    // Eleven characters, four of them more than one byte.
    let name = "héllo wörld";

    assert_eq!(render("[N1]|[N2]|[N8]", name, ""), "h|é|ö");
    assert_eq!(render("[N2-4]", name, ""), "éll");
    assert_eq!(render("[N2-]", name, ""), "éllo wörld");
    assert_eq!(render("[N-1]|[N-3]", name, ""), "d|r");
    assert_eq!(render("[N-3-]", name, ""), "rld");
    assert_eq!(render("[N1-1]", name, ""), "h");
    assert_eq!(render("[E2-3]|[E-2-]", "", "tar.gz"), "ar|gz");
}

#[test]
fn a_range_outside_the_text_is_empty_and_not_an_error() {
    let name = "abc";

    assert_eq!(render("[N4]|[N9-12]|[N5-]|[N-4]|[N5-2]", name, ""), "||||");
    assert_eq!(render("[N2-99]", name, ""), "bc");
    assert_eq!(render("[N-99-]", name, ""), "abc");
    assert_eq!(render("[N99999999999999999999]", name, ""), "");
    assert_eq!(render("[E]|[E1]|[E-1]", name, ""), "||");
}

#[test]
fn brackets_stand_for_themselves_when_doubled() {
    assert_eq!(render("[[N]]", "a", ""), "[N]");
    assert_eq!(render("[[[N]]]", "a", ""), "[a]");
    assert_eq!(render("a[[b]]c", "", ""), "a[b]c");
    assert_eq!(render("]]", "", ""), "]");
}

#[test]
fn the_counter_is_written_as_the_job_gives_it() {
    assert_eq!(render("[N]-[C]", "a", ""), "a-007");
}

#[test]
fn a_placeholder_that_does_not_exist_is_refused() {
    for template in [
        "[]", "[X]", "[n]", "[e]", "[c]", "[y]", "[H]", "[S]", "[ymd]", "[HMS]", "[N0]", "[N0-3]", "[N-0]", "[N-0-]",
        "[N2-3-4]", "[N-]", "[N--3]", "[N 2]", "[N+2]", "[N2.5]", "[C5]", "[Y2]", "[YMD2]", "[ N ]", "[N[N]]",
    ] {
        assert!(refused(template).ends_with("is not a placeholder"), "{template}");
    }
}

#[test]
fn a_bracket_that_is_not_closed_or_not_opened_is_refused() {
    assert!(refused("[N").contains("not closed"));
    assert!(refused("a[").contains("not closed"));
    assert!(refused("N]").contains("no ["));
    assert!(refused("[N]]").contains("no ["));
}

#[test]
fn the_message_quotes_a_long_placeholder_in_part() {
    let message = refused(&format!("[{}]", "Z".repeat(100)));

    assert_eq!(message, format!("[{}…] is not a placeholder", "Z".repeat(16)));
}

#[test]
fn dates_and_times_come_from_the_local_time_of_the_modification() {
    // 2023-11-14 22:13:20 UTC.
    let local = Local::of(1_700_000_000_000, &TimeZone::UTC);

    assert_eq!(
        render_with("[Y]|[M]|[D]|[h]|[m]|[s]", "", "", local),
        "2023|11|14|22|13|20"
    );
    assert_eq!(render_with("[YMD] [hms]", "", "", local), "2023-11-14 22-13-20");
    assert_eq!(
        render_with(
            "[Y]",
            "",
            "",
            Local::of(-62_135_596_800_000 + 86_400_000, &TimeZone::UTC)
        ),
        "0001"
    );
}

#[test]
fn a_time_that_cannot_be_shown_has_no_local_date() {
    assert_eq!(Local::of(i64::MAX, &TimeZone::UTC), None);
}

#[test]
fn only_a_template_with_a_date_placeholder_uses_the_date() {
    assert!(Template::parse("[N][YMD]").expect("parses").uses_date());
    assert!(!Template::parse("[N][C][[Y]]").expect("parses").uses_date());
}

#[test]
fn the_split_is_the_one_the_panes_show() {
    let files = [
        ("a.txt", "a", "txt"),
        ("archive.tar.gz", "archive.tar", "gz"),
        ("ü.txt", "ü", "txt"),
        ("x..y", "x.", "y"),
        (".bashrc", ".bashrc", ""),
        ("trailing.", "trailing.", ""),
        ("noext", "noext", ""),
        ("..", "..", ""),
        (".", ".", ""),
        ("a.b.", "a.b.", ""),
    ];

    for (name, stem, ext) in files {
        assert_eq!(split_name(name, false), (stem, ext), "{name}");
    }
    assert_eq!(split_name("v1.2", true), ("v1.2", ""), "a folder has no extension");
}

#[test]
fn a_template_stops_once_it_is_over_the_limit() {
    let context = Context {
        name: "abcdefgh",
        ext: "",
        counter: "1",
        local: None,
    };
    let template = Template::parse(&"[N]".repeat(1_000)).expect("a template");

    let text = template.render(&context, 100);

    assert!(text.len() > 100 && text.len() <= 100 + 8, "{}", text.len());
}
