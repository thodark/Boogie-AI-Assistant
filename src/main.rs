mod actions;
mod audio;
mod config;
mod confirm;
mod dashboard;
mod dsp;
mod echo;
mod gemini;
mod gui;
mod orb_shader;
mod proactive;
mod tools;
mod util;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, broadcast, mpsc};
use tracing_subscriber::EnvFilter;

use crate::audio::PlaybackCmd;
use crate::confirm::PermissionDecision;
use crate::echo::MicDecision;
use crate::util::truncate_chars;

pub const SEND_SAMPLE_RATE: u32 = 16_000;
pub const RECV_SAMPLE_RATE: u32 = 24_000;
const RESPONSE_STALL_SECS: f64 = 35.0;
const WS_PING_INTERVAL_SECS: u64 = 20;
const TOOL_EXEC_TIMEOUT_SECS: u64 = 25;
const MONITOR_CHECK_INTERVAL_SECS: u64 = 60;
const MAX_SESSION_AGE_SECS: u64 = 14 * 60; // Proactive clean refresh before 15m audio limit

#[derive(Clone, Debug)]
pub enum AppEvent {
    StateChange(AppState),
    AudioLevel(f32),
    Log(String),
    BannerError(Option<String>),
    Visemes(Vec<(f32, f32, f32)>, f64, f64),
    ConfirmRequest {
        request_id: u64,
        title: String,
        detail: String,
    },
    TimerAlert(String),
    Shutdown,
}

#[derive(Clone, Debug)]
pub struct ConfirmResponse {
    pub request_id: u64,
    pub confirmed: bool,
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
    pub push_to_talk: bool,
    pub ptt_active: bool,
    pub resume_handle: Option<String>,
    pub session_log: Vec<String>,
    pub last_user_speech: Instant,
}

impl SharedState {
    fn new(push_to_talk: bool) -> Self {
        Self {
            is_speaking: false,
            muted: false,
            push_to_talk,
            ptt_active: false,
            resume_handle: None,
            session_log: Vec::new(),
            last_user_speech: Instant::now(),
        }
    }
}

#[cfg(windows)]
fn enable_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[cfg(not(windows))]
fn enable_dpi_awareness() {}

