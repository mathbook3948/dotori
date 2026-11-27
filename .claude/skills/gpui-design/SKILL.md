---
name: gpui-design
description: "Use for any change to how dotori's GPUI windows look or move (crates/app/src/window): colors, themes, layout, components, motion. Plans against DESIGN.md, builds with theme tokens only, then runs the real window under Xvfb, or unseen on Windows, and checks screenshots in every theme that exists before calling it done."
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
- No status dots, no borders around cards or buttons (a `rim` along a card's top edge in the dark look is the exception), no gradients, no glows, sparkles or emoji icons, no colored side stripes. Windows are solid: cards and fields are `surface` with the soft shadow from `surface.rs`, and the main button is the text color filled in. The top strip, window buttons and resize edges come from `chrome.rs`, so both windows share them.
- A line around a field, such as the focus or error ring, is `framed` with `hairline` from `surface.rs`, never `border_1`. A one point border falls on half pixels at 150% and GPUI draws one side thicker.

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

`talk` opens the chat window and types into it with `xdotool`, the way the owner talks to dotori, so it needs no other client. It takes your own messages too. `shot ... light 5` saves five frames a few milliseconds apart, for checking that motion shows steps between its start and its end.

`ask "$home" "<what dotori asks>"` stands in for dotori's browser and asks the owner for it, so the browser card shows; run it before `shot`. Given a JSON object instead, such as `'{"card": {...}}'`, it sends that as the browser's state, so a login card or an account card shows. What the windows send back, such as `{"kind":"take"}`, lands in `$home/screen.log`, and the browser window opens as `Dotori 브라우저`, or `Dotori browser` in English.

On Windows, `win.ps1` next to this file does the same without touching the owner's screen, mouse or keyboard. Never drive dotori with the real cursor or bring its window forward: the owner is using the PC.

```powershell
$home_ = "<scratchpad>\unseen-home"
& .claude\skills\gpui-design\win.ps1 start $home_ settings light
& .claude\skills\gpui-design\win.ps1 click $home_ 170 535
& .claude\skills\gpui-design\win.ps1 move $home_ 242 207
& .claude\skills\gpui-design\win.ps1 shot $home_ "$home_\hover.png"
& .claude\skills\gpui-design\win.ps1 stop $home_
```

- The window opens on a desktop of its own that is never shown, so it cannot take focus. Input reaches it as window messages, and `shot` asks the window to draw itself.
- Points are in window pixels before scaling, the units the code uses. Read them off a shot divided by its scale (1.5 on a 150% display).
- `<home>` becomes `DOTORI_HOME`, so the owner's settings and keys are never read or changed. Settings and keys typed into that window land in `<home>`.
- `DOTORI_HOME`, `DOTORI_THEME` and `DOTORI_LANGUAGE` reach only the dotori it starts; the shell that ran `start` keeps its own.
- The fourth argument of `start`, `ko` or `en`, picks the language. Look at every screen in both: English runs longer and breaks lines in other places.
- `move` is how hover is checked. Look at the shot, and compare pixels when the change is faint.
- A hover style that only changes `text_color` does not show: GPUI fixes the color of text when it lays the text out. Change the background instead.

Take every theme that exists in both looks. Read each screenshot yourself.

## 4a. Test what it does

- A window's behavior is tested with GPUI's test platform in its `<name>.test.rs`: open the view with fake `Link` and `ScreenLink` channels, type with `simulate_input` and `simulate_keystrokes`, click an element found by its `debug_selector` with `simulate_click`, and assert what the window sent and what it holds. `crates/app/src/window/chat.test.rs` shows the pattern.
- Give each control a test needs a `debug_selector`. It costs nothing outside tests.
- GPUI 0.2.2 keeps every `debug_selector` it has ever drawn, so `debug_bounds` cannot show that something went away. Assert what the view holds instead, such as `browser_ask` in `chat.test.rs`.
- Motion is tested too. Every motion token records, in tests only, each frame it draws short of its end, and `motion::seen::take()` returns the tokens that moved since the last call. `moved_while` in `settings.test.rs` lets what moved before settle, runs one action and returns what moved meanwhile. Assert that the action the owner took moves only what it should, such as `["switch"]` for a tab, and that opening a section, going back a step or anything else the owner did not move moves nothing. A movement that replays when its part is drawn again fails there, not in front of the owner.
- `crates/app/tests/e2e.rs` runs the real daemon against the fake model in `crates/app/tests/support/model.rs`. When a change touches what the daemon sends the windows, extend it.

## 5. Critique, then fix

For each screenshot, check and write down:

- the Do's and Don'ts of `DESIGN.md`, one by one;
- every string in both languages against Voice & Content Strategy of `DESIGN.md`, rule by rule, with the whole screen in view, so a fact said twice shows;
- text contrast against its background, 4.5:1 at least;
- one main button per screen, for the action the owner most likely takes next, and `primary` only on small marks, never on text;
- alignment and spacing on the token scale;
- Korean lines break between words.

Fix what fails and look again. Then report with the screenshots, light and dark side by side.

## 6. Keep the standard true

- Update `docs/architecture/windows.md` in the same pull request, so it says how the window is after the change.
- When the owner changes the look, change `DESIGN.md` in the same pull request as the code.
- Run `mise run quick` and `mise run window-test` before each commit.
