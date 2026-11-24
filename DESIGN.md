---
version: alpha
name: dotori
description: The look and motion of dotori's windows. A calm assistant you talk to, not a messenger.
colors:
  graphite:
    light:
      canvas: "#F4F5F7"
      surface: "#FFFFFF"
      raised: "#FFFFFF"
      line: "#E2E5EA"
      bubble: "#E6E7E9"
      text: "#1A1D21"
      muted: "#5D6570"
      faint: "#7A838E"
      accent: "#2F6FEB"
      accent-text: "#FFFFFF"
      ok: "#1F8A4C"
      warn: "#B5650C"
      warn-soft: "#FDF0E1"
    dark:
      canvas: "#0F1114"
      surface: "#181B20"
      raised: "#14171B"
      line: "#272C33"
      bubble: "#25272A"
      text: "#E7EAEE"
      muted: "#9AA2AD"
      faint: "#747D88"
      accent: "#5B8DEF"
      accent-text: "#0B1220"
      ok: "#3FB871"
      warn: "#E0A458"
      warn-soft: "#2D2416"
  paper:
    light:
      canvas: "#F6F4EF"
      surface: "#FFFFFF"
      raised: "#FBFAF7"
      line: "#E4DFD5"
      bubble: "#E8E6E1"
      text: "#1E1C19"
      muted: "#68635A"
      faint: "#857F75"
      accent: "#1E1C19"
      accent-text: "#FFFFFF"
      ok: "#2E7D4F"
      warn: "#9E5A10"
      warn-soft: "#F4E9D8"
    dark:
      canvas: "#151412"
      surface: "#1E1D1A"
      raised: "#191816"
      line: "#2D2B27"
      bubble: "#2A2927"
      text: "#ECE9E2"
      muted: "#A19C92"
      faint: "#7D786F"
      accent: "#ECE9E2"
      accent-text: "#151412"
      ok: "#58B47E"
      warn: "#D9A25A"
      warn-soft: "#2C2418"
  forest:
    light:
      canvas: "#F2F4F1"
      surface: "#FFFFFF"
      raised: "#FAFBF9"
      line: "#DCE2D8"
      bubble: "#E4E6E3"
      text: "#1B231F"
      muted: "#59675F"
      faint: "#76847C"
      accent: "#2E6B52"
      accent-text: "#FFFFFF"
      ok: "#2E7D4F"
      warn: "#A8620F"
      warn-soft: "#F6ECDC"
    dark:
      canvas: "#0F1411"
      surface: "#171E1A"
      raised: "#131915"
      line: "#25302A"
      bubble: "#242A26"
      text: "#E3EBE5"
      muted: "#92A299"
      faint: "#6F7F76"
      accent: "#5FAE8A"
      accent-text: "#0C1410"
      ok: "#5FBF85"
      warn: "#DDA35A"
      warn-soft: "#2B2417"
  indigo:
    light:
      canvas: "#F4F4F9"
      surface: "#FFFFFF"
      raised: "#FCFCFF"
      line: "#E1E1EC"
      bubble: "#E6E6EB"
      text: "#1B1A28"
      muted: "#5F5E75"
      faint: "#7C7B92"
      accent: "#5A4ED6"
      accent-text: "#FFFFFF"
      ok: "#1F8A4C"
      warn: "#B0620E"
      warn-soft: "#FBEEDF"
    dark:
      canvas: "#11111D"
      surface: "#1A1A2A"
      raised: "#151523"
      line: "#2A2A40"
      bubble: "#262632"
      text: "#E7E6F4"
      muted: "#A2A1BD"
      faint: "#7B7A96"
      accent: "#8D82F2"
      accent-text: "#11111D"
      ok: "#4CC285"
      warn: "#E3A65B"
      warn-soft: "#2D2518"
  acorn:
    light:
      canvas: "#F7F3EE"
      surface: "#FFFFFF"
      raised: "#FFFDFA"
      line: "#E7DFD5"
      bubble: "#EAE5E0"
      text: "#2A231D"
      muted: "#7C7067"
      faint: "#B3A79C"
      accent: "#8B5A2B"
      accent-text: "#FFFFFF"
      ok: "#3F9A5C"
      warn: "#C9822B"
      warn-soft: "#F8EAD6"
    dark:
      canvas: "#1B1714"
      surface: "#26211D"
      raised: "#211C18"
      line: "#3A322C"
      bubble: "#302C28"
      text: "#F0E8E0"
      muted: "#A89C91"
      faint: "#6F655C"
      accent: "#C98A4B"
      accent-text: "#1B1714"
      ok: "#5CBF7E"
      warn: "#E0A458"
      warn-soft: "#3A2C1B"
  logo:
    light:
      cap: "#5B3A1E"
      nut: "#C98A4B"
    dark:
      cap: "#8B5A2B"
      nut: "#D99A5B"
typography:
  reply:
    fontSize: 15px
    lineHeight: 24px
  body:
    fontSize: 14px
    lineHeight: 21px
  title:
    fontSize: 15px
    fontWeight: 600
    lineHeight: 20px
  small:
    fontSize: 13px
    lineHeight: 18px
  caption:
    fontSize: 12px
    lineHeight: 16px
  hint:
    fontSize: 11px
    lineHeight: 14px
  code:
    fontSize: 12.5px
    lineHeight: 19px
