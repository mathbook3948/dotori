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
