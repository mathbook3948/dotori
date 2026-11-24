//! dotori's replies drawn with the light markdown models write: lists, headings, bold, inline code
//! and fenced code. Anything else stays as written, and copying a reply still copies its text.

use gpui::Div;
use gpui::FontWeight;
use gpui::SharedString;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::window::parts::MONO;
use crate::window::parts::SPACE;
use crate::window::parts::prose;
use crate::window::theme::Theme;

/// A stretch of a word drawn one way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) text: String,
    pub(crate) bold: bool,
    pub(crate) code: bool,
}

/// What a space separates. Empty when spaces follow each other, so their width is kept.
pub(crate) type Word = Vec<Run>;

/// One block of a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Gap,
    Line(Vec<Word>),
    Heading(Vec<Word>),
    Item {
        depth: usize,
        mark: String,
        words: Vec<Word>,
    },
    Code(Vec<String>),
}

/// Splits a reply into the blocks it is drawn as.
pub(crate) fn blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut fenced: Option<(String, Vec<String>)> = None;
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r').replace('\t', "    ");
        if fence(&line) {
            match fenced.take() {
                Some((_, code)) => blocks.push(Block::Code(code)),
                None => fenced = Some((line, Vec::new())),
            }
            continue;
        }
        if let Some((_, code)) = fenced.as_mut() {
            code.push(line);
            continue;
        }
        if line.trim().is_empty() {
            if !matches!(blocks.last(), None | Some(Block::Gap)) {
                blocks.push(Block::Gap);
            }
            continue;
        }
        blocks.push(block(&line));
    }
    match fenced {
        Some((opened, code)) if code.is_empty() => blocks.push(Block::Line(words(&opened))),
        Some((_, code)) => blocks.push(Block::Code(code)),
        None => {}
    }
    while matches!(blocks.last(), Some(Block::Gap)) {
        blocks.pop();
    }
    blocks
}

fn fence(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("```")
        .is_some_and(|tag| !tag.contains('`'))
}

fn block(line: &str) -> Block {
    let body = line.trim_start_matches(' ');
    let indent = line.len() - body.len();
    let hashes = body.chars().take_while(|c| *c == '#').count();
    if (1..=3).contains(&hashes)
        && body[hashes..].starts_with(' ')
        && !body[hashes..].trim().is_empty()
    {
        return Block::Heading(words(body[hashes..].trim()));
    }
    let depth = (indent / 2).min(MAX_DEPTH);
    for bullet in ["- ", "* ", "• "] {
        if let Some(rest) = body.strip_prefix(bullet) {
            return Block::Item {
                depth,
                mark: String::from("•"),
                words: words(rest.trim_start()),
            };
        }
    }
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits)
        && let Some(rest) = body[digits..]
            .strip_prefix(". ")
            .or_else(|| body[digits..].strip_prefix(") "))
    {
        return Block::Item {
            depth,
            mark: format!("{}.", &body[..digits]),
            words: words(rest.trim_start()),
        };
    }
    Block::Line(words(line.trim_end()))
}

/// Splits a line at its spaces into words, reading `**bold**` and `` `code` `` marks. Code keeps
/// its spaces inside one word. A mark without its pair is left as written.
pub(crate) fn words(line: &str) -> Vec<Word> {
    let marks_code = line.matches('`').count().is_multiple_of(2);
    let marks_bold = outside_code(line, marks_code)
        .matches("**")
        .count()
        .is_multiple_of(2);
    let mut words: Vec<Word> = vec![Vec::new()];
    let mut bold = false;
    let mut code = false;
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if marks_code && c == '`' {
            code = !code;
            rest = &rest[1..];
            continue;
        }
        if marks_bold && !code && rest.starts_with("**") {
            bold = !bold;
            rest = &rest[2..];
            continue;
        }
        rest = &rest[c.len_utf8()..];
        if c == ' ' && !code {
            words.push(Vec::new());
            continue;
        }
        let word = words.last_mut().expect("a word is always open");
        match word.last_mut() {
            Some(run) if run.bold == bold && run.code == code => run.text.push(c),
            _ => word.push(Run {
                text: c.to_string(),
                bold,
                code,
            }),
        }
    }
    words
}

fn outside_code(line: &str, marks_code: bool) -> String {
    if !marks_code {
        return line.to_owned();
    }
    line.split('`').step_by(2).collect::<Vec<_>>().join(" ")
}

/// A reply drawn block by block.
pub(crate) fn rich(text: &str, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .children(blocks(text).into_iter().map(|block| {
            match block {
                Block::Gap => div().h(px(6.)),
                Block::Line(words) => line(&words, theme),
                Block::Heading(words) => line(&words, theme)
                    .pt(px(2.))
                    .text_base()
                    .font_weight(FontWeight::BOLD),
                Block::Item { depth, mark, words } => div()
                    .flex()
                    .gap_2()
                    .pl(px(depth as f32 * INDENT))
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(12.))
                            .text_color(theme.accent)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(mark),
                    )
                    .child(div().flex_1().min_w_0().child(line(&words, theme))),
                Block::Code(lines) => div()
                    .my(px(2.))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(theme.canvas)
                    .border_1()
                    .border_color(theme.line)
                    .font_family(MONO)
                    .text_sm()
                    .child(prose(&lines.join("\n"))),
            }
        }))
}

fn line(words: &[Word], theme: &Theme) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_x(px(SPACE))
        .children(words.iter().map(|word| {
            match word.as_slice() {
                [] => div().w(px(0.)),
                [run] => drawn(run, theme),
                runs => div()
                    .flex()
                    .items_start()
                    .min_w_0()
                    .children(runs.iter().map(|run| drawn(run, theme))),
            }
        }))
}

fn drawn(run: &Run, theme: &Theme) -> Div {
    let drawn = div().min_w_0().child(SharedString::from(run.text.clone()));
    let drawn = if run.bold {
        drawn.font_weight(FontWeight::BOLD)
    } else {
        drawn
    };
    if run.code {
        drawn
            .px(px(3.))
            .rounded_sm()
            .bg(theme.canvas)
            .font_family(MONO)
            .text_sm()
    } else {
        drawn
    }
}

const INDENT: f32 = 16.;
const MAX_DEPTH: usize = 3;

#[cfg(test)]
#[path = "rich.test.rs"]
mod tests;
