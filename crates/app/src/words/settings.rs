use crate::window::identity;
use crate::window::identity::Particle;

/// Strings of the settings window, its notes card, and the checks it makes before saving.
#[derive(Debug)]
pub(crate) struct Settings {
    pub(crate) key_needed: &'static str,
    /// The section of the name, the picture, the theme and the language.
    pub(crate) general: &'static str,
    /// The section of the sandbox and the vault.
    pub(crate) safety: &'static str,
    pub(crate) model_title: &'static str,
    /// The caption under the model card's title, which calls the assistant by its name.
    pub(crate) model_about: fn() -> String,
    pub(crate) openai_compatible: &'static str,
    pub(crate) model: &'static str,
    /// What the model picker shows before a model is chosen.
    pub(crate) choose_model: &'static str,
    /// The mark next to the choice a setting left unset takes.
    pub(crate) default_pick: &'static str,
    /// Beside the way back to the chat, while a card there waits on the owner.
    pub(crate) chat_waits: &'static str,
    /// The line in a searched list when nothing matches.
    pub(crate) no_match: &'static str,
    /// What the search field in a long list shows before anything is typed.
    pub(crate) search: &'static str,
    pub(crate) zone_title: &'static str,
    pub(crate) zone_about: &'static str,
    pub(crate) models_need_address: &'static str,
    pub(crate) models_asking: &'static str,
    pub(crate) models_none: &'static str,
    pub(crate) models_failed: &'static str,
    pub(crate) base_url: &'static str,
    pub(crate) api_key: &'static str,
    pub(crate) effort: &'static str,
    pub(crate) effort_hint: &'static str,
    /// The thinking efforts, from `low` to `max`.
    pub(crate) efforts: [&'static str; 5],
    pub(crate) key_on_save: &'static str,
    pub(crate) key_from_environment: &'static str,
    pub(crate) key_kept: &'static str,
    pub(crate) key_missing: &'static str,
    pub(crate) remove: &'static str,
    pub(crate) vault_mismatch_keep: &'static str,
    pub(crate) vault_failed_keep: &'static str,
    pub(crate) vault_mismatch_remove: &'static str,
    pub(crate) vault_failed_remove: &'static str,
    pub(crate) identity_title: &'static str,
    pub(crate) identity_about: &'static str,
    pub(crate) use_picture: &'static str,
    pub(crate) other_picture: &'static str,
    pub(crate) default_picture: &'static str,
    pub(crate) no_picker: &'static str,
    pub(crate) picture_too_large: &'static str,
    pub(crate) picture_unreadable: &'static str,
    pub(crate) picture_not_saved: &'static str,
    pub(crate) name: &'static str,
    /// The refusal of a name longer than the given number of characters.
    pub(crate) name_too_long: fn(usize) -> String,
    pub(crate) name_control: &'static str,
    pub(crate) theme: &'static str,
    pub(crate) theme_overridden: &'static str,
    /// The caption of the Slack card, which calls the assistant by its name.
    pub(crate) slack_about: fn() -> String,
    pub(crate) slack_steps: [&'static str; 3],
    pub(crate) app_token: &'static str,
    pub(crate) bot_token: &'static str,
    pub(crate) slack_owner: &'static str,
    pub(crate) slack_owner_hint: &'static str,
    pub(crate) slack_owner_shape: &'static str,
    pub(crate) telegram_about: fn() -> String,
    pub(crate) telegram_steps: [&'static str; 3],
    pub(crate) telegram_owner: &'static str,
    pub(crate) telegram_owner_hint: &'static str,
    pub(crate) telegram_owner_shape: &'static str,
    pub(crate) discord_about: fn() -> String,
    pub(crate) discord_steps: [&'static str; 3],
    pub(crate) discord_owner: &'static str,
    pub(crate) discord_owner_hint: &'static str,
    pub(crate) discord_owner_shape: &'static str,
    /// The refusal of a token pasted where the name of its environment variable goes.
    pub(crate) not_a_key_name: fn(&str) -> String,
    pub(crate) openai_needs_address: &'static str,
    pub(crate) notes_title: &'static str,
    /// The caption of the notes card, given how many notes there are and how many fit.
    pub(crate) notes_about: fn(usize, i64) -> String,
    pub(crate) no_notes: &'static str,
    pub(crate) notes_example: &'static str,
    /// What a channel turned on still needs, given the channel and the missing fields in order.
    pub(crate) channel_missing: fn(&str, &[&str]) -> String,
    pub(crate) missing_app_token: &'static str,
    pub(crate) missing_bot_token: &'static str,
    pub(crate) missing_member_id: &'static str,
    pub(crate) missing_user_id: &'static str,
    /// The refusal of a key whose name is where a linked site's account is kept.
    pub(crate) name_is_an_account: fn(&str) -> String,
    pub(crate) vault_title: &'static str,
    pub(crate) vault_about: &'static str,
    pub(crate) keys_group: &'static str,
    pub(crate) sites_group: &'static str,
    pub(crate) no_keys: &'static str,
    pub(crate) no_sites: &'static str,
    pub(crate) anthropic_key: &'static str,
    pub(crate) slack_app_key: &'static str,
    pub(crate) slack_bot_key: &'static str,
    pub(crate) telegram_key: &'static str,
    pub(crate) discord_key: &'static str,
    pub(crate) other_use: &'static str,
    /// The line under a key read from an environment variable, given where the key is used.
    pub(crate) read_from_environment: fn(&str) -> String,
    pub(crate) no_id: &'static str,
    pub(crate) trusted: &'static str,
}

pub(crate) const KOREAN: Settings = Settings {
    key_needed: "키를 붙여 넣어야 다음으로 넘어갈 수 있어요.",
    general: "일반",
    safety: "안전",
    model_title: "대화 모델",
    model_about: || {
        format!(
            "{} 생각하고 답할 때 쓰는 모델이에요.",
            identity::called(Particle::Subject)
        )
    },
    openai_compatible: "OpenAI 호환",
    model: "모델",
    choose_model: "모델을 골라 주세요",
    default_pick: "기본",
    no_match: "맞는 항목이 없어요",
    chat_waits: "답을 기다려요",
    search: "찾기",
    zone_title: "시간대",
    zone_about: "루틴의 시간을 이 시간대로 읽어요.",
    models_need_address: "주소를 넣으면 서버의 모델을 불러와요.",
    models_asking: "서버에서 모델을 불러오고 있어요.",
    models_none: "서버에 쓸 수 있는 모델이 없어요.",
    models_failed: "서버에서 모델을 불러오지 못했어요. 주소와 서버가 켜져 있는지 확인해 주세요.",
    base_url: "API 주소",
    api_key: "API 키",
    effort: "생각하는 정도",
    effort_hint: "높을수록 더 깊이 생각하지만 답이 느려지고 비용이 늘어요.",
    efforts: ["낮음", "보통", "높음", "아주 높음", "최대"],
    key_on_save: "붙여 넣으면 암호화되어 보관돼요",
    key_from_environment: "이 PC의 환경 변수에 있는 키를 써요",
    key_kept: "키가 암호화되어 보관돼 있어요",
    key_missing: "아직 넣지 않았어요",
    remove: "지우기",
    vault_mismatch_keep: "이 PC의 금고 키가 금고와 맞지 않아서 키를 넣지 못했어요",
    vault_failed_keep: "키를 금고에 넣지 못했어요. 잠시 뒤에 다시 해 보세요",
    vault_mismatch_remove: "이 PC의 금고 키가 금고와 맞지 않아서 키를 지우지 못했어요",
    vault_failed_remove: "키를 금고에서 지우지 못했어요. 잠시 뒤에 다시 해 보세요",
    identity_title: "이름과 그림",
    identity_about: "채팅창과 답에서 이 이름과 그림으로 보여요.",
    use_picture: "이 그림 쓰기",
    other_picture: "다른 그림 고르기",
    default_picture: "기본 그림으로 되돌리기",
    no_picker: "그림을 고르는 창을 열지 못했어요",
    picture_too_large: "20MB보다 작은 그림을 골라 주세요",
    picture_unreadable: "그림으로 읽지 못했어요. PNG, JPEG, GIF, WebP 그림을 골라 주세요",
    picture_not_saved: "그림을 저장하지 못했어요. 잠시 뒤에 다시 해 보세요",
    name: "이름",
    name_too_long: |longest| format!("이름은 {longest}자까지 쓸 수 있어요"),
    name_control: "이름에는 글자만 써 주세요",
    theme: "테마",
    theme_overridden: "DOTORI_THEME이 정해져 있어서 창은 그 테마로 그려요. 고른 것은 저장돼요.",
    slack_about: || channel_about_ko("Slack", "채널"),
    slack_steps: [
        "api.slack.com/apps에서 앱을 만들고 Socket Mode와 Interactivity를 켜 주세요. connections:write 권한으로 앱 토큰(xapp-)을 만들어 주세요.",
        "봇 권한 app_mentions:read, chat:write, im:history, im:read, im:write, users:read, channels:read, groups:read를 주고, app_mention과 message.im 이벤트를 구독한 뒤 설치해서 봇 토큰(xoxb-)을 받아 주세요.",
        "두 토큰을 아래에 붙여 넣어 주세요.",
    ],
    app_token: "앱 토큰",
    bot_token: "봇 토큰",
    slack_owner: "내 Slack 멤버 ID",
    slack_owner_hint: "Slack에서 내 프로필을 열고 더보기 메뉴의 \"멤버 ID 복사\"로 얻어요. 이 사람의 말만 들어요.",
    slack_owner_shape: "Slack 멤버 ID는 U012AB3CD처럼 U나 W로 시작하는 대문자와 숫자예요",
    telegram_about: || channel_about_ko("Telegram", "그룹"),
    telegram_steps: [
        "Telegram에서 @BotFather에게 /newbot을 보내 봇을 만들고 토큰을 받아 주세요.",
        "토큰을 아래에 붙여 넣어 주세요.",
        "그룹에서도 쓰려면 봇을 그룹에 넣고 @봇이름으로 불러 주세요.",
    ],
    telegram_owner: "내 Telegram 사용자 ID",
    telegram_owner_hint: "@userinfobot에게 아무 말이나 보내면 알려 줘요. 이 사람의 말만 들어요.",
    telegram_owner_shape: "Telegram 사용자 ID는 123456789처럼 숫자예요",
    discord_about: || channel_about_ko("Discord", "서버"),
    discord_steps: [
        "discord.com/developers/applications에서 앱을 만들고 Bot 메뉴에서 토큰을 받아 주세요. 같은 화면에서 Message Content Intent를 켜 주세요.",
        "OAuth2 URL Generator에서 bot 범위와 View Channels, Send Messages, Read Message History 권한을 골라 내 서버에 봇을 초대해 주세요.",
        "토큰을 아래에 붙여 넣어 주세요.",
    ],
    discord_owner: "내 Discord 사용자 ID",
    discord_owner_hint: "설정의 고급에서 개발자 모드를 켜고, 내 프로필을 눌러 \"사용자 ID 복사\"로 얻어요. 이 사람의 말만 들어요.",
    discord_owner_shape: "Discord 사용자 ID는 123456789012345678처럼 긴 숫자예요",
    not_a_key_name: |name| {
        format!("`{name}`은 키 이름이 아니에요. 토큰은 설정창의 칸에 붙여 넣어 주세요")
    },
    openai_needs_address: "주소를 넣고 모델을 골라 주세요",
    notes_title: "메모",
    notes_about: |count, most| {
        format!(
            "{} 답할 때마다 기억하는 말이에요. 채팅창에서 부탁하면 더해지고, 여기서는 지울 수 있어요. ({count}/{most})",
            identity::called(Particle::Subject)
        )
    },
    no_notes: "아직 메모가 없어요",
    notes_example: "채팅창에서 \"앞으로 존댓말 써 줘\"처럼 부탁해도 돼요",
    channel_missing: |channel, missing| {
        let last = missing.last().copied().unwrap_or_default();
        format!(
            "{channel}{} 쓰려면 {}{} 넣어 주세요.",
            object(channel),
            missing.join(", "),
            object(last)
        )
    },
    missing_app_token: "앱 토큰",
    missing_bot_token: "봇 토큰",
    missing_member_id: "내 멤버 ID",
    missing_user_id: "내 사용자 ID",
    name_is_an_account: |name| {
        format!(
            "`{name}` 이름은 연동한 사이트의 계정 자리라서 키를 넣을 수 없어요. 설정에서 키 이름을 바꿔 주세요"
        )
    },
    vault_title: "금고",
    vault_about: "API 키, 토큰, 연동한 사이트의 계정은 암호화되어 보관돼요. 대화하는 AI는 사이트와 아이디까지만 알고, 비밀번호는 보지 못해요.",
    keys_group: "API 키와 토큰",
    sites_group: "연동된 사이트",
    no_keys: "아직 보관한 키가 없어요",
    no_sites: "아직 연동한 사이트가 없어요",
    anthropic_key: "Anthropic API 키",
    slack_app_key: "Slack 앱 토큰",
    slack_bot_key: "Slack 봇 토큰",
    telegram_key: "Telegram 봇 토큰",
    discord_key: "Discord 봇 토큰",
    other_use: "다른 연동",
    read_from_environment: |what| format!("{what}, 환경 변수에서 읽어요"),
    no_id: "아이디 없음",
    trusted: "묻지 않고 로그인",
};

pub(crate) const ENGLISH: Settings = Settings {
    key_needed: "Paste a key to continue.",
    general: "General",
    safety: "Safety",
    model_title: "Chat model",
    model_about: || format!("The model {} thinks and replies with.", identity::name()),
    openai_compatible: "OpenAI compatible",
    model: "Model",
    choose_model: "Choose a model",
    default_pick: "Default",
    no_match: "Nothing matches",
    chat_waits: "Needs you",
    search: "Search",
    zone_title: "Time zone",
    zone_about: "Routine times are read in this zone.",
    models_need_address: "Enter the address and I'll load the server's models.",
    models_asking: "Loading the server's models.",
    models_none: "The server has no models to use.",
    models_failed: "I couldn't load the server's models. Check the address and that it's running.",
    base_url: "API URL",
    api_key: "API key",
    effort: "Thinking effort",
    effort_hint: "Higher thinks deeper, but replies come slower and cost more.",
    efforts: ["Low", "Medium", "High", "Extra high", "Max"],
    key_on_save: "I'll encrypt and keep it once you paste it.",
    key_from_environment: "I'm using the key from an environment variable on this PC.",
    key_kept: "The key is kept encrypted.",
    key_missing: "You haven't added one yet.",
    remove: "Remove",
    vault_mismatch_keep: "This PC's vault key doesn't match the vault, so I couldn't keep the key.",
    vault_failed_keep: "I couldn't put the key in the vault. Try again in a moment.",
    vault_mismatch_remove: "This PC's vault key doesn't match the vault, so I couldn't remove the key.",
    vault_failed_remove: "I couldn't remove the key from the vault. Try again in a moment.",
    identity_title: "Name and picture",
    identity_about: "The chat and my replies show this name and picture.",
    use_picture: "Use this picture",
    other_picture: "Choose another picture",
    default_picture: "Use the default picture",
    no_picker: "I couldn't open the file picker.",
    picture_too_large: "Pick a picture smaller than 20 MB.",
    picture_unreadable: "I couldn't read that as a picture. Pick a PNG, JPEG, GIF or WebP file.",
    picture_not_saved: "I couldn't save the picture. Try again in a moment.",
    name: "Name",
    name_too_long: |longest| format!("A name can be up to {longest} characters."),
    name_control: "Remove line breaks and tabs from the name.",
    theme: "Theme",
    theme_overridden: "DOTORI_THEME is set, so windows use that theme. I still save what you pick.",
    slack_about: || channel_about_en("Slack", "channel"),
    slack_steps: [
        "Create an app at api.slack.com/apps and turn on Socket Mode and Interactivity. Make an app token (xapp-) with the connections:write scope.",
        "Give the bot the app_mentions:read, chat:write, im:history, im:read, im:write, users:read, channels:read and groups:read scopes, subscribe to the app_mention and message.im events, then install it to get a bot token (xoxb-).",
        "Paste both tokens below.",
    ],
    app_token: "App token",
    bot_token: "Bot token",
    slack_owner: "Your Slack member ID",
    slack_owner_hint: "Open your profile in Slack and choose \"Copy member ID\" from the more menu. I only listen to this person.",
    slack_owner_shape: "A Slack member ID starts with U or W and has only capital letters and digits, like U012AB3CD.",
    telegram_about: || channel_about_en("Telegram", "group"),
    telegram_steps: [
        "Send /newbot to @BotFather in Telegram to make a bot and get its token.",
        "Paste the token below.",
        "To use it in a group, add the bot to the group and mention it by its @name.",
    ],
    telegram_owner: "Your Telegram user ID",
    telegram_owner_hint: "Send anything to @userinfobot and it replies with your ID. I only listen to this person.",
    telegram_owner_shape: "A Telegram user ID is a number, like 123456789.",
    discord_about: || channel_about_en("Discord", "server"),
    discord_steps: [
        "Create an app at discord.com/developers/applications and get a token from the Bot menu. Turn on Message Content Intent on the same page.",
        "In OAuth2 URL Generator, pick the bot scope and the View Channels, Send Messages and Read Message History permissions, then invite the bot to your server.",
        "Paste the token below.",
    ],
    discord_owner: "Your Discord user ID",
    discord_owner_hint: "Turn on Developer Mode under Advanced in Discord's settings, then click your profile and choose \"Copy User ID\". I only listen to this person.",
    discord_owner_shape: "A Discord user ID is a long number, like 123456789012345678.",
    not_a_key_name: |name| {
        format!("`{name}` isn't a key name. Paste the token into its field in the settings.")
    },
    openai_needs_address: "Enter the address and choose a model.",
    notes_title: "Notes",
    notes_about: |count, most| {
        format!(
            "What {} keeps in mind for every reply. Ask in the chat to add one, or remove one here. ({count}/{most})",
            identity::name()
        )
    },
    no_notes: "No notes yet.",
    notes_example: "Try asking something like \"Keep your answers short.\"",
    channel_missing: |channel, missing| {
        let listed = match missing {
            [] => String::new(),
            [one] => (*one).to_owned(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        format!("To use {channel}, enter {listed}.")
    },
    missing_app_token: "the app token",
    missing_bot_token: "the bot token",
    missing_member_id: "your member ID",
    missing_user_id: "your user ID",
    name_is_an_account: |name| {
        format!(
            "`{name}` is where a linked site's account is kept, so a key can't go there. Change the key name in the settings."
        )
    },
    vault_title: "Vault",
    vault_about: "API keys, tokens and linked site accounts are kept encrypted. The AI model knows only the site and the username, never the password.",
    keys_group: "API keys and tokens",
    sites_group: "Linked sites",
    no_keys: "No keys kept yet.",
    no_sites: "No linked sites yet.",
    anthropic_key: "Anthropic API key",
    slack_app_key: "Slack app token",
    slack_bot_key: "Slack bot token",
    telegram_key: "Telegram bot token",
    discord_key: "Discord bot token",
    other_use: "Other connection",
    read_from_environment: |what| format!("{what}, from the environment"),
    no_id: "No username",
    trusted: "Logs in without asking",
};

fn object(word: &str) -> &'static str {
    if identity::ends_closed(word) {
        "을"
    } else {
        "를"
    }
}

fn channel_about_ko(channel: &str, place: &str) -> String {
    format!(
        "{channel}에서도 {} 말할 수 있어요. DM을 보내거나 {place}에서 부르면 같은 대화로 이어져요.",
        identity::called(Particle::With)
    )
}

fn channel_about_en(channel: &str, place: &str) -> String {
    format!(
        "You can talk to {} in {channel} too. Send a DM or mention the bot in a {place} to pick up the same conversation.",
        identity::name()
    )
}
