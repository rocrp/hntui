use std::process::{Command, Output};

fn hntui(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hntui"))
        .args(args)
        .output()
        .expect("run hntui")
}

#[test]
fn version_flags_report_the_manifest_version() {
    let expected = format!("hntui {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        let output = hntui(&[flag]);

        assert!(output.status.success(), "{flag} failed: {output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn help_starts_with_version_then_about_text() {
    let output = hntui(&["--help"]);

    assert!(output.status.success(), "--help failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut lines = stdout.lines();
    let expected = format!("hntui {}", env!("CARGO_PKG_VERSION"));
    assert_eq!(lines.next(), Some(expected.as_str()));
    assert_eq!(lines.next(), Some("Hacker News TUI"));
    assert!(output.stderr.is_empty());
}

#[test]
fn positive_numeric_options_reject_invalid_values_during_parsing() {
    for option in [
        "--count",
        "--page-size",
        "--cache-size",
        "--concurrency",
        "--file-cache-ttl-secs",
    ] {
        for invalid_value in ["0", "not-a-number"] {
            let output = hntui(&[option, invalid_value]);

            assert!(
                !output.status.success(),
                "{option} accepted {invalid_value}"
            );
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.starts_with(&format!("error: invalid value '{invalid_value}'")),
                "{option} did not fail in clap:\n{stderr}"
            );
        }
    }
}

#[test]
fn api_backend_is_a_clap_value_enum() {
    let help = hntui(&["--help"]);
    assert!(help.status.success(), "--help failed: {help:?}");
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Possible values:"), "{help}");
    for backend in ["hackerweb", "firebase"] {
        assert!(
            help.contains(&format!("- {backend}:")),
            "{backend} missing from help:\n{help}"
        );
    }

    let invalid = hntui(&["--api-backend", "bogus"]);
    assert!(!invalid.status.success(), "invalid backend was accepted");
    let stderr = String::from_utf8(invalid.stderr).unwrap();
    assert!(
        stderr.starts_with("error: invalid value 'bogus'"),
        "backend did not fail in clap:\n{stderr}"
    );
    assert!(stderr.contains("hackerweb, firebase"), "{stderr}");
}

#[test]
fn empty_values_fail_during_parsing() {
    for option in ["--base-url", "--log-file", "--env-file"] {
        let output = hntui(&[option, ""]);

        assert!(!output.status.success(), "{option} accepted an empty value");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.starts_with("error:") && stderr.contains("try '--help'"),
            "{option} did not fail in clap:\n{stderr}"
        );
    }
}

#[test]
fn config_is_the_only_explicit_config_flag() {
    let help = hntui(&["--help"]);
    assert!(help.status.success(), "--help failed: {help:?}");
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--config <CONFIG>"), "{help}");
    assert!(!help.contains("plugin"), "{help}");

    let old_flag = hntui(&["--plugin-config", "config.toml"]);
    assert!(!old_flag.status.success(), "old config flag was accepted");
    let stderr = String::from_utf8(old_flag.stderr).unwrap();
    assert!(stderr.starts_with("error: unexpected argument"), "{stderr}");
}

#[test]
fn config_flag_loads_the_requested_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("requested.toml");
    std::fs::write(&path, "this is not toml").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_hntui"))
        .arg("--config")
        .arg(&path)
        .output()
        .expect("run hntui");

    assert!(!output.status.success(), "invalid config was not loaded");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("load config"), "{stderr}");
    assert!(stderr.contains(&path.display().to_string()), "{stderr}");
}
