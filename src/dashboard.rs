use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::AppEvent;

pub async fn run_dashboard(
    event_rx: broadcast::Sender<AppEvent>,
    port: u16,
) -> Result<(), String> {
    let state = Arc::new(DashState {
        event_tx: event_rx,
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("dashboard bind {addr}: {e}"))?;

    tracing::info!("dashboard listening on {addr}");

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("dashboard serve: {e}"))
}

#[derive(Clone)]
struct DashState {
    event_tx: broadcast::Sender<AppEvent>,
}

async fn health() -> &'static str {
    "ok"
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<DashState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<DashState>) {
    let mut rx = state.event_tx.subscribe();

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(evt) => {
                        let json = match &evt {
                            AppEvent::StateChange(s) => {
                                serde_json::json!({"type": "state", "value": format!("{s:?}")})
                            }
                            AppEvent::AudioLevel(l) => {
                                serde_json::json!({"type": "level", "value": l})
                            }
                            AppEvent::Log(msg) => {
                                serde_json::json!({"type": "log", "value": msg})
                            }
                            _ => continue,
                        };
                        if socket
                            .send(Message::Text(json.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
