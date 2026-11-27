use dotori_connectors::peer::hub::JoinError;
use gpui::Modifiers;
use gpui::TestAppContext;
use gpui::VisualTestContext;

use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn type_and_leave(
    view: &Entity<SettingsView>,
    cx: &mut VisualTestContext,
    field: fn(&SettingsView, &App) -> Entity<TextInput>,
    text: &str,
) {
    cx.update(|window, cx| {
        window.activate_window();
        let input = field(view.read(cx), cx);
        window.focus(&input.focus_handle(cx));
        input.update(cx, |input, cx| input.set_text(text, cx));
        window.refresh();
    });
    cx.run_until_parked();
    cx.update(|window, _| {
        window.blur();
        window.refresh();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn picking_a_theme_and_a_look_saves_them_and_draws_with_them(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("dotori-look-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), "").expect("settings");
    let home = Home { root: root.clone() };
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (_view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();

    click(cx, "theme-forest");
    click(cx, "look-dark");

    let saved = Settings::load(&root.join("settings.toml"))
        .expect("loaded")
        .window
        .expect("a window section");
    assert_eq!(saved.theme.as_deref(), Some("forest"));
    assert_eq!(saved.look.as_deref(), Some("dark"));
    let (index, look) = crate::window::theme::chosen();
    assert_eq!(crate::window::theme::names().nth(index), Some("forest"));
    assert_eq!(look, Look::Dark);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_pasted_key_goes_into_the_vault_and_can_be_forgotten(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("dotori-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), "").expect("settings");
    let home = Home { root: root.clone() };
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();

    click(cx, "section-model");
    type_and_leave(&view, cx, |view, _| view.api_key.clone(), "sk-ant-pasted");

    let vault = Vault::new(&root);
    assert_eq!(
        vault.get(ANTHROPIC_KEY).expect("read").as_deref(),
        Some("sk-ant-pasted")
    );
    assert!(view.read_with(cx, |view, cx| view.api_key.read(cx).text().is_empty()));
    let file = std::fs::read_to_string(root.join("settings.toml")).expect("settings");
    assert!(
        !file.contains("sk-ant-pasted"),
        "the key never reaches the settings file"
    );

    view.update(cx, |view, cx| view.forget(ANTHROPIC_KEY, cx));

    assert_eq!(vault.get(ANTHROPIC_KEY).expect("read"), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_key_typed_for_one_provider_is_not_saved_for_the_other(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("dotori-provider-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), "").expect("settings");
    let home = Home { root: root.clone() };
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();

    view.update(cx, |view, cx| {
        view.api_key
            .update(cx, |input, cx| input.set_text("sk-ant-typed", cx));
        view.choose(true, cx);
    });

    assert!(view.read_with(cx, |view, cx| view.api_key.read(cx).text().is_empty()));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn forgetting_the_key_dotori_named_lets_the_model_go_without_one(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("dotori-keyless-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(
        root.join("settings.toml"),
        "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n",
    )
    .expect("settings");
    let home = Home { root: root.clone() };
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();
    click(cx, "section-model");
    type_and_leave(&view, cx, |view, _| view.api_key.clone(), "sk-local");
    let named = Settings::load(&root.join("settings.toml")).expect("loaded");
    assert!(
        named
            .model(|name| (name == MODEL_KEY).then(|| String::from("v")))
            .is_ok()
    );

    view.update(cx, |view, cx| view.forget(MODEL_KEY, cx));

    let keyless = Settings::load(&root.join("settings.toml")).expect("loaded");
    assert!(
        keyless.model(|_| None).is_ok(),
        "no key name is left behind"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn the_owner_names_the_assistant_and_gives_it_a_picture(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("dotori-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), "").expect("settings");
    let home = Home { root: root.clone() };
    let icons = home.icons();
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();
    let mut png = Vec::new();
    image::DynamicImage::new_rgba8(40, 30)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("a picture");

    view.update(cx, |view, cx| view.use_icon(icons::keep(&icons, &png), cx));
    type_and_leave(&view, cx, |view, _| view.name.clone(), "도토리");

    let kept = Settings::load(&root.join("settings.toml"))
        .expect("loaded")
        .assistant
        .expect("an assistant section");
    assert_eq!(kept.name.as_deref(), Some("도토리"));
    let icon = kept.icon.expect("a picture");
    assert!(icons::path(&icons, &icon).is_some());

    click(cx, "avatar");
    assert!(
        cx.debug_bounds("avatar-other").is_some(),
        "a picture of the owner's opens the menu"
    );
    click(cx, "avatar-default");
    assert_eq!(
        view.read_with(cx, |view, _| view.open_picker),
        None,
        "the menu closes"
    );

    let back = Settings::load(&root.join("settings.toml"))
        .expect("loaded")
        .assistant
        .expect("still named");
    assert_eq!(back.icon, None);
    assert_eq!(back.name.as_deref(), Some("도토리"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_name_longer_than_allowed_is_refused() {
    let long = "가".repeat(ASSISTANT_NAME_LONGEST + 1);

    assert!(assistant_named(None, &long).is_err());
    assert_eq!(assistant_named(None, ASSISTANT_NAME).expect("fine"), None);
}

#[gpui::test]
fn each_linked_account_has_its_own_row_and_one_can_be_unlinked(cx: &mut TestAppContext) {
    use dotori_mcp::card::Account;
    use dotori_mcp::card::Accounts;

    let root = std::env::temp_dir().join(format!("dotori-sites-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), "").expect("settings");
    let kept = accounts::VaultAccounts {
        vault: Arc::new(Vault::new(&root)),
    };
    let first = Account {
        site: String::from("쿠팡"),
        origin: String::from("https://www.coupang.com"),
        id_label: Some(String::from("아이디")),
        id: Some(String::from("minji.example@mail.com")),
        password_label: String::from("비밀번호"),
        password: String::from("hunter22!"),
        trusted: false,
        page: Some(String::from("/login")),
        kept_ms: 0,
    };
    let second = Account {
        id: Some(String::from("family@mail.com")),
        ..first.clone()
    };
    kept.keep(&first).expect("kept");
    kept.keep(&second).expect("kept");
    let home = Home { root: root.clone() };
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();

    let shown = view.read_with(cx, |view, _| view.sites.clone());
    let ids: Vec<Option<&str>> = shown.iter().map(|site| site.id.as_deref()).collect();
    assert_eq!(
        ids,
        [Some("family@mail.com"), Some("minji.example@mail.com")]
    );
    assert!(
        view.read_with(cx, |view, _| view.kept_keys().is_empty()),
        "an account is not listed with the keys"
    );
    click(cx, "section-safety");
    let name = shown[1].name.clone();
    let selector: &'static str = Box::leak(format!("unlink-{name}").into_boxed_str());
    assert!(
        cx.debug_bounds(selector).is_some(),
        "each account has its own row"
    );

    view.update(cx, |view, cx| view.unlink(&name, cx));

    let left = view.read_with(cx, |view, _| view.sites.clone());
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id.as_deref(), Some("family@mail.com"));
    let ids: Vec<Option<String>> = kept
        .find("https://www.coupang.com")
        .expect("read")
        .into_iter()
        .map(|account| account.id)
        .collect();
    assert_eq!(ids, [second.id]);
    let _ = std::fs::remove_dir_all(&root);
}

fn onboarding_home(name: &str, settings: &str) -> (Home, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("dotori-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), settings).expect("settings");
    (Home { root: root.clone() }, root)
}

fn walked(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> Option<Step> {
    view.read_with(cx, |view, _| view.walk)
}

fn answered(answer: Result<(), CheckError>) -> oneshot::Receiver<Result<(), CheckError>> {
    let (tell, told) = oneshot::channel();
    let _ = tell.send(answer);
    told
}

fn checked_as(view: &Entity<SettingsView>, cx: &mut VisualTestContext, check: Check) {
    view.update(cx, |view, _| view.check = check);
}

fn walk_to(view: &Entity<SettingsView>, cx: &mut VisualTestContext, step: Step) {
    view.update(cx, |view, cx| {
        view.walk = Some(step);
        cx.notify();
    });
    cx.run_until_parked();
}

const NAMED_KEY_ABSENT: &str = "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\napi_key_env = \"DOTORI_TEST_KEY_THAT_IS_NEVER_SET\"\n";

#[gpui::test]
fn onboarding_starts_at_the_greeting_and_holds_on_the_key_until_one_is_there(
    cx: &mut TestAppContext,
) {
    let (home, root) = onboarding_home("onboarding-key", NAMED_KEY_ABSENT);
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    assert_eq!(walked(&view, cx), Some(Step::Welcome));

    click(cx, "next");
    assert_eq!(walked(&view, cx), Some(Step::Key));

    click(cx, "next");
    assert_eq!(walked(&view, cx), Some(Step::Key));
    let note = view.read_with(cx, |view, _| view.note.clone());
    assert!(matches!(note, Some(Note::Failed(error)) if error == words().settings.key_needed));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_model_without_a_key_name_passes_the_key_step(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home(
        "onboarding-keyless",
        "[model]
provider = \"openai-compatible\"
base_url = \"http://127.0.0.1:1/v1\"
model = \"m\"
",
    );
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Key);
    checked_as(&view, cx, |_, _, _| answered(Ok(())));

    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Look));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_typed_key_is_kept_in_the_vault_and_the_walk_moves_on(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("onboarding-vault", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Key);
    view.update(cx, |view, cx| {
        view.api_key
            .update(cx, |input, cx| input.set_text("sk-ant-walked", cx));
    });
    checked_as(&view, cx, |_, _, typed| {
        assert_eq!(
            typed,
            Some((ANTHROPIC_KEY.to_owned(), String::from("sk-ant-walked"))),
            "the typed key is checked before it is kept"
        );
        answered(Ok(()))
    });

    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Look));
    assert_eq!(
        Vault::new(&root)
            .get(ANTHROPIC_KEY)
            .expect("read")
            .as_deref(),
        Some("sk-ant-walked")
    );
    assert!(view.read_with(cx, |view, _| view.note.is_none()));
    let file = std::fs::read_to_string(root.join("settings.toml")).expect("settings");
    assert!(!file.contains("sk-ant-walked"));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_refused_key_holds_the_walk_and_is_not_kept(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("onboarding-refused", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Key);
    view.update(cx, |view, cx| {
        view.api_key
            .update(cx, |input, cx| input.set_text("sk-ant-wrong", cx));
    });
    checked_as(&view, cx, |_, _, _| answered(Err(CheckError::KeyRefused)));

    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Key));
    let note = view.read_with(cx, |view, _| view.note.clone());
    assert!(matches!(note, Some(Note::Failed(error)) if error == words().onboarding.key_refused));
    assert_eq!(Vault::new(&root).get(ANTHROPIC_KEY).expect("read"), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn back_goes_one_step_and_next_reaches_the_other_places(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("onboarding-walk", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Look);

    click(cx, "back");
    assert_eq!(walked(&view, cx), Some(Step::Key));

    walk_to(&view, cx, Step::Look);
    click(cx, "next");
    assert_eq!(walked(&view, cx), Some(Step::Elsewhere));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_channel_turned_on_without_its_details_keeps_the_last_step(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("onboarding-channel", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Elsewhere);

    click(cx, "telegram-switch");
    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Elsewhere));
    assert!(view.read_with(cx, |view, _| matches!(
        &view.note,
        Some(Note::Missing(gaps)) if gaps.iter().any(|gap| gap.part == "telegram")
    )));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_channel_left_half_done_does_not_hold_an_earlier_step(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home(
        "onboarding-back",
        "[model]
provider = \"openai-compatible\"
base_url = \"http://127.0.0.1:1/v1\"
model = \"m\"
",
    );
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    cx.run_until_parked();
    checked_as(&view, cx, |_, _, _| answered(Ok(())));
    walk_to(&view, cx, Step::Elsewhere);
    click(cx, "slack-switch");
    click(cx, "back");
    click(cx, "back");

    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Look));
    assert!(view.read_with(cx, |view, _| view.note.is_none()));
    let saved = Settings::load(&home.settings()).expect("saved");
    assert!(saved.slack.is_none(), "the channel waits for its own step");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_progress_bar_names_the_three_steps_after_the_greeting() {
    assert_eq!(Step::Welcome.label(), None);
    assert_eq!(
        Step::ALL.iter().filter_map(|step| step.label()).count(),
        3,
        "the progress bar counts the three steps after the greeting"
    );
}

thread_local! {
    static LATER: std::cell::RefCell<Option<oneshot::Sender<Result<(), CheckError>>>> =
        const { std::cell::RefCell::new(None) };
}

fn answered_later(
    _: Home,
    _: ModelSettings,
    _: Option<(String, String)>,
) -> oneshot::Receiver<Result<(), CheckError>> {
    let (tell, told) = oneshot::channel();
    LATER.with(|later| *later.borrow_mut() = Some(tell));
    told
}

fn answer_now(cx: &mut VisualTestContext, answer: Result<(), CheckError>) {
    let tell = LATER
        .with(|later| later.borrow_mut().take())
        .expect("a check waits");
    let _ = tell.send(answer);
    cx.run_until_parked();
}

fn key_typed(view: &Entity<SettingsView>, cx: &mut VisualTestContext, key: &str) {
    view.update(cx, |view, cx| {
        view.api_key.update(cx, |input, cx| input.set_text(key, cx));
    });
}

#[gpui::test]
fn the_key_that_was_checked_is_the_one_kept_whatever_changes_meanwhile(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("check-typed", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Key);
    key_typed(&view, cx, "sk-ant-checked");
    checked_as(&view, cx, answered_later);

    click(cx, "next");
    assert!(view.read_with(cx, |view, _| view.checking()));
    view.update_in(cx, |view, window, cx| {
        window.focus(&view.api_key.focus_handle(cx))
    });
    cx.simulate_input("-typed");
    assert_eq!(
        view.read_with(cx, |view, cx| view.api_key.read(cx).text().to_owned()),
        "sk-ant-checked",
        "the field holds still while the key is checked"
    );
    click(cx, "openai");
    assert!(view.read_with(cx, |view, _| !view.openai_compatible));
    key_typed(&view, cx, "sk-ant-other");

    answer_now(cx, Ok(()));

    assert_eq!(walked(&view, cx), Some(Step::Look));
    assert_eq!(
        Vault::new(&root)
            .get(ANTHROPIC_KEY)
            .expect("read")
            .as_deref(),
        Some("sk-ant-checked")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn back_waits_for_the_check_and_an_answer_after_the_walk_moved_is_dropped(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("check-back", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Key);
    key_typed(&view, cx, "sk-ant-late");
    checked_as(&view, cx, answered_later);

    click(cx, "next");
    click(cx, "back");
    assert_eq!(
        walked(&view, cx),
        Some(Step::Key),
        "back waits for the check"
    );

    view.update(cx, |view, cx| {
        view.walk = Some(Step::Welcome);
        view.moves += 1;
        cx.notify();
    });
    answer_now(cx, Ok(()));

    assert_eq!(walked(&view, cx), Some(Step::Welcome));
    assert_eq!(Vault::new(&root).get(ANTHROPIC_KEY).expect("read"), None);
    assert!(view.read_with(cx, |view, _| !view.checking()));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn onboarding_opens_again_at_the_step_left_unfinished(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("resume", "");
    first_run::write(&home.first_run(), FirstRun::At(Step::Look)).expect("written");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    cx.run_until_parked();
    assert_eq!(walked(&view, cx), Some(Step::Look));

    click(cx, "next");
    assert_eq!(walked(&view, cx), Some(Step::Elsewhere));
    assert_eq!(
        first_run::read(&home.first_run()),
        FirstRun::At(Step::Elsewhere)
    );

    view.update(cx, |view, _| view.open_chat = || {});
    click(cx, "next");
    assert_eq!(first_run::read(&home.first_run()), FirstRun::Done);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn slack_turned_on_needs_its_tokens_and_a_channel_left_off_keeps_none(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("tokens", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Elsewhere);
    click(cx, "slack-switch");
    view.update(cx, |view, cx| {
        view.slack_owner
            .update(cx, |input, cx| input.set_text("U012AB3CD", cx));
        view.telegram_token
            .update(cx, |input, cx| input.set_text("123:telegram", cx));
    });

    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Elsewhere));
    view.read_with(cx, |view, _| {
        let Some(Note::Missing(gaps)) = &view.note else {
            panic!("the missing tokens are named");
        };
        assert_eq!(gaps.len(), 1, "Telegram is off");
        let gap = &gaps[0];
        assert_eq!(gap.part, "slack");
        assert_eq!(
            gap.fields,
            vec![
                view.slack_app_token.entity_id(),
                view.slack_bot_token.entity_id()
            ]
        );
        let said = &words().settings;
        assert_eq!(
            gap.words,
            (said.channel_missing)("Slack", &[said.missing_app_token, said.missing_bot_token])
        );
    });
    let vault = Vault::new(&root);
    assert_eq!(
        vault.get(TELEGRAM_KEY).expect("read"),
        None,
        "Telegram is off"
    );
    assert!(
        Settings::load(&home.settings())
            .expect("saved")
            .slack
            .is_none()
    );

    view.update(cx, |view, cx| {
        view.slack_app_token
            .update(cx, |input, cx| input.set_text("xapp-1", cx));
        view.slack_bot_token
            .update(cx, |input, cx| input.set_text("xoxb-1", cx));
        view.open_chat = || {};
    });
    click(cx, "next");

    assert_eq!(
        vault.get(SLACK_APP_KEY).expect("read").as_deref(),
        Some("xapp-1")
    );
    assert_eq!(
        vault.get(SLACK_BOT_KEY).expect("read").as_deref(),
        Some("xoxb-1")
    );
    assert_eq!(
        vault.get(TELEGRAM_KEY).expect("read"),
        None,
        "Telegram is off"
    );
    assert!(
        Settings::load(&home.settings())
            .expect("saved")
            .slack
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_channel_token_waits_for_its_own_step(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home(
        "token-step",
        "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n",
    );
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    cx.run_until_parked();
    checked_as(&view, cx, |_, _, _| answered(Ok(())));
    walk_to(&view, cx, Step::Elsewhere);
    click(cx, "discord-switch");
    view.update(cx, |view, cx| {
        view.discord_token
            .update(cx, |input, cx| input.set_text("discord-token", cx));
    });
    walk_to(&view, cx, Step::Key);

    click(cx, "next");

    assert_eq!(walked(&view, cx), Some(Step::Look));
    assert_eq!(Vault::new(&root).get(DISCORD_KEY).expect("read"), None);
    let _ = std::fs::remove_dir_all(&root);
}

const MODEL_HERE: &str = "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n";

fn a_code() -> String {
    dotori_connectors::peer::Code {
        key: [1; 32],
        secret: [2; 16],
        address: Some(String::from("127.0.0.1:1")),
    }
    .write()
}

fn joined_laptop(
    _: &Home,
    _: &Settings,
    _: dotori_connectors::peer::Code,
    _: String,
) -> oneshot::Receiver<Result<dotori_connectors::peer::Peer, JoinError>> {
    let (tell, told) = oneshot::channel();
    let _ = tell.send(Ok(dotori_connectors::peer::Peer {
        device: String::from("abcdef123456"),
        name: String::from("laptop"),
        key: String::from("aa"),
        address: Some(String::from("127.0.0.1:1")),
        since: 0,
    }));
    told
}

fn refused(
    _: &Home,
    _: &Settings,
    _: dotori_connectors::peer::Code,
    _: String,
) -> oneshot::Receiver<Result<dotori_connectors::peer::Peer, JoinError>> {
    let (tell, told) = oneshot::channel();
    let _ = tell.send(Err(JoinError::There(String::from(
        "the other PC refused the code; make a new one there",
    ))));
    told
}

fn kept_nowhere(
    _: &Home,
    _: &Settings,
    _: dotori_connectors::peer::Code,
    _: String,
) -> oneshot::Receiver<Result<dotori_connectors::peer::Peer, JoinError>> {
    let (tell, told) = oneshot::channel();
    let _ = tell.send(Err(JoinError::Here(String::from("the book is locked"))));
    told
}

fn joining(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> Option<onboarding::Joining> {
    view.read_with(cx, |view, _| {
        view.join.as_ref().map(|join| join.state.clone())
    })
}

fn code_typed(view: &Entity<SettingsView>, cx: &mut VisualTestContext, code: &str) {
    view.update(cx, |view, cx| {
        if let Some(join) = &view.join {
            join.code.update(cx, |input, cx| input.set_text(code, cx));
        }
    });
}

#[gpui::test]
fn the_greeting_joins_an_existing_pc_and_starts_chatting_once_its_model_is_here(
    cx: &mut TestAppContext,
) {
    let (home, root) = onboarding_home("join", MODEL_HERE);
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    view.update(cx, |view, _| {
        view.joiner = joined_laptop;
        view.open_chat = || {};
    });
    cx.run_until_parked();

    click(cx, "join-pc");
    assert_eq!(joining(&view, cx), Some(onboarding::Joining::Typing(None)));
    code_typed(&view, cx, &a_code());
    click(cx, "next");

    assert_eq!(
        joining(&view, cx),
        Some(onboarding::Joining::Joined {
            name: String::from("laptop"),
            ready: true,
        })
    );
    assert_ne!(first_run::read(&home.first_run()), FirstRun::Done);
    click(cx, "next");
    assert_eq!(first_run::read(&home.first_run()), FirstRun::Done);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_code_that_does_not_read_stays_on_the_join_screen_and_says_why(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("join-unread", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    view.update(cx, |view, _| {
        view.joiner = |_, _, _, _| panic!("a code that does not read joins nothing");
    });
    cx.run_until_parked();
    click(cx, "join-pc");
    code_typed(&view, cx, "not a code");

    click(cx, "next");

    assert_eq!(
        joining(&view, cx),
        Some(onboarding::Joining::Typing(Some(
            onboarding::Failure::Typed(crate::window::pcs::Unjoinable::UnreadableCode)
        )))
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_refused_join_says_so_and_back_returns_to_the_greeting(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("join-refused", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    view.update(cx, |view, _| view.joiner = refused);
    cx.run_until_parked();
    click(cx, "join-pc");
    code_typed(&view, cx, &a_code());

    click(cx, "next");

    assert_eq!(
        joining(&view, cx),
        Some(onboarding::Joining::Typing(Some(
            onboarding::Failure::Joining(SharedString::from((words().onboarding.join_failed)()))
        )))
    );
    click(cx, "back");
    assert_eq!(joining(&view, cx), None);
    assert_eq!(walked(&view, cx), Some(Step::Welcome));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_join_that_failed_on_this_pc_says_so_instead_of_blaming_the_other(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("join-here", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::onboarding(home, settings, cx));
    view.update(cx, |view, _| view.joiner = kept_nowhere);
    cx.run_until_parked();
    click(cx, "join-pc");
    code_typed(&view, cx, &a_code());

    click(cx, "next");

    assert_eq!(
        joining(&view, cx),
        Some(onboarding::Joining::Typing(Some(
            onboarding::Failure::Joining(SharedString::from(
                (words().onboarding.join_here_failed)("the book is locked")
            ))
        )))
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn setting_up_here_after_a_join_shows_the_settings_that_arrived(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("join-reload", NAMED_KEY_ABSENT);
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    view.update(cx, |view, _| view.joiner = joined_laptop);
    cx.run_until_parked();
    click(cx, "join-pc");
    code_typed(&view, cx, &a_code());
    click(cx, "next");
    std::fs::write(
        home.settings(),
        NAMED_KEY_ABSENT.replace("model = \"m\"", "model = \"arrived\""),
    )
    .expect("the other PC's model arrives");

    click(cx, "set-up-here");

    let shown = view.read_with(cx, |view, cx| view.model.read(cx).text().to_owned());
    assert_eq!(shown, "arrived");
    assert_eq!(walked(&view, cx), Some(Step::Key));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn joined_before_the_model_arrives_holds_the_chat_and_can_set_up_here_instead(
    cx: &mut TestAppContext,
) {
    let (home, root) = onboarding_home("join-waiting", NAMED_KEY_ABSENT);
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    view.update(cx, |view, _| {
        view.joiner = joined_laptop;
        view.open_chat = || panic!("the chat waits for the model");
    });
    cx.run_until_parked();
    click(cx, "join-pc");
    code_typed(&view, cx, &a_code());
    click(cx, "next");
    assert_eq!(
        joining(&view, cx),
        Some(onboarding::Joining::Joined {
            name: String::from("laptop"),
            ready: false,
        })
    );

    click(cx, "next");
    assert_ne!(first_run::read(&home.first_run()), FirstRun::Done);

    click(cx, "set-up-here");
    assert_eq!(joining(&view, cx), None);
    assert_eq!(walked(&view, cx), Some(Step::Key));
    assert_eq!(first_run::read(&home.first_run()), FirstRun::At(Step::Key));
    let _ = std::fs::remove_dir_all(&root);
}

fn pc_named(view: &Entity<SettingsView>, cx: &mut VisualTestContext, name: &str) {
    click(cx, "section-pcs");
    type_and_leave(view, cx, |view, cx| view.pcs.read(cx).name.clone(), name);
}

#[gpui::test]
fn the_pc_name_typed_in_the_pcs_card_is_saved_without_turning_on_peers(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("pc-name", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();

    pc_named(&view, cx, "서재");

    let saved = Settings::load(&home.settings()).expect("saved");
    assert_eq!(saved.pc.and_then(|pc| pc.name).as_deref(), Some("서재"));
    assert!(
        saved.peers.is_none(),
        "naming this PC does not open it to others"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn an_empty_pc_name_goes_back_to_the_computer_name(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("pc-name-empty", "[pc]\nname = \"서재\"\n");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();

    pc_named(&view, cx, "");

    let saved = Settings::load(&home.settings()).expect("saved");
    assert!(saved.pc.is_none());
    assert_eq!(crate::peers::name(&saved), crate::peers::unnamed(&saved));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn the_list_opens_one_section_at_a_time(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("sections", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.section), Section::General);

    for section in Section::ALL {
        let selector = match section {
            Section::General => "section-general",
            Section::Model => "section-model",
            Section::Elsewhere => "section-elsewhere",
            Section::Memory => "section-memory",
            Section::Routines => "section-routines",
            Section::Pcs => "section-pcs",
            Section::Safety => "section-safety",
        };
        click(cx, selector);
        assert_eq!(view.read_with(cx, |view, _| view.section), section);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn leaving_a_field_saves_only_what_changed(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("leave", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();

    type_and_leave(&view, cx, |view, _| view.name.clone(), "");
    assert!(
        view.read_with(cx, |view, _| view.note.is_none()),
        "nothing changed, so nothing is saved"
    );

    type_and_leave(&view, cx, |view, _| view.name.clone(), "별");
    assert!(
        view.read_with(cx, |view, _| view.note.is_none()),
        "a save says nothing"
    );
    let saved = Settings::load(&home.settings()).expect("saved");
    assert_eq!(
        saved
            .assistant
            .and_then(|assistant| assistant.name)
            .as_deref(),
        Some("별")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_channel_switched_on_waits_quietly_for_its_tokens(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("switch", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();
    click(cx, "section-elsewhere");

    click(cx, "telegram-switch");

    assert!(view.read_with(cx, |view, _| view.note.is_none()));
    assert!(
        Settings::load(&home.settings())
            .expect("loaded")
            .telegram
            .is_none()
    );

    type_and_leave(&view, cx, |view, _| view.telegram_owner.clone(), "42");
    view.read_with(cx, |view, _| {
        assert!(matches!(&view.note, Some(Note::Missing(gaps)) if gaps[0].part == "telegram"));
        assert!(view.alarmed(&view.telegram_token));
        assert!(!view.alarmed(&view.telegram_owner));
    });
    let _ = std::fs::remove_dir_all(&root);
}
#[gpui::test]
fn closing_the_window_saves_the_field_still_being_typed(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("close", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.name.update(cx, |input, cx| input.set_text("밤톨", cx));
    });

    assert!(cx.simulate_close());

    let saved = Settings::load(&home.settings()).expect("saved");
    assert_eq!(
        saved
            .assistant
            .and_then(|assistant| assistant.name)
            .as_deref(),
        Some("밤톨")
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn named(names: &[(&str, &str)]) -> Vec<(String, String)> {
    names
        .iter()
        .map(|(id, name)| ((*id).to_owned(), (*name).to_owned()))
        .collect()
}

fn listed(asked: picker::Asked) -> oneshot::Receiver<Result<Vec<(String, String)>, CheckError>> {
    let (tell, told) = oneshot::channel();
    let names = if asked.openai_compatible {
        named(&[("gemma4", "gemma4"), ("qwen3", "qwen3")])
    } else {
        assert_eq!(asked.key.as_deref(), Some("sk-ant-listed"));
        named(&[
            ("claude-opus-5-5", "Claude Opus 5.5"),
            ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
            ("claude-opus-4-8", "Claude Opus 4.8"),
        ])
    };
    let _ = tell.send(Ok(names));
    told
}

fn unlisted(_: picker::Asked) -> oneshot::Receiver<Result<Vec<(String, String)>, CheckError>> {
    let (tell, told) = oneshot::channel();
    let _ = tell.send(Err(CheckError::NoAnswer));
    told
}

fn saved_model(home: &Home) -> ModelSettings {
    Settings::load(&home.settings()).expect("loaded").model
}

#[gpui::test]
fn a_claude_model_and_its_effort_are_picked_from_lists(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("claude-picks", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();
    click(cx, "section-model");

    click(cx, "model-picker");
    click(cx, "model-picker-claude-opus-5-5");

    assert_eq!(view.read_with(cx, |view, _| view.open_picker), None);
    let ModelSettings::Anthropic { model, effort, .. } = saved_model(&home) else {
        panic!("a Claude model");
    };
    assert_eq!(model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(effort, None);

    click(cx, "effort-picker");
    click(cx, "effort-picker-high");
    let ModelSettings::Anthropic { effort, .. } = saved_model(&home) else {
        panic!("a Claude model");
    };
    assert_eq!(effort.as_deref(), Some("high"));

    click(cx, "effort-picker");
    click(cx, "effort-picker-default");
    click(cx, "model-picker");
    click(cx, "model-picker-default");
    let ModelSettings::Anthropic { model, effort, .. } = saved_model(&home) else {
        panic!("a Claude model");
    };
    assert_eq!(model, None, "the default model leaves the setting unset");
    assert_eq!(effort, None);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_click_outside_the_list_closes_it(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("picker-out", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();
    click(cx, "section-model");

    click(cx, "model-picker");
    assert_eq!(
        view.read_with(cx, |view, _| view.open_picker),
        Some(picker::MODEL_PICKER)
    );
    click(cx, "anthropic");

    assert_eq!(view.read_with(cx, |view, _| view.open_picker), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn an_openai_compatible_server_offers_the_models_it_lists(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home(
        "served",
        "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"gemma4\"\n",
    );
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    view.update(cx, |view, _| view.list = listed);
    cx.run_until_parked();
    click(cx, "section-model");

    click(cx, "model-picker");
    click(cx, "model-picker-qwen3");

    let ModelSettings::OpenaiCompatible { model, .. } = saved_model(&home) else {
        panic!("an OpenAI compatible model");
    };
    assert_eq!(model, "qwen3");
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_server_that_lists_nothing_says_so(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home(
        "unserved",
        "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"gemma4\"\n",
    );
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    view.update(cx, |view, _| view.list = unlisted);
    cx.run_until_parked();
    click(cx, "section-model");

    let line = view.read_with(cx, |view, cx| view.served_line(cx));
    assert_eq!(line, Some((words().settings.models_failed, true)));
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_channel_turned_on_with_nothing_names_every_empty_field_at_once(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("all-gaps", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Elsewhere);
    click(cx, "slack-switch");

    click(cx, "next");

    view.read_with(cx, |view, _| {
        let Some(Note::Missing(gaps)) = &view.note else {
            panic!("the empty fields are named");
        };
        let gap = &gaps[0];
        assert_eq!(gap.fields.len(), 3);
        assert!(view.alarmed(&view.slack_owner));
        assert!(!view.alarmed(&view.telegram_owner));
        let said = &words().settings;
        assert_eq!(
            gap.words,
            (said.channel_missing)(
                "Slack",
                &[
                    said.missing_app_token,
                    said.missing_bot_token,
                    said.missing_member_id
                ]
            )
        );
    });
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn every_channel_turned_on_shows_what_it_lacks_at_once(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("every-gap", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    cx.run_until_parked();
    walk_to(&view, cx, Step::Elsewhere);
    click(cx, "slack-switch");
    click(cx, "telegram-switch");

    click(cx, "next");

    view.read_with(cx, |view, _| {
        let Some(Note::Missing(gaps)) = &view.note else {
            panic!("the empty fields are named");
        };
        let parts: Vec<&str> = gaps.iter().map(|gap| gap.part).collect();
        assert_eq!(parts, ["slack", "telegram"]);
        let said = &words().settings;
        assert_eq!(
            gaps[1].words,
            (said.channel_missing)("Telegram", &[said.missing_bot_token, said.missing_user_id])
        );
        assert!(view.alarmed(&view.slack_owner));
        assert!(view.alarmed(&view.telegram_token));
        assert!(view.alarmed(&view.telegram_owner));
        assert!(!view.alarmed(&view.discord_owner));
    });
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_claude_key_lists_the_models_it_may_use(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("claude-listed", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    view.update(cx, |view, _| view.list = listed);
    cx.run_until_parked();
    click(cx, "section-model");
    assert_eq!(
        view.read_with(cx, |view, _| view.served.clone()),
        Served::NoKey,
        "with no key the known Claude models are offered"
    );

    type_and_leave(&view, cx, |view, _| view.api_key.clone(), "sk-ant-listed");
    click(cx, "model-picker");
    click(cx, "model-picker-claude-opus-4-8");

    let ModelSettings::Anthropic { model, .. } = saved_model(&home) else {
        panic!("a Claude model");
    };
    assert_eq!(model.as_deref(), Some("claude-opus-4-8"));
    let _ = std::fs::remove_dir_all(&root);
}

fn moved_while(
    cx: &mut VisualTestContext,
    act: impl FnOnce(&mut VisualTestContext),
) -> Vec<&'static str> {
    std::thread::sleep(crate::window::motion::STEP.duration * 2);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let _ = crate::window::motion::seen::take();
    act(cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    crate::window::motion::seen::take()
}

#[gpui::test]
fn only_what_the_owner_moves_moves(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("still", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (_view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();
    click(cx, "section-model");

    assert_eq!(
        moved_while(cx, |cx| click(cx, "openai")),
        ["switch"],
        "a tab the owner picks slides"
    );
    click(cx, "section-elsewhere");
    assert_eq!(
        moved_while(cx, |cx| click(cx, "telegram-switch")),
        ["switch"],
        "a switch the owner flips slides"
    );

    for section in Section::ALL {
        let selector = format!("section-{}", section.id()).leak();
        assert_eq!(
            moved_while(cx, |cx| click(cx, selector)),
            Vec::<&str>::new(),
            "opening {selector} moves nothing"
        );
    }
    for section in [Section::Model, Section::Elsewhere] {
        let selector = format!("section-{}", section.id()).leak();
        assert_eq!(
            moved_while(cx, |cx| click(cx, selector)),
            Vec::<&str>::new(),
            "coming back to {selector} replays nothing"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn a_step_of_the_onboarding_slides_in_and_its_switches_stay_still(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("still-walk", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) =
        cx.add_window_view(|_, cx| SettingsView::onboarding(home.clone(), settings, cx));
    cx.run_until_parked();
    checked_as(&view, cx, |_, _, _| answered(Ok(())));
    walk_to(&view, cx, Step::Key);
    click(cx, "openai");
    click(cx, "anthropic");

    let moved = moved_while(cx, |cx| click(cx, "back"));
    assert!(moved.contains(&"greet"), "the greeting plays in: {moved:?}");
    assert!(!moved.contains(&"switch"), "no tab slides again: {moved:?}");
    let moved = moved_while(cx, |cx| click(cx, "next"));
    assert!(moved.contains(&"step"), "a step slides in: {moved:?}");
    assert!(!moved.contains(&"switch"), "no tab slides again: {moved:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn the_time_zone_is_picked_in_general_from_a_list_that_can_be_searched(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("zone", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home.clone(), settings, cx));
    cx.run_until_parked();

    click(cx, "zone-picker");
    cx.simulate_input("seoul");
    click(cx, "zone-picker-Asia/Seoul");

    let zone = || Routines::new(home.routines()).zone_name().expect("read");
    assert_eq!(zone().as_deref(), Some("Asia/Seoul"));
    assert_eq!(view.read_with(cx, |view, _| view.open_picker), None);

    click(cx, "zone-picker");
    click(cx, "zone-picker-default");
    assert_eq!(zone(), None, "back to each PC's own");
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn the_routines_point_to_general_for_the_time_zone(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("zone-link", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();
    click(cx, "section-routines");

    cx.simulate_resize(gpui::size(px(880.), px(480.)));
    click(cx, "zone-general");
    glided(cx);

    assert_eq!(view.read_with(cx, |view, _| view.section), Section::General);
    let card = cx
        .debug_bounds("part-zone")
        .expect("the time zone is drawn");
    let seen = cx.update(|window, _| window.viewport_size().height);
    assert!(
        card.bottom() <= seen,
        "the time zone is scrolled into view: {card:?} in {seen:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn the_language_is_picked_from_a_list(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("language", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();

    click(cx, "language-picker");

    assert_eq!(
        view.read_with(cx, |view, _| view.open_picker),
        Some(picker::LANGUAGE_PICKER)
    );
    for option in ["default", "ko", "en"] {
        let selector = format!("language-picker-{option}").leak();
        assert!(cx.debug_bounds(selector).is_some(), "{selector} is offered");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn going_to_any_part_opens_its_section_with_the_part_in_view(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("go-to", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.simulate_resize(gpui::size(px(880.), px(420.)));
    cx.run_until_parked();

    for part in Part::ALL {
        let shown = view.update(cx, |view, cx| {
            view.go_to(part, cx);
            view.shown_parts(cx).contains(&part)
        });
        glided(cx);
        assert_eq!(
            view.read_with(cx, |view, _| view.section),
            part.section(),
            "{part:?} opens its section"
        );
        if !shown {
            continue;
        }
        let at = cx.debug_bounds(part.id()).expect("the part is drawn");
        let seen = cx.update(|window, _| window.viewport_size().height);
        assert!(
            at.top() >= px(0.) && at.top() < seen,
            "{part:?} starts in view: {at:?} in {seen:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn glided(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        std::thread::sleep(crate::window::motion::GLIDE.duration / 2);
    }
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn a_link_glides_down_to_its_part_from_the_top(cx: &mut TestAppContext) {
    let (home, root) = onboarding_home("glide", "");
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.simulate_resize(gpui::size(px(880.), px(420.)));
    cx.run_until_parked();
    click(cx, "section-routines");

    let moved = moved_while(cx, |cx| {
        click(cx, "zone-general");
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        std::thread::sleep(crate::window::motion::GLIDE.duration / 3);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    });
    assert_eq!(moved, ["glide"], "the pane glides rather than jumps");
    let halfway = view.read_with(cx, |view, _| view.scroll.offset().y);
    glided(cx);
    let there = view.read_with(cx, |view, _| view.scroll.offset().y);
    assert!(
        halfway > there && halfway < px(0.),
        "it passed between the top and the part: {halfway:?} then {there:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
