mod actions;
mod audio;
mod config;
mod confirm;
mod dashboard;
mod dsp;
mod echo;
mod gemini;
mod gui;
mod memory;
mod orb_shader;
mod proactive;
mod study_visual;
mod theme;
mod tools;
mod util;

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, broadcast, mpsc};
use tracing_subscriber::EnvFilter;

use crate::audio::PlaybackCmd;
use crate::confirm::PermissionDecision;
use crate::echo::MicDecision;
use crate::util::truncate_chars;

pub const SEND_SAMPLE_RATE: u32 = 16_000;
pub const RECV_SAMPLE_RATE: u32 = 24_000;
const RESPONSE_STALL_SECS: f64 = 18.0;
const WS_PING_INTERVAL_SECS: u64 = 20;
const TOOL_EXEC_TIMEOUT_SECS: u64 = 25;
const STUDY_TOOL_TIMEOUT_SECS: u64 = 180;
const MONITOR_CHECK_INTERVAL_SECS: u64 = 60;
const MAX_SESSION_AGE_SECS: u64 = 14 * 60; // Proactive clean refresh before 15m audio limit
const MAX_LOG_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
struct TeeMakeWriter {
    file: Option<Arc<StdMutex<std::fs::File>>>,
}

struct TeeWriter {
    stderr: std::io::Stderr,
    file: Option<Arc<StdMutex<std::fs::File>>>,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.stderr.write_all(buf);
        if let Some(f) = &self.file
            && let Ok(mut guard) = f.lock()
        {
            let _ = guard.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = self.stderr.flush();
        if let Some(f) = &self.file
            && let Ok(mut guard) = f.lock()
        {
            let _ = guard.flush();
        }
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            stderr: std::io::stderr(),
            file: self.file.clone(),
        }
    }
}

fn open_log_file() -> (std::path::PathBuf, Option<Arc<StdMutex<std::fs::File>>>) {
    let log_path = config::active_config_dir().join("boogie.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let oversize = std::fs::metadata(&log_path)
        .map(|m| m.len() > MAX_LOG_FILE_BYTES)
        .unwrap_or(false);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true);
    if oversize {
        opts.truncate(true);
    } else {
        opts.append(true);
    }
    let file = opts
        .open(&log_path)
        .ok()
        .map(|f| Arc::new(StdMutex::new(f)));
    (log_path, file)
}

enum AudioCmd {
    Reload {
        input_device: Option<String>,
        output_device: Option<String>,
    },
}

type PlaybackSlot = Arc<StdMutex<Option<mpsc::Sender<PlaybackCmd>>>>;

async fn send_playback(slot: &PlaybackSlot, cmd: PlaybackCmd) {
    let tx = slot.lock().ok().and_then(|g| g.clone());
    if let Some(tx) = tx {
        let _ = tx.send(cmd).await;
    }
}

fn open_audio_devices(
    input_name: Option<&str>,
    output_name: Option<&str>,
    mic_tx: &mpsc::Sender<Vec<u8>>,
    speaker_queued: &Arc<AtomicUsize>,
    playback_slot: &PlaybackSlot,
    event_tx: &broadcast::Sender<AppEvent>,
) -> (Option<audio::AudioInput>, Option<audio::AudioOutput>) {
    let mut errs = Vec::new();
    let in_dev = audio::find_input_device(input_name);
    let mic_stream = match audio::open_mic(in_dev.as_ref(), mic_tx.clone()) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("failed to open mic: {e}");
            errs.push(format!("Mic error: {e}"));
            None
        }
    };

    let out_dev = audio::find_output_device(output_name);
    let (playback_tx, playback_rx) = mpsc::channel::<PlaybackCmd>(1024);
    let speaker_output =
        match audio::open_speaker(out_dev.as_ref(), playback_rx, Arc::clone(speaker_queued)) {
            Ok(s) => {
                if let Ok(mut slot) = playback_slot.lock() {
                    *slot = Some(playback_tx);
                }
                Some(s)
            }
            Err(e) => {
                tracing::error!("failed to open speaker: {e}");
                errs.push(format!("Speaker error: {e}"));
                None
            }
        };

    if errs.is_empty() {
        let _ = event_tx.send(AppEvent::BannerError(None));
    } else {
        let _ = event_tx.send(AppEvent::BannerError(Some(errs.join(" | "))));
    }

    (mic_stream, speaker_output)
}

