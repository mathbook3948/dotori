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