rounded:
  sm: 6px
  md: 8px
  lg: 12px
  xl: 16px
  composer: 18px
  full: 999px
spacing:
  xs: 4px
  sm: 8px
  md: 12px
  lg: 16px
  xl: 20px
  turn: 18px
motion:
  fast: 120ms
  base: 180ms
  slow: 240ms
  rise: 6px
---

# dotori

## Overview

dotori is one person's assistant that lives on their PC. Its windows should feel calm, soft and quick. The owner talks to an assistant, not to a contact in a messenger, so the conversation reads like a page the assistant writes, with the owner's words set apart.

Smoothness matters most. Nothing jumps, nothing flickers, and the window never waits on the network to answer a click or a key.

## Colors

- The owner picks a theme in the settings window. Every window changes at once. Each theme has a light and a dark set, chosen by the system look or by the owner.
- The themes are graphite, paper, forest, indigo and acorn. Graphite is the default.
- `canvas` is the window background. `surface` is anything that sits on it, such as a card, a field or a button. `raised` is the header and the composer area.
- `bubble` is the owner's message. It is a neutral shade of the canvas, never the accent.
- `accent` is kept for the one main action on screen, such as send or allow, and for list marks. It never fills large areas.
- `warn` and `warn-soft` mark a request that waits for the owner, such as a permission card.
- `ok` is used for a result that succeeded, as text or an icon, never as a dot.
- Text on any background reaches 4.5:1 contrast. `faint` is only for placeholders.
- The logo keeps its own colors, `logo.light` on a light theme and `logo.dark` on a dark one.

## Typography

- The windows use the system UI font, and code uses the fixed-width font each OS has.
- dotori's replies use `reply`, everything else in the conversation uses `body`, and labels and notes use `small` or `caption`.
- Korean text breaks only between words.
- Bold marks a few words that matter, such as the time of an event. Headings in replies are a step bigger and bold, never coloured.

## Layout

- The chat window is a header, the conversation, and the composer.
- The header shows the logo, the name, and the tabs. It shows no status while things work. When dotori cannot be reached, one quiet line under the header says so and what to do.
- dotori's reply is plain text on the canvas, with no bubble. A small logo sits by the first line of each reply.
- The owner's message is a `bubble` on the right, at most four fifths of the width.
- A tool call shows as one muted line with an icon and what was done, such as "캘린더에서 이번 주 일정 3개를 찾았어요". It opens to show the details.
- A request for permission sits where dotori's reply would be, as a card.
- Messages carry no time of their own. A line such as "오늘" separates days, and the time shows when the pointer rests on a message.
- Messages from other channels keep their channel, place and sender on a small line above them.

## Elevation & Depth

- Cards and fields are set apart by fill, not by borders. A card is `surface` or a soft tint such as `warn-soft` on the canvas.
- Lines are used only to divide large areas, such as the header from the conversation.
- The composer alone has a light shadow, so it reads as the place to type.

## Shapes

- Messages and cards use `rounded.lg` or `rounded.xl`. Buttons and fields inside them use `rounded.md`. The composer uses `rounded.composer`.
- Only avatars and the send button are round.

## Motion & Micro-interaction

- Every change on screen moves from where it was to where it goes. Nothing appears or disappears in one frame unless the owner asked for it with a key.
- A new message fades in and rises by `motion.rise` over `motion.base`, easing out.
- A tool line opens and closes over `motion.base`, its height and opacity together.
- When the pointer enters or leaves a button, its color changes over `motion.fast`.
- When the conversation is at the bottom, it follows new text by gliding over `motion.base`. When the owner has scrolled up to read, it stays where it is.
- A reply that is being written grows as its text arrives. While dotori works before the first word, the logo by the reply breathes slowly beside a short label such as "생각하는 중".
- Switching tabs or themes cross-fades over `motion.base`.
- Nothing bounces, overshoots or spins without a label, and nothing takes longer than `motion.slow`.
- Typing and sending show within one frame. Work that waits on a disk or the network never runs on the thread that draws the window.

## Voice & Content Strategy

- The windows speak Korean in a short, polite tone (해요체).
- A sentence says what happens or what the owner can do next, such as "허락하면 바로 실행해요".

## Components

- **Owner message**: `bubble` fill, `text`, `rounded.xl`, `body`, right aligned.
- **dotori reply**: no fill, `reply`, the logo at the first line, lists with `accent` marks.
- **Tool line**: `small`, `muted`, an icon on the left and a chevron on the right.
- **Permission card**: `warn-soft` fill, `rounded.lg`, a bold question, the command on `surface` in `code`, and the buttons 거절 (`surface`) and 허락 (`accent`).
- **Composer**: `surface`, `rounded.composer`, a light shadow, a round `accent` send button, and a `hint` line under it.
- **Tabs**: a `canvas` track with the chosen tab on `surface`.

## Do's and Don'ts

- Do use one accent on a screen, for the action the owner is most likely to take.
- Do show a state only when it needs the owner, and say it in words.
- Don't show status with colored dots.
- Don't put borders around cards or buttons.
- Don't use gradients, glass, glows, sparkles, emoji as icons, or cards with a colored stripe on one side.
- Don't draw dotori's replies in bubbles or fill large areas with the accent.
- Don't let anything jump, flicker or wait for the network before it answers the owner.
