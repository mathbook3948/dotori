use dotori_mcp::memory::Score;

use super::*;

#[test]
fn each_line_is_a_question_and_a_broken_line_is_named() {
    let cases = parsed(
        "{\"question\": \"점심 메뉴\", \"expect\": [\"김밥\"]}\n\n{\"question\": \"회의\", \"expect\": []}\n",
    )
    .expect("parsed");

    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].question, "점심 메뉴");
    assert_eq!(cases[0].expect, ["김밥"]);
    let broken =
        parsed("{\"question\": \"a\", \"expect\": []}\n{\"question\": 3}").expect_err("refused");
    assert!(broken.starts_with("line 2: "), "{broken}");
}

#[test]
fn the_report_lines_up_each_way_of_searching() {
    let report = Report {
        scored: 2,
        left_out: vec![(
            String::from("주말 계획"),
            String::from("no record holds the words it expects"),
        )],
        scores: vec![Score {
            method: Method::Words,
            first: 1,
            shown: 1,
            reciprocal: 0.5,
        }],
        no_meaning: Some(String::from("no embedding API is set up")),
        vectored: None,
    };

    assert_eq!(
        shown(&report),
        "Scored 2 questions against the first 10 hits a search shows.\n\
         Left out:\n  주말 계획 (no record holds the words it expects)\n\
         by        first  shown    MRR\n\
         words         1      1  0.500\n\
         Not scored by meaning: no embedding API is set up.\n"
    );
}

#[test]
fn the_report_says_how_many_records_meaning_was_scored_with() {
    let report = Report {
        scored: 1,
        left_out: Vec::new(),
        scores: Vec::new(),
        no_meaning: None,
        vectored: Some((3, 5)),
    };

    assert!(
        shown(&report).ends_with("Meaning was scored with vectors for 3 of 5 records with text.\n")
    );
}
