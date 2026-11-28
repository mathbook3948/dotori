//! A text field of one line or several, adapted from gpui's `examples/input.rs`
//! (Apache-2.0, Copyright Zed Industries). Changed: public constructors and text access,
//! a `Submit` action in place of `Quit`, the project's import style, colors and size taken
//! from the theme and the parent instead of fixed ones, and wrapped lines with a line break
//! on Shift+Enter, moving up and down, scrolling to the cursor, and undo and redo.

use std::ops::Range;

use gpui::App;
use gpui::AvailableSpace;
use gpui::Bounds;
use gpui::ClipboardItem;
use gpui::ContentMask;
use gpui::Context;
use gpui::CursorStyle;
use gpui::ElementId;
use gpui::ElementInputHandler;
use gpui::Entity;
use gpui::EntityInputHandler;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::GlobalElementId;
use gpui::KeyBinding;
use gpui::LayoutId;
use gpui::MouseButton;
use gpui::MouseDownEvent;
use gpui::MouseMoveEvent;
use gpui::MouseUpEvent;
use gpui::PaintQuad;
use gpui::Pixels;
use gpui::Point;
use gpui::ScrollHandle;
use gpui::SharedString;
use gpui::Style;
use gpui::TextAlign;
use gpui::TextRun;
use gpui::UTF16Selection;
use gpui::UnderlineStyle;
use gpui::Window;
use gpui::WrappedLine;
use gpui::actions;
use gpui::div;
use gpui::fill;
use gpui::point;
use gpui::prelude::*;
use gpui::px;
use gpui::relative;
use gpui::size;
use unicode_segmentation::UnicodeSegmentation;

use crate::window::theme::Theme;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Submit,
        Newline,
        Undo,
        Redo,
    ]
);

const MAX_ROWS: f32 = 8.;
const DOT: char = '•';
const MAX_UNDO: usize = 200;

#[derive(Clone)]
struct Snapshot {
    content: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Typing,
    Deleting,
}

/// A text field, of one line or of several.
pub(crate) struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<Shaped>,
    last_bounds: Option<Bounds<Pixels>>,
    shift: Pixels,
    is_selecting: bool,
    multiline: bool,
    masked: bool,
    locked: bool,
    scroll: ScrollHandle,
    followed: Option<(SharedString, usize, Pixels)>,
    column: Option<Pixels>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    run: Option<(Change, usize)>,
}

