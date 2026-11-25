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
fn a_linked_site_shows_in_the_vault_card_and_can_be_unlinked(cx: &mut TestAppContext) {
    use dotori_mcp::card::Account;
    use dotori_mcp::card::Accounts;

    let root = std::env::temp_dir().join(format!("dotori-sites-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    std::fs::write(root.join("settings.toml"), "").expect("settings");
    accounts::VaultAccounts {
        vault: Arc::new(Vault::new(&root)),
    }
    .keep(&Account {
        site: String::from("쿠팡"),
        origin: String::from("https://www.coupang.com"),
        id_label: Some(String::from("아이디")),
        id: Some(String::from("minji.example@mail.com")),
        password_label: String::from("비밀번호"),
        password: String::from("hunter22!"),
        trusted: false,
    })
    .expect("kept");
    let home = Home { root: root.clone() };
    let settings = Settings::load(&home.settings()).expect("loaded");
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(home, settings, cx));
    cx.run_until_parked();

    let shown = view.read_with(cx, |view, _| view.sites.clone());
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].site, "쿠팡");
    assert_eq!(shown[0].id.as_deref(), Some("minji.example@mail.com"));
    assert!(
        view.read_with(cx, |view, _| view.kept_keys().is_empty()),
        "an account is not listed with the keys"
    );

    assert!(cx.debug_bounds("unlink-https://www.coupang.com").is_some());
    view.update(cx, |view, cx| view.unlink("https://www.coupang.com", cx));

    assert!(view.read_with(cx, |view, _| view.sites.is_empty()));
    assert_eq!(
        Vault::new(&root)
            .get("site:https://www.coupang.com")
            .expect("read"),
        None
    );
    let _ = std::fs::remove_dir_all(&root);
}
