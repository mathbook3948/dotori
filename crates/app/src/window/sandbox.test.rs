#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;

#[cfg(target_os = "linux")]
use gpui::Modifiers;
use gpui::TestAppContext;
#[cfg(target_os = "linux")]
use gpui::VisualTestContext;

use super::*;

#[cfg(target_os = "linux")]
static RUNS: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn home() -> Home {
    Home {
        root: std::env::temp_dir().join(format!("dotori-sandbox-card-{}", std::process::id())),
    }
}

#[cfg(target_os = "linux")]
#[gpui::test]
fn a_refused_admin_prompt_offers_the_command_and_a_retry_that_turns_it_on(cx: &mut TestAppContext) {
    RUNS.store(false, Ordering::SeqCst);
    let work = Work {
        works: |_| Ok(RUNS.load(Ordering::SeqCst)),
        set_up: || Err(Failure::Refused),
    };
    let (view, cx) = cx.add_window_view(|_, cx| SandboxView::with(home(), work, cx));
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert_eq!(view.state, State::Off));

    click(cx, "sandbox-install");

    view.read_with(cx, |view, _| {
        assert_eq!(view.state, State::Failed(Failure::Refused));
    });
    click(cx, "sandbox-copy");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(sandbox::by_hand())
    );

    view.update(cx, |view, _| {
        view.work.set_up = || {
            RUNS.store(true, Ordering::SeqCst);
            Ok(())
        };
    });
    click(cx, "sandbox-retry");

    view.read_with(cx, |view, _| assert_eq!(view.state, State::On));
}

#[cfg(target_os = "linux")]
#[gpui::test]
fn an_install_that_leaves_the_sandbox_off_says_so(cx: &mut TestAppContext) {
    let work = Work {
        works: |_| Ok(false),
        set_up: || Ok(()),
    };
    let (view, cx) = cx.add_window_view(|_, cx| SandboxView::with(home(), work, cx));
    cx.run_until_parked();

    click(cx, "sandbox-install");

    view.read_with(cx, |view, _| {
        assert_eq!(view.state, State::Failed(Failure::StillOff));
    });
}

#[gpui::test]
fn a_sandbox_that_runs_is_shown_as_on(cx: &mut TestAppContext) {
    let work = Work {
        works: |_| Ok(true),
        set_up: || Err(Failure::Other),
    };
    let (view, cx) = cx.add_window_view(|_, cx| SandboxView::with(home(), work, cx));
    cx.run_until_parked();

    view.read_with(cx, |view, _| {
        let expected = if cfg!(windows) {
            State::Checking
        } else {
            State::On
        };
        assert_eq!(view.state, expected);
        assert_eq!(view.shown(), !cfg!(windows));
    });
}

#[gpui::test]
fn a_sandbox_that_does_not_run_is_shown_as_blocked_not_as_on(cx: &mut TestAppContext) {
    let work = Work {
        works: |_| Err(Blocked::WorkNotPlain),
        set_up: || Err(Failure::Other),
    };
    let (view, cx) = cx.add_window_view(|_, cx| SandboxView::with(home(), work, cx));
    cx.run_until_parked();

    view.read_with(cx, |view, _| {
        let expected = if cfg!(windows) {
            State::Checking
        } else {
            State::Blocked(Blocked::WorkNotPlain)
        };
        assert_eq!(view.state, expected);
    });
}
