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

    click(cx, "forget-ANTHROPIC_API_KEY");

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
