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
- No status dots, no borders on cards or buttons, no gradients, glass, glows, sparkles or emoji icons, no colored side stripes.

## 4. Look at the real window

Run it, do not trust the code. In a container without a screen:

1. Start Xvfb and a window manager, for example `Xvfb :97 -screen 0 1280x900x24` and `DISPLAY=:97 openbox`.
2. Make a fresh `DOTORI_HOME` in the scratchpad whose `settings.toml` points the model at a local fake OpenAI-compatible server that answers with a fixed conversation: a reminder, a list, a tool call and a shell command, which always asks the owner first.
3. Start `dotori daemon` with `DISPLAY` unset, so it runs without a tray.
4. Send the messages with `dotori chat`, keeping its input open until the replies came back, for example `(echo 안녕; sleep 5) | dotori chat`.
5. Open `dotori window` on the display, then save it with `xdotool search --name '^dotori$'` and `import -window <id> -frame <file>.png`. Set `DOTORI_THEME=light` or `dark` for each look.

Take every theme that exists in both looks. Read each screenshot yourself. For motion, take a few screenshots a few frames apart while it runs, and check that each shows a step between the start and the end.

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
