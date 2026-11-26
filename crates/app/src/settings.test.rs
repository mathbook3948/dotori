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
    assert!(matches!(missing, ModelError::MissingKey { name, .. } if name == "MY_KEY"));
    assert!(
        settings
            .model(|name| (name == "MY_KEY").then(|| String::from("k")))
            .is_ok()
    );
    let error = Settings::default()
        .model(|_| None)
        .err()
        .expect("needs a key");
    assert!(matches!(&error, ModelError::MissingKey { name, .. } if name == "ANTHROPIC_API_KEY"));
    assert_eq!(
        error.to_string(),
        "set ANTHROPIC_API_KEY to your Anthropic API key"
    );
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

fn channels(
    settings: &Settings,
    key: impl Fn(&str) -> Option<String>,
) -> Result<Vec<Arc<dyn dotori_core::channel::Channel>>, String> {
    let (built, off) = settings.channels(key);
    if off.is_empty() {
        Ok(built
            .into_iter()
            .map(|built| built as Arc<dyn dotori_core::channel::Channel>)
            .collect())
    } else {
        Err(off.join("; "))
    }
}

#[test]
fn without_a_slack_section_there_is_no_other_channel() {
    assert!(
        channels(&Settings::default(), |_| None)
            .expect("built")
            .is_empty()
    );
}

#[test]
fn slack_takes_both_tokens_from_their_variables_and_names_the_owner() {
    let settings = parse("[slack]\nowner = \"U012AB3CD\"").expect("parsed");
    let missing = channels(&settings, |_| None).err().expect("needs tokens");
    assert!(missing.contains("SLACK_APP_TOKEN"), "{missing}");
    let only_app = channels(&settings, |name| {
        (name == "SLACK_APP_TOKEN").then(|| String::from("xapp"))
    })
    .err()
    .expect("needs the bot token");
    assert!(only_app.contains("SLACK_BOT_TOKEN"), "{only_app}");

    let built = channels(&settings, |_| Some(String::from("token"))).expect("built");
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].id().as_str(), "slack");
}

#[test]
fn slack_stays_off_until_it_knows_whose_messages_to_hear() {
    let key = |_: &str| Some(String::from("token"));
    for text in ["[slack]", "[slack]\nowner = \"  \""] {
        let settings = parse(text).expect("parsed");
        let error = channels(&settings, key).err().expect("needs the owner");
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

#[test]
fn relayed_servers_come_by_address_or_command_and_take_tokens_from_variables() {
    let settings = parse(
        "[[mcp]]\nname = \"notes\"\nurl = \"https://notes.example/mcp\"\ntoken_env = \"NOTES\"\n\n\
         [[mcp]]\nname = \"files\"\ncommand = \"files-mcp\"\nargs = [\"--root\", \"/x\"]\ntools = [\"read\"]\n\n\
         [[mcp]]\nname = \"both\"\nurl = \"https://a\"\ncommand = \"b\"",
    )
    .expect("parsed");
    let (chosen, wrong) = settings.relays(|_| None, false, &[]);
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].name, "files");
    assert_eq!(chosen[0].tools, Some(vec![String::from("read")]));
    assert_eq!(wrong.len(), 2);
    assert!(wrong[0].contains("NOTES"));
    let (chosen, wrong) = settings.relays(
        |name| (name == "NOTES").then(|| String::from("t")),
        false,
        &[],
    );
    assert_eq!(chosen.len(), 2);
    assert_eq!(wrong.len(), 1);
    let text = toml::to_string(&settings).expect("written");
    assert_eq!(parse(&text).expect("read back"), settings);
}

#[test]
fn a_relayed_token_written_into_the_file_is_refused() {
    assert!(parse("[[mcp]]\nname = \"n\"\nurl = \"https://a\"\ntoken = \"t\"").is_err());
}

#[test]
fn the_peers_section_says_how_other_pcs_reach_this_one_and_survives_a_save() {
    let settings = parse(
        "[peers]\nlisten = \"0.0.0.0:47820\"\naddress = \"desk.ts.net:47820\"\nname = \"데스크톱\"",
    )
    .expect("parsed");
    let peers = settings.peers.clone().expect("peers");
    assert_eq!(peers.address.as_deref(), Some("desk.ts.net:47820"));
    assert_eq!(crate::peers::name(&settings), "데스크톱");
    let text = toml::to_string(&settings).expect("written");
    assert_eq!(parse(&text).expect("read back"), settings);
    assert!(parse("[peers]\nsecret = \"x\"").is_err());
}

