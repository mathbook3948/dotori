---
name: gpui-design
description: "Use for any change to how dotori's GPUI windows look or move (crates/app/src/window): colors, themes, layout, components, motion. Plans against DESIGN.md, builds with theme tokens only, then runs the real window under Xvfb and checks screenshots in every theme that exists before calling it done."
---

# GPUI design

`DESIGN.md` at the repository root is the design the windows are moving to: its front matter holds the tokens (colors per theme and look, type, spacing, radius, motion) and its sections hold the rules. Until a window has moved, `docs/architecture/windows.md` and the code describe it, and a small fix follows them. This skill is how a change moves a window to `DESIGN.md`.

## 1. Read before drawing

- Read `DESIGN.md` whole, the Do's and Don'ts last.
- Read `docs/architecture/windows.md` and the window code you will touch under `crates/app/src/window/`, `theme.rs` included.

## 2. Plan in a few lines

Write down, before code:

- which components change, and which tokens each one uses;
- what moves, from where to where, with which motion token;
- what the owner sees in the light and dark look of each theme.

A value that is not a token is a new token: add it to `DESIGN.md` first, in every theme and look, and say why. Never add a one-off color, size or duration in window code.

## 3. Build

- Colors come only from `Theme`, whose fields follow the color tokens. Fields that still carry older names, such as `accent` for `primary`, are renamed by the change that moves a window. Sizes, radii and gaps use the token values, never a size that is not in `DESIGN.md`.
- Motion uses GPUI's `with_animation` with the duration of the `motion` token. `ease-out` is `ease_out_quint()` (a constructor you call) and `ease-in-out` is `ease_in_out`. Nothing bounces.
- A state that waits on a disk or the network is drawn at once as waiting, and the work runs off the drawing thread.
- No status dots, no borders around cards or buttons (a `rim` on the top edge of a glass surface is the exception), no gradients, glows, sparkles or emoji icons, no colored side stripes. Glass is the windows' material as DESIGN.md's Elevation & Depth says: the canvas lets the blur through, fills stay solid. The top strip, window buttons and resize edges come from `chrome.rs`, so both windows share them.

## 4. Look at the real window

Run it, do not trust the code. `demo.sh` next to this file drives the real windows on a virtual screen with the fake model the end-to-end test uses, so the conversation is the same every time: a reminder that sets a routine, a list of the week, and a shell command that waits for the owner.

```bash
home=<scratchpad>/demo-home
bash .claude/skills/gpui-design/demo.sh start "$home"
bash .claude/skills/gpui-design/demo.sh talk "$home"
bash .claude/skills/gpui-design/demo.sh shot "$home" "$home/light.png" light
bash .claude/skills/gpui-design/demo.sh shot "$home" "$home/dark.png" dark
bash .claude/skills/gpui-design/demo.sh stop "$home"
```

`settings "$home" <out.png> [look]` opens the settings window instead. It is taller than a screen, and scrolling does not reach GPUI under Xvfb, so to see all of it run a second, tall Xvfb (for example `Xvfb :98 -screen 0 900x3800x24` with `openbox`), open `dotori settings` there with `DOTORI_HOME` and `DOTORI_THEME` set, make the window tall with `xdotool windowsize`, and capture it with `import`.

`talk` takes your own messages too. `shot ... light 5` saves five frames a few milliseconds apart, for checking that motion shows steps between its start and its end.

`ask "$home" "<what dotori asks>"` stands in for dotori's browser and asks the owner for it, so the browser card shows; run it before `shot`. What the windows send back, such as `{"kind":"take"}`, lands in `$home/screen.log`, and the browser window opens as `dotori 브라우저`.

Take every theme that exists in both looks. Read each screenshot yourself.

## 4a. Test what it does

- A window's behavior is tested with GPUI's test platform in its `<name>.test.rs`: open the view with fake `Link` and `ScreenLink` channels, type with `simulate_input` and `simulate_keystrokes`, click an element found by its `debug_selector` with `simulate_click`, and assert what the window sent and what it holds. `crates/app/src/window/chat.test.rs` shows the pattern.
- Give each control a test needs a `debug_selector`. It costs nothing outside tests.
- GPUI 0.2.2 keeps every `debug_selector` it has ever drawn, so `debug_bounds` cannot show that something went away. Assert what the view holds instead, such as `browser_ask` in `chat.test.rs`.
- `crates/app/tests/e2e.rs` runs the real daemon against the fake model in `crates/app/tests/support/model.rs`. When a change touches what the daemon sends the windows, extend it.

## 5. Critique, then fix

For each screenshot, check and write down:

- the Do's and Don'ts of `DESIGN.md`, one by one;
- text contrast against its background, 4.5:1 at least;
- `primary` fills only the action the owner most likely takes next, and colors no text on the canvas;
- alignment and spacing on the token scale;
- Korean lines break between words.

Fix what fails and look again. Then report with the screenshots, light and dark side by side.

## 6. Keep the standard true

- Update `docs/architecture/windows.md` in the same pull request, so it says how the window is after the change.
- When the owner changes the look, change `DESIGN.md` in the same pull request as the code.
- Run `mise run check` before each commit.
