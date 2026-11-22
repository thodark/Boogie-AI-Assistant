use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{DEFAULT_API_VERSION, DEFAULT_LIVE_MODEL};
use crate::util::truncate_chars;

#[derive(Debug)]
pub enum GeminiEvent {
    SetupComplete,
    Audio(Vec<u8>),
    OutputTranscript(String),
    InputTranscript(String),
    TurnComplete,
    Interrupted,
    ToolCall(Vec<FunctionCall>),
    SessionResumption { handle: String, resumable: bool },
    GoAway { time_left: Option<String> },
    Closed(String),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_args")]
    pub args: Value,
}

fn default_args() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    pub id: String,
    pub name: String,
    pub response: Value,
}

pub struct LiveSession {
    ws_tx: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    pub event_rx: mpsc::Receiver<GeminiEvent>,
}

#[derive(Clone)]
pub struct LiveConfig {
    pub api_key: String,
    pub model: String,
    pub api_version: String,
    pub system_instruction: String,
    pub voice: String,
    pub tools: Vec<Value>,
    pub resume_handle: Option<String>,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: DEFAULT_LIVE_MODEL.to_string(),
            api_version: DEFAULT_API_VERSION.to_string(),
            system_instruction: String::new(),
            voice: "Aoede".to_string(),
            tools: Vec::new(),
            resume_handle: None,
        }
    }
}