#[test]
fn saving_some_sections_keeps_the_owners_comments_and_the_rest_of_the_file() {
    let folder = std::env::temp_dir().join(format!("dotori-parts-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("folder");
    let path = folder.join("settings.toml");
    std::fs::write(
        &path,
        "# 내 설정\n[model]\nprovider = \"anthropic\"\n\n# 노트북만\n[peers]\nname = \"노트북\" # 이름\n\n[memory]\n",
    )
    .expect("written");
    let mut settings = Settings::load(&path).expect("loaded");
    settings.model = ModelSettings::OpenaiCompatible {
        base_url: String::from("http://x"),
        model: String::from("m"),
        api_key_env: None,
        max_tokens: None,
    };
    settings.memory = None;

    settings
        .save_parts(&path, &["model", "memory"])
        .expect("saved");

    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("# 내 설정"), "{text}");
    assert!(text.contains("# 노트북만"), "{text}");
    assert!(text.contains("name = \"노트북\" # 이름"), "{text}");
    assert!(!text.contains("[memory]"), "{text}");
    assert_eq!(Settings::load(&path).expect("read back"), settings);
}

#[test]
fn the_slack_form_turns_into_a_section_or_none() {
    let off = SlackForm::default();
    assert_eq!(off.settings(), Ok(None));

    let on = SlackForm {
        on: true,
        owner: Some(String::from("U012AB3CD")),
        app_token_env: None,
        bot_token_env: Some(String::from("MY_BOT")),
    };
    let section = on.settings().expect("valid").expect("on");
    assert_eq!(section.owner.as_deref(), Some("U012AB3CD"));
    assert_eq!(section.bot_token_env.as_deref(), Some("MY_BOT"));
    assert_eq!(SlackForm::of(Some(&section)), on);
    assert_eq!(SlackForm::of(None), off);
}

#[test]
fn the_slack_form_refuses_a_missing_owner_and_a_pasted_token() {
    let mut form = SlackForm {
        on: true,
        ..SlackForm::default()
    };
    assert!(form.settings().is_err());
    form.owner = Some(String::from("someone"));
    assert!(form.settings().is_err());
    form.owner = Some(String::from("u012ab3cd"));
    assert_eq!(
        form.settings()
            .expect("valid")
            .expect("on")
            .owner
            .as_deref(),
        Some("U012AB3CD")
    );
    form.app_token_env = Some(String::from("xapp-1-A0-123-abc"));
    let refused = form.settings().expect_err("a token is not a name");
    assert_eq!(
        refused,
        (crate::words::words().settings.not_a_key_name)("xapp-1-A0-123-abc")
    );
}

#[test]
fn relayed_tools_the_owner_listed_wait_for_their_ok() {
    let settings = parse(
        "[[mcp]]\nname = \"mail\"\ncommand = \"mail-mcp\"\nconfirm = [\"send\"]\n\n\
         [[mcp]]\nname = \"chat\"\nurl = \"https://chat.example/mcp\"\nconfirm = [\"*\"]\n\n\
         [[mcp]]\nname = \"notes\"\ncommand = \"notes-mcp\"",
    )
    .expect("parsed");

    let asked = settings.asked();

    let shared: Vec<String> = settings
        .relays(|_| None, false, &[])
        .0
        .into_iter()
        .map(|config| config.name)
        .collect();
    let guarded: Vec<String> = settings
        .relays(|_| None, true, &[])
        .0
        .into_iter()
        .map(|config| config.name)
        .collect();
    assert_eq!(shared, ["notes"]);
    assert_eq!(guarded, ["mail", "chat"]);
    assert!(asked.needs("mail__send"));
    assert!(!asked.needs("mail__read"));
    assert!(asked.needs("chat__post"));
    assert!(!asked.needs("notes__write"));
    assert!(!asked.needs("send"));
    assert_eq!(
        asked.unmatched(["mail__send", "chat__post"].into_iter()),
        Vec::<String>::new()
    );
    assert_eq!(
        asked.unmatched(["mail__Send"].into_iter()),
        ["mail__send", "chat__"]
    );
}

#[test]
fn a_mistyped_confirm_name_makes_its_whole_server_wait_and_its_token_stays_hidden() {
    let settings = parse(
        "[[mcp]]\nname = \"mail\"\nurl = \"https://mail.example/mcp\"\ntoken_env = \"MAIL_TOKEN\"\n\
         confirm = [\"send_mesage\"]\n\n\
         [[mcp]]\nname = \"notes\"\ncommand = \"notes-mcp\"\ntoken_env = \"NOTES_TOKEN\"\n\n\
         [[mcp]]\nname = \"mail\"\ncommand = \"other-mail\"",
    )
    .expect("parsed");
    let mut asked = settings.asked();

    for name in asked.unmatched(["mail__send_message"].into_iter()) {
        asked.close_server_of(&name);
    }

    assert!(asked.needs("mail__send_message"));
    assert!(asked.needs("mail__read"));
    assert!(!asked.needs("notes__write"));
    assert_eq!(settings.guarded_secrets(), ["MAIL_TOKEN"]);
    let (shared, wrong) = settings.relays(|_| Some(String::from("t")), false, &[]);
    let names: Vec<&str> = shared.iter().map(|config| config.name.as_str()).collect();
    assert_eq!(names, ["notes"]);
    assert!(wrong[0].contains("already has this name"), "{wrong:?}");
}

#[test]
fn telegram_needs_its_token_and_owner_and_stays_apart_from_slack() {
    let settings =
        parse("[slack]\nowner = \"U1\"\n\n[telegram]\nowner = 42\ntoken_env = \"MY_TG\"")
            .expect("parsed");

    let (built, off) = settings.channels(|name| (name != "MY_TG").then(|| String::from("t")));
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].id().as_str(), "slack");
    assert!(
        off[0].contains("Telegram is off") && off[0].contains("MY_TG"),
        "{off:?}"
    );

    let (built, off) = settings.channels(|_| Some(String::from("123:t")));
    assert!(off.is_empty(), "{off:?}");
    let names: Vec<&str> = built.iter().map(|channel| channel.id().as_str()).collect();
    assert_eq!(names, ["slack", "telegram"]);

    let (_, off) = parse("[telegram]")
        .expect("parsed")
        .channels(|_| Some(String::from("t")));
    assert!(off[0].contains("owner"), "{off:?}");
    assert!(parse("[telegram]\ntoken = \"123:t\"").is_err());
}

