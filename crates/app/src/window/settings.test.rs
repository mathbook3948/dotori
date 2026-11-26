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

    view.update(cx, |view, cx| {
        view.api_key
            .update(cx, |input, cx| input.set_text("sk-ant-pasted", cx));
    });
    click(cx, "save");

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
    view.update(cx, |view, cx| {
        view.api_key
            .update(cx, |input, cx| input.set_text("sk-local", cx));
    });
    click(cx, "save");
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

    view.update(cx, |view, cx| {
        view.name
            .update(cx, |input, cx| input.set_text("도토리", cx));
        view.use_icon(icons::keep(&icons, &png), cx);
    });
    click(cx, "save");

    let kept = Settings::load(&root.join("settings.toml"))
        .expect("loaded")
        .assistant
        .expect("an assistant section");
    assert_eq!(kept.name.as_deref(), Some("도토리"));
    let icon = kept.icon.expect("a picture");
    assert!(icons::path(&icons, &icon).is_some());

    click(cx, "icon-default");

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
    assert!(view.read_with(cx, |view, _| matches!(view.note, Some(Note::Failed(_)))));
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
fn the_steps_run_from_the_greeting_to_the_other_places() {
    assert_eq!(Step::Welcome.previous(), None);
    assert_eq!(Step::Welcome.next(), Some(Step::Key));
    assert_eq!(Step::Look.next(), Some(Step::Elsewhere));
    assert_eq!(Step::Elsewhere.next(), None);
    assert_eq!(Step::Welcome.label(), None);
    assert_eq!(
        Step::ALL.iter().filter_map(|step| step.label()).count(),
        3,
        "the progress bar counts the three steps after the greeting"
    );
}
