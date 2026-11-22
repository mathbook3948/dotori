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
    let (channel, owner) = &channels[0];
    assert_eq!(channel.id().as_str(), "slack");
    let owner = owner.as_ref().expect("owner set");
    assert_eq!(owner.channel.as_str(), "slack");
    assert_eq!(owner.user.as_str(), "U012AB3CD");
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
