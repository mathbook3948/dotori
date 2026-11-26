//! This PC's place among the owner's PCs: its key, how the others reach it, and what the
//! windows call it.

use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;

use dotori_connectors::peer::Identity;
use dotori_connectors::peer::PORT;
use dotori_connectors::peer::hub::Heard;
use dotori_connectors::peer::hub::Hub;
use dotori_connectors::peer::hub::HubConfig;
use dotori_connectors::peer::wire::Hello;
use tokio::sync::mpsc;

use crate::device;
use crate::home::Home;
use crate::settings::Settings;

pub(crate) fn config(home: &Home, settings: &Settings) -> Result<HubConfig, String> {
    let listen = match &settings.peers {
        None => None,
        Some(peers) => {
            let text = peers
                .listen
                .clone()
                .unwrap_or_else(|| format!("0.0.0.0:{PORT}"));
            Some(
                text.parse::<SocketAddr>()
                    .map_err(|_| format!("[peers] listen is not an address and port: {text}"))?,
            )
        }
    };
    Ok(HubConfig {
        identity: Identity::load_or_make(&home.peer_key())?,
        device: device::name(&home.device())?,
        name: name(settings),
        address: settings
            .peers
            .as_ref()
            .and_then(|peers| peers.address.clone()),
        listen,
        book: home.peers(),
        invite: home.invite(),
        seen: home.peers_seen(),
    })
}

/// What this PC says about itself to the others.
pub(crate) fn hello(home: &Home, settings: &Settings) -> Result<Hello, String> {
    let config = config(home, settings)?;
    Ok(Hello {
        device: config.device,
        name: config.name,
        address: config.address,
    })
}

/// Links this PC with the others. Without a working setup the PC runs alone.
pub(crate) async fn start(
    home: &Home,
    settings: &Settings,
) -> Option<(Arc<Hub>, mpsc::UnboundedReceiver<Heard>)> {
    let started = match config(home, settings) {
        Ok(config) => Hub::start(config).await,
        Err(error) => Err(error),
    };
    started
        .inspect_err(|error| eprintln!("dotori: this PC runs alone: {error}"))
        .ok()
}

/// What the windows call this PC: the name the owner gave it, or else `unnamed`.
pub(crate) fn name(settings: &Settings) -> String {
    settings
        .pc
        .as_ref()
        .and_then(|pc| pc.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| unnamed(settings))
}

/// What this PC is called when the owner gave it no name: the name `[peers]` kept before `[pc]`,
/// or else the computer's own name.
pub(crate) fn unnamed(settings: &Settings) -> String {
    settings
        .peers
        .as_ref()
        .and_then(|peers| peers.name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(computer_name)
        .unwrap_or_else(|| String::from("이 PC"))
}

fn computer_name() -> Option<String> {
    let from = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|name| !name.trim().is_empty())
    };
    from("COMPUTERNAME")
        .or_else(|| from("HOSTNAME"))
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .filter(|name| !name.is_empty())
        })
}

#[cfg(test)]
#[path = "peers.test.rs"]
mod tests;
