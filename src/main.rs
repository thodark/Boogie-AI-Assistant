#[allow(dead_code)]
mod actions;
#[allow(dead_code)]
mod audio;
#[allow(dead_code)]
mod config;
mod dashboard;
mod dsp;
#[allow(dead_code)]
mod echo;
#[allow(dead_code)]
mod gemini;
mod gui;
mod orb_shader;
mod proactive;
#[allow(dead_code)]
mod tools;

use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing_subscriber::EnvFilter;

const SEND_SAMPLE_RATE: u32 = 16_000;
const RECV_SAMPLE_RATE: u32 = 24_000;
const STALL_LIMIT_SECS: f64 = 30.0;

#[derive(Clone, Debug)]
pub enum AppEvent {
    StateChange(AppState),
    AudioLevel(f32),
    Log(String),
    Visemes(Vec<(f32, f32, f32)>, f64, f64),
    Reconnect { keep_context: bool, reason: String },
    ConfirmRequest { title: String, detail: String },
    Interrupt,
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum ConfirmResponse {
    Confirmed,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppState {
    Listening,
    Speaking,
    Thinking,
    Sleeping,
}

pub struct SharedState {
    pub is_speaking: bool,
    pub muted: bool,
    pub resume_handle: Option<String>,
    pub session_log: Vec<String>,
    pub last_user_speech: std::time::Instant,
}

impl SharedState {
    fn new() -> Self {
        Self {
            is_speaking: false,
            muted: false,
            resume_handle: None,
            session_log: Vec::new(),
            last_user_speech: std::time::Instant::now(),
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("boogie=debug")),
        )
        .init();

    let cfg = match config::ApiConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            eprintln!("place api_keys.json next to the binary under config/");
            std::process::exit(1);
        }
    };

    let shared = Arc::new(Mutex::new(SharedState::new()));
    let (event_tx, _) = broadcast::channel::<AppEvent>(256);
    let (mic_tx, mic_rx) = mpsc::channel::<Vec<u8>>(200);
    let (playback_tx, playback_rx) = mpsc::channel::<Vec<u8>>(200);
    let (confirm_tx, confirm_rx) = mpsc::channel::<ConfirmResponse>(4);
    let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>(4);

    let _mic_stream = match audio::open_mic(None, mic_tx) {
        Ok(s) => {
            tracing::info!("mic opened");
            Some(s)
        }
        Err(e) => {
            tracing::error!("failed to open mic: {e}");
            None
        }
    };

    let _speaker_stream = match audio::open_speaker(None, playback_rx) {
        Ok(s) => {
            tracing::info!("speaker opened");
            Some(s)
        }
        Err(e) => {
            tracing::error!("failed to open speaker: {e}");
            None
        }
    };

    let mut tool_registry = tools::ToolRegistry::new();
    actions::register_all(&mut tool_registry);
    let tool_registry = Arc::new(tool_registry);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let event_tx_dash = event_tx.clone();
    rt.spawn(async move {
        if let Err(e) = dashboard::run_dashboard(event_tx_dash, 8765).await {
            tracing::error!("dashboard: {e}");
        }
    });

    let event_tx_session = event_tx.clone();
    let shared_session = shared.clone();
    let tools_session = tool_registry.clone();
    let api_key = cfg.api_key().to_string();
    let voice = cfg.voice().to_string();
    let cfg_clone = cfg.clone();

    rt.spawn(async move {
        run_session(
            api_key,
            cfg_clone,
            voice,
            tools_session,
            event_tx_session,
            shared_session,
            mic_rx,
            playback_tx,
            confirm_rx,
            interrupt_rx,
        )
        .await;
    });

    tracing::info!("Boogie starting GUI...");
    let gui_rx = event_tx.subscribe();
    if let Err(e) = gui::run_gui(gui_rx, shared, confirm_tx, interrupt_tx) {
        tracing::error!("gui: {e}");
    }
}

const CONFIRM_ACTIONS: &[&str] = &[
    "shutdown", "restart", "reboot", "poweroff", "toggle_wifi",
    "format ", "del /f", "rm -rf", "rmdir /s",
];

const CONFIRM_CS_ACTIONS: &[&str] = &["shutdown", "restart", "toggle_wifi"];

fn needs_confirmation(tool_name: &str, args: &serde_json::Value) -> Option<(String, String)> {
    if tool_name == "run_command" {
        if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
            let lower = cmd.to_lowercase();
            for kw in CONFIRM_ACTIONS {
                if lower.contains(kw) {
                    return Some((
                        "Run dangerous command".to_string(),
                        format!("Command: {cmd}"),
                    ));
                }
            }
        }
    }
    if tool_name == "computer_settings" {
        if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
            let lower = action.to_lowercase().replace('-', "_").replace(' ', "_");
            for kw in CONFIRM_CS_ACTIONS {
                if lower == *kw {
                    return Some((
                        format!("Computer settings: {action}"),
                        "This action is irreversible".to_string(),
                    ));
                }
            }
        }
    }
    if tool_name == "kill_process" {
        let target = args.get("name").and_then(|v| v.as_str())
            .or_else(|| args.get("pid").and_then(|v| v.as_str()))
            .unwrap_or("?");
        return Some((
            "Kill process".to_string(),
            format!("Target: {target}"),
        ));
    }
    None
}