impl LiveSession {
    pub async fn connect(config: LiveConfig) -> Result<Self, String> {
        let url = format!(
            "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.{}.GenerativeService.BidiGenerateContent?key={}",
            config.api_version, config.api_key,
        );

        tracing::debug!("ws endpoint: {}", url.split('?').next().unwrap_or("?"));

        let connector = tokio_tungstenite::Connector::NativeTls(
            native_tls::TlsConnector::new().map_err(|e| format!("TLS init: {e}"))?,
        );

        let connect_fut =
            tokio_tungstenite::connect_async_tls_with_config(&url, None, false, Some(connector));

        let (ws_stream, resp) =
            tokio::time::timeout(std::time::Duration::from_secs(15), connect_fut)
                .await
                .map_err(|_| "WebSocket connect timed out after 15s".to_string())?
                .map_err(|e| format!("WebSocket connect failed: {e}"))?;

        tracing::info!("WebSocket connected, status={}", resp.status());

        let (ws_tx, mut ws_rx) = ws_stream.split();
        let (event_tx, event_rx) = mpsc::channel(256);

        tokio::spawn(async move {
            tracing::debug!("ws reader loop started");
            while let Some(msg) = ws_rx.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Err(e) = Self::parse_message(&text, &event_tx).await {
                            tracing::error!("parse error: {e}");
                            let _ = event_tx.send(GeminiEvent::Error(e)).await;
                        }
                    }
                    Ok(Message::Binary(data)) => {
                        if let Ok(text) = std::str::from_utf8(&data) {
                            if let Err(e) = Self::parse_message(text, &event_tx).await {
                                tracing::error!("parse error: {e}");
                                let _ = event_tx.send(GeminiEvent::Error(e)).await;
                            }
                        } else {
                            let _ = event_tx.send(GeminiEvent::Audio(data.to_vec())).await;
                        }
                    }
                    Ok(Message::Close(frame)) => {
                        let reason = frame
                            .map(|f| format!("code={} reason={}", f.code, f.reason))
                            .unwrap_or_else(|| "no close frame".to_string());
                        tracing::info!("ws closed: {reason}");
                        let _ = event_tx.send(GeminiEvent::Closed(reason)).await;
                        break;
                    }
                    Err(e) => {
                        tracing::error!("ws read error: {e}");
                        let _ = event_tx
                            .send(GeminiEvent::Error(format!("ws error: {e}")))
                            .await;
                        break;
                    }
                    _ => {
                        tracing::trace!("ws control frame");
                    }
                }
            }
            tracing::debug!("ws reader loop ended");
        });

        Ok(Self { ws_tx, event_rx })
    }

    pub async fn parse_message(text: &str, tx: &mpsc::Sender<GeminiEvent>) -> Result<(), String> {
        use base64::Engine as _;

        tracing::trace!("ws recv: {}", truncate_chars(text, 200));

        let v: Value = serde_json::from_str(text).map_err(|e| format!("json parse: {e}"))?;

        if let Some(err) = v.pointer("/error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            tracing::error!("gemini error: {msg}");
            let _ = tx.send(GeminiEvent::Error(msg.to_string())).await;
            return Ok(());
        }

        if v.pointer("/setupComplete").is_some() {
            tracing::info!("Gemini setup complete");
            let _ = tx.send(GeminiEvent::SetupComplete).await;
            return Ok(());
        }

        if let Some(go_away) = v.pointer("/goAway") {
            let time_left = go_away
                .get("timeLeft")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());
            tracing::warn!("Gemini goAway received (timeLeft={:?})", time_left);
            let _ = tx.send(GeminiEvent::GoAway { time_left }).await;
        }

        if v.pointer("/serverContent/interrupted")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            tracing::debug!("Gemini serverContent.interrupted");
            let _ = tx.send(GeminiEvent::Interrupted).await;
        }

        if let Some(parts) = v
            .pointer("/serverContent/modelTurn/parts")
            .and_then(|p| p.as_array())
        {
            for part in parts {
                if let Some(b64) = part.pointer("/inlineData/data").and_then(|d| d.as_str())
                    && let Ok(pcm) = base64::engine::general_purpose::STANDARD.decode(b64)
                {
                    tracing::trace!("audio chunk: {} bytes", pcm.len());
                    let _ = tx.send(GeminiEvent::Audio(pcm)).await;
                }
            }
        }

        if let Some(data) = v.pointer("/serverContent/outputTranscription/text")
            && let Some(t) = data.as_str()
        {
            let _ = tx.send(GeminiEvent::OutputTranscript(t.to_string())).await;
        }

        if let Some(data) = v.pointer("/serverContent/inputTranscription/text")
            && let Some(t) = data.as_str()
        {
            let _ = tx.send(GeminiEvent::InputTranscript(t.to_string())).await;
        }

        if v.pointer("/serverContent/turnComplete")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            let _ = tx.send(GeminiEvent::TurnComplete).await;
        }

        if let Some(calls) = v
            .pointer("/toolCall/functionCalls")
            .and_then(|v| v.as_array())
        {
            tracing::info!("tool call received: {} function(s)", calls.len());
            let mut fcs = Vec::new();
            for c in calls {
                match serde_json::from_value::<FunctionCall>(c.clone()) {
                    Ok(fc) => fcs.push(fc),
                    Err(e) => {
                        tracing::error!("failed to parse function call: {e}");
                    }
                }
            }
            if !fcs.is_empty() {
                let _ = tx.send(GeminiEvent::ToolCall(fcs)).await;
            }
        }

        if let Some(sru) = v.pointer("/sessionResumptionUpdate")
            && let Some(handle) = sru.get("newHandle").and_then(|h| h.as_str())
        {
            let resumable = sru
                .get("resumable")
                .and_then(|r| r.as_bool())
                .unwrap_or(false);
            let _ = tx
                .send(GeminiEvent::SessionResumption {
                    handle: handle.to_string(),
                    resumable,
                })
                .await;
        }

        Ok(())
    }

    pub async fn send_setup(&mut self, config: &LiveConfig) -> Result<(), String> {
        let mut setup = serde_json::json!({
            "setup": {
                "model": config.model,
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "speechConfig": {
                        "voiceConfig": {
                            "prebuiltVoiceConfig": {
                                "voiceName": config.voice
                            }
                        }
                    }
                },
                "systemInstruction": {
                    "parts": [{"text": config.system_instruction}]
                },
                "tools": config.tools,
                "outputAudioTranscription": {},
                "inputAudioTranscription": {},
                "sessionResumption": {},
                "contextWindowCompression": {
                    "slidingWindow": {}
                }
            }
        });

        if let Some(handle) = &config.resume_handle {
            setup["setup"]["sessionResumption"] = serde_json::json!({
                "handle": handle
            });
        }

        tracing::info!(
            "sending setup (model={}, api={}, voice={}, resuming={})",
            config.model,
            config.api_version,
            config.voice,
            config.resume_handle.is_some()
        );
        tracing::trace!(
            "setup payload bytes={}",
            serde_json::to_string(&setup).map(|s| s.len()).unwrap_or(0)
        );
        self.send_json(&setup).await
    }

    pub async fn send_audio(&mut self, pcm_bytes: &[u8]) -> Result<(), String> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(pcm_bytes);
        let msg = serde_json::json!({
            "realtimeInput": {
                "audio": {
                    "data": b64,
                    "mimeType": "audio/pcm;rate=16000"
                }
            }
        });
        self.send_json(&msg).await
    }

    pub async fn send_audio_stream_end(&mut self) -> Result<(), String> {
        let msg = serde_json::json!({
            "realtimeInput": {
                "audioStreamEnd": true
            }
        });
        self.send_json(&msg).await
    }

    /// Sends a JPEG vision frame into Gemini Live `realtimeInput.video`.
    pub async fn send_image(&mut self, jpeg_bytes: &[u8]) -> Result<(), String> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_bytes);
        let msg = serde_json::json!({
            "realtimeInput": {
                "video": {
                    "data": b64,
                    "mimeType": "image/jpeg"
                }
            }
        });
        tracing::debug!(
            "sending realtime vision frame ({} bytes JPEG)",
            jpeg_bytes.len()
        );
        self.send_json(&msg).await
    }

    pub async fn send_text(&mut self, text: &str) -> Result<(), String> {
        let msg = serde_json::json!({
            "clientContent": {
                "turns": [{"role": "user", "parts": [{"text": text}]}],
                "turnComplete": true
            }
        });
        self.send_json(&msg).await
    }

    pub async fn send_tool_response(
        &mut self,
        responses: &[FunctionResponse],
    ) -> Result<(), String> {
        let msg = serde_json::json!({
            "toolResponse": {
                "functionResponses": responses
            }
        });
        let names: Vec<&str> = responses.iter().map(|r| r.name.as_str()).collect();
        tracing::debug!("sending tool response for: {:?}", names);
        self.send_json(&msg).await
    }

    pub async fn send_ping(&mut self) -> Result<(), String> {
        self.ws_tx
            .send(Message::Ping(Vec::new().into()))
            .await
            .map_err(|e| format!("ws ping: {e}"))
    }

    async fn send_json(&mut self, value: &Value) -> Result<(), String> {
        let text = serde_json::to_string(value).map_err(|e| format!("json encode: {e}"))?;
        self.ws_tx
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| format!("ws send: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_interrupted_goaway_and_vietnamese_transcript_without_panic() {
        let (tx, mut rx) = mpsc::channel(16);

        let vn_text = "Xin chào! ".repeat(40);
        let msg = serde_json::json!({
            "serverContent": {
                "interrupted": true,
                "outputTranscription": { "text": vn_text },
                "turnComplete": true
            }
        })
        .to_string();

        LiveSession::parse_message(&msg, &tx).await.unwrap();
        assert!(matches!(rx.recv().await, Some(GeminiEvent::Interrupted)));
        assert!(matches!(
            rx.recv().await,
            Some(GeminiEvent::OutputTranscript(_))
        ));
        assert!(matches!(rx.recv().await, Some(GeminiEvent::TurnComplete)));

        let go_away = r#"{"goAway": {"timeLeft": "30s"}}"#;
        LiveSession::parse_message(go_away, &tx).await.unwrap();
        match rx.recv().await {
            Some(GeminiEvent::GoAway { time_left }) => {
                assert_eq!(time_left.as_deref(), Some("30s"));
            }
            other => panic!("expected GoAway, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parses_setup_complete_and_tool_call() {
        let (tx, mut rx) = mpsc::channel(16);

        LiveSession::parse_message(r#"{"setupComplete": {}}"#, &tx)
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(GeminiEvent::SetupComplete)));

        let tc = r#"{"toolCall": {"functionCalls": [{"id": "call-1", "name": "system_status", "args": {}}]}}"#;
        LiveSession::parse_message(tc, &tx).await.unwrap();
        match rx.recv().await {
            Some(GeminiEvent::ToolCall(calls)) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "system_status");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }
}
