//! The suggestions of the Connect dialog: the plain names after `Host` in `~/.ssh/config`. Nothing else of the file is
//! read here (where a host leads is OpenSSH's to say, when it is connected to), so an alias is all a suggestion holds.

use std::io::Read;
use std::path::Path;

use pairpane_core::model::SshHost;

use crate::known_hosts::valid_host;
use crate::regular::open_regular;

/// The most hosts listed.
const MAX_HOSTS: usize = 200;

/// The most that is read of the file; a bigger one gives no suggestions.
const MAX_FILE: u64 = 1024 * 1024;

/// The names after `Host` lines of `~/.ssh/config` of the user whose home is `home`, in file order, without repeats and
/// without patterns (`*`, `?`, `!`). The file's `Include`s are not followed. Empty when there is no readable file.
pub(crate) fn hosts(home: &Path) -> Vec<SshHost> {
    let Ok(file) = open_regular(&home.join(".ssh").join("config")) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();

    if file.take(MAX_FILE + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_FILE {
        return Vec::new();
    }

    let mut aliases: Vec<String> = Vec::new();

    for line in String::from_utf8_lossy(&bytes).lines() {
        for alias in host_names(line) {
            if aliases.len() < MAX_HOSTS && !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
    }

    aliases
        .into_iter()
        .map(|alias| SshHost {
            alias,
            host_name: None,
            user: None,
            port: None,
        })
        .collect()
}

/// The names of a `Host` line (the keyword in any case, separated from its value by spaces or `=`); none for another
/// line. A name that could not be typed into the dialog is left out.
fn host_names(line: &str) -> impl Iterator<Item = String> + '_ {
    let line = line.trim();
    let end = line.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(line.len());
    let (keyword, value) = line.split_at(end);
    let names = if keyword.eq_ignore_ascii_case("host") {
        value.trim_start_matches(|c: char| c.is_whitespace() || c == '=')
    } else {
        ""
    };

    // A word that starts with `#` begins a comment, which ends the line (`Host web # the web server`).
    names
        .split_whitespace()
        .take_while(|name| !name.starts_with('#'))
        .map(|name| name.trim_matches('"'))
        .filter(|name| valid_host(name))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;

    use super::*;

    fn aliases(text: &str) -> Vec<String> {
        let home = tempfile::tempdir().expect("temp dir");

        fs::create_dir(home.path().join(".ssh")).expect("dir");
        fs::write(home.path().join(".ssh").join("config"), text).expect("written");

        hosts(home.path()).into_iter().map(|host| host.alias).collect()
    }

    #[test]
    fn the_names_without_patterns_are_listed_once_in_file_order() {
        assert_eq!(
            aliases(
                "# comment\nHost work *.example !secret\n  HostName ignored\nhost=lower\nHOST \"quoted\" work\n\
                 Match host x\nHost a?b\nHostname notahost\nInclude other\n"
            ),
            ["work", "lower", "quoted"]
        );
    }

    #[test]
    fn a_comment_at_the_end_of_a_host_line_is_not_a_host() {
        assert_eq!(
            aliases("Host web # the web server\nHost db #cache\nHost # nothing\nHost a #b c\n"),
            ["web", "db", "a"]
        );
    }

    #[test]
    fn there_is_no_suggestion_without_a_readable_file() {
        let home = tempfile::tempdir().expect("temp dir");

        assert_eq!(hosts(home.path()), Vec::new());
        fs::create_dir_all(home.path().join(".ssh").join("config")).expect("a folder in its place");
        assert_eq!(hosts(home.path()), Vec::new());
    }

    #[test]
    fn the_list_has_a_limit() {
        let text: String = (0..MAX_HOSTS + 20).map(|n| format!("Host h{n}\n")).collect();

        assert_eq!(aliases(&text).len(), MAX_HOSTS);
    }
}
