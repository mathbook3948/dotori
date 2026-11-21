use super::*;

#[test]
fn content_keeps_the_text_as_given() {
    assert_eq!(
        Content::parse(" hi ").map(|content| content.as_str().to_owned()),
        Ok(String::from(" hi "))
    );
}

#[test]
fn blank_content_is_rejected() {
    assert_eq!(Content::parse(""), Err(EmptyContentError));
    assert_eq!(Content::parse(" \n\t"), Err(EmptyContentError));
}
