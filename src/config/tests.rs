use super::*;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    name: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let previous = std::env::var(name).ok();
        std::env::set_var(name, value);
        Self { name, previous }
    }

    fn unset(name: &'static str) -> Self {
        let previous = std::env::var(name).ok();
        std::env::remove_var(name);
        Self { name, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.name, previous);
        } else {
            std::env::remove_var(self.name);
        }
    }
}

#[test]
fn discovery_uses_first_existing_candidate() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = dir.path().join("config.toml");
    let legacy = dir.path().join("plugin-config.toml");
    std::fs::write(&first, "[summarize]\nmodel = \"openai/first\"\n").expect("write first config");
    std::fs::write(&legacy, "[summarize]\nmodel = \"openai/legacy\"\n")
        .expect("write legacy config");

    let config = Config::load_from(vec![first.clone(), legacy], dir.path().join("default.toml"))
        .expect("load config");

    assert_eq!(config.path(), first);
    assert_eq!(
        config.summarize().expect("summarize config").model,
        "openai/first"
    );
}

#[test]
fn canonical_discovery_candidates_precede_all_legacy_candidates() {
    let candidates = discovery_candidates();
    let legacy_start = candidates
        .iter()
        .position(|path| {
            path.file_name()
                .is_some_and(|name| name == "plugin-config.toml")
        })
        .expect("legacy candidates present");

    assert!(legacy_start > 0);
    assert!(candidates[..legacy_start]
        .iter()
        .all(|path| path.file_name().is_some_and(|name| name == "config.toml")));
    assert!(candidates[legacy_start..].iter().all(|path| path
        .file_name()
        .is_some_and(|name| name == "plugin-config.toml")));
}

#[test]
fn a_missing_config_is_seeded_from_the_commented_template() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("nested/config.toml");
    let config = Config::load_from(vec![path.clone()], path.clone()).expect("load empty");

    config.ensure_file_exists().expect("seed config");

    let seeded = std::fs::read_to_string(&path).expect("template written");
    assert!(
        seeded.contains("[summarize]"),
        "a starter file, not a blank one"
    );
    assert!(
        seeded.contains('#'),
        "the template's comments are the documentation"
    );

    std::fs::write(&path, "[summarize]\nmodel = \"edited\"\n").expect("edit");
    config.ensure_file_exists().expect("second call");
    assert!(
        std::fs::read_to_string(&path)
            .expect("still there")
            .contains("edited"),
        "an existing config is never overwritten"
    );
}

#[test]
fn reload_re_reads_the_file_the_config_came_from() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[summarize]\nmodel = \"openai/old\"\n").expect("write");
    let config = Config::load_from(vec![path.clone()], path.clone()).expect("load");

    std::fs::write(&path, "[summarize]\nmodel = \"openai/new\"\n").expect("rewrite");
    let reloaded = config.reload().expect("reload");

    assert_eq!(reloaded.summarize().expect("summarize").model, "openai/new");
    assert_eq!(reloaded.path(), path);
}

#[test]
fn summary_timeout_defaults_and_reloads_from_config() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[summarize]\nmodel = \"openai/test\"\n").expect("write");
    let config = Config::load_from_for_test(path.clone()).expect("load");
    assert_eq!(config.summarize().expect("summarize").timeout_secs, 600);

    std::fs::write(
        &path,
        "[summarize]\nmodel = \"openai/test\"\ntimeout_secs = 900\n",
    )
    .expect("rewrite");
    let reloaded = config.reload().expect("reload");
    assert_eq!(reloaded.summarize().expect("summarize").timeout_secs, 900);
}

#[test]
fn values_that_parse_but_cannot_work_are_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, body, expected) in [
        (
            "empty-model",
            "[summarize]\nmodel = \"\"\n",
            "model must not be empty",
        ),
        (
            "zero-comments",
            "[summarize]\nmodel = \"x/y\"\nmax_comments = 0\n",
            "max_comments must be greater than 0",
        ),
        (
            "zero-timeout",
            "[summarize]\nmodel = \"x/y\"\ntimeout_secs = 0\n",
            "timeout_secs must be greater than 0",
        ),
        (
            "zero-chars",
            "[summarize]\nmodel = \"x/y\"\nmax_article_chars = 0\n",
            "max_article_chars must be greater than 0",
        ),
    ] {
        let path = dir.path().join(format!("{name}.toml"));
        std::fs::write(&path, body).expect("write config");

        let error = Config::load_from(vec![path.clone()], path)
            .expect_err("a config that cannot work must not load");

        assert!(
            format!("{error:#}").contains(expected),
            "for {name}: {error:#}"
        );
    }
}

#[test]
fn hntui_env_key_reports_that_it_shadows_the_file() {
    let _lock = env_lock().lock().expect("env lock poisoned");
    let _env = EnvGuard::set("HNTUI_LLM_API_KEY", "from-env");
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[summarize]\nmodel = \"openai/test\"\napi_key = \"from-file\"\n",
    )
    .expect("write config");
    let config = Config::load_from(vec![path.clone()], path).expect("load config");

    let effective = config.effective_api_key();

    assert_eq!(effective.value.as_deref(), Some("from-env"));
    assert_eq!(effective.source, ValueSource::HntuiEnv);
    assert!(effective.file_shadowed);
}

#[test]
fn file_key_precedes_provider_env_without_becoming_a_provider_override() {
    let _lock = env_lock().lock().expect("env lock poisoned");
    let _hntui = EnvGuard::unset("HNTUI_LLM_API_KEY");
    let _provider = EnvGuard::set("OPENAI_API_KEY", "from-provider");
    let dir = tempfile::tempdir().expect("temp dir");
    let with_key = dir.path().join("with-key.toml");
    let provider_only = dir.path().join("provider-only.toml");
    std::fs::write(
        &with_key,
        "[summarize]\nmodel = \"openai/test\"\napi_key = \"from-file\"\n",
    )
    .expect("write file-key config");
    std::fs::write(&provider_only, "[summarize]\nmodel = \"openai/test\"\n")
        .expect("write provider config");

    let file_config =
        Config::load_from(vec![with_key.clone()], with_key).expect("load file config");
    let provider_config = Config::load_from(vec![provider_only.clone()], provider_only)
        .expect("load provider config");

    assert!(matches!(
        file_config.effective_api_key().source,
        ValueSource::File(_)
    ));
    assert_eq!(file_config.api_key_override().as_deref(), Some("from-file"));
    assert_eq!(
        provider_config.effective_api_key().source,
        ValueSource::ProviderEnv("OPENAI_API_KEY".to_string())
    );
    assert_eq!(provider_config.api_key_override(), None);
}

#[test]
fn article_bin_defaults_to_path_lookup_and_honours_an_override() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bare = dir.path().join("bare.toml");
    let overridden = dir.path().join("overridden.toml");
    std::fs::write(&bare, "[summarize]\nmodel = \"openai/test\"\n").expect("write bare config");
    std::fs::write(&overridden, "[article]\nbin = \"/opt/bin/localwebrs\"\n")
        .expect("write overridden config");

    let bare = Config::load_from(vec![bare.clone()], bare).expect("load bare config");
    let overridden =
        Config::load_from(vec![overridden.clone()], overridden).expect("load override config");

    assert_eq!(bare.article_bin(), "localwebrs");
    assert_eq!(overridden.article_bin(), "/opt/bin/localwebrs");
}
