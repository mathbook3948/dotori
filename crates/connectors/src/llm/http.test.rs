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

async fn answering(status: &'static str) -> String {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bound");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accepted");
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).await;
        let reply =
            format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}");
        let _ = stream.write_all(reply.as_bytes()).await;
    });
    format!("http://{address}/v1/models")
}

#[tokio::test]
async fn a_check_fails_only_on_a_refused_key_or_an_address_that_does_not_answer() {
    let client = client().expect("client");

    assert_eq!(check(client.get(answering("200 OK").await)).await, Ok(()));
    assert_eq!(
        check(client.get(answering("401 Unauthorized").await)).await,
        Err(CheckError::KeyRefused)
    );
    assert_eq!(
        check(client.get(answering("403 Forbidden").await)).await,
        Err(CheckError::KeyRefused)
    );
    for passing in ["404 Not Found", "429 Too Many Requests", "529 Overloaded"] {
        assert_eq!(
            check(client.get(answering(passing).await)).await,
            Ok(()),
            "{passing}"
        );
    }
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bound")
        .local_addr()
        .expect("address");
    assert_eq!(
        check(client.get(format!("http://{closed}/v1/models"))).await,
        Err(CheckError::NoAnswer)
    );
}