fn main() {
    enable_dpi_awareness();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("boogie=info")),
        )
        .init();

    let (initial_cfg, initial_cfg_err) = match config::ApiConfig::load() {
        Ok(c) => (Some(c), None),
        Err(e) => {
            tracing::warn!("config unavailable at startup (opening setup UI): {e}");
            (None, Some(e))
        }
    };

    let initial_ptt = initial_cfg
        .as_ref()
        .is_some_and(config::ApiConfig::push_to_talk);
    let shared = Arc::new(Mutex::new(SharedState::new(initial_ptt)));

    let (event_tx, _) = broadcast::channel::<AppEvent>(256);
    let (mic_tx, mic_rx) = mpsc::channel::<Vec<u8>>(200);
    let (playback_tx, playback_rx) = mpsc::channel::<PlaybackCmd>(200);
    let (confirm_tx, confirm_rx) = mpsc::channel::<ConfirmResponse>(8);
    let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>(8);
    let (config_tx, config_rx) = mpsc::channel::<config::ApiConfig>(2);

    let in_dev = initial_cfg
        .as_ref()
        .and_then(|c| audio::find_input_device(c.input_device.as_deref()));
    let out_dev = initial_cfg
        .as_ref()
        .and_then(|c| audio::find_output_device(c.output_device.as_deref()));

    let _mic_stream = match audio::open_mic(in_dev.as_ref(), mic_tx) {
        Ok(s) => {
            tracing::info!("mic opened");
            Some(s)
        }
        Err(e) => {
            tracing::error!("failed to open mic: {e}");
            let _ = event_tx.send(AppEvent::BannerError(Some(format!("Mic error: {e}"))));
            None
        }
    };

    let speaker_output = match audio::open_speaker(out_dev.as_ref(), playback_rx) {
        Ok(s) => {
            tracing::info!("speaker opened");
            Some(s)
        }
        Err(e) => {
            tracing::error!("failed to open speaker: {e}");
            let _ = event_tx.send(AppEvent::BannerError(Some(format!("Speaker error: {e}"))));
            None
        }
    };

    let speaker_queued = speaker_output
        .as_ref()
        .map(audio::AudioOutput::queued_samples_handle)
        .unwrap_or_else(|| Arc::new(AtomicUsize::new(0)));

    let mut tool_registry = tools::ToolRegistry::new();
    actions::register_all(&mut tool_registry, event_tx.clone());
    let tool_registry = Arc::new(tool_registry);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let event_tx_sup = event_tx.clone();
    let shared_sup = shared.clone();
    let tools_sup = tool_registry.clone();

    rt.spawn(async move {
        run_supervisor(
            initial_cfg,
            config_rx,
            tools_sup,
            event_tx_sup,
            shared_sup,
            mic_rx,
            playback_tx,
            speaker_queued,
            confirm_rx,
            interrupt_rx,
        )
        .await;
    });

    tracing::info!("Boogie starting GUI...");
    let gui_rx = event_tx.subscribe();
    if let Err(e) = gui::run_gui(
        gui_rx,
        shared,
        confirm_tx,
        interrupt_tx,
        config_tx,
        initial_cfg_err,
    ) {
        tracing::error!("gui: {e}");
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_supervisor(
    initial_cfg: Option<config::ApiConfig>,
    mut config_rx: mpsc::Receiver<config::ApiConfig>,
    tools: Arc<tools::ToolRegistry>,
    event_tx: broadcast::Sender<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    mic_rx: mpsc::Receiver<Vec<u8>>,
    playback_tx: mpsc::Sender<PlaybackCmd>,
    speaker_queued: Arc<AtomicUsize>,
    confirm_rx: mpsc::Receiver<ConfirmResponse>,
    interrupt_rx: mpsc::Receiver<()>,
) {
    let cfg = match initial_cfg {
        Some(c) => c,
        None => {
            let _ = event_tx.send(AppEvent::StateChange(AppState::Sleeping));
            match config_rx.recv().await {
                Some(c) => {
                    {
                        let mut s = shared.lock().await;
                        s.push_to_talk = c.push_to_talk();
                    }
                    c
                }
                None => return,
            }
        }
    };

    if cfg.dashboard_enabled() {
        let event_tx_dash = event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = dashboard::run_dashboard(event_tx_dash, 8765).await {
                tracing::error!("dashboard: {e}");
            }
        });
    } else {
        tracing::info!(
            "local WebSocket dashboard disabled (set dashboard_enabled: true to enable)"
        );
    }

    // Wrap channels in Arc<Mutex<...>> so the supervisor can recover cleanly if run_session ever panics
    let mic_rx = Arc::new(Mutex::new(mic_rx));
    let confirm_rx = Arc::new(Mutex::new(confirm_rx));
    let interrupt_rx = Arc::new(Mutex::new(interrupt_rx));

    loop {
        let cfg_run = cfg.clone();
        let tools_run = tools.clone();
        let event_tx_run = event_tx.clone();
        let shared_run = shared.clone();
        let mic_rx_run = mic_rx.clone();
        let playback_tx_run = playback_tx.clone();
        let speaker_q_run = speaker_queued.clone();
        let confirm_rx_run = confirm_rx.clone();
        let interrupt_rx_run = interrupt_rx.clone();

        let handle = tokio::spawn(async move {
            run_session(
                cfg_run,
                tools_run,
                event_tx_run,
                shared_run,
                mic_rx_run,
                playback_tx_run,
                speaker_q_run,
                confirm_rx_run,
                interrupt_rx_run,
            )
            .await
        });

        match handle.await {
            Ok(()) => {
                tracing::info!("session supervisor exiting cleanly on shutdown");
                break;
            }
            Err(join_err) => {
                tracing::error!("run_session panicked — restarting supervisor: {join_err}");
                let _ = event_tx.send(AppEvent::BannerError(Some(format!(
                    "Session recovered from internal error: {join_err}"
                ))));
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

struct ToolBatchOutcome {
    responses: Vec<gemini::FunctionResponse>,
    vision_frames: Vec<Vec<u8>>,
    saw_untrusted: bool,
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    cfg: config::ApiConfig,
    tools: Arc<tools::ToolRegistry>,
    event_tx: broadcast::Sender<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    mic_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    playback_tx: mpsc::Sender<PlaybackCmd>,
    speaker_queued: Arc<AtomicUsize>,
    confirm_rx: Arc<Mutex<mpsc::Receiver<ConfirmResponse>>>,
    interrupt_rx: Arc<Mutex<mpsc::Receiver<()>>>,
) {
    tracing::info!(
        "run_session starting (model={}, api={}, tools={})",
        cfg.model(),
        cfg.api_version(),
        tools.declarations().len()
    );

    let tool_json = tools.gemini_tool_json();
    let tools_vec: Vec<serde_json::Value> = serde_json::from_value(tool_json).unwrap_or_default();

    let mut backoff_secs: u64 = 2;
    let mut internal_rx = event_tx.subscribe();
    let (monitor_tx, mut monitor_rx) = mpsc::channel::<Vec<String>>(4);

    loop {
        let resume_handle = {
            let s = shared.lock().await;
            s.resume_handle.clone()
        };
        let used_resume = resume_handle.is_some();

        let recent_log: Vec<String> = {
            let s = shared.lock().await;
            s.session_log
                .iter()
                .rev()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        };

        let memory = config::load_memory();
        // Inject recent context inside system_instruction (never as a turnComplete:true user turn,
        // which would cause Gemini to re-execute old requests like opening URLs on reconnect!)
        let ctx_for_prompt: &[String] = if used_resume { &[] } else { &recent_log };
        let system_prompt = config::build_full_prompt(
            &cfg,
            tools.declarations(),
            &memory,
            cfg.user_name.as_deref(),
            ctx_for_prompt,
        );

        let live_config = gemini::LiveConfig {
            api_key: cfg.api_key().to_string(),
            model: cfg.model().to_string(),
            api_version: cfg.api_version().to_string(),
            system_instruction: system_prompt,
            voice: cfg.voice().to_string(),
            tools: tools_vec.clone(),
            resume_handle: resume_handle.clone(),
        };

        let _ = event_tx.send(AppEvent::StateChange(AppState::Thinking));
        tracing::info!("connecting to Gemini Live...");

        let mut session = match gemini::LiveSession::connect(live_config.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("gemini connect: {e}");
                let _ = event_tx.send(AppEvent::BannerError(Some(format!("Connect error: {e}"))));
                let _ = event_tx.send(AppEvent::Log(format!("connect failed: {e}")));
                if used_resume {
                    shared.lock().await.resume_handle = None;
                }
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(30);
                continue;
            }
        };

        if let Err(e) = session.send_setup(&live_config).await {
            tracing::error!("setup failed: {e}");
            let _ = event_tx.send(AppEvent::BannerError(Some(format!("Setup failed: {e}"))));
            if used_resume {
                shared.lock().await.resume_handle = None;
            }
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(30);
            continue;
        }

        // Per-session isolated tool worker — aborted cleanly on disconnect so stale tool calls
        // can NEVER leak into a reconnected session.
        let (tool_job_tx, mut tool_job_rx) = mpsc::channel::<(Vec<gemini::FunctionCall>, bool)>(16);
        let (tool_done_tx, mut tool_done_rx) = mpsc::channel::<ToolBatchOutcome>(16);
        let tool_worker_handle = {
            let tools_worker = tools.clone();
            let event_tx_worker = event_tx.clone();
            let confirm_rx_worker = confirm_rx.clone();
            tokio::spawn(async move {
                while let Some((calls, mut saw_untrusted)) = tool_job_rx.recv().await {
                    let mut responses = Vec::new();
                    let mut vision_frames = Vec::new();

                    for call in calls {
                        let args_str = call.args.to_string();
                        let args_preview = truncate_chars(&args_str, 160);
                        let _ = event_tx_worker.send(AppEvent::Log(format!(
                            "tool: {} {}",
                            call.name, args_preview
                        )));

                        let decision =
                            confirm::evaluate_tool_call(&call.name, &call.args, saw_untrusted);

                        let result = match decision {
                            PermissionDecision::Deny(reason) => {
                                confirm::write_audit_log(&call.name, "DENIED_POLICY", &reason);
                                tools::ToolResult::err(reason)
                            }
                            PermissionDecision::RequireConfirm { title, detail } => {
                                let req_id = confirm::next_request_id();
                                {
                                    let mut rx = confirm_rx_worker.lock().await;
                                    while rx.try_recv().is_ok() {}
                                }
                                let _ = event_tx_worker.send(AppEvent::ConfirmRequest {
                                    request_id: req_id,
                                    title: title.clone(),
                                    detail: detail.clone(),
                                });
                                let _ = event_tx_worker.send(AppEvent::Log(format!(
                                    "Waiting for confirmation: {title}"
                                )));

                                let confirmed =
                                    await_matching_confirm(&confirm_rx_worker, req_id).await;
                                match confirmed {
                                    Some(true) => {
                                        confirm::write_audit_log(&call.name, "APPROVED", &detail);
                                        let _ = event_tx_worker
                                            .send(AppEvent::Log("Confirmed".to_string()));
                                        execute_tool_isolated(&tools_worker, &call.name, call.args)
                                            .await
                                    }
                                    Some(false) => {
                                        confirm::write_audit_log(
                                            &call.name,
                                            "DENIED_USER",
                                            &detail,
                                        );
                                        let _ = event_tx_worker
                                            .send(AppEvent::Log("Denied by user".to_string()));
                                        tools::ToolResult::err("User denied this action.")
                                    }
                                    None => {
                                        confirm::write_audit_log(&call.name, "TIMEOUT", &detail);
                                        let _ = event_tx_worker.send(AppEvent::Log(
                                            "Confirmation timed out".to_string(),
                                        ));
                                        tools::ToolResult::err(
                                            "Confirmation timed out (90s). Action cancelled.",
                                        )
                                    }
                                }
                            }
                            PermissionDecision::Allow => {
                                execute_tool_isolated(&tools_worker, &call.name, call.args).await
                            }
                        };

                        if result.is_untrusted() {
                            saw_untrusted = true;
                        }
                        if let Some(jpeg) = result.image_jpeg.clone() {
                            vision_frames.push(jpeg);
                        }

                        let formatted = result.formatted_for_model();
                        let output = if result.is_error {
                            serde_json::json!({"error": formatted})
                        } else {
                            serde_json::json!({"result": formatted})
                        };

                        responses.push(gemini::FunctionResponse {
                            id: call.id,
                            name: call.name,
                            response: output,
                        });
                    }

                    if tool_done_tx
                        .send(ToolBatchOutcome {
                            responses,
                            vision_frames,
                            saw_untrusted,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            })
        };

        let _ = event_tx.send(AppEvent::BannerError(None));
        let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
        let _ = event_tx.send(AppEvent::Log("connected".to_string()));

        let mut echo_guard = echo::EchoGuard::new();
        let session_started = Instant::now();
        let mut last_ping = Instant::now();
        let mut awaiting_response_since: Option<Instant> = None;
        let mut response_timer: Option<Instant> = None;
        let mut last_monitor_check = Instant::now();
        let mut proactive = proactive::ProactiveEngine::new(cfg.proactive_audio());
        let mut saw_untrusted_in_turn = false;
        let mut should_shutdown = false;

        // Interval created OUTSIDE select! so keepalive ping, stall detection, monitors & proactive always tick
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut mic_guard = mic_rx.lock().await;
        let mut int_guard = interrupt_rx.lock().await;

        loop {
            tokio::select! {
                Some(mic_data) = mic_guard.recv() => {
                    let (is_muted, ptt_mode, ptt_active) = {
                        let s = shared.lock().await;
                        (s.muted, s.push_to_talk, s.ptt_active)
                    };

                    let samples: Vec<i16> = mic_data
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&c| i16::from_le_bytes(c))
                        .collect();
                    let level = dsp::pcm_level(&samples);
                    let _ = event_tx.send(AppEvent::AudioLevel(if is_muted { 0.0 } else { level }));

                    if is_muted {
                        continue;
                    }

                    let speaker_playing = speaker_queued.load(Ordering::Acquire) > 0;
                    let decision = if ptt_mode {
                        if ptt_active {
                            MicDecision::SendAudio
                        } else {
                            echo_guard.evaluate_mic(0.0, speaker_playing, Instant::now())
                        }
                    } else {
                        echo_guard.evaluate_mic(level, speaker_playing, Instant::now())
                    };

                    match decision {
                        MicDecision::SendAudio => {
                            if let Err(e) = session.send_audio(&mic_data).await {
                                tracing::error!("send audio: {e}");
                                break;
                            }
                            shared.lock().await.last_user_speech = Instant::now();
                        }
                        MicDecision::SendStreamEnd => {
                            if let Err(e) = session.send_audio_stream_end().await {
                                tracing::warn!("send audioStreamEnd: {e}");
                            }
                            let now = Instant::now();
                            awaiting_response_since.get_or_insert(now);
                            response_timer.get_or_insert(now);
                        }
                        MicDecision::Suppress => {}
                    }
                }

                ev = session.event_rx.recv() => {
                    let Some(event) = ev else {
                        tracing::warn!("Gemini WebSocket event channel closed — reconnecting");
                        if used_resume && session_started.elapsed() < Duration::from_secs(10) {
                            shared.lock().await.resume_handle = None;
                        }
                        break;
                    };

                    match event {
                        gemini::GeminiEvent::SetupComplete => {
                            backoff_secs = 2;
                            let _ = event_tx.send(AppEvent::BannerError(None));
                        }

                        gemini::GeminiEvent::Audio(data) => {
                            awaiting_response_since = None;
                            if let Some(t0) = response_timer.take() {
                                tracing::info!(
                                    "first audio response in {}ms",
                                    t0.elapsed().as_millis()
                                );
                            }
                            let samples: Vec<i16> = data
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|&c| i16::from_le_bytes(c))
                                .collect();
                            let level = dsp::pcm_level(&samples);
                            echo_guard.note_output(level);

                            let visemes = dsp::pcm_visemes(&samples, RECV_SAMPLE_RATE);
                            if !visemes.is_empty() {
                                let _ = event_tx.send(AppEvent::Visemes(visemes, 0.02, 0.0));
                            }

                            shared.lock().await.is_speaking = true;
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Speaking));
                            let _ = playback_tx.send(PlaybackCmd::Audio(data)).await;
                        }

                        gemini::GeminiEvent::OutputTranscript(text) => {
                            awaiting_response_since = None;
                            let _ = event_tx.send(AppEvent::Log(format!("Boogie: {text}")));
                            shared.lock().await.session_log.push(format!("assistant: {text}"));
                        }

                        gemini::GeminiEvent::InputTranscript(text) => {
                            saw_untrusted_in_turn = false;
                            let _ = event_tx.send(AppEvent::Log(format!("User: {text}")));
                            shared.lock().await.session_log.push(format!("user: {text}"));
                        }

                        gemini::GeminiEvent::Interrupted => {
                            awaiting_response_since = None;
                            response_timer = None;
                            tracing::info!("server interrupted playback — clearing speaker buffer");
                            let _ = playback_tx.send(PlaybackCmd::Clear).await;
                            echo_guard.reset();
                            shared.lock().await.is_speaking = false;
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                        }

                        gemini::GeminiEvent::TurnComplete => {
                            awaiting_response_since = None;
                            response_timer = None;
                            saw_untrusted_in_turn = false;
                            shared.lock().await.is_speaking = false;
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                        }

                        gemini::GeminiEvent::ToolCall(calls) => {
                            awaiting_response_since = None;
                            if let Some(t0) = response_timer.take() {
                                tracing::info!(
                                    "tool call dispatched in {}ms",
                                    t0.elapsed().as_millis()
                                );
                            }
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Thinking));
                            if tool_job_tx.send((calls, saw_untrusted_in_turn)).await.is_err() {
                                tracing::error!("tool worker channel closed");
                            }
                        }

                        gemini::GeminiEvent::SessionResumption { handle, resumable } => {
                            if resumable {
                                shared.lock().await.resume_handle = Some(handle);
                            }
                        }

                        gemini::GeminiEvent::GoAway { time_left } => {
                            tracing::info!("Gemini GoAway ({:?}) — refreshing session cleanly", time_left);
                            let _ = event_tx.send(AppEvent::Log("refreshing session...".to_string()));
                            break;
                        }

                        gemini::GeminiEvent::Closed(reason) => {
                            tracing::warn!("Gemini WebSocket closed: {reason}");
                            if used_resume && session_started.elapsed() < Duration::from_secs(10) {
                                shared.lock().await.resume_handle = None;
                            }
                            let _ = event_tx.send(AppEvent::BannerError(Some(format!(
                                "Connection closed: {reason}"
                            ))));
                            break;
                        }

                        gemini::GeminiEvent::Error(e) => {
                            tracing::error!("gemini error: {e}");
                            if used_resume && session_started.elapsed() < Duration::from_secs(15) {
                                tracing::warn!("clearing potentially expired resume_handle after error");
                                shared.lock().await.resume_handle = None;
                            }
                            let _ = event_tx.send(AppEvent::BannerError(Some(format!("Gemini: {e}"))));
                            let _ = event_tx.send(AppEvent::Log(format!("error: {e}")));
                            break;
                        }
                    }
                }

                Some(outcome) = tool_done_rx.recv() => {
                    saw_untrusted_in_turn |= outcome.saw_untrusted;
                    for jpeg in outcome.vision_frames {
                        if let Err(e) = session.send_image(&jpeg).await {
                            tracing::error!("send vision image failed: {e}");
                        }
                    }
                    if let Err(e) = session.send_tool_response(&outcome.responses).await {
                        tracing::error!("send tool response: {e}");
                        break;
                    }
                    let now = Instant::now();
                    awaiting_response_since = Some(now);
                    response_timer = Some(now);
                }

                Some(()) = int_guard.recv() => {
                    tracing::info!("user interrupt — clearing speaker buffer");
                    let _ = playback_tx.send(PlaybackCmd::Clear).await;
                    echo_guard.reset();
                    awaiting_response_since = None;
                    response_timer = None;
                    shared.lock().await.is_speaking = false;
                    let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                    let _ = event_tx.send(AppEvent::Log("interrupted".to_string()));
                }

                Some(alerts) = monitor_rx.recv() => {
                    for alert in alerts {
                        tracing::info!("monitor alert: {}", truncate_chars(&alert, 80));
                        let wrapped = crate::util::wrap_untrusted("web", &alert);
                        if let Err(e) = session.send_text(&wrapped).await {
                            tracing::error!("monitor alert send: {e}");
                        }
                    }
                }

                Ok(app_ev) = internal_rx.recv() => {
                    match app_ev {
                        AppEvent::TimerAlert(msg) => {
                            let prompt = format!("[TIMER_ALERT] The user's timer just finished: {msg}");
                            if let Err(e) = session.send_text(&prompt).await {
                                tracing::error!("timer alert send: {e}");
                            }
                        }
                        AppEvent::Shutdown => {
                            should_shutdown = true;
                            break;
                        }
                        _ => {}
                    }
                }

                _ = tick.tick() => {
                    // Keep WebSocket alive during quiet periods and detect dead TCP sockets
                    if last_ping.elapsed().as_secs() >= WS_PING_INTERVAL_SECS {
                        last_ping = Instant::now();
                        if let Err(e) = session.send_ping().await {
                            tracing::warn!("ws keepalive ping failed ({e}) — reconnecting");
                            break;
                        }
                    }

                    // Only trigger stall reconnect if we sent a turn/toolResponse and got zero reply
                    if let Some(since) = awaiting_response_since
                        && since.elapsed().as_secs_f64() > RESPONSE_STALL_SECS
                    {
                        tracing::warn!("response stall after {RESPONSE_STALL_SECS}s — reconnecting");
                        let _ = event_tx.send(AppEvent::Log("connection stalled: reconnecting".to_string()));
                        break;
                    }

                    if session_started.elapsed().as_secs() >= MAX_SESSION_AGE_SECS {
                        tracing::info!("session reached 14m — rotating connection cleanly");
                        break;
                    }

                    if last_monitor_check.elapsed().as_secs() >= MONITOR_CHECK_INTERVAL_SECS {
                        last_monitor_check = Instant::now();
                        let mtx = monitor_tx.clone();
                        tokio::spawn(async move {
                            let alerts = actions::background_monitor::check_all_monitors().await;
                            if !alerts.is_empty() {
                                let _ = mtx.send(alerts).await;
                            }
                        });
                    }

                    let last_speech = shared.lock().await.last_user_speech;
                    if proactive.should_trigger(last_speech) {
                        let mem = config::load_memory();
                        let recent: Vec<String> = {
                            let s = shared.lock().await;
                            s.session_log
                                .iter()
                                .rev()
                                .take(6)
                                .cloned()
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect()
                        };
                        let prompt = proactive.build_prompt(&mem, &recent);
                        proactive.mark_triggered();
                        tracing::info!("proactive check-in triggered");
                        if let Err(e) = session.send_text(&prompt).await {
                            tracing::error!("proactive send: {e}");
                        }
                    }
                }
            }
        }

        // Cancel per-session tool worker cleanly so stale jobs never leak across reconnects
        tool_worker_handle.abort();
        drop(mic_guard);
        drop(int_guard);

        // Save compact session summary with real detected language
        {
            let mem = config::load_memory();
            let lang = config::detect_language(&mem);
            let s = shared.lock().await;
            if !s.session_log.is_empty() {
                let last_entries: Vec<&String> = s.session_log.iter().rev().take(20).collect();
                let summary = last_entries
                    .iter()
                    .rev()
                    .map(|line| line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                config::save_session_summary(&summary, &lang);
            }
        }

        if should_shutdown {
            return;
        }

        tracing::info!("session ended, reconnecting in {}s...", backoff_secs);
        let _ = event_tx.send(AppEvent::Log(format!("reconnecting in {backoff_secs}s...")));
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(30);
    }
}