async fn run_session(
    api_key: String,
    cfg: config::ApiConfig,
    voice: String,
    tools: Arc<tools::ToolRegistry>,
    event_tx: broadcast::Sender<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    mut mic_rx: mpsc::Receiver<Vec<u8>>,
    playback_tx: mpsc::Sender<Vec<u8>>,
    mut confirm_rx: mpsc::Receiver<ConfirmResponse>,
    mut interrupt_rx: mpsc::Receiver<()>,
) {
    tracing::info!("run_session starting, api_key len={}", api_key.len());

    let tool_json = tools.gemini_tool_json();
    let tools_vec: Vec<serde_json::Value> =
        serde_json::from_value(tool_json).unwrap_or_default();

    tracing::info!("registered {} tool declarations", tools_vec.len());

    loop {
        let resume_handle = {
            let s = shared.lock().await;
            s.resume_handle.clone()
        };

        // Reload memory every reconnect so newly saved facts appear in context
        let memory = config::load_memory();
        let system_prompt = config::build_full_prompt(
            &cfg,
            tools.declarations(),
            &memory,
            cfg.user_name.as_deref(),
        );

        // Collect recent conversation for context injection
        let recent_log: Vec<String> = {
            let s = shared.lock().await;
            s.session_log.iter().rev().take(30).cloned().collect::<Vec<_>>().into_iter().rev().collect()
        };

        let live_config = gemini::LiveConfig {
            api_key: api_key.clone(),
            system_instruction: system_prompt,
            voice: voice.clone(),
            tools: tools_vec.clone(),
            resume_handle: resume_handle.clone(),
            ..Default::default()
        };

        tracing::info!("connecting to Gemini Live...");
        let session = match gemini::LiveSession::connect(live_config.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("gemini connect: {e}");
                let _ = event_tx.send(AppEvent::Log(format!("connect failed: {e}")));
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
        };

        let mut session = session;

        tracing::info!("sending setup...");
        if let Err(e) = session.send_setup(&live_config).await {
            tracing::error!("setup failed: {e}");
            continue;
        }

        // If no resume handle (fresh session), inject recent conversation context
        if resume_handle.is_none() && !recent_log.is_empty() {
            let context = format!(
                "[CONVERSATION CONTEXT — this is what was said recently, use it to maintain continuity]\n{}",
                recent_log.join("\n")
            );
            if let Err(e) = session.send_text(&context).await {
                tracing::warn!("failed to inject context: {e}");
            } else {
                tracing::info!("injected {} lines of conversation context", recent_log.len());
            }
        }

        let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
        let _ = event_tx.send(AppEvent::Log("connected".to_string()));

        let mut echo_guard = echo::EchoGuard::new();
        let mut last_activity = std::time::Instant::now();
        let mut proactive = proactive::ProactiveEngine::new();

        loop {
            tokio::select! {
                Some(mic_data) = mic_rx.recv() => {
                    let is_muted = {
                        let s = shared.lock().await;
                        s.muted
                    };

                    let samples: Vec<i16> = mic_data
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let level = dsp::pcm_level(&samples);
                    let _ = event_tx.send(AppEvent::AudioLevel(if is_muted { 0.0 } else { level }));

                    if !is_muted && echo_guard.is_user_speech(&samples, SEND_SAMPLE_RATE, level) {
                        if let Err(e) = session.send_audio(&mic_data).await {
                            tracing::error!("send audio: {e}");
                            break;
                        }
                        last_activity = std::time::Instant::now();
                        {
                            let mut s = shared.lock().await;
                            s.last_user_speech = std::time::Instant::now();
                        }
                    }
                }

                Some(event) = session.event_rx.recv() => {
                    last_activity = std::time::Instant::now();
                    match event {
                        gemini::GeminiEvent::Audio(data) => {
                            let samples: Vec<i16> = data
                                .chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect();
                            let level = dsp::pcm_level(&samples);
                            echo_guard.note_output(&samples, RECV_SAMPLE_RATE, level);

                            let visemes = dsp::pcm_visemes(&samples, RECV_SAMPLE_RATE);
                            if !visemes.is_empty() {
                                let _ = event_tx.send(AppEvent::Visemes(visemes, 0.02, 0.0));
                            }

                            let _ = event_tx.send(AppEvent::StateChange(AppState::Speaking));
                            let _ = playback_tx.send(data).await;
                        }

                        gemini::GeminiEvent::OutputTranscript(text) => {
                            let _ = event_tx.send(AppEvent::Log(format!("Boogie: {text}")));
                            let mut s = shared.lock().await;
                            s.session_log.push(format!("assistant: {text}"));
                        }

                        gemini::GeminiEvent::InputTranscript(text) => {
                            let _ = event_tx.send(AppEvent::Log(format!("User: {text}")));
                            let mut s = shared.lock().await;
                            s.session_log.push(format!("user: {text}"));
                        }

                        gemini::GeminiEvent::TurnComplete => {
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                            echo_guard.reset();
                            {
                                let mut s = shared.lock().await;
                                s.is_speaking = false;
                            }
                        }

                        gemini::GeminiEvent::ToolCall(calls) => {
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Thinking));
                            let mut responses = Vec::new();
                            for call in &calls {
                                let _ = event_tx.send(AppEvent::Log(
                                    format!("tool: {} {:?}", call.name, call.args),
                                ));

                                let result = if let Some((title, detail)) = needs_confirmation(&call.name, &call.args) {
                                    let _ = event_tx.send(AppEvent::ConfirmRequest {
                                        title: title.clone(),
                                        detail: detail.clone(),
                                    });
                                    let _ = event_tx.send(AppEvent::Log(
                                        format!("⚠ Waiting for confirmation: {title}"),
                                    ));

                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(90),
                                        confirm_rx.recv(),
                                    ).await {
                                        Ok(Some(ConfirmResponse::Confirmed)) => {
                                            let _ = event_tx.send(AppEvent::Log("✓ Confirmed".to_string()));
                                            tools.execute(&call.name, call.args.clone()).await
                                        }
                                        Ok(Some(ConfirmResponse::Denied)) => {
                                            let _ = event_tx.send(AppEvent::Log("✗ Denied by user".to_string()));
                                            tools::ToolResult::err("User denied this action.")
                                        }
                                        _ => {
                                            let _ = event_tx.send(AppEvent::Log("⏱ Confirmation timed out".to_string()));
                                            tools::ToolResult::err("Confirmation timed out (90s). Action cancelled.")
                                        }
                                    }
                                } else {
                                    tools.execute(&call.name, call.args.clone()).await
                                };

                                let output = if result.is_error {
                                    serde_json::json!({"error": result.output})
                                } else {
                                    serde_json::json!({"result": result.output})
                                };
                                responses.push(gemini::FunctionResponse {
                                    id: call.id.clone(),
                                    name: call.name.clone(),
                                    response: output,
                                });
                            }
                            if let Err(e) = session.send_tool_response(&responses).await {
                                tracing::error!("send tool response: {e}");
                                break;
                            }
                        }

                        gemini::GeminiEvent::SessionResumption { handle, resumable } => {
                            if resumable {
                                let mut s = shared.lock().await;
                                s.resume_handle = Some(handle);
                            }
                        }

                        gemini::GeminiEvent::Error(e) => {
                            tracing::error!("gemini: {e}");
                            let _ = event_tx.send(AppEvent::Log(format!("error: {e}")));
                            break;
                        }
                    }
                }

                Some(()) = interrupt_rx.recv() => {
                    tracing::info!("user interrupt");
                    while playback_tx.try_send(Vec::new()).is_ok() {}
                    let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                    let _ = event_tx.send(AppEvent::Log("interrupted".to_string()));
                    echo_guard.reset();
                    {
                        let mut s = shared.lock().await;
                        s.is_speaking = false;
                    }
                }

                _ = tokio::time::sleep(std::time::Duration::from_secs_f64(STALL_LIMIT_SECS)) => {
                    if last_activity.elapsed().as_secs_f64() > STALL_LIMIT_SECS {
                        tracing::warn!("stall detected, reconnecting");
                        let _ = event_tx.send(AppEvent::Log("stall watchdog: reconnecting".to_string()));
                        break;
                    }

                    let last_speech = {
                        let s = shared.lock().await;
                        s.last_user_speech
                    };
                    let monitor_alerts = actions::background_monitor::check_all_monitors().await;
                    for alert in &monitor_alerts {
                        tracing::info!("monitor alert: {}", &alert[..alert.len().min(80)]);
                        if let Err(e) = session.send_text(alert).await {
                            tracing::error!("monitor alert send: {e}");
                        }
                        last_activity = std::time::Instant::now();
                    }

                    if proactive.should_trigger(last_speech) {
                        let memory = config::load_memory();
                        let recent = {
                            let s = shared.lock().await;
                            s.session_log.iter().rev().take(6).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>()
                        };
                        let prompt = proactive.build_prompt(&memory, &recent);
                        proactive.mark_triggered();
                        tracing::info!("proactive check-in triggered");
                        if let Err(e) = session.send_text(&prompt).await {
                            tracing::error!("proactive send: {e}");
                        }
                        last_activity = std::time::Instant::now();
                    }
                }
            }
        }

        {
            let s = shared.lock().await;
            if !s.session_log.is_empty() {
                let last_entries: Vec<&String> = s.session_log.iter().rev().take(20).collect();
                let summary = last_entries.iter().rev().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
                config::save_session_summary(&summary, "auto");
            }
        }

        tracing::info!("session ended, reconnecting in 2s...");
        let _ = event_tx.send(AppEvent::Log("reconnecting...".to_string()));
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
