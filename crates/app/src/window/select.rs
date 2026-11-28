//! Selecting the words of the conversation with the mouse, as a page lets the owner do, so they
//! can be copied. Every word the chat window draws is a piece with its place in the
//! conversation; a press fixes where the selection starts, a drag moves where it ends, two presses
//! take the word there and three its line, and copying joins the pieces in between the way they
//! were written.

use std::cell::Cell;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::Bounds;
use gpui::Div;
use gpui::HighlightStyle;
use gpui::Hsla;
use gpui::Pixels;
use gpui::Point;
use gpui::SharedString;
use gpui::StyledText;
use gpui::TextLayout;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::window::parts::SPACE;

/// What goes between a piece and the one before it when the selection is copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Joint {
    /// The same word, as a bold part next to a plain one.
    Joined,
    Space,
    Newline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    line: usize,
    piece: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Spot {
    key: Key,
    byte: usize,
}

struct Piece {
    key: Key,
    text: SharedString,
    joint: Joint,
    layout: TextLayout,
}

#[derive(Default)]
struct State {
    pieces: Vec<Piece>,
    anchor: Option<Spot>,
    head: Option<Spot>,
    dragging: bool,
    marked: Option<Hsla>,
}

impl State {
    fn ends(&self) -> Option<(Spot, Spot)> {
        let (anchor, head) = (self.anchor?, self.head?);
        (anchor != head).then(|| (anchor.min(head), anchor.max(head)))
    }

    fn marked_in(&self, key: Key, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ends()?;
        if key < start.key || key > end.key {
            return None;
        }
        let from = if key == start.key { start.byte } else { 0 };
        let to = if key == end.key { end.byte } else { len };
        (from < to).then_some(from..to)
    }

    fn spot_at(&self, position: Point<Pixels>, between: bool) -> Option<(Spot, bool)> {
        let mut best: Option<(Pixels, Pixels, &Piece, Bounds<Pixels>)> = None;
        for piece in &self.pieces {
            let bounds = piece.layout.bounds();
            let dy = distance(position.y, bounds.top(), bounds.bottom());
            let dx = distance(position.x, bounds.left(), bounds.right());
            if best.is_none_or(|(y, x, _, _)| (dy, dx) < (y, x)) {
                best = Some((dy, dx, piece, bounds));
            }
        }
        let (dy, dx, piece, bounds) = best?;
        let byte = if position.y < bounds.top() || position.x <= bounds.left() {
            0
        } else if position.y > bounds.bottom() || position.x >= bounds.right() {
            piece.text.len()
        } else if between {
            nearest_byte(piece, position)
        } else {
            byte_under(piece, position)
        };
        let on = dy == Pixels::ZERO && dx == Pixels::ZERO;
        Some((
            Spot {
                key: piece.key,
                byte,
            },
            on,
        ))
    }

    fn word_around(&self, at: Spot) -> (Spot, Spot) {
        let Some(found) = self.pieces.iter().position(|piece| piece.key == at.key) else {
            return (at, at);
        };
        let text = |index: usize| self.pieces[index].text.as_ref();
        let joined = |index: usize| {
            index > 0
                && self.pieces[index].joint == Joint::Joined
                && self.pieces[index - 1].key.line == self.pieces[index].key.line
        };
        let here = text(found);
        let kind = match (
            here[at.byte..].chars().next(),
            here[..at.byte].chars().last(),
        ) {
            (Some(char), _) | (None, Some(char)) => Kind::of(char),
            (None, None) => return (at, at),
        };
        let (mut first, mut start) = (found, at.byte);
        loop {
            match text(first)[..start].chars().last() {
                Some(char) if Kind::of(char) == kind => start -= char.len_utf8(),
                Some(_) => break,
                None if joined(first) => {
                    first -= 1;
                    start = text(first).len();
                }
                None => break,
            }
        }
        let (mut last, mut end) = (found, at.byte);
        loop {
            match text(last)[end..].chars().next() {
                Some(char) if Kind::of(char) == kind => end += char.len_utf8(),
                Some(_) => break,
                None if last + 1 < self.pieces.len() && joined(last + 1) => {
                    last += 1;
                    end = 0;
                }
                None => break,
            }
        }
        (
            Spot {
                key: self.pieces[first].key,
                byte: start,
            },
            Spot {
                key: self.pieces[last].key,
                byte: end,
            },
        )
    }

