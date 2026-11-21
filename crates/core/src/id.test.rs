use super::*;

#[test]
fn channel_id_accepts_lowercase_digits_and_hyphens() {
    assert_eq!(
        ChannelId::parse("slack-2").map(|id| id.as_str().to_owned()),
        Ok(String::from("slack-2"))
    );
}

#[test]
fn channel_id_rejects_empty_and_other_characters() {
    assert_eq!(ChannelId::parse(""), Err(ChannelIdError::Empty));
    assert_eq!(ChannelId::parse("Slack"), Err(ChannelIdError::Invalid('S')));
    assert_eq!(
        ChannelId::parse("my app"),
        Err(ChannelIdError::Invalid(' '))
    );
}
