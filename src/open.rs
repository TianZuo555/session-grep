use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Error, anyhow};

use crate::model::Session;

/// The shell equivalent of `exec`, for `--print-command` and for the log line.
pub fn describe(session: &Session) -> String {
    let command: Vec<String> = session
        .resume_command()
        .iter()
        .map(|a| shell_quote(a))
        .collect();
    format!("cd {} && {}", shell_quote(&session.cwd), command.join(" "))
}

/// Replace this process with the agent, in the directory of the session.
/// Returns only when the agent cannot start.
pub fn exec(session: &Session) -> Error {
    // The agents look up a session by its project, so a different directory
    // does not work.
    if !Path::new(&session.cwd).is_dir() {
        return anyhow!("the session directory {:?} does not exist", session.cwd);
    }
    let argv = session.resume_command();
    let error = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&session.cwd)
        .exec();
    anyhow!("cannot start {}: {error}", argv[0])
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || "._-/:=@%+".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(shell_quote("/a/b-c_d.e"), "/a/b-c_d.e");
        assert_eq!(shell_quote("/a b"), "'/a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
