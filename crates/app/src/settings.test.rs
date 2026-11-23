use super::*;

fn parse(text: &str) -> Result<Settings, toml::de::Error> {
    toml::from_str(text)
}

#[test]
fn an_empty_file_means_anthropic_with_every_default() {
    assert_eq!(parse("").expect("parsed"), Settings::default());
}

#[test]
fn a_missing_file_means_every_default() {
    let path = std::env::temp_dir().join("dotori-settings-that-does-not-exist.toml");

    assert_eq!(Settings::load(&path).expect("loaded"), Settings::default());
}

#[test]
fn an_openai_compatible_server_needs_an_address_and_a_model() {
    let settings = parse(
        r#"
[model]
provider = "openai-compatible"
base_url = "http://localhost:11434/v1"
model = "qwen3"
"#,
    )
    .expect("parsed");

    assert_eq!(
        settings.model,
        ModelSettings::OpenaiCompatible {
            base_url: String::from("http://localhost:11434/v1"),
            model: String::from("qwen3"),
            api_key_env: None,
            max_tokens: None,
        }
    );
    assert!(parse("[model]\nprovider = \"openai-compatible\"\nmodel = \"x\"").is_err());
}

#[test]
fn a_key_written_into_the_file_is_refused() {
    assert!(parse("[model]\nprovider = \"anthropic\"\napi_key = \"sk-ant\"").is_err());
}

#[test]
fn the_anthropic_key_comes_from_the_named_variable() {
    let settings =
        parse("[model]\nprovider = \"anthropic\"\napi_key_env = \"MY_KEY\"").expect("parsed");

    let missing = settings.model(|_| None).err().expect("needs a key");
    assert!(missing.contains("MY_KEY"));
    assert!(
        settings
            .model(|name| (name == "MY_KEY").then(|| String::from("k")))
            .is_ok()
    );
    let error = Settings::default()
        .model(|_| None)
        .err()
        .expect("needs a key");
    assert!(error.contains("ANTHROPIC_API_KEY"));
}

#[test]
fn settings_written_back_read_the_same() {
    let settings = parse("[model]\nprovider = \"anthropic\"\neffort = \"low\"").expect("parsed");

    let text = toml::to_string(&settings).expect("written");

    assert_eq!(parse(&text).expect("parsed"), settings);
}

#[test]
fn a_form_round_trips_and_keeps_what_it_does_not_show() {
    let earlier = ModelSettings::Anthropic {
        api_key_env: Some(String::from("KEY")),
        model: None,
        base_url: None,
        max_tokens: Some(8000),
        effort: Some(String::from("low")),
    };

    let form = Form::of(&earlier);

    assert_eq!(form.clone().settings(&earlier).expect("valid"), earlier);
    let switched = Form {
        openai_compatible: true,
        model: Some(String::from("qwen3")),
        base_url: Some(String::from("http://localhost:11434/v1")),
        ..form
    };
    assert_eq!(
        switched.settings(&earlier).expect("valid"),
        ModelSettings::OpenaiCompatible {
            base_url: String::from("http://localhost:11434/v1"),
            model: String::from("qwen3"),
            api_key_env: Some(String::from("KEY")),
            max_tokens: Some(8000),
        }
    );
}

#[test]
fn an_openai_compatible_form_without_an_address_is_refused() {
    let form = Form {
        openai_compatible: true,
        model: Some(String::from("qwen3")),
        ..Form::default()
    };

    assert!(form.settings(&ModelSettings::default()).is_err());
}

#[test]
fn saved_settings_load_back() {
    let path = std::env::temp_dir()
        .join(format!("dotori-settings-{}", std::process::id()))
        .join("settings.toml");
    let settings = parse("[model]\nprovider = \"anthropic\"\neffort = \"high\"").expect("parsed");

    settings.save(&path).expect("saved");

    assert_eq!(Settings::load(&path).expect("loaded"), settings);
}

#[test]
fn without_a_slack_section_there_is_no_other_channel() {
    assert!(
        Settings::default()
            .channels(|_| None)
            .expect("built")
            .is_empty()
    );
}

