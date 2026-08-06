//! WebSocket relay to the local Discord client's RPC socket.
//!
//! Streamkit's bundle connects straight to `ws://127.0.0.1:<port>` (the Discord
//! desktop app, which listens on 6463-6472). That socket checks the handshake's
//! `Origin` header against Discord's own allowlist, so a browser page served
//! from `127.0.0.1:3000` is closed immediately with `4001 Invalid Origin`.
//!
//! We cannot change the `Origin` a browser sends, so the connection is relayed
//! here instead: the page opens a socket to us, and we dial Discord ourselves
//! with the `Origin` it expects.

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as UpstreamCloseFrame;
use tokio_tungstenite::tungstenite::Message as UpstreamMessage;

/// The origin the Discord client accepts for Streamkit overlays.
pub const RPC_ORIGIN: &str = "https://streamkit.discord.com";

/// Ports the Discord desktop client binds its RPC socket to.
const RPC_PORT_RANGE: std::ops::RangeInclusive<u16> = 6463..=6472;

pub fn is_rpc_port(port: u16) -> bool {
    RPC_PORT_RANGE.contains(&port)
}

/// Pump messages between the browser socket and the Discord RPC socket until
/// either side hangs up.
pub async fn relay(browser: WebSocket, port: u16, query: Option<String>) {
    let url = match query {
        Some(query) => format!("ws://127.0.0.1:{port}/?{query}"),
        None => format!("ws://127.0.0.1:{port}/"),
    };

    let request = match rpc_request(&url) {
        Ok(request) => request,
        Err(err) => {
            tracing::warn!(%url, %err, "could not build the Discord RPC handshake");
            return;
        }
    };

    let (discord, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        Err(err) => {
            // Expected while the page scans ports for a running Discord client.
            tracing::debug!(%url, %err, "Discord RPC socket not reachable");
            let mut browser = browser;
            let _ = browser.close().await;
            return;
        }
    };

    tracing::info!(port, "relaying Discord RPC websocket");
    pump(browser, discord).await;
}

/// Build the upstream handshake, replacing the browser's `Origin` with the one
/// the Discord client allows.
fn rpc_request(
    url: &str,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request, Box<dyn std::error::Error>> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("Origin", RPC_ORIGIN.parse().unwrap());
    Ok(request)
}

async fn pump(
    browser: WebSocket,
    discord: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let (mut browser_tx, mut browser_rx) = browser.split();
    let (mut discord_tx, mut discord_rx) = discord.split();

    loop {
        tokio::select! {
            incoming = browser_rx.next() => match incoming {
                Some(Ok(message)) => {
                    if discord_tx.send(to_upstream(message)).await.is_err() {
                        break;
                    }
                }
                _ => break,
            },
            incoming = discord_rx.next() => match incoming {
                Some(Ok(message)) => {
                    let Some(message) = to_browser(message) else { continue };
                    if browser_tx.send(message).await.is_err() {
                        break;
                    }
                }
                _ => break,
            },
        }
    }

    let _ = discord_tx.close().await;
    let _ = browser_tx.close().await;
}

fn to_upstream(message: Message) -> UpstreamMessage {
    match message {
        Message::Text(text) => UpstreamMessage::Text(text.as_str().into()),
        Message::Binary(data) => UpstreamMessage::Binary(data),
        Message::Ping(data) => UpstreamMessage::Ping(data),
        Message::Pong(data) => UpstreamMessage::Pong(data),
        Message::Close(frame) => UpstreamMessage::Close(frame.map(|frame| UpstreamCloseFrame {
            code: CloseCode::from(frame.code),
            reason: frame.reason.as_str().into(),
        })),
    }
}

/// `None` for frames tungstenite handles internally and axum cannot represent.
fn to_browser(message: UpstreamMessage) -> Option<Message> {
    Some(match message {
        UpstreamMessage::Text(text) => Message::Text(text.as_str().into()),
        UpstreamMessage::Binary(data) => Message::Binary(data),
        UpstreamMessage::Ping(data) => Message::Ping(data),
        UpstreamMessage::Pong(data) => Message::Pong(data),
        UpstreamMessage::Close(frame) => Message::Close(frame.map(|frame| CloseFrame {
            code: frame.code.into(),
            reason: frame.reason.as_str().into(),
        })),
        UpstreamMessage::Frame(_) => return None,
    })
}