async fn execute_tool_isolated(
    tools: &Arc<tools::ToolRegistry>,
    name: &str,
    args: serde_json::Value,
) -> tools::ToolResult {
    let t = Arc::clone(tools);
    let tool_name = name.to_string();
    let started = Instant::now();
    let handle = tokio::spawn(async move { t.execute(&tool_name, args).await });

    let result =
        match tokio::time::timeout(Duration::from_secs(TOOL_EXEC_TIMEOUT_SECS), handle).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => tools::ToolResult::err(format!("tool panicked: {e}")),
            Err(_) => tools::ToolResult::err(format!(
                "tool '{name}' timed out after {TOOL_EXEC_TIMEOUT_SECS}s"
            )),
        };

    tracing::info!(
        "tool '{}' finished in {}ms (ok={})",
        name,
        started.elapsed().as_millis(),
        !result.is_error
    );
    result
}

async fn await_matching_confirm(
    confirm_rx: &Arc<Mutex<mpsc::Receiver<ConfirmResponse>>>,
    req_id: u64,
) -> Option<bool> {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut rx = confirm_rx.lock().await;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(resp)) if resp.request_id == req_id => return Some(resp.confirmed),
            Ok(Some(_stale)) => continue,
            Ok(None) | Err(_) => return None,
        }
    }
}
