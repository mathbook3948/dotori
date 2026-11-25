use serde_json::json;

use super::*;

fn coupang() -> Value {
    json!({
        "title": "쿠팡 로그인",
        "site": "쿠팡",
        "fields": [
            { "ref": 12, "label": "아이디(이메일)", "kind": "text" },
            { "ref": 13, "label": "비밀번호", "kind": "secret" }
        ],
        "checks": [{ "ref": 15, "label": "로그인 상태 유지" }],
        "submit": { "ref": 16, "label": "로그인" },
        "links": [
            { "ref": 17, "label": "아이디 찾기" },
            { "ref": 18, "label": "비밀번호 찾기" },
            { "ref": 19, "label": "회원가입" }
        ],
        "keep": true
    })
}

#[test]
fn a_login_card_is_read_with_what_each_element_must_be() {
    let spec = FillSpec::parse(&coupang()).expect("read");

    assert_eq!(spec.title, "쿠팡 로그인");
    assert!(spec.keep);
    assert_eq!(
        spec.roles(),
        [
            (12, Role::Text),
            (13, Role::Secret),
            (15, Role::Check),
            (16, Role::Submit),
            (17, Role::Link),
            (18, Role::Link),
            (19, Role::Link)
        ]
    );
    assert_eq!(
        spec.note(),
        "금고에 안전하게 보관돼요. 대화하는 AI는 보지 못해요."
    );
}

#[test]
fn the_card_shows_the_host_it_is_given_and_never_one_the_model_wrote() {
    let mut asked = coupang();
    asked["host"] = json!("evil.example");
    let spec = FillSpec::parse(&asked).expect("read");

    let shown = spec.shown(7, "login.coupang.com", &[(15, true)]);

    assert_eq!(shown["host"], "login.coupang.com");
    assert_eq!(shown["id"], 7);
    assert_eq!(shown["fields"][1]["kind"], "secret");
    assert_eq!(shown["fields"][1]["ref"], 13);
    assert_eq!(shown["checks"][0]["on"], true);
    assert!(shown.get("value").is_none());
}

#[test]
fn a_card_the_model_cannot_prefill_or_misname_is_refused() {
    let refused = |change: fn(&mut Value)| {
        let mut asked = coupang();
        change(&mut asked);
        FillSpec::parse(&asked).expect_err("refused")
    };

    assert!(refused(|asked| asked["submit"]["ref"] = json!(13)).contains("twice"));
    assert!(refused(|asked| asked["fields"][0]["kind"] = json!("value")).contains("kind"));
    assert!(refused(|asked| asked["fields"][0]["ref"] = json!("12")).contains("ref"));
    assert!(refused(|asked| asked["title"] = json!(" ")).contains("title"));
    assert!(
        refused(|asked| asked["fields"][0]["label"] = json!("a".repeat(41))).contains("at most")
    );
    assert!(refused(|asked| asked["site"] = Value::Null).contains("site"));
    assert!(refused(|asked| asked["fields"][0]["kind"] = json!("secret")).contains("one secret"));
    assert!(
        refused(|asked| {
            asked.as_object_mut().expect("object").remove("submit");
        })
        .contains("submit")
    );

    let mut prefilled = coupang();
    prefilled["fields"][0]["value"] = json!("me@example.com");
    let spec = FillSpec::parse(&prefilled).expect("read");
    assert!(
        !spec
            .shown(1, "a.example", &[])
            .to_string()
            .contains("me@example.com")
    );
}

#[test]
fn each_card_says_what_happens_to_what_the_owner_types() {
    let code = FillSpec::parse(&json!({
        "title": "쿠팡 본인 확인",
        "fields": [{ "ref": 3, "label": "인증번호", "kind": "code" }],
        "submit": { "ref": 4, "label": "확인" }
    }))
    .expect("read");
    let forgot = FillSpec::parse(&json!({
        "title": "쿠팡 비밀번호 찾기",
        "fields": [{ "ref": 3, "label": "이름" }, { "ref": 5, "label": "이메일" }],
        "submit": { "ref": 4, "label": "다음" }
    }))
    .expect("read");

    assert_eq!(code.note(), "인증번호는 보관하지 않아요.");
    assert_eq!(forgot.note(), "대화하는 AI는 적은 내용을 보지 못해요.");
    assert!(!forgot.keep);
}

#[test]
fn what_the_model_is_told_names_the_fields_but_never_their_values() {
    let spec = FillSpec::parse(&coupang()).expect("read");

    let told = pressed(&spec, 16, &[12, 13], &[(15, true)], Ok(Some("쿠팡")));

    assert!(
        told.contains(
            "주인이 '로그인'을 눌렀어요. 채운 칸: 아이디(이메일), 비밀번호 (값은 보이지 않아요)."
        ),
        "{told}"
    );
    assert!(told.contains("로그인 상태 유지 켬"), "{told}");
    assert!(told.contains("금고에 쿠팡 계정을 넣었어요"), "{told}");
    let (shown, _) = Shown::read(&told).expect("headed");
    assert_eq!(
        shown,
        Shown::Pressed {
            title: String::from("쿠팡 로그인"),
            pressed: String::from("로그인"),
            link: false,
            filled: vec![String::from("아이디(이메일)"), String::from("비밀번호")],
            codes: false,
            kept: Some(String::from("쿠팡")),
        }
    );

    let link = pressed(&spec, 18, &[], &[], Ok(None));
    assert!(
        link.contains("주인이 '비밀번호 찾기'를 눌렀어요."),
        "{link}"
    );
    assert!(matches!(
        Shown::read(&link),
        Some((Shown::Pressed { link: true, .. }, _))
    ));
}

