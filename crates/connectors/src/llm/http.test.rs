use super::*;

#[test]
fn an_error_body_is_read_for_its_type_and_message() {
    let body = r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#;

    let detail = status_detail("anthropic", StatusCode::TOO_MANY_REQUESTS, body);

    assert_eq!(
        detail,
        "anthropic answered 429 Too Many Requests: rate_limit_error slow down"
    );
}

#[test]
fn an_unreadable_error_body_is_shown_cut_short() {
    let body = "x".repeat(10_000);

    let detail = status_detail("openai-compatible", StatusCode::BAD_GATEWAY, &body);

    assert!(detail.starts_with("openai-compatible answered 502 Bad Gateway: xxx"));
    assert!(detail.len() < 400);
}

#[test]
fn only_passing_failures_are_tried_again() {
    for code in [408, 409, 429, 500, 503, 529] {
        assert!(
            passing(StatusCode::from_u16(code).expect("status")),
            "{code}"
        );
    }
    for code in [400, 401, 403, 404, 413, 422] {
        assert!(
            !passing(StatusCode::from_u16(code).expect("status")),
            "{code}"
        );
    }
}
