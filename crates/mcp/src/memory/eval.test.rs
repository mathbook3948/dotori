use super::super::tests::config;
use super::super::tests::embedded;
use super::super::tests::embedding_api;
use super::super::tests::memory;
use super::super::tests::record;
use super::*;

fn case(question: &str, expect: &[&str]) -> Case {
    Case {
        question: question.to_owned(),
        expect: expect.iter().map(|words| (*words).to_owned()).collect(),
    }
}

#[tokio::test]
async fn words_are_scored_and_a_question_that_cannot_be_scored_is_left_out_with_why() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 미팅은 3시에 해요")),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = memory(config);

    let report = memory
        .evaluate(vec![
            case("점심 메뉴", &["점심 메뉴"]),
            case("회의 시간", &["3시"]),
            case("주말 계획", &["캠핑"]),
            case("?!", &["3시"]),
        ])
        .await
        .expect("evaluated");

    assert_eq!(
        report,
        Report {
            scored: 2,
            left_out: vec![
                (
                    String::from("주말 계획"),
                    String::from("no record holds the words it expects")
                ),
                (
                    String::from("?!"),
                    String::from("it has no words to search for")
                ),
            ],
            scores: vec![Score {
                method: Method::Words,
                first: 1,
                shown: 1,
                reciprocal: 0.5,
            }],
            no_meaning: Some(String::from("no embedding API is set up")),
            vectored: None,
        }
    );
}

#[tokio::test]
async fn meaning_is_scored_once_records_have_vectors() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 미팅은 3시에 해요")),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = embedded(config, embedding_api(false).await);
    let cases = || vec![case("meeting", &["3시"])];

    let before = memory.evaluate(cases()).await.expect("evaluated");
    while memory.fill_once().await.expect("filled") {}
    let after = memory.evaluate(cases()).await.expect("evaluated");

    assert_eq!(
        before.no_meaning.as_deref(),
        Some("no record has a vector yet")
    );
    assert_eq!(before.scores.len(), 1);
    assert_eq!(after.no_meaning, None);
    assert_eq!(after.vectored, Some((2, 2)));
    let shown: Vec<(Method, usize, usize)> = after
        .scores
        .iter()
        .map(|score| (score.method, score.first, score.shown))
        .collect();
    assert_eq!(
        shown,
        [
            (Method::Words, 0, 0),
            (Method::Meaning, 1, 1),
            (Method::Both, 1, 1)
        ]
    );
}