fn spawn_audio_manager(
    initial_input: Option<String>,
    initial_output: Option<String>,
    mic_tx: mpsc::Sender<Vec<u8>>,
    speaker_queued: Arc<AtomicUsize>,
    playback_slot: PlaybackSlot,
    event_tx: broadcast::Sender<AppEvent>,
) -> std::sync::mpsc::Sender<AudioCmd> {
    let (audio_cmd_tx, audio_cmd_rx) = std::sync::mpsc::channel::<AudioCmd>();
    std::thread::spawn(move || {
        // Keep cpal::Stream (!Send on Windows) strictly confined to this dedicated OS thread.
        let (mut mic_stream, mut speaker_output) = open_audio_devices(
            initial_input.as_deref(),
            initial_output.as_deref(),
            &mic_tx,
            &speaker_queued,
            &playback_slot,
            &event_tx,
        );

        while let Ok(cmd) = audio_cmd_rx.recv() {
            match cmd {
                AudioCmd::Reload {
                    input_device,
                    output_device,
                } => {
                    tracing::info!(
                        "reloading audio devices (mic={:?}, speaker={:?})",
                        input_device.as_deref().unwrap_or("System Default"),
                        output_device.as_deref().unwrap_or("System Default"),
                    );
                    if let Ok(mut slot) = playback_slot.lock() {
                        *slot = None;
                    }
                    drop(mic_stream.take());
                    drop(speaker_output.take());
                    (mic_stream, speaker_output) = open_audio_devices(
                        input_device.as_deref(),
                        output_device.as_deref(),
                        &mic_tx,
                        &speaker_queued,
                        &playback_slot,
                        &event_tx,
                    );
                }
            }
        }
    });
    audio_cmd_tx
}

#[derive(Clone, Copy, Debug)]
pub enum ChimeKind {
    StartAck,
    ScreenCaptured,
}