#[test]
fn the_telegram_form_needs_a_numeric_owner_and_a_variable_name() {
    let mut form = TelegramForm {
        on: true,
        ..TelegramForm::default()
    };
    assert!(form.settings().is_err());
    form.owner = Some(String::from("@me"));
    assert!(form.settings().is_err());
    form.owner = Some(String::from("123456789"));
    form.token_env = Some(String::from("123:secret"));
    assert_eq!(
        form.settings().expect_err("a token"),
        (crate::words::words().settings.not_a_key_name)("123:secret")
    );
    form.token_env = None;

    let section = form.settings().expect("valid").expect("on");

    assert_eq!(section.owner, Some(123_456_789));
    assert_eq!(TelegramForm::of(Some(&section)), form);
    assert_eq!(TelegramForm::default().settings(), Ok(None));
}

#[test]
fn discord_needs_its_token_and_owner_and_stays_apart_from_the_others() {
    let settings = parse(
        "[telegram]\nowner = 42\n\n[discord]\nowner = \"123456789012345678\"\ntoken_env = \"MY_DC\"",
    )
    .expect("parsed");

    let (built, off) = settings.channels(|name| (name != "MY_DC").then(|| String::from("t")));
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].id().as_str(), "telegram");
    assert!(
        off[0].contains("Discord is off") && off[0].contains("MY_DC"),
        "{off:?}"
    );

    let (built, off) = settings.channels(|_| Some(String::from("t")));
    assert!(off.is_empty(), "{off:?}");
    let names: Vec<&str> = built.iter().map(|channel| channel.id().as_str()).collect();
    assert_eq!(names, ["telegram", "discord"]);

    let (_, off) = parse("[discord]\nowner = \"me\"")
        .expect("parsed")
        .channels(|_| Some(String::from("t")));
    assert!(off[0].contains("owner"), "{off:?}");
    assert!(parse("[discord]\ntoken = \"t\"").is_err());
}

#[test]
fn the_discord_form_needs_a_snowflake_owner_and_a_variable_name() {
    let mut form = DiscordForm {
        on: true,
        ..DiscordForm::default()
    };
    assert!(form.settings().is_err());
    form.owner = Some(String::from("yuno#1234"));
    assert!(form.settings().is_err());
    form.owner = Some(String::from("42"));
    assert!(form.settings().is_err());
    form.owner = Some(String::from("123456789012345678"));
    form.token_env = Some(String::from("MTIz.abc.def"));
    assert_eq!(
        form.settings().expect_err("a token"),
        (crate::words::words().settings.not_a_key_name)("MTIz.abc.def")
    );
    form.token_env = None;

    let section = form.settings().expect("valid").expect("on");

    assert_eq!(section.owner.as_deref(), Some("123456789012345678"));
    assert_eq!(DiscordForm::of(Some(&section)), form);
    assert_eq!(DiscordForm::default().settings(), Ok(None));
}

