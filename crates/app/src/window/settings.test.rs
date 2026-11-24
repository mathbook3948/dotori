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