#[derive(Clone, Debug)]
pub struct TaskProgress {
    pub title: String,
    pub stage: String,
    pub progress: f32,
    pub detail: String,
}

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
    PlayChime(ChimeKind),
    TaskProgress(Option<TaskProgress>),
    StudyVisual(Box<study_visual::StudyBoardData>),
    StudyVisualStep(String),
    CloseStudyVisual,
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
    pub consolidated_turns: usize,
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
            consolidated_turns: 0,
            last_user_speech: Instant::now(),
        }
    }

    pub fn push_turn(&mut self, speaker: &str, chunk: &str) {
        let before = self.session_log.len();
        memory::push_transcript_turn(&mut self.session_log, speaker, chunk);
        if before == 200 && self.session_log.len() == 200 {
            self.consolidated_turns = self.consolidated_turns.saturating_sub(1);
        }
    }

    pub fn take_unsummarized_turns(&mut self) -> Vec<String> {
        if self.consolidated_turns >= self.session_log.len() {
            return Vec::new();
        }
        let slice = self.session_log[self.consolidated_turns..].to_vec();
        self.consolidated_turns = self.session_log.len();
        slice
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

    let (log_path, log_file) = open_log_file();
    let make_writer = TeeMakeWriter { file: log_file };
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(make_writer)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("boogie=info")),
        )
        .init();

    tracing::info!(
        "Boogie v{} starting (log: {})",
        env!("CARGO_PKG_VERSION"),
        log_path.display()
    );

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

    let (event_tx, _) = broadcast::channel::<AppEvent>(4096);
    let (mic_tx, mic_rx) = mpsc::channel::<Vec<u8>>(200);
    let (confirm_tx, confirm_rx) = mpsc::channel::<ConfirmResponse>(8);
    let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>(8);
    let (config_tx, config_rx) = mpsc::channel::<config::ApiConfig>(8);

    let speaker_queued = Arc::new(AtomicUsize::new(0));
    let playback_slot: PlaybackSlot = Arc::new(StdMutex::new(None));
    let audio_cmd_tx = spawn_audio_manager(
        initial_cfg.as_ref().and_then(|c| c.input_device.clone()),
        initial_cfg.as_ref().and_then(|c| c.output_device.clone()),
        mic_tx,
        Arc::clone(&speaker_queued),
        Arc::clone(&playback_slot),
        event_tx.clone(),
    );

    let mut tool_registry = tools::ToolRegistry::new();
    actions::register_all(&mut tool_registry, event_tx.clone());
    let tool_registry = Arc::new(tool_registry);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let latest_cfg = Arc::new(Mutex::new(initial_cfg.clone()));

    let event_tx_sup = event_tx.clone();
    let shared_sup = shared.clone();
    let tools_sup = tool_registry.clone();
    let cfg_sup = initial_cfg.clone();
    let latest_cfg_sup = latest_cfg.clone();

    rt.spawn(async move {
        run_supervisor(
            cfg_sup,
            latest_cfg_sup,
            config_rx,
            audio_cmd_tx,
            tools_sup,
            event_tx_sup,
            shared_sup,
            mic_rx,
            playback_slot,
            speaker_queued,
            confirm_rx,
            interrupt_rx,
        )
        .await;
    });

    tracing::info!("Boogie starting GUI...");
    let gui_rx = event_tx.subscribe();
    let shared_exit = shared.clone();
    if let Err(e) = gui::run_gui(
        gui_rx,
        shared,
        confirm_tx,
        interrupt_tx,
        config_tx,
        initial_cfg,
        initial_cfg_err,
    ) {
        tracing::error!("gui: {e}");
    }

    // Ensure conversation memory is saved cleanly even when user closes the window via 'X'
    let _ = event_tx.send(AppEvent::Shutdown);
    rt.block_on(async move {
        let pending_turns = shared_exit.lock().await.take_unsummarized_turns();
        if pending_turns.is_empty() {
            return;
        }
        let cfg_opt = latest_cfg.lock().await.clone();
        let mem = config::load_memory();
        let lang = config::detect_language(&mem);
        if let Some(cfg) = cfg_opt {
            tracing::info!(
                "flushing {} unsummarized conversation turn(s) before exit",
                pending_turns.len()
            );
            memory::consolidate_session(&cfg, &pending_turns, &lang, true).await;
        } else {
            let fallback = memory::build_local_fallback_summary(&pending_turns);
            memory::save_session_summary_rich(&fallback, &lang, Vec::new(), false);
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_supervisor(
    initial_cfg: Option<config::ApiConfig>,
    latest_cfg: Arc<Mutex<Option<config::ApiConfig>>>,
    mut config_rx: mpsc::Receiver<config::ApiConfig>,
    audio_cmd_tx: std::sync::mpsc::Sender<AudioCmd>,
    tools: Arc<tools::ToolRegistry>,
    event_tx: broadcast::Sender<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    mic_rx: mpsc::Receiver<Vec<u8>>,
    playback_slot: PlaybackSlot,
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
                    *latest_cfg.lock().await = Some(c.clone());
                    {
                        let mut s = shared.lock().await;
                        s.push_to_talk = c.push_to_talk();
                    }
                    let _ = audio_cmd_tx.send(AudioCmd::Reload {
                        input_device: c.input_device.clone(),
                        output_device: c.output_device.clone(),
                    });
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
    let active_cfg = Arc::new(Mutex::new(cfg));
    let config_rx = Arc::new(Mutex::new(config_rx));
    let mic_rx = Arc::new(Mutex::new(mic_rx));
    let confirm_rx = Arc::new(Mutex::new(confirm_rx));
    let interrupt_rx = Arc::new(Mutex::new(interrupt_rx));

    loop {
        let active_cfg_run = active_cfg.clone();
        let latest_cfg_run = latest_cfg.clone();
        let config_rx_run = config_rx.clone();
        let audio_cmd_run = audio_cmd_tx.clone();
        let tools_run = tools.clone();
        let event_tx_run = event_tx.clone();
        let shared_run = shared.clone();
        let mic_rx_run = mic_rx.clone();
        let playback_slot_run = playback_slot.clone();
        let speaker_q_run = speaker_queued.clone();
        let confirm_rx_run = confirm_rx.clone();
        let interrupt_rx_run = interrupt_rx.clone();

        let handle = tokio::spawn(async move {
            run_session(
                active_cfg_run,
                latest_cfg_run,
                config_rx_run,
                audio_cmd_run,
                tools_run,
                event_tx_run,
                shared_run,
                mic_rx_run,
                playback_slot_run,
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

fn config_requires_reconnect(old: &config::ApiConfig, new: &config::ApiConfig) -> bool {
    old.api_key() != new.api_key()
        || old.model() != new.model()
        || old.api_version() != new.api_version()
        || old.voice() != new.voice()
        || old.assistant_name() != new.assistant_name()
        || old.user_name != new.user_name
}

#[allow(clippy::too_many_arguments)]
async fn apply_config_update(
    cfg: &mut config::ApiConfig,
    new_cfg: config::ApiConfig,
    active_cfg: &Arc<Mutex<config::ApiConfig>>,
    latest_cfg: &Arc<Mutex<Option<config::ApiConfig>>>,
    shared: &Arc<Mutex<SharedState>>,
    audio_cmd_tx: &std::sync::mpsc::Sender<AudioCmd>,
    event_tx: &broadcast::Sender<AppEvent>,
) -> bool {
    let devices_changed =
        cfg.input_device != new_cfg.input_device || cfg.output_device != new_cfg.output_device;
    let reconnect_needed = config_requires_reconnect(cfg, &new_cfg);

    {
        let mut s = shared.lock().await;
        s.push_to_talk = new_cfg.push_to_talk();
        if !s.push_to_talk {
            s.ptt_active = false;
        }
        if reconnect_needed {
            s.resume_handle = None;
        }
    }

    if devices_changed {
        let _ = audio_cmd_tx.send(AudioCmd::Reload {
            input_device: new_cfg.input_device.clone(),
            output_device: new_cfg.output_device.clone(),
        });
    }

    *cfg = new_cfg.clone();
    *latest_cfg.lock().await = Some(new_cfg.clone());
    *active_cfg.lock().await = new_cfg;
    let _ = event_tx.send(AppEvent::Log("Settings saved".to_string()));
    tracing::info!(
        "settings updated (ptt={}, proactive={}, ai_mem={}, graphics={}, theme={}, reconnect={reconnect_needed}, audio_reload={devices_changed})",
        cfg.push_to_talk(),
        cfg.proactive_audio(),
        cfg.auto_memory_consolidation(),
        cfg.graphics_quality().as_str(),
        cfg.color_theme().as_str(),
    );
    reconnect_needed
}

#[allow(clippy::too_many_arguments)]
async fn sleep_or_reconfig(
    secs: u64,
    cfg: &mut config::ApiConfig,
    active_cfg: &Arc<Mutex<config::ApiConfig>>,
    latest_cfg: &Arc<Mutex<Option<config::ApiConfig>>>,
    config_rx: &Arc<Mutex<mpsc::Receiver<config::ApiConfig>>>,
    shared: &Arc<Mutex<SharedState>>,
    audio_cmd_tx: &std::sync::mpsc::Sender<AudioCmd>,
    event_tx: &broadcast::Sender<AppEvent>,
) -> bool {
    let mut cg = config_rx.lock().await;
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(secs)) => false,
        Some(new_cfg) = cg.recv() => {
            apply_config_update(cfg, new_cfg, active_cfg, latest_cfg, shared, audio_cmd_tx, event_tx).await;
            true
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    active_cfg: Arc<Mutex<config::ApiConfig>>,
    latest_cfg: Arc<Mutex<Option<config::ApiConfig>>>,
    config_rx: Arc<Mutex<mpsc::Receiver<config::ApiConfig>>>,
    audio_cmd_tx: std::sync::mpsc::Sender<AudioCmd>,
    tools: Arc<tools::ToolRegistry>,
    event_tx: broadcast::Sender<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    mic_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    playback_slot: PlaybackSlot,
    speaker_queued: Arc<AtomicUsize>,
    confirm_rx: Arc<Mutex<mpsc::Receiver<ConfirmResponse>>>,
    interrupt_rx: Arc<Mutex<mpsc::Receiver<()>>>,
) {
    let mut cfg = active_cfg.lock().await.clone();
    tracing::info!(
        "run_session starting (model={}, api={}, tools={}, ptt={})",
        cfg.model(),
        cfg.api_version(),
        tools.declarations().len(),
        cfg.push_to_talk()
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
                if sleep_or_reconfig(
                    backoff_secs,
                    &mut cfg,
                    &active_cfg,
                    &latest_cfg,
                    &config_rx,
                    &shared,
                    &audio_cmd_tx,
                    &event_tx,
                )
                .await
                {
                    backoff_secs = 2;
                } else {
                    backoff_secs = (backoff_secs * 2).min(30);
                }
                continue;
            }
        };

        if let Err(e) = session.send_setup(&live_config).await {
            tracing::error!("setup failed: {e}");
            let _ = event_tx.send(AppEvent::BannerError(Some(format!("Setup failed: {e}"))));
            if used_resume {
                shared.lock().await.resume_handle = None;
            }
            if sleep_or_reconfig(
                backoff_secs,
                &mut cfg,
                &active_cfg,
                &latest_cfg,
                &config_rx,
                &shared,
                &audio_cmd_tx,
                &event_tx,
            )
            .await
            {
                backoff_secs = 2;
            } else {
                backoff_secs = (backoff_secs * 2).min(30);
            }
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
            let playback_worker = playback_slot.clone();
            tokio::spawn(async move {
                while let Some((calls, mut saw_untrusted)) = tool_job_rx.recv().await {
                    let mut responses = Vec::new();
                    let vision_frames = Vec::new();

                    for call in calls {
                        let args_str = call.args.to_string();
                        let args_preview = truncate_chars(&args_str, 160);
                        tracing::info!("executing tool '{}' args={}", call.name, args_preview);
                        let _ = event_tx_worker.send(AppEvent::Log(format!(
                            "tool: {} {}",
                            call.name, args_preview
                        )));

                        // Play StartAck immediately ONLY for non-screen slow calls.
                        // For screen-capture calls, the crisp ScreenCaptured shutter chime fires
                        // AFTER capture completes so hearing the beep means it's 100% safe to Alt+Tab!
                        let will_capture_screen = match call.name.as_str() {
                            "study_solve" => {
                                let src = call
                                    .args
                                    .get("source")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("screen");
                                !matches!(src.to_ascii_lowercase().as_str(), "text" | "none")
                            }
                            "study_board" => {
                                let art = call
                                    .args
                                    .get("art_mode")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                let src = call
                                    .args
                                    .get("source")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(if art { "screen" } else { "text" });
                                matches!(src.to_ascii_lowercase().as_str(), "screen" | "clipboard")
                            }
                            "generate_document" => {
                                let src = call
                                    .args
                                    .get("source")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("text");
                                matches!(src.to_ascii_lowercase().as_str(), "screen" | "clipboard")
                            }
                            _ => false,
                        };
                        if !will_capture_screen
                            && matches!(
                                call.name.as_str(),
                                "generate_document" | "study_board" | "study_solve"
                            )
                        {
                            send_playback(
                                &playback_worker,
                                PlaybackCmd::Audio(synth_chime_pcm(ChimeKind::StartAck)),
                            )
                            .await;
                        }

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
                        // Bind captured screenshots atomically inside `functionResponse.parts`
                        // (with MEDIA_RESOLUTION_HIGH) instead of async `realtimeInput.video`,
                        // eliminating low-res video downscaling AND stale-tab frame lag!
                        let multimodal_parts = result.image_jpeg.as_ref().map(|jpeg| {
                            use base64::Engine as _;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
                            tracing::debug!(
                                "attaching crisp screenshot ({} KB JPEG) atomically to toolResponse '{}'",
                                jpeg.len() / 1024,
                                call.name
                            );
                            vec![serde_json::json!({
                                "inlineData": {
                                    "mimeType": "image/jpeg",
                                    "data": b64
                                }
                            })]
                        });

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
                            parts: multimodal_parts,
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
        let mut ptt_was_active = false;
        let mut model_turn_active = false;
        let mut tools_in_flight: usize = 0;
        let mut deferred_refresh = false;
        let mut last_sent_audio_level: f32 = -1.0;
        let session_started = Instant::now();
        let mut last_ping = Instant::now();
        let mut awaiting_response_since: Option<Instant> = None;
        let mut response_timer: Option<Instant> = None;
        let mut last_monitor_check = Instant::now();
        let mut proactive = proactive::ProactiveEngine::new(cfg.proactive_audio());
        let mut saw_untrusted_in_turn = false;
        let mut should_shutdown = false;
        let mut immediate_reconnect = false;

        // Interval created OUTSIDE select! so keepalive ping, stall detection, monitors & proactive always tick
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut mic_guard = mic_rx.lock().await;
        let mut int_guard = interrupt_rx.lock().await;
        let mut cfg_guard = config_rx.lock().await;

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
                    let ui_level = if is_muted || (ptt_mode && !ptt_active) {
                        0.0
                    } else {
                        level
                    };
                    if ui_level > 0.0 || last_sent_audio_level != 0.0 {
                        last_sent_audio_level = ui_level;
                        let _ = event_tx.send(AppEvent::AudioLevel(ui_level));
                    }

                    let speaker_playing = speaker_queued.load(Ordering::Acquire) > 0;
                    if !model_turn_active && !speaker_playing {
                        let finished_speaking = {
                            let mut s = shared.lock().await;
                            if s.is_speaking {
                                s.is_speaking = false;
                                true
                            } else {
                                false
                            }
                        };
                        if finished_speaking {
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                        }
                    }

                    if is_muted {
                        continue;
                    }

                    if ptt_mode {
                        if ptt_active && !ptt_was_active {
                            tracing::info!("PTT pressed — streaming mic audio");
                            let was_speaking = {
                                let mut s = shared.lock().await;
                                let prev = s.is_speaking;
                                s.is_speaking = false;
                                prev
                            };
                            if speaker_playing || was_speaking {
                                send_playback(&playback_slot, PlaybackCmd::Clear).await;
                                echo_guard.reset();
                                let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                            }
                        } else if !ptt_active && ptt_was_active {
                            tracing::info!("PTT released — finishing utterance");
                        }
                        ptt_was_active = ptt_active;
                    } else {
                        ptt_was_active = false;
                    }

                    let decision = if ptt_mode {
                        echo_guard.evaluate_ptt(ptt_active, Instant::now())
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
                            tracing::info!("sending audioStreamEnd — awaiting response");
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
                            if !model_turn_active {
                                model_turn_active = true;
                                echo_guard.on_model_response_started();
                            }
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
                            send_playback(&playback_slot, PlaybackCmd::Audio(data)).await;
                        }

                        gemini::GeminiEvent::OutputTranscript(text) => {
                            awaiting_response_since = None;
                            let _ = event_tx.send(AppEvent::Log(format!("Boogie: {text}")));
                            shared.lock().await.push_turn("assistant", &text);
                        }

                        gemini::GeminiEvent::InputTranscript(text) => {
                            saw_untrusted_in_turn = false;
                            let _ = event_tx.send(AppEvent::Log(format!("User: {text}")));
                            shared.lock().await.push_turn("user", &text);
                        }

                        gemini::GeminiEvent::Interrupted => {
                            model_turn_active = false;
                            awaiting_response_since = None;
                            response_timer = None;
                            tracing::info!("server interrupted playback — clearing speaker buffer");
                            send_playback(&playback_slot, PlaybackCmd::Clear).await;
                            echo_guard.reset();
                            shared.lock().await.is_speaking = false;
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                            if deferred_refresh {
                                tracing::info!(
                                    "executing deferred session refresh after interruption"
                                );
                                immediate_reconnect = true;
                                break;
                            }
                        }

                        gemini::GeminiEvent::TurnComplete => {
                            model_turn_active = false;
                            awaiting_response_since = None;
                            response_timer = None;
                            saw_untrusted_in_turn = false;
                            if speaker_queued.load(Ordering::Acquire) == 0 {
                                shared.lock().await.is_speaking = false;
                                let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                            }
                            if deferred_refresh {
                                tracing::info!(
                                    "executing deferred session refresh after turn complete"
                                );
                                immediate_reconnect = true;
                                break;
                            }
                        }

                        gemini::GeminiEvent::ToolCall(calls) => {
                            awaiting_response_since = None;
                            if let Some(t0) = response_timer.take() {
                                tracing::info!(
                                    "tool call dispatched in {}ms",
                                    t0.elapsed().as_millis()
                                );
                            }
                            tools_in_flight += 1;
                            let _ = event_tx.send(AppEvent::StateChange(AppState::Thinking));
                            if tool_job_tx.send((calls, saw_untrusted_in_turn)).await.is_err() {
                                tools_in_flight = tools_in_flight.saturating_sub(1);
                                tracing::error!("tool worker channel closed");
                            }
                        }

                        gemini::GeminiEvent::SessionResumption { handle, resumable } => {
                            if resumable {
                                shared.lock().await.resume_handle = Some(handle);
                            }
                        }

                        gemini::GeminiEvent::GoAway { time_left } => {
                            if model_turn_active
                                || tools_in_flight > 0
                                || awaiting_response_since.is_some()
                            {
                                tracing::info!(
                                    "Gemini GoAway ({:?}) — deferring refresh until current turn/tool finishes",
                                    time_left
                                );
                                deferred_refresh = true;
                            } else {
                                tracing::info!(
                                    "Gemini GoAway ({:?}) — refreshing session cleanly",
                                    time_left
                                );
                                let _ =
                                    event_tx.send(AppEvent::Log("refreshing session...".to_string()));
                                immediate_reconnect = true;
                                break;
                            }
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
                    tools_in_flight = tools_in_flight.saturating_sub(1);
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
                    model_turn_active = false;
                    send_playback(&playback_slot, PlaybackCmd::Clear).await;
                    echo_guard.reset();
                    awaiting_response_since = None;
                    response_timer = None;
                    shared.lock().await.is_speaking = false;
                    let _ = event_tx.send(AppEvent::StateChange(AppState::Listening));
                    let _ = event_tx.send(AppEvent::Log("interrupted".to_string()));
                }

                Some(new_cfg) = cfg_guard.recv() => {
                    let old_ptt = cfg.push_to_talk();
                    let reconnect = apply_config_update(
                        &mut cfg,
                        new_cfg,
                        &active_cfg,
                        &latest_cfg,
                        &shared,
                        &audio_cmd_tx,
                        &event_tx,
                    )
                    .await;
                    proactive.set_enabled(cfg.proactive_audio());
                    if old_ptt != cfg.push_to_talk() {
                        echo_guard.reset();
                        ptt_was_active = false;
                    }
                    if reconnect {
                        immediate_reconnect = true;
                        break;
                    }
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

                res = internal_rx.recv() => {
                    match res {
                        Ok(AppEvent::TimerAlert(msg)) => {
                            let prompt = format!("[TIMER_ALERT] The user's timer just finished: {msg}");
                            if let Err(e) = session.send_text(&prompt).await {
                                tracing::error!("timer alert send: {e}");
                            }
                        }
                        Ok(AppEvent::PlayChime(kind)) => {
                            send_playback(&playback_slot, PlaybackCmd::Audio(synth_chime_pcm(kind)))
                                .await;
                        }
                        Ok(AppEvent::Shutdown) => {
                            should_shutdown = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("internal_rx lagged by {n} events");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
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
                    if tools_in_flight == 0
                        && let Some(since) = awaiting_response_since
                        && since.elapsed().as_secs_f64() > RESPONSE_STALL_SECS
                    {
                        tracing::warn!("response stall after {RESPONSE_STALL_SECS}s — reconnecting");
                        let _ = event_tx.send(AppEvent::Log("connection stalled: reconnecting".to_string()));
                        break;
                    }

                    if session_started.elapsed().as_secs() >= MAX_SESSION_AGE_SECS {
                        if model_turn_active
                            || tools_in_flight > 0
                            || awaiting_response_since.is_some()
                        {
                            if !deferred_refresh {
                                tracing::info!(
                                    "session reached 14m during active turn — deferring rotation until turn finishes"
                                );
                                deferred_refresh = true;
                            }
                        } else {
                            tracing::info!("session reached 14m — rotating connection cleanly");
                            immediate_reconnect = true;
                            break;
                        }
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
        let _ = event_tx.send(AppEvent::TaskProgress(None));
        drop(mic_guard);
        drop(int_guard);
        drop(cfg_guard);

        // Consolidate unsummarized conversation turns (non-blocking on reconnect, awaited on shutdown)
        {
            let pending_turns = shared.lock().await.take_unsummarized_turns();
            if !pending_turns.is_empty() {
                let mem = config::load_memory();
                let lang = config::detect_language(&mem);
                let cfg_snap = cfg.clone();
                if should_shutdown {
                    memory::consolidate_session(&cfg_snap, &pending_turns, &lang, true).await;
                } else {
                    tokio::spawn(async move {
                        memory::consolidate_session(&cfg_snap, &pending_turns, &lang, false).await;
                    });
                }
            }
        }

        if should_shutdown {
            return;
        }

        if immediate_reconnect {
            tracing::info!("reconnecting immediately to apply updated settings...");
            backoff_secs = 2;
            continue;
        }

        tracing::info!("session ended, reconnecting in {}s...", backoff_secs);
        let _ = event_tx.send(AppEvent::Log(format!("reconnecting in {backoff_secs}s...")));
        if sleep_or_reconfig(
            backoff_secs,
            &mut cfg,
            &active_cfg,
            &latest_cfg,
            &config_rx,
            &shared,
            &audio_cmd_tx,
            &event_tx,
        )
        .await
        {
            backoff_secs = 2;
        } else {
            backoff_secs = (backoff_secs * 2).min(30);
        }
    }
}

/// Synthesizes crisp 24kHz mono i16 PCM UI chimes:
/// - `StartAck`: soft ascending crystal chime (E5 -> A5, 170ms) when a text study/doc task starts.
/// - `ScreenCaptured`: bright crisp shutter double-pip (C6 -> G6, 130ms) fired the exact millisecond
///   AFTER screen capture finishes in RAM so the user knows it's 100% safe to Alt+Tab!
fn synth_chime_pcm(kind: ChimeKind) -> Vec<u8> {
    let sr = RECV_SAMPLE_RATE as f32;
    let (dur_total, split, f1, f2, amp) = match kind {
        ChimeKind::StartAck => (0.17_f32, 0.075_f32, 659.25_f32, 880.0_f32, 0.17_f32),
        ChimeKind::ScreenCaptured => (0.13_f32, 0.055_f32, 1046.5_f32, 1567.98_f32, 0.20_f32),
    };
    let total = (sr * dur_total) as usize;
    let mut bytes = Vec::with_capacity(total * 2);
    for i in 0..total {
        let t = i as f32 / sr;
        let (freq, local_t, dur) = if t < split {
            (f1, t, split)
        } else {
            (f2, t - split, dur_total - split)
        };
        let env = (local_t / 0.008).min(1.0) * (1.0 - (local_t / dur).clamp(0.0, 1.0)).powf(1.7);
        let wave = (std::f32::consts::TAU * freq * t).sin() * 0.74
            + (std::f32::consts::TAU * freq * 2.0 * t).sin() * 0.16;
        let sample = (wave * env * amp * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
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
    // Guarded by AbortOnDrop so timeout or session reconnect ALWAYS cancels the inner tool task
    // and prevents zombie Save As dialogs from popping up after reconnect!
    let mut handle = AbortOnDrop(tokio::spawn(
        async move { t.execute(&tool_name, args).await },
    ));

    let timeout_secs = match name {
        "study_solve" | "study_board" | "generate_document" | "write_file" | "generate_qrcode" => {
            STUDY_TOOL_TIMEOUT_SECS
        }
        _ => TOOL_EXEC_TIMEOUT_SECS,
    };

    let result = match tokio::time::timeout(Duration::from_secs(timeout_secs), &mut handle.0).await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => tools::ToolResult::err(format!("tool panicked: {e}")),
        Err(_) => {
            handle.0.abort();
            tools::ToolResult::err(format!("tool '{name}' timed out after {timeout_secs}s"))
        }
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
