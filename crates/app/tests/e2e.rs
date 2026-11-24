mod support;

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use dotori_connectors::channel::app::FromWindow;
use dotori_connectors::channel::app::Hello;
use dotori_connectors::channel::app::ToWindow;
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::Lines;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::process::Child;
use tokio::process::Command;
use tokio::time;

use support::model;
use support::model::Script;

const WAIT: Duration = Duration::from_secs(120);
const KEY: &str = "DOTORI_E2E_KEY";
const OFFLINE: &str = "http://127.0.0.1:9";

struct Running {
    daemon: Child,
    home: PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.daemon.start_kill();
        if std::thread::panicking()
            && let Ok(log) = std::fs::read_to_string(self.home.join("daemon.log"))
        {
            eprintln!("daemon.log:\n{log}");
        }
        for _ in 0..50 {
            if std::fs::remove_dir_all(&self.home).is_ok() || !self.home.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

struct Window {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
}

impl Window {
    async fn open(home: &Path) -> Self {
        let door = home.join("door.json");
        let stream = time::timeout(WAIT, async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&door)
                    && let Ok(found) = serde_json::from_str::<Value>(&text)
                    && let Some(port) = found["port"].as_u64()
                    && let Ok(stream) = TcpStream::connect(("127.0.0.1", port as u16)).await
                {
                    let token = found["token"].as_str().unwrap_or_default().to_owned();
                    return (stream, token);
                }
                time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .expect("the daemon opens its door in time");
        let (stream, token) = stream;
        let (read, write) = stream.into_split();
        let mut window = Self {
            lines: BufReader::new(read).lines(),
            write,
        };
        window.send(&Hello { token }).await;
        window
    }

    async fn send(&mut self, message: &impl serde::Serialize) {
        let mut line = serde_json::to_string(message).expect("json");
        line.push('\n');
        self.write
            .write_all(line.as_bytes())
            .await
            .expect("sent to the daemon");
    }

    async fn say(&mut self, text: &str) {
        self.send(&FromWindow::Say {
            text: text.to_owned(),
        })
        .await;
    }

    async fn until<T>(&mut self, what: &str, mut pick: impl FnMut(&ToWindow) -> Option<T>) -> T {
        time::timeout(WAIT, async {
            loop {
                let line = self
                    .lines
                    .next_line()
                    .await
                    .expect("read from the daemon")
                    .expect("the daemon keeps the window open");
                if let Ok(event) = serde_json::from_str::<ToWindow>(&line)
                    && let Some(found) = pick(&event)
                {
                    return found;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{what} in time"))
    }

    async fn reply(&mut self, expected: &str) {
        self.until(&format!("the reply {expected:?}"), |event| match event {
            ToWindow::Reply { text, .. } if text == expected => Some(()),
            _ => None,
        })
        .await;
    }

    async fn asked_to_run(&mut self, command: &str) -> u64 {
        self.until("a request to run the command", |event| match event {
            ToWindow::Ask {
                id, tool, input, ..
            } if tool.ends_with("run_shell") && input.contains(command) => Some(*id),
            _ => None,
        })
        .await
    }

    async fn decide(&mut self, id: u64, ok: bool) {
        self.send(&FromWindow::Decide { id, ok }).await;
        self.until("the decision shown", |event| match event {
            ToWindow::Decided {
                id: decided,
                ok: shown,
            } if *decided == id && *shown == ok => Some(()),
            _ => None,
        })
        .await;
    }
}

fn home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("dotori-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("a home");
    home
}

fn settings(home: &Path, model: std::net::SocketAddr) {
    let text = format!(
        "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://{model}/v1\"\nmodel = \"fake\"\napi_key_env = \"{KEY}\"\n\n[browser]\npath = \"no-browser-in-e2e\"\n"
    );
    std::fs::write(home.join("settings.toml"), text).expect("settings");
}

fn daemon(home: &Path) -> Child {
    let log = std::fs::File::create(home.join("daemon.log")).expect("a log");
    Command::new(env!("CARGO_BIN_EXE_dotori"))
        .arg("daemon")
        .env("DOTORI_HOME", home)
        .env(KEY, "fake")
        .env("HTTPS_PROXY", OFFLINE)
        .env("HTTP_PROXY", OFFLINE)
        .env("ALL_PROXY", OFFLINE)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .stdin(Stdio::null())
        .stdout(log.try_clone().expect("a log"))
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .expect("the daemon starts")
}

fn routines_named(home: &Path, name: &str) -> i64 {
    let routines = rusqlite::Connection::open(home.join("routines.sqlite3")).expect("routines");
    routines
        .query_row(
            "SELECT count(*) FROM routines WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("counted")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_talks_to_dotori_and_decides_on_a_command() {
    let command = "echo e2e";
    let tools = std::sync::Arc::default();
    let model = model::start(Script {
        command: command.to_owned(),
        think: Duration::from_millis(200),
        tools: std::sync::Arc::clone(&tools),
    })
    .await;
    let home = home("decide");
    settings(&home, model);
    let _running = Running {
        daemon: daemon(&home),
        home: home.clone(),
    };
    let mut window = Window::open(&home).await;
    window
        .until("the backlog to end", |event| {
            matches!(event, ToWindow::Live).then_some(())
        })
        .await;

    window
        .say("내일 오전 10시 30분에 치과 예약 있어. 아침 8시에 알려 줘")
        .await;
    window
        .until("dotori at work", |event| {
            matches!(event, ToWindow::Working { busy: true }).then_some(())
        })
        .await;
    window.reply(model::REMINDED).await;
    let named: Vec<String> = tools.lock().expect("tools").clone();
    assert!(named.len() > 5, "dotori offers its tools: {named:?}");
    let misnamed: Vec<&String> = named
        .iter()
        .filter(|name| !dotori_core::tool::well_named(name))
        .collect();
    assert!(
        misnamed.is_empty(),
        "tools must be named verb_object: {misnamed:?}"
    );
    assert_eq!(routines_named(&home, "치과 알림"), 1);

    window.say("이번 주 일정 정리해 줘").await;
    window.reply(model::LISTED).await;

    window.say("확인 명령 실행해 줘").await;
    let allowed = window.asked_to_run(command).await;
    window.decide(allowed, true).await;
    window.reply(model::RAN).await;

    window.say("한 번 더 실행해 줘").await;
    let declined = window.asked_to_run(command).await;
    window.decide(declined, false).await;
    window.reply(model::SKIPPED).await;
}