impl TextInput {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            content: self.content.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.content = snapshot.content;
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.marked_range = None;
        self.column = None;
        self.run = None;
        cx.notify();
    }

    fn record(&mut self, range: &Range<usize>, new_text: &str) {
        let change = if new_text.is_empty() {
            Change::Deleting
        } else {
            Change::Typing
        };
        let word_starts = new_text.starts_with(char::is_whitespace)
            && !self.content[..range.start].ends_with(char::is_whitespace);
        let continues = match (change, self.run) {
            (Change::Typing, Some((Change::Typing, at))) => {
                range.is_empty() && range.start == at && !word_starts
            }
            (Change::Deleting, Some((Change::Deleting, at))) => {
                range.start == at || range.end == at
            }
            _ => false,
        };
        if !continues {
            self.undo.push(self.snapshot());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.run = Some((change, range.start + new_text.len()));
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked || self.marked_range.is_some() {
            return;
        }
        if let Some(before) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(before, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked || self.marked_range.is_some() {
            return;
        }
        if let Some(after) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(after, cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let (to, column) = self.vertical(-1.);
        self.move_to(to, cx);
        self.column = column;
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let (to, column) = self.vertical(1.);
        self.move_to(to, cx);
        self.column = column;
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let (to, column) = self.vertical(-1.);
        self.select_to(to, cx);
        self.column = column;
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let (to, column) = self.vertical(1.);
        self.select_to(to, cx);
        self.column = column;
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline && self.marked_range.is_none() {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    fn vertical(&self, rows: f32) -> (usize, Option<Pixels>) {
        let Some(shaped) = self.last_layout.as_ref() else {
            return (self.cursor_offset(), None);
        };
        let at = shaped.position(self.shown(self.cursor_offset()));
        let x = self.column.unwrap_or(at.x);
        let y = at.y + shaped.line_height * (rows + 0.5);
        let to = if y < Pixels::ZERO {
            0
        } else if y >= shaped.height() {
            self.content.len()
        } else {
            self.within(self.hidden(shaped.index(point(x, y))))
        };
        (to, Some(x))
    }

    fn within(&self, index: usize) -> usize {
        let mut index = index.min(self.content.len());
        while !self.content.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    fn follow(&self, shaped: &Shaped, bounds: Bounds<Pixels>) -> bool {
        let view = self.scroll.bounds();
        if view.size.height <= Pixels::ZERO {
            return false;
        }
        let cursor = bounds.top() + shaped.position(self.shown(self.cursor_offset())).y;
        let mut offset = self.scroll.offset();
        if cursor < view.top() {
            offset.y += view.top() - cursor;
        } else if cursor + shaped.line_height > view.bottom() {
            offset.y -= cursor + shaped.line_height - view.bottom();
        }
        self.scroll.set_offset(offset);
        true
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let start = self.content[..cursor].rfind('\n').map_or(0, |at| at + 1);
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let end = self.content[cursor..]
            .find('\n')
            .map_or(self.content.len(), |at| cursor + at);
        self.move_to(end, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n");
            let text = if self.multiline {
                text
            } else {
                text.replace('\n', " ")
            };
            self.run = None;
            self.replace_text_in_range(None, &text, window, cx);
            self.run = None;
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.column = None;
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(shaped)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        self.within(
            self.hidden(shaped.index(position - bounds.origin + point(self.shift, Pixels::ZERO))),
        )
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.column = None;
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    /// A field of one line.
    pub(crate) fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            shift: Pixels::ZERO,
            is_selecting: false,
            multiline: false,
            masked: false,
            locked: false,
            scroll: ScrollHandle::new(),
            followed: None,
            column: None,
            undo: Vec::new(),
            redo: Vec::new(),
            run: None,
        }
    }

    /// A field of one line that shows a dot for each character, for a password. It never hands
    /// the input method the real text.
    pub(crate) fn masked(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            masked: true,
            ..Self::new(placeholder, cx)
        }
    }

    fn shown(&self, index: usize) -> usize {
        if self.masked {
            self.content[..index.min(self.content.len())]
                .chars()
                .count()
                * DOT.len_utf8()
        } else {
            index
        }
    }

    fn hidden(&self, shown: usize) -> usize {
        if !self.masked {
            return shown;
        }
        self.content
            .char_indices()
            .nth(shown / DOT.len_utf8())
            .map_or(self.content.len(), |(at, _)| at)
    }

    /// A field that grows to several lines as the text wraps, and takes a line break on
    /// Shift+Enter.
    pub(crate) fn multiline(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            multiline: true,
            ..Self::new(placeholder, cx)
        }
    }

    /// Shows `placeholder` while the field is empty, from now on.
    pub(crate) fn set_placeholder(&mut self, placeholder: SharedString, cx: &mut Context<Self>) {
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            cx.notify();
        }
    }

    /// Holds the text as it is while `locked`, so neither typing nor pasting changes it.
    pub(crate) fn set_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        if self.locked != locked {
            self.locked = locked;
            self.marked_range = None;
            cx.notify();
        }
    }

    /// Whether some of the text is selected.
    pub(crate) fn has_selection(&self) -> bool {
        !self.selected_range.is_empty()
    }

    pub(crate) fn text(&self) -> &str {
        &self.content
    }

    pub(crate) fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.reset();
        self.content = SharedString::from(text.to_owned());
        self.selected_range = text.len()..text.len();
        cx.notify();
    }

    /// Whether an input method is still composing text in the field.
    pub(crate) fn composing(&self) -> bool {
        self.marked_range.is_some()
    }

    pub(crate) fn take(&mut self, cx: &mut Context<Self>) -> String {
        let text = self.content.to_string();
        self.reset();
        cx.notify();
        text
    }

    fn reset(&mut self) {
        self.content = "".into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.is_selecting = false;
        self.undo.clear();
        self.redo.clear();
        self.run = None;
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        if self.masked {
            return Some(
                self.content[range]
                    .chars()
                    .flat_map(|c| std::iter::repeat_n(DOT, c.len_utf16()))
                    .collect(),
            );
        }
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.locked {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if self.marked_range.is_some() {
            self.run = Some((Change::Typing, range.start + new_text.len()));
        } else if range.is_empty() && new_text.is_empty() {
            return;
        } else {
            self.record(&range, new_text);
        }
        self.column = None;
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.locked {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if self.marked_range.is_none() && !new_text.is_empty() {
            self.record(&range, new_text);
        }
        self.column = None;
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let shaped = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = shaped.position(self.shown(range.start));
        let end = shaped.position(self.shown(range.end));
        let origin = bounds.origin - point(self.shift, Pixels::ZERO);
        Some(Bounds::from_corners(
            origin + start,
            origin + point(end.x.max(start.x), start.y + shaped.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let inside = self.last_bounds?.localize(&point)? + gpui::point(self.shift, Pixels::ZERO);
        let shaped = self.last_layout.as_ref()?;
        Some(self.offset_to_utf16(self.within(self.hidden(shaped.index(inside)))))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

#[derive(Clone)]
struct Shaped {
    lines: Vec<WrappedLine>,
    starts: Vec<usize>,
    line_height: Pixels,
}

impl Shaped {
    fn new(
        text: &SharedString,
        runs: &[TextRun],
        font_size: Pixels,
        wrap: Option<Pixels>,
        line_height: Pixels,
        window: &Window,
    ) -> Self {
        let lines: Vec<WrappedLine> = window
            .text_system()
            .shape_text(text.clone(), font_size, runs, wrap, None)
            .map(|lines| lines.into_vec())
            .unwrap_or_default();
        let mut starts = Vec::with_capacity(lines.len());
        let mut start = 0;
        for line in &lines {
            starts.push(start);
            start += line.len() + 1;
        }
        Self {
            lines,
            starts,
            line_height,
        }
    }

    fn height(&self) -> Pixels {
        self.lines
            .iter()
            .map(|line| line.size(self.line_height).height)
            .fold(Pixels::ZERO, |sum, height| sum + height)
            .max(self.line_height)
    }

    fn position(&self, index: usize) -> Point<Pixels> {
        let mut top = Pixels::ZERO;
        for (n, line) in self.lines.iter().enumerate() {
            let start = self.starts[n];
            if index <= start + line.len() {
                let at = line
                    .position_for_index(index.saturating_sub(start), self.line_height)
                    .unwrap_or_else(|| point(line.width(), Pixels::ZERO));
                return point(at.x, top + at.y);
            }
            top += line.size(self.line_height).height;
        }
        point(Pixels::ZERO, top)
    }

    fn index(&self, at: Point<Pixels>) -> usize {
        let mut top = Pixels::ZERO;
        let last = self.lines.len().saturating_sub(1);
        for (n, line) in self.lines.iter().enumerate() {
            let height = line.size(self.line_height).height;
            if at.y < top + height || n == last {
                let inside = point(
                    at.x.max(Pixels::ZERO),
                    (at.y - top).clamp(Pixels::ZERO, height - px(1.)),
                );
                let found = line
                    .closest_index_for_position(inside, self.line_height)
                    .unwrap_or_else(|edge| edge);
                return self.starts[n] + found;
            }
            top += height;
        }
        0
    }

    fn rows(&self, range: &Range<usize>, width: Pixels) -> Vec<Bounds<Pixels>> {
        let from = self.position(range.start);
        let to = self.position(range.end);
        let height = self.line_height;
        if from.y == to.y {
            return vec![Bounds::from_corners(from, point(to.x, to.y + height))];
        }
        let mut rows = vec![Bounds::from_corners(from, point(width, from.y + height))];
        if to.y > from.y + height {
            rows.push(Bounds::from_corners(
                point(Pixels::ZERO, from.y + height),
                point(width, to.y),
            ));
        }
        rows.push(Bounds::from_corners(
            point(Pixels::ZERO, to.y),
            point(to.x, to.y + height),
        ));
        rows
    }
}

struct PrepaintState {
    shaped: Option<Shaped>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    shift: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl TextInput {
    fn display(&self, window: &Window) -> (SharedString, Vec<TextRun>, Pixels) {
        let style = window.text_style();
        let theme = Theme::of(window);
        let (text, color) = if self.content.is_empty() {
            (self.placeholder.clone(), theme.muted.into())
        } else if self.masked {
            (
                SharedString::from(DOT.to_string().repeat(self.content.chars().count())),
                style.color,
            )
        } else {
            (self.content.clone(), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked = self
            .marked_range
            .as_ref()
            .map(|marked| self.shown(marked.start)..self.shown(marked.end));
        let runs = match marked.as_ref() {
            Some(marked) => [
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        (text, runs, style.font_size.to_pixels(window.rem_size()))
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.input.read(cx);
        if !input.multiline {
            style.size.height = window.line_height().into();
            return (window.request_layout(style, [], cx), ());
        }
        let (text, runs, font_size) = input.display(window);
        let line_height = window.line_height();
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let shaped = Shaped::new(&text, &runs, font_size, width, line_height, window);
            size(width.unwrap_or_default(), shaped.height())
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let (text, runs, font_size) = input.display(window);
        let wrap = input.multiline.then_some(bounds.size.width);
        let shaped = Shaped::new(&text, &runs, font_size, wrap, window.line_height(), window);
        let theme = Theme::of(window);
        let selected = input.selected_range.clone();
        let cursor_at = shaped.position(if input.content.is_empty() {
            0
        } else {
            input.shown(input.cursor_offset())
        });
        let shift = if input.multiline {
            Pixels::ZERO
        } else {
            let room = bounds.size.width - px(2.);
            let widest = shaped
                .lines
                .first()
                .map_or(Pixels::ZERO, |line| line.width());
            let mut shift = input.shift;
            if cursor_at.x - shift > room {
                shift = cursor_at.x - room;
            }
            if cursor_at.x < shift {
                shift = cursor_at.x;
            }
            shift
                .min((widest - room).max(Pixels::ZERO))
                .max(Pixels::ZERO)
        };
        let origin = bounds.origin - point(shift, Pixels::ZERO);
        let (selection, cursor) = if selected.is_empty() || input.content.is_empty() {
            (
                Vec::new(),
                Some(fill(
                    Bounds::new(origin + cursor_at, size(px(1.5), shaped.line_height)),
                    theme.primary,
                )),
            )
        } else {
            (
                shaped
                    .rows(
                        &(input.shown(selected.start)..input.shown(selected.end)),
                        bounds.size.width,
                    )
                    .into_iter()
                    .map(|row| fill(Bounds::new(origin + row.origin, row.size), theme.selection))
                    .collect(),
                None,
            )
        };
        self.input.update(cx, |input, _| input.shift = shift);
        PrepaintState {
            shaped: Some(shaped),
            cursor,
            selection,
            shift,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let Some(shaped) = prepaint.shaped.take() else {
            return;
        };
        let shown = focus_handle.is_focused(window) && window.is_window_active();
        let mask = ContentMask {
            bounds: Bounds::new(
                bounds.origin - point(px(2.), Pixels::ZERO),
                size(bounds.size.width + px(4.), bounds.size.height),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            for selection in prepaint.selection.drain(..) {
                window.paint_quad(selection)
            }
            let mut top = bounds.top();
            for line in &shaped.lines {
                let _ = line.paint(
                    point(bounds.left() - prepaint.shift, top),
                    shaped.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                top += line.size(shaped.line_height).height;
            }
            if shown && let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        });

        let followed = self.input.update(cx, |input, _cx| {
            let cursor = (
                input.content.clone(),
                input.cursor_offset(),
                bounds.size.width,
            );
            let moved = input.multiline
                && input.followed.as_ref() != Some(&cursor)
                && input.follow(&shaped, bounds);
            if moved {
                input.followed = Some(cursor);
            }
            input.last_layout = Some(shaped);
            input.last_bounds = Some(bounds);
            moved
        });
        if followed {
            window.request_animation_frame();
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = div()
            .flex()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .child(TextElement { input: cx.entity() });
        if !self.multiline {
            return field.into_any_element();
        }
        div()
            .id("field")
            .w_full()
            .max_h(window.line_height() * MAX_ROWS)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(field)
            .into_any_element()
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The keys every text field answers to. `secondary` is Cmd on macOS and Ctrl elsewhere.
pub(crate) fn bindings() -> Vec<KeyBinding> {
    let field = Some("TextInput");
    vec![
        KeyBinding::new("backspace", Backspace, field),
        KeyBinding::new("delete", Delete, field),
        KeyBinding::new("left", Left, field),
        KeyBinding::new("right", Right, field),
        KeyBinding::new("shift-left", SelectLeft, field),
        KeyBinding::new("shift-right", SelectRight, field),
        KeyBinding::new("up", Up, field),
        KeyBinding::new("down", Down, field),
        KeyBinding::new("shift-up", SelectUp, field),
        KeyBinding::new("shift-down", SelectDown, field),
        KeyBinding::new("secondary-a", SelectAll, field),
        KeyBinding::new("secondary-v", Paste, field),
        KeyBinding::new("secondary-c", Copy, field),
        KeyBinding::new("secondary-x", Cut, field),
        KeyBinding::new("home", Home, field),
        KeyBinding::new("end", End, field),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, field),
        KeyBinding::new("enter", Submit, field),
        KeyBinding::new("shift-enter", Newline, field),
        KeyBinding::new("secondary-z", Undo, field),
        KeyBinding::new("secondary-shift-z", Redo, field),
        KeyBinding::new("secondary-y", Redo, field),
    ]
}

#[cfg(test)]
#[path = "input.test.rs"]
mod tests;
