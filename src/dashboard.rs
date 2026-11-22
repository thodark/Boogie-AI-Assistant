use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::AppEvent;

pub fn generate_token() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 24];
    let _ = rng.fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn verify_token(expected: &str, provided: &str) -> bool {
    if expected.is_empty() || expected.len() != provided.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&a, &b) in expected.as_bytes().iter().zip(provided.as_bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Only allow non-browser clients (no Origin header) or local origins on loopback.
pub fn is_allowed_origin(origin: Option<&str>) -> bool {
    let Some(orig) = origin else {
        return true;
    };
    let lower = orig.trim().to_lowercase();
    if let Ok(parsed) = url::Url::parse(&lower) {
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return false;
        }
        matches!(
            parsed.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("[::1]")
        )
    } else {
        false
    }
}

pub async fn run_dashboard(
    event_rx: broadcast::Sender<AppEvent>,
    port: u16,
) -> Result<String, String> {
    let token = generate_token();
    let state = Arc::new(DashState {
        event_tx: event_rx,
        auth_token: token.clone(),
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("dashboard bind {addr}: {e}"))?;

    tracing::info!("dashboard listening on http://{addr} (auth token required for /ws)");
    tracing::info!("dashboard WS token: {token}");

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("dashboard serve: {e}"))?;

    Ok(token)
}

#[derive(Clone)]
struct DashState {
    event_tx: broadcast::Sender<AppEvent>,
    auth_token: String,
}

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn health() -> &'static str {
    "ok"
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<DashState>>,
) -> impl IntoResponse {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !is_allowed_origin(origin) {
        tracing::warn!("dashboard WS rejected disallowed Origin");
        return (StatusCode::FORBIDDEN, "Disallowed Origin").into_response();
    }

    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let provided = bearer.or(query.token.as_deref()).unwrap_or("");
    if !verify_token(&state.auth_token, provided) {
        tracing::warn!("dashboard WS rejected invalid or missing auth token");
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(move |socket| handle_ws(socket, state))
        .into_response()
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
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::debug!("dashboard ws client lagged, skipped {skipped} events");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_validation_blocks_external_sites() {
        assert!(is_allowed_origin(None));
        assert!(is_allowed_origin(Some("http://127.0.0.1:8765")));
        assert!(is_allowed_origin(Some("http://localhost:3000")));
        assert!(!is_allowed_origin(Some("https://evil.example.com")));
        assert!(!is_allowed_origin(Some("http://127.0.0.1.evil.com")));
        assert!(!is_allowed_origin(Some("null")));
    }

    #[test]
    fn token_verification_works_constant_time() {
        let tok = generate_token();
        assert_eq!(tok.len(), 48);
        assert!(verify_token(&tok, &tok));
        assert!(!verify_token(&tok, "wrong"));
        assert!(!verify_token(&tok, ""));
    }
}