    fn line_around(&self, at: Spot) -> (Spot, Spot) {
        let Some(found) = self.pieces.iter().position(|piece| piece.key == at.key) else {
            return (at, at);
        };
        let starts = |index: usize| {
            let piece = &self.pieces[index];
            piece.key.piece == 0 || piece.joint == Joint::Newline
        };
        let mut first = found;
        while !starts(first) && first > 0 {
            first -= 1;
        }
        let mut last = found;
        while last + 1 < self.pieces.len() && !starts(last + 1) {
            last += 1;
        }
        (
            Spot {
                key: self.pieces[first].key,
                byte: 0,
            },
            Spot {
                key: self.pieces[last].key,
                byte: self.pieces[last].text.len(),
            },
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Blank,
    Mark,
}

impl Kind {
    fn of(char: char) -> Self {
        if char.is_alphanumeric() || char == '_' {
            Self::Word
        } else if char.is_whitespace() {
            Self::Blank
        } else {
            Self::Mark
        }
    }
}

fn byte_under(piece: &Piece, position: Point<Pixels>) -> usize {
    match piece.layout.index_for_position(position) {
        Ok(byte) | Err(byte) => byte.min(piece.text.len()),
    }
}

fn nearest_byte(piece: &Piece, position: Point<Pixels>) -> usize {
    let text = piece.text.as_ref();
    let under = byte_under(piece, position);
    let Some(after) = text[under..]
        .chars()
        .next()
        .map(|char| under + char.len_utf8())
    else {
        return under;
    };
    let x = |byte| piece.layout.position_for_index(byte).map(|at| at.x);
    match (x(under), x(after)) {
        (Some(from), Some(to)) if position.x - from > to - position.x => after,
        _ => under,
    }
}

fn distance(at: Pixels, from: Pixels, to: Pixels) -> Pixels {
    if at < from {
        from - at
    } else if at > to {
        at - to
    } else {
        Pixels::ZERO
    }
}

/// The selection of one chat window, shared by what draws the conversation and what takes the
/// mouse.
#[derive(Clone, Default)]
pub(crate) struct Selection(Rc<RefCell<State>>);

impl Selection {
    /// Starts a frame of the conversation, whose pieces are marked with `marked`.
    pub(crate) fn begin(&self, marked: Hsla) {
        let mut state = self.0.borrow_mut();
        state.pieces.clear();
        state.marked = Some(marked);
    }

    /// What draws the pieces of the conversation's line `line`.
    pub(crate) fn pen(&self, line: usize) -> Pen {
        Pen {
            selection: self.clone(),
            line,
            next: Cell::new(0),
        }
    }

    /// Starts selecting at `position` if a word is there, and says whether one was. The second
    /// press in a row selects the word there and the third its whole line. A press anywhere else
    /// lets the selection go.
    pub(crate) fn press(&self, position: Point<Pixels>, presses: usize) -> bool {
        let mut state = self.0.borrow_mut();
        let pressed = state
            .spot_at(position, presses < 2)
            .filter(|(_, on)| *on)
            .map(|(spot, _)| spot);
        let ends = pressed.map(|spot| match presses {
            0 | 1 => (spot, spot),
            2 => state.word_around(spot),
            _ => state.line_around(spot),
        });
        state.anchor = ends.map(|(start, _)| start);
        state.head = ends.map(|(_, end)| end);
        state.dragging = pressed.is_some() && presses < 2;
        pressed.is_some()
    }

    /// Moves where the selection ends to the word nearest `position`, while the mouse is held.
    /// Says whether the selection changed.
    pub(crate) fn drag(&self, position: Point<Pixels>) -> bool {
        let mut state = self.0.borrow_mut();
        if !state.dragging {
            return false;
        }
        let head = state.spot_at(position, true).map(|(spot, _)| spot);
        let moved = head.is_some() && head != state.head;
        if moved {
            state.head = head;
        }
        moved
    }

    /// Ends the drag; the selection stays.
    pub(crate) fn release(&self) {
        self.0.borrow_mut().dragging = false;
    }

    /// Whether the mouse is selecting now.
    pub(crate) fn dragging(&self) -> bool {
        self.0.borrow().dragging
    }

    /// Lets the selection go. Says whether there was one.
    pub(crate) fn clear(&self) -> bool {
        let mut state = self.0.borrow_mut();
        let had = state.ends().is_some();
        state.anchor = None;
        state.head = None;
        state.dragging = false;
        had
    }

    /// The selected words as they were written, or nothing when no word is selected.
    pub(crate) fn copied(&self) -> Option<String> {
        let state = self.0.borrow();
        let (start, _) = state.ends()?;
        let mut text = String::new();
        for piece in &state.pieces {
            let Some(range) = state.marked_in(piece.key, piece.text.len()) else {
                continue;
            };
            if piece.key != start.key {
                text.push_str(match piece.joint {
                    _ if piece.key.piece == 0 => "\n\n",
                    Joint::Joined => "",
                    Joint::Space => " ",
                    Joint::Newline => "\n",
                });
            }
            text.push_str(&piece.text[range]);
        }
        (!text.is_empty()).then_some(text)
    }

    #[cfg(test)]
    pub(crate) fn bounds_of(&self, word: &str) -> Option<Bounds<Pixels>> {
        self.0
            .borrow()
            .pieces
            .iter()
            .find(|piece| piece.text.as_ref() == word)
            .map(|piece| piece.layout.bounds())
    }
}

/// Draws the selectable pieces of one line of the conversation, in the order they are read.
pub(crate) struct Pen {
    selection: Selection,
    line: usize,
    next: Cell<usize>,
}

impl Pen {
    /// A piece of text that `joint` separates from the piece before it, followed by the gap
    /// between words when `spaced`. A piece selected through to the next one is marked with
    /// its gap, so a selection reads as one band and not as words apart.
    pub(crate) fn word(&self, text: impl Into<SharedString>, joint: Joint, spaced: bool) -> Div {
        let text = text.into();
        let key = Key {
            line: self.line,
            piece: self.next.get(),
        };
        self.next.set(key.piece + 1);
        let mut state = self.selection.0.borrow_mut();
        let len = text.len();
        let range = state.marked_in(key, len);
        let goes_on = state.ends().is_some_and(|(_, end)| end.key > key);
        let through = spaced && goes_on && range == Some(0..len);
        let marked = range
            .filter(|_| !through)
            .zip(state.marked)
            .map(|(range, color)| {
                (
                    range,
                    HighlightStyle {
                        background_color: Some(color),
                        ..HighlightStyle::default()
                    },
                )
            });
        let styled = StyledText::new(text.clone()).with_highlights(marked);
        state.pieces.push(Piece {
            key,
            text,
            joint,
            layout: styled.layout().clone(),
        });
        div()
            .min_w_0()
            .cursor_text()
            .when(spaced, |word| word.pr(px(SPACE)))
            .when_some(state.marked.filter(|_| through), |word, color| {
                word.bg(color)
            })
            .child(styled)
    }
}

#[cfg(test)]
#[path = "select.test.rs"]
mod tests;