#[test]
fn slack_takes_both_tokens_from_their_variables_and_names_the_owner() {
    let settings = parse("[slack]\nowner = \"U012AB3CD\"").expect("parsed");
    let missing = settings.channels(|_| None).err().expect("needs tokens");
    assert!(missing.contains("SLACK_APP_TOKEN"), "{missing}");
    let only_app = settings
        .channels(|name| (name == "SLACK_APP_TOKEN").then(|| String::from("xapp")))
        .err()
        .expect("needs the bot token");
    assert!(only_app.contains("SLACK_BOT_TOKEN"), "{only_app}");

    let channels = settings
        .channels(|_| Some(String::from("token")))
        .expect("built");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].id().as_str(), "slack");
}

#[test]
fn slack_stays_off_until_it_knows_whose_messages_to_hear() {
    let key = |_: &str| Some(String::from("token"));
    for text in ["[slack]", "[slack]\nowner = \"  \""] {
        let settings = parse(text).expect("parsed");
        let error = settings.channels(key).err().expect("needs the owner");
        assert!(error.contains("owner"), "{error}");
    }
}

#[test]
fn a_slack_token_written_into_the_file_is_refused() {
    assert!(parse("[slack]\nbot_token = \"xoxb-1\"").is_err());
}

#[test]
fn slack_settings_survive_a_save() {
    let settings = parse(
        "[model]\nprovider = \"anthropic\"\n\n[slack]\nbot_token_env = \"MY_BOT\"\nowner = \"U1\"",
    )
    .expect("parsed");
    let text = toml::to_string(&settings).expect("written");
    assert_eq!(parse(&text).expect("read back"), settings);
}

#[test]
fn a_browser_section_from_before_still_loads_and_forgets_headless() {
    let settings = parse("[browser]\npath = \"/opt/chrome\"\nheadless = true").expect("parsed");
    assert!(
        !toml::to_string(&settings)
            .expect("written")
            .contains("headless")
    );
    assert_eq!(
        settings.browser,
        Some(BrowserSettings {
            path: Some(PathBuf::from("/opt/chrome")),
            old_headless: Some(true),
        })
    );
    assert!(parse("[browser]\nprofile = \"x\"").is_err());
}

#[test]
fn the_sub_session_section_names_a_runner() {
    let settings = parse("[sub_sessions]\nrunner = \"claude-code\"").expect("parsed");
    assert_eq!(
        settings.sub_sessions,
        Some(SubSessionSettings {
            runner: Some(RunnerChoice::ClaudeCode),
            path: None,
        })
    );
    assert!(parse("[sub_sessions]\nrunner = \"cursor\"").is_err());
}

#[test]
fn a_file_written_before_the_rename_still_names_the_runner() {
    let settings = parse("[sub_tasks]\nrunner = \"codex\"").expect("parsed");
    assert_eq!(
        settings.sub_sessions.and_then(|chosen| chosen.runner),
        Some(RunnerChoice::Codex)
    );
}

#[test]
fn embeddings_are_off_unless_named_and_take_their_key_from_a_variable() {
    let none = Settings::default();
    assert_eq!(none.embedding(|_| None), Ok(None));

    let settings = parse(
        "[memory.embeddings]\nbase_url = \"https://api.openai.com/v1\"\nmodel = \"text-embedding-3-small\"\napi_key_env = \"OPENAI_API_KEY\"\ndimensions = 512",
    )
    .expect("parsed");
    let config = settings
        .embedding(|name| (name == "OPENAI_API_KEY").then(|| String::from("sk-test")))
        .expect("built")
        .expect("on");
    assert_eq!(config.model, "text-embedding-3-small");
    assert_eq!(config.api_key.as_deref(), Some("sk-test"));
    assert_eq!(config.dimensions, Some(512));
    let missing = settings.embedding(|_| None).expect_err("no key");
    assert!(missing.contains("OPENAI_API_KEY"), "{missing}");

    let local =
        parse("[memory.embeddings]\nbase_url = \"http://localhost:11434/v1\"\nmodel = \"bge-m3\"")
            .expect("parsed");
    assert_eq!(
        local
            .embedding(|_| None)
            .expect("built")
            .expect("on")
            .api_key,
        None
    );
    assert!(
        parse("[memory.embeddings]\nmodel = \"x\"\nbase_url = \"y\"\napi_key = \"sk\"").is_err()
    );
}
