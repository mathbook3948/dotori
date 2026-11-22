use super::*;

#[test]
fn without_a_dictionary_words_lose_common_particles() {
    assert_eq!(
        Analyzer::Plain.terms("민지는 Slack에서 회의를 잡았다"),
        ["민지", "slack", "회의", "잡았다"]
    );
    assert_eq!(Analyzer::Plain.terms("회의 일정"), ["회의", "일정"]);
}

#[test]
fn a_dictionary_word_keeps_its_base_form() {
    let details = |text: &str| -> Vec<String> { text.split(',').map(String::from).collect() };
    assert_eq!(
        lemma(
            "했었",
            &details("VV+EP,*,T,했었,Inflect,VV,EP,하/VV/*+았었/EP/*")
        ),
        None
    );
    assert_eq!(
        lemma(
            "갔었",
            &details("VV+EP,*,T,갔었,Inflect,VV,EP,가/VV/*+았었/EP/*")
        ),
        Some(String::from("가"))
    );
    assert_eq!(
        lemma(
            "사무실",
            &details("NNG,장소,T,사무실,Compound,*,*,사무/NNG/*+실/NNG/*")
        ),
        Some(String::from("사무실"))
    );
    assert_eq!(lemma("는", &details("JX,*,T,는,*,*,*,*")), None);
    assert_eq!(
        lemma("Slack", &details("SL,*,*,*,*,*,*,*")),
        Some(String::from("slack"))
    );
}
