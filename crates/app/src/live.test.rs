use std::sync::Mutex;

use dotori_core::llm::ModelReply;

use super::*;

struct Named(&'static str, Mutex<Vec<&'static str>>);

impl Model for Named {
    fn complete(&self, _: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        self.1.lock().expect("unpoisoned").push(self.0);
        Box::pin(async {
            Err(ModelError {
                detail: String::from("only the call is counted"),
            })
        })
    }

    fn summarize(&self, _: SummaryRequest) -> BoxFuture<'_, Result<String, ModelError>> {
        let name = self.0;
        Box::pin(async move { Ok(String::from(name)) })
    }
}

fn request() -> ModelRequest {
    ModelRequest {
        conversation: Vec::new(),
        tools: Vec::new(),
        context: None,
        time_zone: None,
    }
}

fn summary_request() -> SummaryRequest {
    SummaryRequest {
        previous: None,
        conversation: Vec::new(),
        time_zone: None,
    }
}

#[tokio::test]
async fn a_model_the_settings_cannot_build_fails_each_request_with_why() {
    let model = Unbuilt(String::from("set ANTHROPIC_API_KEY"));

    let failed = model.complete(request()).await.expect_err("no model");

    assert_eq!(failed.detail, "set ANTHROPIC_API_KEY");
}

#[tokio::test]
async fn a_model_the_settings_cannot_build_fails_a_summary_with_why_too() {
    let model = Unbuilt(String::from("set ANTHROPIC_API_KEY"));

    let failed = model
        .summarize(summary_request())
        .await
        .expect_err("no model");

    assert_eq!(failed.detail, "set ANTHROPIC_API_KEY");
}

#[tokio::test]
async fn the_next_summary_is_asked_of_the_model_swapped_in() {
    let first = Arc::new(Named("first", Mutex::new(Vec::new())));
    let second = Arc::new(Named("second", Mutex::new(Vec::new())));
    let live = LiveModel::new(Arc::clone(&first) as Arc<dyn Model>);

    let before = live.summarize(summary_request()).await.expect("summary");
    live.swap(Arc::clone(&second) as Arc<dyn Model>);
    let after = live.summarize(summary_request()).await.expect("summary");

    assert_eq!((before.as_str(), after.as_str()), ("first", "second"));
}

#[tokio::test]
async fn the_next_request_goes_to_the_model_swapped_in() {
    let first = Arc::new(Named("first", Mutex::new(Vec::new())));
    let second = Arc::new(Named("second", Mutex::new(Vec::new())));
    let live = LiveModel::new(Arc::clone(&first) as Arc<dyn Model>);

    let _ = live.complete(request()).await;
    live.swap(Arc::clone(&second) as Arc<dyn Model>);
    let _ = live.complete(request()).await;

    assert_eq!(*first.1.lock().expect("unpoisoned"), ["first"]);
    assert_eq!(*second.1.lock().expect("unpoisoned"), ["second"]);
}

#[test]
fn turning_a_channel_on_changes_its_fingerprint_alone_and_the_model_changes_none() {
    let root = std::env::temp_dir().join(format!("dotori-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a home");
    let vault = Vault::new(&root);
    let parse = |text: &str| toml::from_str::<Settings>(text).expect("settings");

    let alone = channels_of(&parse(""), &vault);
    let other_model = channels_of(
        &parse("[model]\nprovider = \"anthropic\"\nmodel = \"claude-opus-5-5\"\n"),
        &vault,
    );
    let telegram = channels_of(&parse("[telegram]\nowner = 42\n"), &vault);

    assert_eq!(alone, other_model);
    let differ: Vec<&str> = alone
        .iter()
        .zip(&telegram)
        .filter(|(before, after)| before != after)
        .map(|(before, _)| before.0)
        .collect();
    assert_eq!(differ, [TELEGRAM]);
    let _ = std::fs::remove_dir_all(&root);
}
