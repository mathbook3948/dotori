use super::*;

fn plain(text: &str) -> Run {
    Run {
        text: text.to_owned(),
        bold: false,
        code: false,
    }
}

fn bold(text: &str) -> Run {
    Run {
        text: text.to_owned(),
        bold: true,
        code: false,
    }
}

fn code(text: &str) -> Run {
    Run {
        text: text.to_owned(),
        bold: false,
        code: true,
    }
}

#[test]
fn a_reply_with_a_list_is_split_into_its_lines_and_items() {
    let shown =
        blocks("오늘은 일정이 있어요.\n\n- 10:00 리뷰\n  * 회의실 A\n2. 치과\n\n\n끝이에요.\n\n");

    assert_eq!(
        shown,
        [
            Block::Line(vec![
                vec![plain("오늘은")],
                vec![plain("일정이")],
                vec![plain("있어요.")]
            ]),
            Block::Gap,
            Block::Item {
                depth: 0,
                mark: String::from("•"),
                words: vec![vec![plain("10:00")], vec![plain("리뷰")]],
            },
            Block::Item {
                depth: 1,
                mark: String::from("•"),
                words: vec![vec![plain("회의실")], vec![plain("A")]],
            },
            Block::Item {
                depth: 0,
                mark: String::from("2."),
                words: vec![vec![plain("치과")]],
            },
            Block::Gap,
            Block::Line(vec![vec![plain("끝이에요.")]]),
        ]
    );
}

#[test]
fn fenced_code_keeps_its_lines_and_an_open_fence_runs_to_the_end() {
    assert_eq!(
        blocks("이렇게요.\n```rust\nfn main() {\n    run();\n}\n```\n## 다음"),
        [
            Block::Line(vec![vec![plain("이렇게요.")]]),
            Block::Code(vec![
                String::from("fn main() {"),
                String::from("    run();"),
                String::from("}"),
            ]),
            Block::Heading(vec![vec![plain("다음")]]),
        ]
    );
    assert_eq!(
        blocks("```\n- 목록 아님"),
        [Block::Code(vec![String::from("- 목록 아님")])]
    );
}

#[test]
fn bold_and_code_marks_style_their_words_and_a_lone_mark_stays() {
    assert_eq!(
        words("**치과**에 `git status` 실행"),
        [
            vec![bold("치과"), plain("에")],
            vec![code("git status")],
            vec![plain("실행")],
        ]
    );
    assert_eq!(
        words("2 ** 3 = 8"),
        [
            vec![plain("2")],
            vec![plain("**")],
            vec![plain("3")],
            vec![plain("=")],
            vec![plain("8")],
        ]
    );
    assert_eq!(words("a  b"), [vec![plain("a")], vec![], vec![plain("b")]]);
}

#[test]
fn a_line_that_only_looks_like_a_list_or_heading_stays_a_line() {
    assert_eq!(
        blocks("#태그\n-1도\n2026. 11월"),
        [
            Block::Line(vec![vec![plain("#태그")]]),
            Block::Line(vec![vec![plain("-1도")]]),
            Block::Line(vec![vec![plain("2026.")], vec![plain("11월")]]),
        ]
    );
}

#[test]
fn bold_marks_inside_code_are_not_counted_and_a_one_line_fence_is_text() {
    assert_eq!(
        words("`a**b` 와 **굵게**"),
        [vec![code("a**b")], vec![plain("와")], vec![bold("굵게")]]
    );
    assert_eq!(
        blocks("```ls -la```\n다음 줄"),
        [
            Block::Line(vec![vec![code("ls -la")]]),
            Block::Line(vec![vec![plain("다음")], vec![plain("줄")]]),
        ]
    );
    assert_eq!(blocks("```"), [Block::Line(vec![vec![plain("```")]])]);
    assert_eq!(blocks("```\n\n"), [Block::Line(vec![vec![plain("```")]])]);
}

#[test]
fn only_ascii_spaces_indent_an_item_and_an_empty_heading_stays_a_line() {
    assert_eq!(
        blocks("\u{3000}- 항목\n#   "),
        [
            Block::Line(vec![vec![plain("\u{3000}-")], vec![plain("항목")]]),
            Block::Line(vec![vec![plain("#")]]),
        ]
    );
}

mod properties {
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::*;

    const MARKUP_HEAVY: &str = "[ a-z가-힣`*#•\\-.)0-9\t\r\n]{0,120}";

    fn spelled(words: &[Word]) -> String {
        words
            .iter()
            .map(|word| word.iter().map(|run| run.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn every_run(blocks: &[Block]) -> Vec<&Run> {
        blocks
            .iter()
            .flat_map(|block| match block {
                Block::Line(words) | Block::Heading(words) | Block::Item { words, .. } => {
                    words.iter().flatten().collect()
                }
                Block::Gap | Block::Code(_) => Vec::new(),
            })
            .collect()
    }

    proptest! {
        #[test]
        fn rendering_any_string_never_panics(text in any::<String>()) {
            let _ = blocks(&text);
        }

        #[test]
        fn rendering_marks_in_any_order_never_panics(text in MARKUP_HEAVY) {
            let _ = blocks(&text);
        }

        #[test]
        fn a_reply_never_starts_or_ends_with_a_gap_nor_repeats_one(text in MARKUP_HEAVY) {
            let shown = blocks(&text);
            prop_assert!(!matches!(shown.first(), Some(Block::Gap)));
            prop_assert!(!matches!(shown.last(), Some(Block::Gap)));
            prop_assert!(!shown
                .windows(2)
                .any(|pair| pair[0] == Block::Gap && pair[1] == Block::Gap));
        }

        #[test]
        fn an_item_never_hangs_deeper_than_the_limit(text in MARKUP_HEAVY) {
            for block in blocks(&text) {
                if let Block::Item { depth, .. } = block {
                    prop_assert!(depth <= MAX_DEPTH);
                }
            }
        }

        #[test]
        fn text_without_marks_is_one_plain_paragraph_with_the_same_text(
            text in "[a-z가-힣][a-z가-힣 ,.!?]{0,60}",
        ) {
            let shown = blocks(&text);
            prop_assert_eq!(shown.len(), 1);
            let Block::Line(words) = &shown[0] else {
                return Err(TestCaseError::fail(format!("not a line: {shown:?}")));
            };
            prop_assert_eq!(spelled(words), text.trim_end());
            prop_assert!(every_run(&shown).iter().all(|run| !run.bold && !run.code));
        }

        #[test]
        fn a_mark_without_its_pair_stays_as_written(
            before in "[a-z가-힣 ]{0,20}",
            after in "[a-z가-힣 ]{0,20}",
            mark in prop_oneof![Just("`"), Just("**")],
        ) {
            let line = format!("x{before}{mark}{after}y");
            let shown = words(&line);
            prop_assert_eq!(spelled(&shown), line);
            prop_assert!(shown.iter().flatten().all(|run| !run.bold && !run.code));
        }

        #[test]
        fn a_particle_stays_in_the_word_of_the_code_it_follows(
            span in "[a-z][a-z ]{0,10}",
            particle in "[가-힣]{1,3}",
        ) {
            let shown = words(&format!("`{span}`{particle}"));
            prop_assert_eq!(shown, vec![vec![code(&span), plain(&particle)]]);
        }

        #[test]
        fn fenced_code_keeps_every_line_as_written(
            lines in vec("[a-z0-9 #*\\-]{0,12}", 1..6),
        ) {
            let text = format!("```\n{}\n```", lines.join("\n"));
            prop_assert_eq!(blocks(&text), vec![Block::Code(lines)]);
        }
    }
}
