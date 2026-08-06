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
use axum::http::HeaderValue;
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

/// A live socket to the Discord client.
pub type DiscordSocket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// Dial the Discord client, presenting the `Origin` it allows.
///
/// This runs *before* the browser's websocket upgrade is accepted on purpose:
/// the overlay probes 6463-6472 to find a running Discord, and it must see a
/// plain connection failure on the ports where nothing is listening.
pub async fn connect(port: u16, query: Option<String>) -> Result<DiscordSocket, Error> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let url = match query {
        Some(query) => format!("ws://127.0.0.1:{port}/?{query}"),
        None => format!("ws://127.0.0.1:{port}/"),
    };

    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("Origin", HeaderValue::from_static(RPC_ORIGIN));

    let (socket, _) = tokio_tungstenite::connect_async(request).await?;
    tracing::info!(port, "bridged the Discord RPC websocket");
    Ok(socket)
}

pub type Error = tokio_tungstenite::tungstenite::Error;

/// Pump messages between the two sockets until either side hangs up.
pub async fn pump(browser: WebSocket, discord: DiscordSocket) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    /// Stand-in for the Discord client: records the handshake `Origin`, then
    /// echoes one message back.
    async fn fake_discord(listener: TcpListener, seen_origin: oneshot::Sender<Option<String>>) {
        let (stream, _) = listener.accept().await.unwrap();
        let mut origin = None;
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, |req: &_, res| {
            let req: &tokio_tungstenite::tungstenite::handshake::server::Request = req;
            origin = req
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            Ok(res)
        })
        .await
        .unwrap();

        let _ = seen_origin.send(origin);
        let received = socket.next().await.unwrap().unwrap();
        socket.send(received).await.unwrap();
    }

    #[tokio::test]
    async fn relays_frames_and_presents_the_origin_discord_expects() {
        // Bind the fake RPC socket on a real Discord RPC port; skip when the
        // desktop client (or anything else) already holds it.
        let Ok(discord) = TcpListener::bind("127.0.0.1:6472").await else {
            eprintln!("port 6472 busy — skipping RPC relay test");
            return;
        };
        let (origin_tx, origin_rx) = oneshot::channel();
        tokio::spawn(fake_discord(discord, origin_tx));

        // Serve just the bridge route.
        let bridge = axum::Router::new().route(
            "/rpc/{port}/",
            axum::routing::get(
                |ws: axum::extract::ws::WebSocketUpgrade,
                 axum::extract::Path(port): axum::extract::Path<u16>,
                 axum::extract::RawQuery(query): axum::extract::RawQuery| async move {
                    let discord = connect(port, query).await.unwrap();
                    ws.on_upgrade(move |socket| pump(socket, discord))
                },
            ),
        );
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(proxy, bridge).await.unwrap() });

        let (mut client, _) = tokio_tungstenite::connect_async(format!(
            "ws://{proxy_addr}/rpc/6472/?v=1&client_id=123"
        ))
        .await
        .unwrap();

        client
            .send(UpstreamMessage::Text("hello".into()))
            .await
            .unwrap();
        let echoed = client.next().await.unwrap().unwrap();

        assert_eq!(echoed, UpstreamMessage::Text("hello".into()));
        assert_eq!(origin_rx.await.unwrap().as_deref(), Some(RPC_ORIGIN));
    }
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
