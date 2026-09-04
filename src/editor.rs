//! Editor — the external program hntui hands the terminal to for editing the
//! config file.
//!
//! The app owns the terminal in raw mode on an alternate screen, so handing it
//! over is the caller's job (see `Tui::suspend`). This module only resolves
//! which program to run and runs it.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// What came back from the Editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutcome {
    /// The Editor exited cleanly. Whether anything actually changed is the
    /// ConfigReload's business.
    Finished,
    /// The Editor exited non-zero — vim's `:cq`, or a crash. Git reads this as
    /// "forget I said anything", and so do we.
    Cancelled { status: i32 },
}

/// The Editor to run: `$VISUAL`, then `$EDITOR`, then `vi`.
///
/// The value is a shell word list, not a program name — `code --wait` and
/// `emacsclient -t` are both normal things to have in `$EDITOR`.
fn editor_command() -> String {
    for name in ["VISUAL", "EDITOR"] {
        if let Some(value) = std::env::var_os(name) {
            let value = value.to_string_lossy().trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    "vi".to_string()
}

/// Runs the user's Editor on `path` and blocks until it exits.
pub fn edit(path: &Path) -> Result<EditOutcome> {
    run(&editor_command(), path)
}

/// Runs one editor command on `path`.
///
/// The command goes through `sh` so a `$EDITOR` carrying arguments works, and
/// the path is passed as a positional argument rather than interpolated, so a
/// path with spaces or quotes cannot turn into extra shell words.
fn run(command: &str, path: &Path) -> Result<EditOutcome> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{command} \"$@\""))
        .arg(command)
        .arg(path)
        .status()
        .with_context(|| format!("run editor {command}"))?;

    if status.success() {
        return Ok(EditOutcome::Finished);
    }
    Ok(EditOutcome::Cancelled {
        status: status.code().unwrap_or(-1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        name: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, previous }
        }

        fn unset(name: &'static str) -> Self {
            let previous = std::env::var_os(name);
            std::env::remove_var(name);
            Self { name, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn visual_wins_then_editor_then_vi() {
        let _lock = env_lock().lock().expect("env lock");

        let visual = EnvGuard::set("VISUAL", "nvim");
        let _editor = EnvGuard::set("EDITOR", "nano");
        assert_eq!(editor_command(), "nvim");
        drop(visual);

        // Dropping the guard restores whatever the developer's shell had set,
        // so unset it explicitly to see EDITOR take over.
        let no_visual = EnvGuard::unset("VISUAL");
        assert_eq!(editor_command(), "nano");

        let blank = EnvGuard::set("EDITOR", "   ");
        assert_eq!(editor_command(), "vi", "a blank value is not a choice");
        drop(blank);

        let _none = EnvGuard::unset("EDITOR");
        assert_eq!(editor_command(), "vi");
        drop(no_visual);
    }

    #[test]
    fn a_multi_word_editor_keeps_its_arguments_and_gets_the_path_whole() {
        let directory = tempfile::tempdir().expect("temp dir");
        let marker = directory.path().join("args");
        let script = directory.path().join("fake-editor");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", marker.display()),
        )
        .expect("write fake editor");
        std::fs::set_permissions(
            &script,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
        )
        .expect("chmod fake editor");
        let target = directory.path().join("name with spaces.toml");
        std::fs::write(&target, "").expect("write target");

        let outcome = run(&format!("{} --wait", script.display()), &target).expect("editor runs");

        assert_eq!(outcome, EditOutcome::Finished);
        let recorded = std::fs::read_to_string(&marker).expect("editor ran");
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            vec!["--wait", target.to_str().expect("utf-8 path")],
            "the editor keeps its own arguments and the path stays one word"
        );
    }

    #[test]
    fn a_non_zero_exit_is_a_cancellation() {
        let directory = tempfile::tempdir().expect("temp dir");
        let target = directory.path().join("config.toml");
        std::fs::write(&target, "").expect("write target");

        assert_eq!(
            run("exit 3 #", &target).expect("editor runs"),
            EditOutcome::Cancelled { status: 3 }
        );
    }
}
