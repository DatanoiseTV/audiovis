//! The axum server: static assets, the shared `.proto`, and the websocket
//! endpoint. Runs inside the web thread's tokio runtime.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::sync::broadcast;

use super::proto;
use super::{client_to_events, AppState};

/// Embedded browser assets (index.html, app.js, style.css, protobuf.min.js).
#[derive(rust_embed::RustEmbed)]
#[folder = "webui/"]
struct Assets;

/// Entry point for the web thread: build the runtime and serve until exit.
pub fn run(addr: String, state: AppState) {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("web runtime failed: {e}");
            return;
        }
    };
    rt.block_on(async move {
        let app = Router::new()
            .route("/ws", get(ws_upgrade))
            .route("/control.proto", get(proto_file))
            .fallback(get(static_handler))
            .with_state(state);

        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                tracing::info!("web control surface on http://{addr}");
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::error!("web server stopped: {e}");
                }
            }
            Err(e) => tracing::warn!("web disabled, cannot bind {addr}: {e}"),
        }
    });
}

/// Serve `proto/control.proto` so the browser can parse the schema at runtime.
async fn proto_file() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/plain")], include_str!("../../proto/control.proto"))
}

/// Serve an embedded asset, defaulting `/` to index.html.
async fn static_handler(uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".into();
    }
    match Assets::get(&path) {
        Some(file) => {
            let mime = file.metadata.mimetype();
            // No caching: during a live set the operator may reload after a
            // rebuild, and a stale cached app.js is a classic "stuck connecting".
            (
                [
                    (header::CONTENT_TYPE, mime.to_string()),
                    (header::CACHE_CONTROL, "no-store".to_string()),
                ],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// One client connection: send the full state, then pump deltas out and
/// commands in until either side closes.
async fn handle_socket(socket: WebSocket, state: AppState) {
    tracing::info!("web client connected");
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.out.subscribe();

    // Bring the client fully up to date in one message.
    if sink.send(Message::Binary(initial_state(&state).into())).await.is_err() {
        tracing::warn!("web client dropped during initial sync");
        return;
    }

    loop {
        tokio::select! {
            outbound = rx.recv() => match outbound {
                Ok(bytes) => {
                    if sink.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                // A slow client may miss deltas; it still has the snapshot and
                // will catch up on the next change, so just keep going.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            inbound = stream.next() => match inbound {
                Some(Ok(Message::Binary(b))) => {
                    if let Ok(msg) = proto::ClientMsg::decode(&b[..]) {
                        for ev in client_to_events(msg) {
                            let _ = state.bus_tx.send(ev);
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ignore text/ping/pong
                Some(Err(_)) => break,
            },
        }
    }
    tracing::info!("web client disconnected");
}

/// Encode the complete current state as a single `ServerMsg`.
fn initial_state(state: &AppState) -> Vec<u8> {
    let s = state.snapshot.read().unwrap();
    let msg = proto::ServerMsg {
        schema: s.schema.clone(),
        generators: s.generators.clone(),
        changes: s.values.values().cloned().collect(),
        telemetry: Some(s.telemetry.clone()),
        text: s.text.clone(),
        mod_sources: s.mod_sources.clone(),
        mod_routes: s.mod_routes.clone(),
        mod_routes_present: true,
        presets: s.presets.clone(),
        current_preset: s.current_preset.clone(),
        mappings: s.mappings.clone(),
        mappings_present: true,
    };
    msg.encode_to_vec()
}
