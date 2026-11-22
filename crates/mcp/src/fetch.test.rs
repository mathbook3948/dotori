use std::net::IpAddr;

use super::*;

fn ip(text: &str) -> IpAddr {
    text.parse().expect("an address")
}

#[test]
fn only_public_addresses_are_public() {
    for private in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "::1",
        "fc00::1",
        "fe80::1",
        "::ffff:127.0.0.1",
        "::127.0.0.1",
        "64:ff9b::7f00:1",
        "2002:7f00:1::",
        "2001::1",
        "2001:db8::1",
        "fec0::1",
        "192.0.0.8",
    ] {
        assert!(!public(ip(private)), "{private}");
    }
    for open in [
        "1.1.1.1",
        "93.184.215.14",
        "2606:4700::1111",
        "2002:101:101::",
    ] {
        assert!(public(ip(open)), "{open}");
    }
}

#[test]
fn only_http_to_hosts_not_written_as_private_is_allowed() {
    let url = |text: &str| Url::parse(text).expect("a URL");
    assert!(allowed(&url("https://example.com/a")).is_ok());
    assert!(allowed(&url("http://1.1.1.1/")).is_ok());
    for refused in [
        "file:///etc/passwd",
        "ftp://example.com/",
        "http://127.0.0.1:8080/",
        "http://[::1]/",
        "http://localhost/",
        "http://localhost./",
        "http://app.localhost/",
        "http://192.168.0.1/admin",
    ] {
        assert!(allowed(&url(refused)).is_err(), "{refused}");
    }
}

#[test]
fn html_becomes_markdown_and_other_text_stays() {
    let html =
        b"<html><body><h1>Title</h1><p>See <a href=\"https://x.io\">this</a>.</p></body></html>";
    let page = read("text/html; charset=utf-8", html, false).expect("read");
    assert!(page.contains("# Title"), "{page}");
    assert!(page.contains("[this](https://x.io)"), "{page}");
    assert!(!page.contains("color") && !page.contains("x()"), "{page}");
    assert_eq!(
        read("text/html", html, true).expect("raw"),
        String::from_utf8_lossy(html)
    );
    assert_eq!(
        read("application/json", b"{\"a\":1}", false).expect("json"),
        "{\"a\":1}"
    );
    assert!(
        read("", b"<!DOCTYPE html><html><p>hi</p></html>", false)
            .expect("sniffed")
            .contains("hi")
    );
    assert!(read("image/png", b"\x89PNG", false).is_err());
}

#[test]
fn a_long_page_comes_in_parts() {
    let text = "가나다라마바사";
    assert_eq!(window(text, 0, 100), text);
    let first = window(text, 0, 3);
    assert!(first.starts_with("가나다"), "{first}");
    assert!(first.contains("start_index 3"), "{first}");
    assert_eq!(window(text, 3, 100), "라마바사");
    assert!(window(text, 50, 10).contains("7 characters"));
    assert_eq!(window(text, 1, usize::MAX), "나다라마바사");
}

#[tokio::test]
async fn a_local_address_is_refused_before_any_request() {
    let fetch = WebFetch::new().expect("built");
    let refused = fetch
        .call(json!({ "url": "http://127.0.0.1:9/" }))
        .await
        .expect_err("refused");
    assert!(refused.contains("not a public address"), "{refused}");
    let missing = fetch.call(json!({})).await.expect_err("needs a url");
    assert!(missing.contains("url"), "{missing}");
}
