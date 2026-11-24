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
fn the_owner_picks_the_time_zone_routines_are_read_in(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("dotori-zone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    let home = Home { root: root.clone() };
    let (view, cx) = cx.add_window_view(|_, cx| RoutinesView::new(&home, cx));
    cx.run_until_parked();

    click(cx, "zone");
    cx.simulate_input("Mars/Olympus");
    click(cx, "zone-save");
    assert!(
        view.read_with(cx, |view, _| view.failed.is_some()),
        "an unknown name is refused"
    );
    assert_eq!(
        Routines::new(home.routines()).zone_name().expect("read"),
        None
    );

    view.update(cx, |view, cx| {
        view.zone
            .update(cx, |input, cx| input.set_text("Asia/Seoul", cx))
    });
    click(cx, "zone-save");
    assert!(view.read_with(cx, |view, _| view.failed.is_none() && view.zone_saved));
    assert_eq!(
        Routines::new(home.routines())
            .zone_name()
            .expect("read")
            .as_deref(),
        Some("Asia/Seoul")
    );

    view.update(cx, |view, cx| {
        view.zone.update(cx, |input, cx| input.set_text("", cx))
    });
    click(cx, "zone-save");
    assert_eq!(
        Routines::new(home.routines()).zone_name().expect("read"),
        None,
        "back to each PC's own"
    );
    let _ = std::fs::remove_dir_all(&root);
}
