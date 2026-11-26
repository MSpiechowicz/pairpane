//! A look at what the `ssh` of the machine really prints for the files `known_hosts` is read from, in the CI log: the
//! forms that `resolve::known` takes were written from the documentation, and this shows the real ones. It checks
//! nothing and always passes (see `--nocapture`, or a failing run, for the output).

use std::process::Command;

use super::ssh_config::Lab;
use super::ssh_program;
use crate::resolve::raw_lines;

#[test]
fn what_windows_openssh_prints_for_the_known_hosts_files() {
    let Some(program) = ssh_program() else {
        return;
    };
    let Some(lab) = Lab::new(&[(
        "config",
        "Host files\n  UserKnownHostsFile {dir}/kh1 {dir}/kh2\n  GlobalKnownHostsFile {dir}/global\n\
         Host none\n  UserKnownHostsFile none\n  GlobalKnownHostsFile none\n\
         Host null\n  UserKnownHostsFile /dev/null\n  GlobalKnownHostsFile /dev/null\n\
         Host spaced\n  UserKnownHostsFile {dir}/with space/kh\n\
         Host quoted\n  UserKnownHostsFile \"{dir}/with space/kh\" \"{dir}/other kh\"\n\
         Host backslashes\n  UserKnownHostsFile \"C:\\\\x\\\\kh\"\n\
         Host defaults\n  HostName defaults.example\n",
    )]) else {
        return;
    };
    let config = lab.path("config");

    match Command::new(&program).arg("-V").output() {
        Ok(output) => eprintln!(
            "ssh -V: {:?} {:?} (program {})",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            program.display()
        ),
        Err(error) => eprintln!("ssh -V did not run: {error}"),
    }

    eprintln!("the folder of the files: {}", lab.dir.path().display());
    eprintln!("ProgramData: {:?}", std::env::var("ProgramData").ok());

    for host in ["files", "none", "null", "spaced", "quoted", "backslashes", "defaults"] {
        eprintln!(
            "ssh -G {host}:\n{}",
            raw_lines(
                &program,
                &config,
                host,
                &["userknownhostsfile", "globalknownhostsfile", "identityfile"]
            )
        );
    }
}
