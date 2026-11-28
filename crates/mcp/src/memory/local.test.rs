use super::*;

fn folder(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dotori-{name}-{}", std::process::id()))
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

#[test]
fn a_download_of_another_size_or_digest_is_refused() {
    let source = &KOREAN.tokenizer;
    let refused = checked(source, source.size, &[0; 32]).expect_err("damaged");
    assert!(refused.contains("damaged"), "{refused}");
    let digest: Vec<u8> = (0..32)
        .map(|at| u8::from_str_radix(&source.digest[at * 2..at * 2 + 2], 16).expect("hex"))
        .collect();
    assert!(checked(source, source.size, &digest).is_ok());
    assert!(checked(source, source.size - 1, &digest).is_err());
}

#[test]
fn only_drafts_left_long_ago_by_another_download_of_the_file_are_dropped() {
    let folder = folder("drafts");
    std::fs::create_dir_all(&folder).expect("made");
    let long_ago = std::time::SystemTime::now() - WAIT - Duration::from_secs(60);
    for (name, old) in [
        ("tokenizer.part.1", true),
        ("tokenizer.part.2", false),
        ("model.part.3", true),
    ] {
        let file = std::fs::File::create(folder.join(name)).expect("made");
        if old {
            file.set_modified(long_ago).expect("aged");
        }
    }
    drop_abandoned_drafts(&folder.join("tokenizer.json"));
    let mut left: Vec<String> = std::fs::read_dir(&folder)
        .expect("listed")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    left.sort();
    assert_eq!(left, ["model.part.3", "tokenizer.part.2"]);
    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn a_model_not_downloaded_yet_makes_no_vector() {
    let local = Local::new(LocalModel::Korean, &folder("no-model"));
    let refused = local
        .embed(&[String::from("치과 예약")], Use::Query)
        .await
        .expect_err("no model");
    assert!(
        refused.detail.contains("not downloaded"),
        "{}",
        refused.detail
    );
    assert!(!refused.by_text);
}

#[test]
fn each_language_has_its_own_model() {
    assert_ne!(
        Local::new(LocalModel::Korean, Path::new("m")).name(),
        Local::new(LocalModel::English, Path::new("m")).name()
    );
}

#[tokio::test]
#[ignore = "downloads both embedding models, about 76 MB, from Hugging Face"]
async fn each_question_finds_the_record_that_answers_it() {
    let models = std::env::temp_dir().join("dotori-models-test");
    let cases = [
        (
            LocalModel::Korean,
            [
                (
                    "민수형 요즘 어디서 일해?",
                    "민수형 이번에 강남에 있는 회사로 이직했대",
                ),
                (
                    "다음 주 회의 언제였지",
                    "팀 회의는 다음 주 화요일 오후 3시로 잡혔어",
                ),
                (
                    "엄마 생일 선물 뭐 사기로 했더라",
                    "엄마 생일에는 스카프를 사 드리기로 했어",
                ),
                ("치과 예약 시간", "치과는 목요일 오전 10시 반으로 예약했어"),
            ],
        ),
        (
            LocalModel::English,
            [
                (
                    "where does Minsu work now",
                    "Minsu just moved to a company in Gangnam.",
                ),
                (
                    "when is the team meeting",
                    "The team meeting is next Tuesday at 3pm.",
                ),
                (
                    "what gift for mom's birthday",
                    "We decided to get mom a scarf for her birthday.",
                ),
                (
                    "dentist appointment time",
                    "The dentist appointment is Thursday at 10:30.",
                ),
            ],
        ),
    ];
    for (model, pairs) in cases {
        let local = Local::new(model, &models);
        local.prepare().await.expect("downloaded");
        let questions: Vec<String> = pairs.iter().map(|(asked, _)| (*asked).to_owned()).collect();
        let records: Vec<String> = pairs.iter().map(|(_, said)| (*said).to_owned()).collect();
        let asked = local
            .embed(&questions, Use::Query)
            .await
            .expect("questions");
        let said = local.embed(&records, Use::Record).await.expect("records");
        for (at, question) in asked.iter().enumerate() {
            assert_eq!(question.len(), 384);
            assert!((dot(question, question) - 1.0).abs() < 1e-4);
            let nearest = said
                .iter()
                .enumerate()
                .max_by(|a, b| dot(question, a.1).total_cmp(&dot(question, b.1)))
                .map(|(found, _)| found);
            assert_eq!(nearest, Some(at), "{model:?}: {}", questions[at]);
        }
    }
}