#[test]
fn an_answer_from_the_window_is_read_and_never_printed() {
    let (id, answer) = answer_of(&json!({
        "kind": "fill", "id": 4, "press": 16,
        "values": [{ "ref": 12, "value": "me@example.com" }, { "ref": 13, "value": "hunter22" }],
        "checks": [{ "ref": 15, "on": true }]
    }))
    .expect("an answer");

    assert_eq!(id, 4);
    assert_eq!(
        answer,
        Answer::Press {
            at: 16,
            values: vec![
                (12, String::from("me@example.com")),
                (13, String::from("hunter22"))
            ],
            checks: vec![(15, true)],
        }
    );
    let printed = format!("{answer:?}");
    assert!(
        !printed.contains("hunter22") && !printed.contains("me@example"),
        "{printed}"
    );
    assert_eq!(
        answer_of(&json!({ "kind": "fill", "id": 4, "later": true })),
        Some((4, Answer::Later))
    );
    assert_eq!(
        answer_of(&json!({ "kind": "fill", "id": 5, "allow": true, "trust": true, "account": 1 })),
        Some((
            5,
            Answer::Allow {
                trust: true,
                account: 1
            }
        ))
    );
    assert_eq!(
        answer_of(&json!({ "kind": "fill", "id": 5, "allow": false })),
        Some((5, Answer::Decline))
    );
    assert_eq!(answer_of(&json!({ "kind": "fill" })), None);
}

#[test]
fn an_account_is_kept_only_for_an_https_origin() {
    assert!(keepable("https://www.coupang.com"));
    assert!(keepable("https://login.example:8443"));
    assert!(!keepable("http://www.coupang.com"));
    assert!(!keepable("null"));
    assert!(!keepable("https://"));
    assert!(!keepable("data:text/html,x"));
}

#[test]
fn an_account_never_prints_its_id_or_password() {
    let account = Account {
        site: String::from("쿠팡"),
        origin: String::from("https://www.coupang.com"),
        id_label: Some(String::from("아이디")),
        id: Some(String::from("me@example.com")),
        password_label: String::from("비밀번호"),
        password: String::from("hunter22"),
        trusted: false,
        page: None,
        kept_ms: 0,
    };

    let printed = format!("{account:?}");

    assert!(printed.contains("쿠팡"), "{printed}");
    assert!(
        !printed.contains("hunter22") && !printed.contains("me@example"),
        "{printed}"
    );
}

#[test]
fn an_id_is_shortened_for_the_card() {
    assert_eq!(shortened("minji.example@mail.com"), "minji.example");
    assert_eq!(shortened("minji_k"), "minji_k");
    assert_eq!(shortened("a-very-long-user-name-here"), "a-very-long-use…");
}

#[test]
fn typed_values_are_hidden_from_what_the_tools_return() {
    let mut hidden = Hidden::default();
    hidden.add("me+shop@example.com");
    hidden.add("hunter22");
    hidden.add("123");

    let shown = hidden.scrub(
        "[3] input email: Email = me+shop@example.com\nWelcome me+shop@example.com\n\
         https://a.example/?u=me%2Bshop%40example.com pin 123 hunter22 order 41234 page 1",
    );

    assert!(!shown.contains("me+shop"), "{shown}");
    assert!(!shown.contains("me%2Bshop"), "{shown}");
    assert!(!shown.contains("hunter22"), "{shown}");
    assert!(
        shown.contains("pin [hidden]"),
        "a short value is hidden where it stands alone: {shown}"
    );
    assert!(
        shown.contains("order 41234 page 1"),
        "and nowhere else: {shown}"
    );
    assert!(shown.contains("[hidden]"), "{shown}");
    assert!(!format!("{hidden:?}").contains("hunter22"));
}

#[test]
fn a_kept_account_logs_in_by_the_elements_the_model_names() {
    let spec = LogInSpec::parse(&json!({ "id": 12, "password": 13, "submit": 16 })).expect("read");
    assert_eq!(
        spec.roles(),
        [(12, Role::Text), (13, Role::Secret), (16, Role::Submit)]
    );
    let password_only = LogInSpec::parse(&json!({ "password": 13, "submit": 16 })).expect("read");
    assert_eq!(password_only.id, None);
    assert!(LogInSpec::parse(&json!({ "id": 12, "password": 12, "submit": 16 })).is_err());
    assert!(LogInSpec::parse(&json!({ "id": 12, "submit": 16 })).is_err());
}

#[test]
fn a_value_longer_than_a_field_takes_is_refused_by_name() {
    let spec = FillSpec::parse(&coupang()).expect("read");

    let refused = spec
        .too_long(&[(12, String::from("me")), (13, "x".repeat(MAX_VALUE + 1))])
        .expect("refused");

    assert!(refused.contains("'비밀번호'"), "{refused}");
    assert!(!refused.contains("xxxx"), "{refused}");
    assert!(spec.too_long(&[(13, "x".repeat(MAX_VALUE))]).is_none());
}

#[test]
fn a_card_is_shown_only_on_an_https_page_with_a_host() {
    assert!(keepable("https://www.coupang.com"));
    assert!(!keepable("http://www.coupang.com"));
    assert!(!keepable("null"));
    assert!(!keepable("https://"));
}