#[test]
fn the_assistant_is_told_the_name_the_owner_gave_it() {
    let settings = Settings {
        assistant: Some(AssistantSettings {
            name: Some(String::from("별")),
            icon: None,
        }),
        ..Settings::default()
    };

    let told = settings.named(DEFAULT_SYSTEM);

    assert!(
        told.starts_with("You are 별, the owner's personal assistant"),
        "{told}"
    );
    assert!(
        told.contains(&format!(
            "When nothing shows it, write in {}.",
            crate::language::current().english_name()
        )),
        "{told}"
    );
    assert!(
        settings
            .named(SUB_SESSION_SYSTEM)
            .contains("sub-session of 별,")
    );
}

#[test]
fn every_name_a_secret_is_read_under_is_a_secret_name() {
    for model in [
        "[model]\nprovider = \"anthropic\"\napi_key_env = \"MY_ANTHROPIC\"",
        "[model]\nprovider = \"anthropic\"",
        "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://a\"\nmodel = \"m\"\napi_key_env = \"MY_OPENAI\"",
    ] {
        let settings = parse(&format!(
            "{model}\n\n[slack]\napp_token_env = \"MY_APP\"\nbot_token_env = \"MY_BOT\"\nowner = \"U1\"\n\n\
             [telegram]\nowner = 42\ntoken_env = \"MY_TG\"\n\n\
             [discord]\nowner = \"123456789012345678\"\n\n\
             [memory.embeddings]\nbase_url = \"http://e\"\nmodel = \"e\"\napi_key_env = \"MY_EMBED\"\n\n\
             [[mcp]]\nname = \"mail\"\nurl = \"https://mail.example/mcp\"\ntoken_env = \"MAIL_TOKEN\"\nconfirm = [\"*\"]\n\n\
             [[mcp]]\nname = \"notes\"\ncommand = \"notes-mcp\"\ntoken_env = \"NOTES_TOKEN\""
        ))
        .expect("parsed");
        let asked = std::cell::RefCell::new(Vec::new());
        let key = |name: &str| {
            asked.borrow_mut().push(name.to_owned());
            Some(String::from("value"))
        };

        settings.model(key).expect("model");
        settings.sub_session_model(key).expect("sub-session model");
        settings.embedding(key).expect("embedding");
        let (_, off) = settings.channels(key);
        assert!(off.is_empty(), "{off:?}");
        settings.relays(key, true, &[]);
        settings.relays(key, false, &[]);

        let names = settings.secret_names();
        let asked = asked.into_inner();
        assert!(asked.len() >= 8, "{asked:?}");
        for name in asked {
            assert!(names.contains(&name), "{name} is read but not hidden");
        }
        assert!(names.iter().any(|name| name == MODEL_KEY));
    }
}

#[test]
fn a_command_server_gets_its_token_by_name_and_the_hidden_names() {
    let settings =
        parse("[[mcp]]\nname = \"notes\"\ncommand = \"notes-mcp\"\ntoken_env = \"NOTES\"")
            .expect("parsed");
    let hidden = vec![String::from("NOTES"), String::from("ANTHROPIC_API_KEY")];

    let (left_out, wrong) = settings.relays(|_| None, false, &hidden);
    assert!(left_out.is_empty());
    assert!(wrong[0].contains("NOTES"), "{wrong:?}");
    let (chosen, _) = settings.relays(|_| Some(String::from("t")), false, &hidden);

    assert_eq!(
        chosen[0].reach,
        Reach::Command {
            program: String::from("notes-mcp"),
            args: Vec::new(),
            token: Some((String::from("NOTES"), String::from("t"))),
            hidden_env: hidden,
        }
    );
}

#[test]
fn a_system_variable_is_never_a_token_nor_kept_from_commands() {
    let settings = parse(
        "[[mcp]]\nname = \"notes\"\ncommand = \"notes-mcp\"\ntoken_env = \"PATH\"\n\n\
         [[mcp]]\nname = \"files\"\nurl = \"https://files.example/mcp\"\ntoken_env = \"home\"",
    )
    .expect("parsed");

    let (chosen, wrong) = settings.relays(|_| Some(String::from("t")), false, &[]);

    assert!(chosen.is_empty(), "{chosen:?}");
    assert_eq!(wrong.len(), 2, "{wrong:?}");
    assert!(wrong[0].contains("PATH"), "{wrong:?}");
    let names = settings.secret_names();
    assert!(!names.iter().any(|name| reserved(name)), "{names:?}");
    assert!(reserved("LC_ALL") && reserved("XDG_RUNTIME_DIR") && !reserved("NOTES_TOKEN"));
}
