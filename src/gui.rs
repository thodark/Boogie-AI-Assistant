use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId,
    Order, Pos2, Rect, ResizeDirection, RichText, Sense, Stroke, Vec2, ViewportCommand,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::sync::{Mutex, broadcast, mpsc};

use crate::config::{ApiConfig, ColorTheme, GraphicsQuality};
use crate::orb_shader::{OrbRenderer, OrbUniforms};
use crate::theme::{self, ThemePalette};
use crate::{AppEvent, AppState, ConfirmResponse, SharedState};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- Base Obsidian Palette (Dynamic theme accents come from `crate::theme::ThemePalette`) ---
const BG_OBSIDIAN: Color32 = Color32::from_rgb(7, 6, 16);
const VIOLET: Color32 = Color32::from_rgb(148, 108, 255);
const MINT: Color32 = Color32::from_rgb(64, 242, 184);
const AMBER: Color32 = Color32::from_rgb(255, 162, 72);
const CRIMSON: Color32 = Color32::from_rgb(255, 76, 114);
const TEXT_BRIGHT: Color32 = Color32::from_rgb(240, 238, 255);
const TEXT_SOFT: Color32 = Color32::from_rgb(168, 162, 204);
const TEXT_MUTED: Color32 = Color32::from_rgb(104, 98, 138);

const HEADER_H: f32 = 42.0;
const RESIZE_MARGIN: f32 = 6.0;
const CAPTION_FADE_START: f64 = 10.0;
const CAPTION_FADE_END: f64 = 18.0;
const NUM_SPEC_BARS: usize = 14;
const APP_LOGO_PNG: &[u8] = include_bytes!("../assets/logo_icon.png");

#[derive(Clone, PartialEq)]
enum LogKind {
    User,
    Boogie,
    Tool,
    System,
}

struct ChatEntry {
    kind: LogKind,
    title: String,
    body: String,
    updated_at: f64,
}

#[derive(Clone)]
struct ConfirmDisplay {
    request_id: u64,
    title: String,
    detail: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Memory,
}

struct SetupForm {
    is_settings: bool,
    active_tab: SettingsTab,
    base_config: ApiConfig,
    api_key: String,
    show_api_key: bool,
    graphics_quality: GraphicsQuality,
    color_theme: ColorTheme,
    voice: String,
    assistant_name: String,
    user_name: String,
    input_device: String,
    output_device: String,
    push_to_talk: bool,
    proactive_audio: bool,
    auto_memory_consolidation: bool,
    available_inputs: Vec<String>,
    available_outputs: Vec<String>,
    memory_filter: String,
    memory_rows: Vec<crate::memory::MemoryRow>,
    recent_sessions: Vec<crate::memory::SessionSummary>,
    confirm_clear_memory: bool,
    error_msg: Option<String>,
}

impl SetupForm {
    fn new_setup(initial_error: Option<String>) -> Self {
        let available_inputs = crate::audio::list_input_devices()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let available_outputs = crate::audio::list_output_devices()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        Self {
            is_settings: false,
            active_tab: SettingsTab::General,
            base_config: ApiConfig::default(),
            api_key: String::new(),
            show_api_key: false,
            graphics_quality: GraphicsQuality::High,
            color_theme: ColorTheme::Obsidian,
            voice: "Aoede".to_string(),
            assistant_name: "Boogie".to_string(),
            user_name: String::new(),
            input_device: String::new(),
            output_device: String::new(),
            push_to_talk: false,
            proactive_audio: true,
            auto_memory_consolidation: true,
            available_inputs,
            available_outputs,
            memory_filter: String::new(),
            memory_rows: Vec::new(),
            recent_sessions: Vec::new(),
            confirm_clear_memory: false,
            error_msg: initial_error,
        }
    }

    fn from_config(cfg: Option<&ApiConfig>) -> Self {
        let available_inputs = crate::audio::list_input_devices()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let available_outputs = crate::audio::list_output_devices()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let base = cfg.cloned().unwrap_or_default();
        Self {
            is_settings: true,
            active_tab: SettingsTab::General,
            api_key: base.gemini_api_key.clone(),
            show_api_key: false,
            graphics_quality: base.graphics_quality(),
            color_theme: base.color_theme(),
            voice: base.voice().to_string(),
            assistant_name: base.assistant_name().to_string(),
            user_name: base.user_name.clone().unwrap_or_default(),
            input_device: base.input_device.clone().unwrap_or_default(),
            output_device: base.output_device.clone().unwrap_or_default(),
            push_to_talk: base.push_to_talk(),
            proactive_audio: base.proactive_audio(),
            auto_memory_consolidation: base.auto_memory_consolidation(),
            available_inputs,
            available_outputs,
            memory_filter: String::new(),
            memory_rows: crate::memory::list_memory_rows(),
            recent_sessions: crate::memory::load_recent_sessions(10),
            confirm_clear_memory: false,
            error_msg: None,
            base_config: base,
        }
    }

    fn reload_memories(&mut self) {
        self.memory_rows = crate::memory::list_memory_rows();
        self.recent_sessions = crate::memory::load_recent_sessions(10);
        self.confirm_clear_memory = false;
    }
}

struct SysMetrics {
    sys: sysinfo::System,
    cpu: f32,
    mem_used_gb: f64,
    last_update: Instant,
}

impl SysMetrics {
    fn new() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let mem_used_gb = sys.used_memory() as f64 / 1_073_741_824.0;
        Self {
            sys,
            cpu: 0.0,
            mem_used_gb,
            last_update: Instant::now() - std::time::Duration::from_secs(2),
        }
    }

    fn refresh(&mut self) {
        if self.last_update.elapsed().as_secs() < 2 {
            return;
        }
        self.last_update = Instant::now();
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.cpu = self.sys.global_cpu_usage().clamp(0.0, 100.0);
        self.mem_used_gb = self.sys.used_memory() as f64 / 1_073_741_824.0;
    }
}

pub struct BoogieGui {
    state: AppState,
    raw_audio_level: f32,
    viseme_queue: VecDeque<(f32, f32, f32)>,
    viseme_hop_secs: f64,
    viseme_accum: f64,

    // Smooth animated visual parameters
    smooth_audio: f32,
    smooth_openness: f32,
    smooth_width: f32,
    state_weights: [f32; 4], // [listening, speaking, thinking, sleeping]
    smooth_muted: f32,
    smooth_mouse: [f32; 2],
    click_pulse: f32,
    drawer_open: bool,
    drawer_anim: f32,
    spec_bars: [f32; NUM_SPEC_BARS],

    // Graphics tier & color theme (with smooth live-morphing shader colors)
    graphics_quality: GraphicsQuality,
    color_theme: ColorTheme,
    smooth_col_primary: [f32; 3],
    smooth_col_secondary: [f32; 3],
    smooth_col_accent: [f32; 3],
    smooth_bg_center: [f32; 3],
    smooth_bg_edge: [f32; 3],
    smooth_rainbow: f32,

    entries: Vec<ChatEntry>,
    event_rx: broadcast::Receiver<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    confirm_tx: mpsc::Sender<ConfirmResponse>,
    interrupt_tx: mpsc::Sender<()>,
    config_tx: mpsc::Sender<ApiConfig>,
    current_cfg: Option<ApiConfig>,
    muted: bool,
    push_to_talk: bool,
    ptt_holding: bool,
    confirm_pending: Option<ConfirmDisplay>,
    banner_error: Option<String>,
    setup_form: Option<SetupForm>,
    shutdown_requested: bool,
    scroll_to_bottom: bool,
    metrics: SysMetrics,

    orb_renderer: Option<Arc<StdMutex<OrbRenderer>>>,
    logo_texture: Option<egui::TextureHandle>,
    last_frame_time: f64,
    egui_time: f64,
}

impl BoogieGui {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        event_rx: broadcast::Receiver<AppEvent>,
        shared: Arc<Mutex<SharedState>>,
        confirm_tx: mpsc::Sender<ConfirmResponse>,
        interrupt_tx: mpsc::Sender<()>,
        config_tx: mpsc::Sender<ApiConfig>,
        initial_cfg: Option<ApiConfig>,
        initial_cfg_err: Option<String>,
    ) -> Self {
        setup_fonts(&cc.egui_ctx);
        apply_obsidian_theme(&cc.egui_ctx);

        let logo_texture = load_logo_texture(&cc.egui_ctx);

        let initial_quality = initial_cfg
            .as_ref()
            .map(ApiConfig::graphics_quality)
            .unwrap_or_default();
        let initial_theme = initial_cfg
            .as_ref()
            .map(ApiConfig::color_theme)
            .unwrap_or_default();
        let initial_pal = ThemePalette::for_theme(initial_theme);

        let orb_renderer =
            cc.gl
                .as_ref()
                .and_then(|gl| match OrbRenderer::new(gl, initial_quality) {
                    Ok(r) => {
                        tracing::info!(
                            "3D Liquid Glass Orb GLSL shader compiled (quality={:?}, theme={:?})",
                            initial_quality,
                            initial_theme
                        );
                        Some(Arc::new(StdMutex::new(r)))
                    }
                    Err(e) => {
                        tracing::error!("Orb GLSL shader init failed (using 2D fallback): {e}");
                        None
                    }
                });

        let ptt = shared.try_lock().map(|s| s.push_to_talk).unwrap_or(false);
        let setup_form = initial_cfg_err.map(|e| SetupForm::new_setup(Some(e)));
        let initial_weights = if setup_form.is_some() {
            [1.0, 0.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0, 0.0]
        };
        let (init_p, init_s, init_a) = initial_pal.blended_shader_colors(initial_weights, 0.0);

        Self {
            state: if setup_form.is_some() {
                AppState::Listening
            } else {
                AppState::Thinking
            },
            raw_audio_level: 0.0,
            viseme_queue: VecDeque::new(),
            viseme_hop_secs: 0.02,
            viseme_accum: 0.0,

            smooth_audio: 0.0,
            smooth_openness: 0.0,
            smooth_width: 0.0,
            state_weights: initial_weights,
            smooth_muted: 0.0,
            smooth_mouse: [0.0, 0.0],
            click_pulse: 0.0,
            drawer_open: false,
            drawer_anim: 0.0,
            spec_bars: [0.0; NUM_SPEC_BARS],

            graphics_quality: initial_quality,
            color_theme: initial_theme,
            smooth_col_primary: init_p,
            smooth_col_secondary: init_s,
            smooth_col_accent: init_a,
            smooth_bg_center: initial_pal.bg_center,
            smooth_bg_edge: initial_pal.bg_edge,
            smooth_rainbow: initial_pal.rainbow,

            entries: Vec::new(),
            event_rx,
            shared,
            confirm_tx,
            interrupt_tx,
            config_tx,
            current_cfg: initial_cfg,
            muted: false,
            push_to_talk: ptt,
            ptt_holding: false,
            confirm_pending: None,
            banner_error: None,
            setup_form,
            shutdown_requested: false,
            scroll_to_bottom: false,
            metrics: SysMetrics::new(),

            orb_renderer,
            logo_texture,
            last_frame_time: 0.0,
            egui_time: 0.0,
        }
    }

    fn process_events(&mut self) {
        loop {
            match self.event_rx.try_recv() {
                Ok(event) => match event {
                    AppEvent::StateChange(s) => {
                        if s != AppState::Speaking {
                            self.viseme_queue.clear();
                        }
                        self.state = s;
                    }
                    AppEvent::AudioLevel(l) => {
                        self.raw_audio_level = l.clamp(0.0, 1.0);
                    }
                    AppEvent::Visemes(frames, hop_secs, _) => {
                        if hop_secs > 0.002 {
                            self.viseme_hop_secs = hop_secs;
                        }
                        for frame in frames {
                            if self.viseme_queue.len() > 9000 {
                                self.viseme_queue.pop_front();
                            }
                            self.viseme_queue.push_back(frame);
                        }
                    }
                    AppEvent::ConfirmRequest {
                        request_id,
                        title,
                        detail,
                    } => {
                        self.confirm_pending = Some(ConfirmDisplay {
                            request_id,
                            title,
                            detail,
                        });
                        self.click_pulse = 0.85;
                    }
                    AppEvent::BannerError(err) => {
                        self.banner_error = err;
                    }
                    AppEvent::TimerAlert(_) => {
                        self.click_pulse = 1.0;
                    }
                    AppEvent::Shutdown => {
                        self.shutdown_requested = true;
                    }
                    AppEvent::Log(msg) => {
                        self.push_log_message(msg);
                    }
                },
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    tracing::warn!("GUI event_rx lagged by {n} events");
                }
                Err(
                    broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed,
                ) => break,
            }
        }
    }

    fn push_log_message(&mut self, msg: String) {
        let now = self.egui_time;

        if let Some(rest) = msg.strip_prefix("User: ") {
            let chunk = rest.trim();
            if chunk.is_empty() {
                return;
            }
            if let Some(last) = self.entries.last_mut()
                && last.kind == LogKind::User
                && (now - last.updated_at) < 4.5
            {
                append_transcript_chunk(&mut last.body, chunk);
                last.updated_at = now;
                self.scroll_to_bottom = true;
                return;
            }
            self.entries.push(ChatEntry {
                kind: LogKind::User,
                title: "You".to_string(),
                body: chunk.to_string(),
                updated_at: now,
            });
        } else if let Some(rest) = msg.strip_prefix("Boogie: ") {
            let chunk = rest.trim();
            if chunk.is_empty() {
                return;
            }
            if let Some(last) = self.entries.last_mut()
                && last.kind == LogKind::Boogie
                && (now - last.updated_at) < 5.0
            {
                append_transcript_chunk(&mut last.body, chunk);
                last.updated_at = now;
                self.scroll_to_bottom = true;
                return;
            }
            self.entries.push(ChatEntry {
                kind: LogKind::Boogie,
                title: "Boogie".to_string(),
                body: chunk.to_string(),
                updated_at: now,
            });
        } else if let Some(rest) = msg.strip_prefix("tool: ") {
            let (tool_title, tool_detail) = format_tool_log(rest);
            self.entries.push(ChatEntry {
                kind: LogKind::Tool,
                title: tool_title,
                body: tool_detail,
                updated_at: now,
            });
        } else {
            let trimmed = msg.trim();
            if trimmed.is_empty() {
                return;
            }
            self.entries.push(ChatEntry {
                kind: LogKind::System,
                title: String::new(),
                body: trimmed.to_string(),
                updated_at: now,
            });
        }

        self.scroll_to_bottom = true;
        if self.entries.len() > 300 {
            self.entries.drain(..60);
        }
    }

    fn active_quality(&self) -> GraphicsQuality {
        self.setup_form
            .as_ref()
            .map_or(self.graphics_quality, |f| f.graphics_quality)
    }

    fn active_theme(&self) -> ColorTheme {
        self.setup_form
            .as_ref()
            .map_or(self.color_theme, |f| f.color_theme)
    }

    fn active_palette(&self) -> ThemePalette {
        ThemePalette::for_theme(self.active_theme())
    }

    fn update_animations(&mut self, ctx: &egui::Context, dt: f32) {
        // Advance viseme playback queue at hop rate
        let (vis_level, vis_open, vis_width) = if !self.viseme_queue.is_empty() {
            self.viseme_accum += dt as f64;
            let mut current = *self.viseme_queue.front().unwrap();
            while self.viseme_accum >= self.viseme_hop_secs && !self.viseme_queue.is_empty() {
                self.viseme_accum -= self.viseme_hop_secs;
                current = self.viseme_queue.pop_front().unwrap();
            }
            current
        } else {
            self.viseme_accum = 0.0;
            (0.0, 0.0, 0.0)
        };

        // Combine mic input and Boogie speech output energy
        let target_audio = match self.state {
            AppState::Speaking => vis_level.max(self.raw_audio_level * 0.45),
            AppState::Listening => {
                if self.muted {
                    0.0
                } else {
                    self.raw_audio_level
                }
            }
            AppState::Thinking => 0.14 + 0.08 * (self.egui_time as f32 * 3.2).sin().abs(),
            AppState::Sleeping => 0.0,
        };

        let audio_rate = if target_audio > self.smooth_audio {
            20.0
        } else {
            6.5
        };
        self.smooth_audio = exp_lerp(self.smooth_audio, target_audio, audio_rate, dt);
        self.smooth_openness = exp_lerp(self.smooth_openness, vis_open, 14.0, dt);
        self.smooth_width = exp_lerp(self.smooth_width, vis_width, 12.0, dt);

        // Smooth state blend weights
        let target_w = match self.state {
            AppState::Listening => [1.0, 0.0, 0.0, 0.0],
            AppState::Speaking => [0.0, 1.0, 0.0, 0.0],
            AppState::Thinking => [0.0, 0.0, 1.0, 0.0],
            AppState::Sleeping => [0.0, 0.0, 0.0, 1.0],
        };
        for (w, target) in self.state_weights.iter_mut().zip(target_w.iter()) {
            *w = exp_lerp(*w, *target, 3.8, dt);
        }

        self.smooth_muted = exp_lerp(
            self.smooth_muted,
            if self.muted { 1.0 } else { 0.0 },
            7.0,
            dt,
        );

        // Smoothly morph shader palette across state & theme preset changes
        let pal = self.active_palette();
        let (target_p, target_s, target_a) =
            pal.blended_shader_colors(self.state_weights, self.smooth_muted);
        exp_lerp_rgb(&mut self.smooth_col_primary, target_p, 6.0, dt);
        exp_lerp_rgb(&mut self.smooth_col_secondary, target_s, 6.0, dt);
        exp_lerp_rgb(&mut self.smooth_col_accent, target_a, 6.0, dt);
        exp_lerp_rgb(&mut self.smooth_bg_center, pal.bg_center, 5.0, dt);
        exp_lerp_rgb(&mut self.smooth_bg_edge, pal.bg_edge, 5.0, dt);
        self.smooth_rainbow = exp_lerp(self.smooth_rainbow, pal.rainbow, 5.5, dt);

        self.drawer_anim = exp_lerp(
            self.drawer_anim,
            if self.drawer_open { 1.0 } else { 0.0 },
            9.5,
            dt,
        );
        self.click_pulse = exp_lerp(self.click_pulse, 0.0, 4.2, dt);

        // Smooth mouse parallax normalized to [-1, 1]
        let screen = ctx.screen_rect();
        let target_mouse = ctx.input(|i| {
            i.pointer.hover_pos().map_or([0.0, 0.0], |pos| {
                let nx = ((pos.x - screen.center().x) / (screen.width().max(1.0) * 0.5))
                    .clamp(-1.0, 1.0);
                let ny = -((pos.y - screen.center().y) / (screen.height().max(1.0) * 0.5))
                    .clamp(-1.0, 1.0);
                [nx, ny]
            })
        });
        self.smooth_mouse[0] = exp_lerp(self.smooth_mouse[0], target_mouse[0], 5.5, dt);
        self.smooth_mouse[1] = exp_lerp(self.smooth_mouse[1], target_mouse[1], 5.5, dt);

        // Update 14-bar dock spectrum visualizer
        let t = self.egui_time as f32;
        let energy = self.smooth_audio;
        let open = self.smooth_openness;
        for (i, bar) in self.spec_bars.iter_mut().enumerate() {
            let norm = (i as f32 + 0.5) / NUM_SPEC_BARS as f32;
            let center_env = 1.0 - (norm - 0.5).abs() * 1.65;
            let center_env = center_env.clamp(0.18, 1.0);

            let wave1 = (t * 6.5 + i as f32 * 0.68).sin() * 0.5 + 0.5;
            let wave2 = (t * 10.2 - i as f32 * 1.15).cos() * 0.5 + 0.5;
            let idle = if self.muted {
                0.08
            } else {
                0.12 + 0.08 * (t * 2.2 + i as f32 * 0.45).sin().abs()
            };
            let active = (energy * 0.85 + open * 0.35) * (0.42 + 0.42 * wave1 + 0.28 * wave2);
            let target = (idle + active * center_env * 1.35).clamp(0.08, 1.0);
            *bar = exp_lerp(*bar, target, 18.0, dt);
        }
    }

    fn state_color(&self) -> Color32 {
        if self.muted {
            return CRIMSON;
        }
        let pal = self.active_palette();
        if self.push_to_talk && self.state == AppState::Listening {
            return if self.ptt_holding {
                pal.ui_speaking
            } else {
                pal.ui_primary
            };
        }
        match self.state {
            AppState::Listening => pal.ui_primary,
            AppState::Speaking => pal.ui_secondary,
            AppState::Thinking => pal.ui_thinking,
            AppState::Sleeping => TEXT_MUTED,
        }
    }

    fn state_label(&self) -> &'static str {
        if self.muted {
            return "MUTED";
        }
        if self.push_to_talk && self.state == AppState::Listening {
            return if self.ptt_holding {
                "PTT LIVE"
            } else {
                "HOLD SPACE"
            };
        }
        match self.state {
            AppState::Listening => "LISTENING",
            AppState::Speaking => "SPEAKING",
            AppState::Thinking => "THINKING",
            AppState::Sleeping => "SLEEPING",
        }
    }

    /// Enables full 8-direction window resizing on frameless windows.
    fn handle_frameless_resize(&self, ctx: &egui::Context, screen: Rect) {
        let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if is_max {
            return;
        }

        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };

        let left = pos.x <= screen.left() + RESIZE_MARGIN;
        let right = pos.x >= screen.right() - RESIZE_MARGIN;
        let top = pos.y <= screen.top() + RESIZE_MARGIN;
        let bottom = pos.y >= screen.bottom() - RESIZE_MARGIN;

        let dir = match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        };

        if let Some(d) = dir {
            let cursor = match d {
                ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
                ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
                ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
                ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
            };
            ctx.set_cursor_icon(cursor);

            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
            }
        }
    }

    fn paint_orb_backdrop(&self, ui: &mut egui::Ui, rect: Rect) {
        // Center Y in OpenGL bottom-up normalized coordinates [0..1]
        // Shifts gently upward when the transcript drawer slides open
        let center_y = 0.58 + self.drawer_anim * 0.16;
        let orb_scale = (0.43 - self.drawer_anim * 0.11).clamp(0.24, 0.48);

        let uniforms = OrbUniforms {
            time: self.egui_time as f32,
            orb_center: [0.5, center_y],
            orb_scale,
            audio: self.smooth_audio,
            openness: self.smooth_openness,
            width: self.smooth_width,
            state_weights: self.state_weights,
            mouse: self.smooth_mouse,
            muted: self.smooth_muted,
            pulse: self.click_pulse,
            quality: self.active_quality(),
            col_primary: self.smooth_col_primary,
            col_secondary: self.smooth_col_secondary,
            col_accent: self.smooth_col_accent,
            bg_center: self.smooth_bg_center,
            bg_edge: self.smooth_bg_edge,
            rainbow: self.smooth_rainbow,
        };

        if let Some(renderer_arc) = &self.orb_renderer {
            let renderer = Arc::clone(renderer_arc);
            let cb = eframe::egui_glow::CallbackFn::new(move |info, painter| {
                let vp = info.viewport_in_pixels();
                if let Ok(mut r) = renderer.lock() {
                    r.paint(
                        painter.gl(),
                        [vp.left_px as f32, vp.from_bottom_px as f32],
                        [vp.width_px as f32, vp.height_px as f32],
                        &uniforms,
                    );
                }
            });

            ui.painter().add(egui::PaintCallback {
                rect,
                callback: Arc::new(cb),
            });
        } else {
            // Safe 2D fallback (using proper unmultiplied alpha)
            self.paint_fallback_2d_orb(ui.painter(), rect, center_y, orb_scale);
        }
    }

    fn paint_fallback_2d_orb(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center_y_gl: f32,
        orb_scale: f32,
    ) {
        let pal = self.active_palette();
        painter.rect_filled(rect, CornerRadius::ZERO, pal.ui_bg_tint);
        let center = Pos2::new(
            rect.center().x,
            rect.top() + (1.0 - center_y_gl) * rect.height(),
        );
        let r = rect.width().min(rect.height()) * orb_scale * 0.55;
        let col = self.state_color();
        let rim = pal.ui_secondary;
        for i in (1..=8).rev() {
            let rad = r + i as f32 * 10.0;
            let a = (24 - i * 2).max(2) as u8;
            painter.circle_filled(
                center,
                rad,
                Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), a),
            );
        }
        painter.circle_filled(center, r, Color32::from_rgba_unmultiplied(18, 14, 42, 235));
        painter.circle_stroke(
            center,
            r,
            Stroke::new(
                2.0_f32,
                Color32::from_rgba_unmultiplied(rim.r(), rim.g(), rim.b(), 180),
            ),
        );
    }

    fn draw_frameless_header(&mut self, ui: &mut egui::Ui, screen: Rect) {
        let header_rect =
            Rect::from_min_size(screen.left_top(), Vec2::new(screen.width(), HEADER_H));
        let painter = ui.painter();

        // Ultra-subtle frosted header wash + bottom hairline
        painter.rect_filled(
            header_rect,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(7, 6, 18, 95),
        );
        painter.line_segment(
            [header_rect.left_bottom(), header_rect.right_bottom()],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 14)),
        );

        let controls_w = 142.0;
        let drag_rect = Rect::from_min_max(
            Pos2::new(
                header_rect.left() + RESIZE_MARGIN,
                header_rect.top() + RESIZE_MARGIN,
            ),
            Pos2::new(header_rect.right() - controls_w, header_rect.bottom()),
        );
        let drag_resp = ui.interact(
            drag_rect,
            ui.id().with("titlebar_drag"),
            Sense::click_and_drag(),
        );
        if drag_resp.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag_resp.double_clicked() {
            let is_max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!is_max));
        }

        // Left: Brand mark + BOOGIE + Status Pill
        let pal = self.active_palette();
        let state_col = self.state_color();
        let pulse_alpha = (150.0 + 105.0 * (self.egui_time as f32 * 3.0).sin().abs()) as u8;

        let brand_x = header_rect.left() + 14.0;
        let cy = header_rect.center().y;

        // Neon Boogie B logo icon in header
        let logo_rect = Rect::from_center_size(Pos2::new(brand_x + 10.0, cy), Vec2::splat(22.0));
        if let Some(tex) = &self.logo_texture {
            painter.image(
                tex.id(),
                logo_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            painter.rect_stroke(
                logo_rect,
                CornerRadius::same(5),
                Stroke::new(
                    0.8_f32,
                    Color32::from_rgba_unmultiplied(
                        pal.ui_secondary.r(),
                        pal.ui_secondary.g(),
                        pal.ui_secondary.b(),
                        95,
                    ),
                ),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.circle_filled(
                logo_rect.center(),
                7.5,
                Color32::from_rgba_unmultiplied(
                    pal.ui_primary.r(),
                    pal.ui_primary.g(),
                    pal.ui_primary.b(),
                    40,
                ),
            );
            painter.circle_stroke(
                logo_rect.center(),
                5.0,
                Stroke::new(
                    1.3_f32,
                    Color32::from_rgba_unmultiplied(
                        pal.ui_secondary.r(),
                        pal.ui_secondary.g(),
                        pal.ui_secondary.b(),
                        210,
                    ),
                ),
            );
        }

        painter.text(
            Pos2::new(brand_x + 28.0, cy),
            Align2::LEFT_CENTER,
            "BOOGIE",
            FontId::new(12.0, FontFamily::Proportional),
            TEXT_BRIGHT,
        );

        // Status pill
        let pill_x = brand_x + 88.0;
        let label = self.state_label();
        let pill_w = 18.0 + label.len() as f32 * 6.4 + 12.0;
        let pill_rect = Rect::from_center_size(
            Pos2::new(pill_x + pill_w * 0.5, cy),
            Vec2::new(pill_w, 20.0),
        );
        painter.rect_filled(
            pill_rect,
            CornerRadius::same(10),
            Color32::from_rgba_unmultiplied(state_col.r(), state_col.g(), state_col.b(), 26),
        );
        painter.rect_stroke(
            pill_rect,
            CornerRadius::same(10),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(state_col.r(), state_col.g(), state_col.b(), 72),
            ),
            egui::StrokeKind::Inside,
        );
        // Pulsing status dot
        let dot_pos = Pos2::new(pill_rect.left() + 10.0, cy);
        painter.circle_filled(
            dot_pos,
            5.0,
            Color32::from_rgba_unmultiplied(state_col.r(), state_col.g(), state_col.b(), 48),
        );
        painter.circle_filled(
            dot_pos,
            2.6,
            Color32::from_rgba_unmultiplied(
                state_col.r(),
                state_col.g(),
                state_col.b(),
                pulse_alpha,
            ),
        );
        painter.text(
            Pos2::new(pill_rect.left() + 18.5, cy),
            Align2::LEFT_CENTER,
            label,
            FontId::new(9.5, FontFamily::Monospace),
            Color32::from_rgba_unmultiplied(state_col.r(), state_col.g(), state_col.b(), 235),
        );

        // Right: Telemetry (only show if window wide enough) + Window Control Buttons
        if screen.width() >= 430.0 {
            let telemetry = format!(
                "{:.0}% · {:.1}G",
                self.metrics.cpu, self.metrics.mem_used_gb
            );
            painter.text(
                Pos2::new(header_rect.right() - controls_w - 8.0, cy),
                Align2::RIGHT_CENTER,
                telemetry,
                FontId::new(10.0, FontFamily::Monospace),
                Color32::from_rgba_unmultiplied(
                    TEXT_MUTED.r(),
                    TEXT_MUTED.g(),
                    TEXT_MUTED.b(),
                    180,
                ),
            );
        }

        // Custom crisp vector window & settings buttons (Settings, Min, Max, Close)
        let btn_size = Vec2::new(32.0, 26.0);
        let right_edge = header_rect.right() - 8.0;

        let close_rect = Rect::from_center_size(Pos2::new(right_edge - 16.0, cy), btn_size);
        let max_rect = Rect::from_center_size(Pos2::new(right_edge - 50.0, cy), btn_size);
        let min_rect = Rect::from_center_size(Pos2::new(right_edge - 84.0, cy), btn_size);
        let settings_rect = Rect::from_center_size(Pos2::new(right_edge - 118.0, cy), btn_size);

        let settings_open = self.setup_form.as_ref().is_some_and(|f| f.is_settings);
        if draw_win_button(
            ui,
            settings_rect,
            "win_settings",
            WinBtnKind::Settings(settings_open),
            pal.ui_secondary,
        ) {
            if settings_open {
                self.setup_form = None;
            } else {
                self.setup_form = Some(SetupForm::from_config(self.current_cfg.as_ref()));
            }
        }
        if draw_win_button(
            ui,
            min_rect,
            "win_min",
            WinBtnKind::Minimize,
            pal.ui_secondary,
        ) {
            ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        let is_max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        if draw_win_button(
            ui,
            max_rect,
            "win_max",
            WinBtnKind::Maximize(is_max),
            pal.ui_secondary,
        ) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!is_max));
        }
        if draw_win_button(
            ui,
            close_rect,
            "win_close",
            WinBtnKind::Close,
            pal.ui_secondary,
        ) {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn draw_live_captions(&mut self, ui: &mut egui::Ui, screen: Rect, dock_top: f32) {
        if self.drawer_anim > 0.65 || self.entries.is_empty() {
            return;
        }
        let visibility = (1.0 - self.drawer_anim * 1.6).clamp(0.0, 1.0);
        let now = self.egui_time;

        // Show up to 3 most recent non-expired items cleanly centered above the dock
        let mut recent: Vec<&ChatEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| e.kind != LogKind::System || (now - e.updated_at) < 4.5)
            .take(3)
            .collect();
        recent.reverse();

        if recent.is_empty() {
            return;
        }

        let max_w = (screen.width() - 56.0).clamp(240.0, 620.0);
        let caption_area = Rect::from_min_max(
            Pos2::new(screen.center().x - max_w * 0.5, dock_top - 135.0),
            Pos2::new(screen.center().x + max_w * 0.5, dock_top - 14.0),
        );

        let mut cursor_y = caption_area.bottom();
        let painter = ui.painter();

        for entry in recent.iter().rev() {
            let age = now - entry.updated_at;
            let fade = if age < CAPTION_FADE_START {
                1.0_f32
            } else if age < CAPTION_FADE_END {
                1.0 - ((age - CAPTION_FADE_START) / (CAPTION_FADE_END - CAPTION_FADE_START)) as f32
            } else {
                0.0
            };
            let alpha = fade * visibility;
            if alpha < 0.02 {
                continue;
            }

            match entry.kind {
                LogKind::Tool => {
                    let chip_text = if entry.body.is_empty() {
                        format!("⚡ {}", entry.title)
                    } else {
                        format!("⚡ {} · {}", entry.title, truncate_str(&entry.body, 42))
                    };
                    let galley = ui.fonts(|f| {
                        f.layout_no_wrap(
                            chip_text,
                            FontId::new(11.0, FontFamily::Proportional),
                            Color32::from_rgba_unmultiplied(
                                AMBER.r(),
                                AMBER.g(),
                                AMBER.b(),
                                (alpha * 235.0) as u8,
                            ),
                        )
                    });
                    let pill_size = galley.rect.size() + Vec2::new(22.0, 10.0);
                    let pill_rect = Rect::from_center_size(
                        Pos2::new(screen.center().x, cursor_y - pill_size.y * 0.5),
                        pill_size,
                    );
                    painter.rect_filled(
                        pill_rect,
                        CornerRadius::same(12),
                        Color32::from_rgba_unmultiplied(24, 18, 36, (alpha * 185.0) as u8),
                    );
                    painter.rect_stroke(
                        pill_rect,
                        CornerRadius::same(12),
                        Stroke::new(
                            1.0_f32,
                            Color32::from_rgba_unmultiplied(
                                AMBER.r(),
                                AMBER.g(),
                                AMBER.b(),
                                (alpha * 80.0) as u8,
                            ),
                        ),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(
                        pill_rect.center() - galley.rect.size() * 0.5,
                        galley,
                        Color32::TRANSPARENT,
                    );
                    cursor_y -= pill_size.y + 8.0;
                }
                LogKind::Boogie | LogKind::User | LogKind::System => {
                    let (font_sz, base_col, prefix) = match entry.kind {
                        LogKind::Boogie => (15.0, TEXT_BRIGHT, ""),
                        LogKind::User => (12.5, Color32::from_rgb(196, 178, 255), "You: "),
                        LogKind::System => (11.0, TEXT_MUTED, ""),
                        LogKind::Tool => unreachable!(),
                    };
                    let display = format!("{prefix}{}", truncate_tail(&entry.body, 150));
                    let col = Color32::from_rgba_unmultiplied(
                        base_col.r(),
                        base_col.g(),
                        base_col.b(),
                        (alpha * 245.0) as u8,
                    );
                    let galley = ui.fonts(|f| {
                        f.layout(
                            display,
                            FontId::new(font_sz, FontFamily::Proportional),
                            col,
                            max_w - 24.0,
                        )
                    });
                    let h = galley.rect.height();
                    let text_pos =
                        Pos2::new(screen.center().x - galley.rect.width() * 0.5, cursor_y - h);
                    // Subtle shadow pill behind bright captions for effortless legibility
                    let bg_rect = Rect::from_min_size(text_pos, galley.rect.size())
                        .expand2(Vec2::new(12.0, 4.0));
                    painter.rect_filled(
                        bg_rect,
                        CornerRadius::same(10),
                        Color32::from_rgba_unmultiplied(6, 5, 16, (alpha * 115.0) as u8),
                    );
                    painter.galley(text_pos, galley, Color32::TRANSPARENT);
                    cursor_y -= h + 12.0;
                }
            }

            if cursor_y < caption_area.top() {
                break;
            }
        }

        // Clicking the caption area smoothly opens the full transcript drawer
        let resp = ui.interact(caption_area, ui.id().with("caption_click"), Sense::click());
        if resp.clicked() {
            self.drawer_open = true;
            self.scroll_to_bottom = true;
        }
    }

    fn draw_transcript_drawer(&mut self, ui: &mut egui::Ui, screen: Rect, dock_top: f32) {
        if self.drawer_anim < 0.02 {
            return;
        }

        let target_h = (screen.height() * 0.44).clamp(180.0, 380.0);
        let current_h = target_h * self.drawer_anim;
        let side_pad = ((screen.width() - 680.0) * 0.5).max(16.0);
        let drawer_rect = Rect::from_min_max(
            Pos2::new(screen.left() + side_pad, dock_top - current_h - 12.0),
            Pos2::new(screen.right() - side_pad, dock_top - 12.0),
        );

        let a = self.drawer_anim;
        let pal = self.active_palette();
        let painter = ui.painter();

        // Frosted obsidian glass sheet
        painter.rect_filled(
            drawer_rect,
            CornerRadius::same(18),
            Color32::from_rgba_unmultiplied(11, 10, 26, (a * 224.0) as u8),
        );
        painter.rect_stroke(
            drawer_rect,
            CornerRadius::same(18),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    pal.ui_primary.r(),
                    pal.ui_primary.g(),
                    pal.ui_primary.b(),
                    (a * 52.0) as u8,
                ),
            ),
            egui::StrokeKind::Inside,
        );
        // Subtle top glass specular highlight
        painter.line_segment(
            [
                Pos2::new(drawer_rect.left() + 24.0, drawer_rect.top() + 1.0),
                Pos2::new(drawer_rect.right() - 24.0, drawer_rect.top() + 1.0),
            ],
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    pal.ui_secondary.r(),
                    pal.ui_secondary.g(),
                    pal.ui_secondary.b(),
                    (a * 65.0) as u8,
                ),
            ),
        );

        if self.drawer_anim < 0.35 {
            return;
        }

        // Drawer mini header
        let bar_rect =
            Rect::from_min_size(drawer_rect.left_top(), Vec2::new(drawer_rect.width(), 32.0));
        painter.text(
            Pos2::new(bar_rect.left() + 16.0, bar_rect.center().y),
            Align2::LEFT_CENTER,
            "CONVERSATION",
            FontId::new(10.0, FontFamily::Monospace),
            Color32::from_rgba_unmultiplied(
                TEXT_MUTED.r(),
                TEXT_MUTED.g(),
                TEXT_MUTED.b(),
                (a * 220.0) as u8,
            ),
        );

        let close_pill = Rect::from_center_size(
            Pos2::new(bar_rect.right() - 22.0, bar_rect.center().y),
            Vec2::new(22.0, 22.0),
        );
        let close_resp = ui.interact(close_pill, ui.id().with("drawer_close"), Sense::click());
        if close_resp.hovered() {
            painter.circle_filled(
                close_pill.center(),
                10.0,
                Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            );
        }
        let cx = close_pill.center();
        let xcol = if close_resp.hovered() {
            TEXT_BRIGHT
        } else {
            TEXT_MUTED
        };
        painter.line_segment(
            [cx + Vec2::new(-3.5, -1.5), cx + Vec2::new(0.0, 2.0)],
            Stroke::new(1.4_f32, xcol),
        );
        painter.line_segment(
            [cx + Vec2::new(0.0, 2.0), cx + Vec2::new(3.5, -1.5)],
            Stroke::new(1.4_f32, xcol),
        );
        if close_resp.clicked() {
            self.drawer_open = false;
        }

        // Scrollable chat bubbles inside drawer
        let content_rect = Rect::from_min_max(
            Pos2::new(drawer_rect.left() + 12.0, bar_rect.bottom() + 2.0),
            Pos2::new(drawer_rect.right() - 10.0, drawer_rect.bottom() - 10.0),
        );

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        let scroll = egui::ScrollArea::vertical()
            .id_salt("boogie_transcript_scroll")
            .auto_shrink([false; 2])
            .stick_to_bottom(true);

        scroll.show(&mut child, |ui| {
            ui.set_min_width(ui.available_width());

            if self.entries.is_empty() {
                ui.add_space(28.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Say anything to Boogie — transcripts appear here.")
                            .size(12.5)
                            .color(TEXT_MUTED),
                    );
                });
                return;
            }

            let max_bubble_w = (ui.available_width() * 0.82).max(180.0);

            for entry in &self.entries {
                match entry.kind {
                    LogKind::User => {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            egui::Frame::new()
                                .fill(Color32::from_rgba_unmultiplied(
                                    pal.ui_primary.r(),
                                    pal.ui_primary.g(),
                                    pal.ui_primary.b(),
                                    44,
                                ))
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(
                                        pal.ui_primary.r(),
                                        pal.ui_primary.g(),
                                        pal.ui_primary.b(),
                                        82,
                                    ),
                                ))
                                .corner_radius(CornerRadius {
                                    nw: 14,
                                    ne: 14,
                                    sw: 14,
                                    se: 4,
                                })
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            ui.set_max_width(max_bubble_w);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&entry.body)
                                                        .size(13.0)
                                                        .color(TEXT_BRIGHT),
                                                )
                                                .wrap(),
                                            );
                                        },
                                    );
                                });
                        });
                    }
                    LogKind::Boogie => {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                            egui::Frame::new()
                                .fill(Color32::from_rgba_unmultiplied(22, 24, 48, 175))
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(
                                        pal.ui_secondary.r(),
                                        pal.ui_secondary.g(),
                                        pal.ui_secondary.b(),
                                        58,
                                    ),
                                ))
                                .corner_radius(CornerRadius {
                                    nw: 4,
                                    ne: 14,
                                    sw: 14,
                                    se: 14,
                                })
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            ui.set_max_width(max_bubble_w);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&entry.body)
                                                        .size(13.0)
                                                        .color(TEXT_BRIGHT),
                                                )
                                                .wrap(),
                                            );
                                        },
                                    );
                                });
                        });
                    }
                    LogKind::Tool => {
                        ui.horizontal_wrapped(|ui| {
                            egui::Frame::new()
                                .fill(Color32::from_rgba_unmultiplied(255, 162, 72, 22))
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(255, 162, 72, 65),
                                ))
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(egui::Margin::symmetric(9, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("⚡ {}", entry.title))
                                            .size(11.0)
                                            .color(AMBER)
                                            .strong(),
                                    );
                                    if !entry.body.is_empty() {
                                        ui.label(
                                            RichText::new(format!("· {}", entry.body))
                                                .size(11.0)
                                                .color(TEXT_SOFT),
                                        );
                                    }
                                });
                        });
                    }
                    LogKind::System => {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(&entry.body)
                                    .size(10.5)
                                    .color(TEXT_MUTED)
                                    .family(FontFamily::Monospace),
                            );
                        });
                    }
                }
                ui.add_space(6.0);
            }

            if self.scroll_to_bottom {
                let (_, bottom_rect) = ui.allocate_space(Vec2::new(1.0, 1.0));
                ui.scroll_to_rect(bottom_rect, Some(egui::Align::BOTTOM));
                if self.drawer_anim > 0.95 {
                    self.scroll_to_bottom = false;
                }
            }
        });
    }

    fn draw_floating_dock(&mut self, ui: &mut egui::Ui, screen: Rect) -> f32 {
        let dock_w = 304.0_f32.min(screen.width() - 32.0);
        let dock_h = 54.0_f32;
        let dock_rect = Rect::from_center_size(
            Pos2::new(screen.center().x, screen.bottom() - 18.0 - dock_h * 0.5),
            Vec2::new(dock_w, dock_h),
        );

        let painter = ui.painter();

        // Soft drop glow under dock
        painter.rect_filled(
            dock_rect.expand(4.0),
            CornerRadius::same(30),
            Color32::from_rgba_unmultiplied(0, 0, 0, 95),
        );
        // Frosted glass pill container
        painter.rect_filled(
            dock_rect,
            CornerRadius::same(27),
            Color32::from_rgba_unmultiplied(14, 13, 30, 212),
        );
        painter.rect_stroke(
            dock_rect,
            CornerRadius::same(27),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(180, 170, 255, 42)),
            egui::StrokeKind::Inside,
        );
        // Crisp specular top highlight line
        painter.line_segment(
            [
                Pos2::new(dock_rect.left() + 28.0, dock_rect.top() + 1.0),
                Pos2::new(dock_rect.right() - 28.0, dock_rect.top() + 1.0),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
        );

        let cy = dock_rect.center().y;
        let pal = self.active_palette();

        // 1. Mic Toggle Button (left)
        let mic_center = Pos2::new(dock_rect.left() + 30.0, cy);
        let mic_rect = Rect::from_center_size(mic_center, Vec2::splat(38.0));
        let mic_resp = ui
            .interact(mic_rect, ui.id().with("dock_mic"), Sense::click())
            .on_hover_text(if self.muted {
                "Unmute microphone"
            } else {
                "Mute microphone"
            });

        let mic_col = if self.muted {
            CRIMSON
        } else {
            pal.ui_secondary
        };
        let mic_fill_a = if mic_resp.hovered() { 58 } else { 32 };
        painter.circle_filled(
            mic_center,
            19.0,
            Color32::from_rgba_unmultiplied(mic_col.r(), mic_col.g(), mic_col.b(), mic_fill_a),
        );
        painter.circle_stroke(
            mic_center,
            19.0,
            Stroke::new(
                1.1_f32,
                Color32::from_rgba_unmultiplied(mic_col.r(), mic_col.g(), mic_col.b(), 110),
            ),
        );
        draw_mic_icon(painter, mic_center, mic_col, self.muted);

        if mic_resp.clicked() {
            self.muted = !self.muted;
            self.click_pulse = 0.55;
            if let Ok(mut s) = self.shared.try_lock() {
                s.muted = self.muted;
            }
        }

        // 2. Live 14-Bar Spectrum Visualizer (center)
        let spec_left = dock_rect.left() + 64.0;
        let spec_right = dock_rect.right() - 102.0;
        let spec_w = (spec_right - spec_left).max(40.0);
        let step = spec_w / NUM_SPEC_BARS as f32;
        let bar_w = (step * 0.46).clamp(2.2, 4.0);
        let max_bar_h = 26.0_f32;
        let t_now = self.egui_time as f32;

        for (i, &val) in self.spec_bars.iter().enumerate() {
            let x = spec_left + (i as f32 + 0.5) * step;
            let h = (val * max_bar_h).clamp(3.0, max_bar_h);
            let frac = i as f32 / (NUM_SPEC_BARS - 1) as f32;
            let base = if self.muted {
                Color32::from_rgba_unmultiplied(CRIMSON.r(), CRIMSON.g(), CRIMSON.b(), 95)
            } else {
                let c = if pal.rainbow > 0.5 {
                    ThemePalette::rainbow_srgb(frac * 0.85 + t_now * 0.16)
                } else {
                    match self.state {
                        AppState::Speaking => lerp_color(pal.ui_secondary, pal.ui_speaking, frac),
                        AppState::Thinking => lerp_color(pal.ui_primary, pal.ui_thinking, frac),
                        _ => lerp_color(pal.ui_primary, pal.ui_secondary, frac),
                    }
                };
                Color32::from_rgba_unmultiplied(
                    c.r(),
                    c.g(),
                    c.b(),
                    (120.0 + val * 135.0).min(255.0) as u8,
                )
            };

            let r = Rect::from_center_size(Pos2::new(x, cy), Vec2::new(bar_w, h));
            painter.rect_filled(r, CornerRadius::same(2), base);
        }

        // 3. Interrupt / Stop Button
        let stop_center = Pos2::new(dock_rect.right() - 72.0, cy);
        let stop_rect = Rect::from_center_size(stop_center, Vec2::splat(34.0));
        let stop_resp = ui
            .interact(stop_rect, ui.id().with("dock_stop"), Sense::click())
            .on_hover_text("Interrupt / Stop speaking");

        let is_speaking = self.state == AppState::Speaking;
        let stop_fill_a = if stop_resp.hovered() {
            52
        } else if is_speaking {
            34
        } else {
            16
        };
        let stop_col = if stop_resp.hovered() || is_speaking {
            CRIMSON
        } else {
            TEXT_SOFT
        };
        painter.circle_filled(
            stop_center,
            17.0,
            Color32::from_rgba_unmultiplied(CRIMSON.r(), CRIMSON.g(), CRIMSON.b(), stop_fill_a),
        );
        painter.circle_stroke(
            stop_center,
            17.0,
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    stop_col.r(),
                    stop_col.g(),
                    stop_col.b(),
                    if is_speaking || stop_resp.hovered() {
                        115
                    } else {
                        36
                    },
                ),
            ),
        );
        painter.rect_filled(
            Rect::from_center_size(stop_center, Vec2::splat(9.5)),
            CornerRadius::same(2),
            stop_col,
        );

        if stop_resp.clicked() {
            self.click_pulse = 0.7;
            self.viseme_queue.clear();
            let _ = self.interrupt_tx.try_send(());
        }

        // 4. Transcript Drawer Toggle Button (right)
        let chat_center = Pos2::new(dock_rect.right() - 30.0, cy);
        let chat_rect = Rect::from_center_size(chat_center, Vec2::splat(34.0));
        let chat_resp = ui
            .interact(chat_rect, ui.id().with("dock_history"), Sense::click())
            .on_hover_text(if self.drawer_open {
                "Hide conversation"
            } else {
                "Show conversation"
            });

        let chat_col = if self.drawer_open {
            pal.ui_secondary
        } else if chat_resp.hovered() {
            TEXT_BRIGHT
        } else {
            TEXT_SOFT
        };
        let chat_bg = if self.drawer_open {
            Color32::from_rgba_unmultiplied(
                pal.ui_secondary.r(),
                pal.ui_secondary.g(),
                pal.ui_secondary.b(),
                38,
            )
        } else if chat_resp.hovered() {
            Color32::from_rgba_unmultiplied(255, 255, 255, 20)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 8)
        };
        painter.circle_filled(chat_center, 17.0, chat_bg);
        painter.circle_stroke(
            chat_center,
            17.0,
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    chat_col.r(),
                    chat_col.g(),
                    chat_col.b(),
                    if self.drawer_open { 110 } else { 38 },
                ),
            ),
        );
        draw_transcript_icon(painter, chat_center, chat_col);

        // Tiny glowing unread badge if conversation has messages and drawer is closed
        if !self.drawer_open && !self.entries.is_empty() {
            painter.circle_filled(chat_center + Vec2::new(9.5, -9.5), 3.0, pal.ui_primary);
        }

        if chat_resp.clicked() {
            self.drawer_open = !self.drawer_open;
            if self.drawer_open {
                self.scroll_to_bottom = true;
            }
        }

        dock_rect.top()
    }

    fn draw_confirm_dialog(&mut self, ctx: &egui::Context) {
        let pending = match &self.confirm_pending {
            Some(c) => c.clone(),
            None => return,
        };

        // Dim scrim over entire window
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("confirm_scrim"))
            .fixed_pos(screen.left_top())
            .order(Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(4, 3, 12, 170),
                );
                ui.allocate_rect(screen, Sense::click());
            });

        let mut response = None;

        egui::Window::new("confirm_action")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(15, 13, 32, 244))
                    .stroke(Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(AMBER.r(), AMBER.g(), AMBER.b(), 120),
                    ))
                    .corner_radius(CornerRadius::same(18))
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(280.0);
                ui.set_max_width(360.0);

                ui.label(
                    RichText::new("CONFIRMATION REQUIRED")
                        .size(10.0)
                        .color(AMBER)
                        .family(FontFamily::Monospace),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(&pending.title)
                        .size(15.5)
                        .color(TEXT_BRIGHT)
                        .strong(),
                );
                if !pending.detail.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&pending.detail).size(12.0).color(TEXT_SOFT));
                }

                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Approve")
                                    .size(12.5)
                                    .color(BG_OBSIDIAN)
                                    .strong(),
                            )
                            .fill(MINT)
                            .corner_radius(CornerRadius::same(14))
                            .min_size(Vec2::new(104.0, 34.0)),
                        )
                        .clicked()
                    {
                        response = Some(ConfirmResponse {
                            request_id: pending.request_id,
                            confirmed: true,
                        });
                    }
                    ui.add_space(10.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Deny").size(12.5).color(CRIMSON).strong(),
                            )
                            .fill(Color32::from_rgba_unmultiplied(
                                CRIMSON.r(),
                                CRIMSON.g(),
                                CRIMSON.b(),
                                26,
                            ))
                            .stroke(Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(
                                    CRIMSON.r(),
                                    CRIMSON.g(),
                                    CRIMSON.b(),
                                    95,
                                ),
                            ))
                            .corner_radius(CornerRadius::same(14))
                            .min_size(Vec2::new(96.0, 34.0)),
                        )
                        .clicked()
                    {
                        response = Some(ConfirmResponse {
                            request_id: pending.request_id,
                            confirmed: false,
                        });
                    }
                });
            });

        if let Some(resp) = response {
            let _ = self.confirm_tx.try_send(resp);
            self.confirm_pending = None;
        }
    }

    fn draw_error_banner(&mut self, ui: &mut egui::Ui, screen: Rect) {
        let Some(msg) = self.banner_error.clone() else {
            return;
        };
        let max_w = (screen.width() - 36.0).clamp(240.0, 560.0);
        let banner_rect = Rect::from_center_size(
            Pos2::new(screen.center().x, screen.top() + HEADER_H + 24.0),
            Vec2::new(max_w, 32.0),
        );
        let painter = ui.painter();
        painter.rect_filled(
            banner_rect,
            CornerRadius::same(12),
            Color32::from_rgba_unmultiplied(46, 12, 24, 224),
        );
        painter.rect_stroke(
            banner_rect,
            CornerRadius::same(12),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(CRIMSON.r(), CRIMSON.g(), CRIMSON.b(), 145),
            ),
            egui::StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(banner_rect.left() + 14.0, banner_rect.center().y),
            Align2::LEFT_CENTER,
            truncate_str(&msg, 72),
            FontId::new(11.5, FontFamily::Proportional),
            Color32::from_rgb(255, 185, 200),
        );

        let close_rect = Rect::from_center_size(
            Pos2::new(banner_rect.right() - 16.0, banner_rect.center().y),
            Vec2::splat(20.0),
        );
        let resp = ui.interact(close_rect, ui.id().with("banner_dismiss"), Sense::click());
        let c = close_rect.center();
        let xcol = if resp.hovered() {
            TEXT_BRIGHT
        } else {
            TEXT_SOFT
        };
        painter.line_segment(
            [c + Vec2::new(-3.5, -3.5), c + Vec2::new(3.5, 3.5)],
            Stroke::new(1.3_f32, xcol),
        );
        painter.line_segment(
            [c + Vec2::new(3.5, -3.5), c + Vec2::new(-3.5, 3.5)],
            Stroke::new(1.3_f32, xcol),
        );
        if resp.clicked() {
            self.banner_error = None;
        }
    }

    fn draw_setup_dialog(&mut self, ctx: &egui::Context) {
        let Some(is_settings) = self.setup_form.as_ref().map(|f| f.is_settings) else {
            return;
        };
        let pal = self.active_palette();
        let accent = pal.ui_secondary;

        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("setup_scrim"))
            .fixed_pos(screen.left_top())
            .order(Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(4, 3, 12, 148),
                );
                ui.allocate_rect(screen, Sense::click());
            });

        let mut close_modal = is_settings && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut submitted_cfg: Option<ApiConfig> = None;

        let Some(form) = self.setup_form.as_mut() else {
            return;
        };

        let max_scroll_h = (screen.height() - 180.0).clamp(240.0, 560.0);

        egui::Window::new("boogie_setup")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(13, 12, 28, 236))
                    .stroke(Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 105),
                    ))
                    .corner_radius(CornerRadius::same(18))
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.set_max_width(440.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if form.is_settings {
                            "BOOGIE SETTINGS"
                        } else {
                            "BOOGIE SETUP"
                        })
                        .size(10.5)
                        .color(accent)
                        .family(FontFamily::Monospace),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{APP_VERSION}"))
                                .size(10.0)
                                .color(TEXT_MUTED)
                                .family(FontFamily::Monospace),
                        );
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if form.is_settings {
                        "Graphics, Theme & Preferences"
                    } else {
                        "Connect Gemini Live API"
                    })
                    .size(16.0)
                    .color(TEXT_BRIGHT)
                    .strong(),
                );
                if let Some(err) = &form.error_msg {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.0).color(AMBER));
                }

                if form.is_settings {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let tab_w = ((ui.available_width() - 8.0) / 2.0).max(120.0);
                        let mem_label = format!("Memory ({})", form.memory_rows.len());
                        for (tab, label) in [
                            (SettingsTab::General, "General".to_string()),
                            (SettingsTab::Memory, mem_label),
                        ] {
                            let active = form.active_tab == tab;
                            let fill = if active {
                                Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    38,
                                )
                            } else {
                                Color32::from_rgba_unmultiplied(255, 255, 255, 10)
                            };
                            let stroke = if active {
                                Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    180,
                                )
                            } else {
                                Color32::from_rgba_unmultiplied(255, 255, 255, 24)
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(label)
                                            .size(11.5)
                                            .color(if active { TEXT_BRIGHT } else { TEXT_SOFT })
                                            .strong(),
                                    )
                                    .fill(fill)
                                    .stroke(Stroke::new(1.0_f32, stroke))
                                    .corner_radius(CornerRadius::same(10))
                                    .min_size(Vec2::new(tab_w, 28.0)),
                                )
                                .clicked()
                            {
                                form.active_tab = tab;
                                if tab == SettingsTab::Memory {
                                    form.reload_memories();
                                }
                            }
                        }
                    });
                }

                ui.add_space(10.0);

                if form.is_settings && form.active_tab == SettingsTab::Memory {
                    draw_memory_tab(ui, form, max_scroll_h, accent);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("boogie_setup_scroll")
                        .max_height(max_scroll_h)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                        // --- Gemini API Key ---
                        ui.label(RichText::new("Gemini API Key").size(12.0).color(TEXT_SOFT));
                        ui.horizontal(|ui| {
                            let key_w = (ui.available_width() - 62.0).max(180.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut form.api_key)
                                    .password(!form.show_api_key)
                                    .hint_text("AIza...")
                                    .desired_width(key_w),
                            );
                            ui.checkbox(&mut form.show_api_key, "Show");
                        });

                        // --- Graphics Performance Tier (Live Preview) ---
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("Graphics Quality (Orb Performance)")
                                .size(12.0)
                                .color(TEXT_SOFT),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let btn_w = ((ui.available_width() - 12.0) / 3.0).max(96.0);
                            for quality in GraphicsQuality::ALL {
                                let selected = form.graphics_quality == quality;
                                let fill = if selected {
                                    Color32::from_rgba_unmultiplied(
                                        accent.r(),
                                        accent.g(),
                                        accent.b(),
                                        44,
                                    )
                                } else {
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 10)
                                };
                                let stroke_col = if selected {
                                    Color32::from_rgba_unmultiplied(
                                        accent.r(),
                                        accent.g(),
                                        accent.b(),
                                        190,
                                    )
                                } else {
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 28)
                                };
                                let txt_col = if selected { TEXT_BRIGHT } else { TEXT_SOFT };
                                let resp = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(theme::quality_label(quality))
                                                .size(11.5)
                                                .color(txt_col)
                                                .strong(),
                                        )
                                        .fill(fill)
                                        .stroke(Stroke::new(1.0_f32, stroke_col))
                                        .corner_radius(CornerRadius::same(10))
                                        .min_size(Vec2::new(btn_w, 30.0)),
                                    )
                                    .on_hover_text(theme::quality_description(quality));
                                if resp.clicked() {
                                    form.graphics_quality = quality;
                                }
                            }
                        });
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(theme::quality_description(form.graphics_quality))
                                .size(10.5)
                                .color(TEXT_MUTED),
                        );

                        // --- Color Theme Presets (Live Preview) ---
                        ui.add_space(10.0);
                        let active_pal = ThemePalette::for_theme(form.color_theme);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Color Theme").size(12.0).color(TEXT_SOFT));
                            ui.label(
                                RichText::new(format!("· {}", active_pal.display_name))
                                    .size(11.5)
                                    .color(active_pal.ui_secondary),
                            );
                        });
                        ui.add_space(4.0);
                        let chip_w = ((ui.available_width() - 18.0) / 4.0).max(82.0);
                        for row in ColorTheme::ALL.chunks(4) {
                            ui.horizontal(|ui| {
                                for &preset in row {
                                    let p = ThemePalette::for_theme(preset);
                                    let selected = form.color_theme == preset;
                                    if draw_theme_chip(ui, &p, selected, chip_w) {
                                        form.color_theme = preset;
                                    }
                                }
                            });
                            ui.add_space(4.0);
                        }

                        // --- Voice & Identity ---
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Voice:").size(12.0).color(TEXT_SOFT));
                            egui::ComboBox::from_id_salt("setup_voice")
                                .selected_text(&form.voice)
                                .show_ui(ui, |ui| {
                                    for v in ["Aoede", "Charon", "Kore", "Fenrir", "Puck"] {
                                        ui.selectable_value(&mut form.voice, v.to_string(), v);
                                    }
                                });
                        });

                        ui.add_space(6.0);
                        ui.label(RichText::new("Assistant Name").size(12.0).color(TEXT_SOFT));
                        ui.add(
                            egui::TextEdit::singleline(&mut form.assistant_name)
                                .hint_text("Boogie")
                                .desired_width(f32::INFINITY),
                        );

                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Your Name (optional)")
                                .size(12.0)
                                .color(TEXT_SOFT),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut form.user_name)
                                .hint_text("Leave blank for default")
                                .desired_width(f32::INFINITY),
                        );

                        if !form.available_inputs.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Mic:").size(12.0).color(TEXT_SOFT));
                                let sel = if form.input_device.is_empty() {
                                    "System Default"
                                } else {
                                    form.input_device.as_str()
                                };
                                egui::ComboBox::from_id_salt("setup_mic")
                                    .selected_text(truncate_str(sel, 30))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut form.input_device,
                                            String::new(),
                                            "System Default",
                                        );
                                        for dev in &form.available_inputs {
                                            ui.selectable_value(
                                                &mut form.input_device,
                                                dev.clone(),
                                                dev,
                                            );
                                        }
                                    });
                            });
                        }

                        if !form.available_outputs.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Speaker:").size(12.0).color(TEXT_SOFT));
                                let sel = if form.output_device.is_empty() {
                                    "System Default"
                                } else {
                                    form.output_device.as_str()
                                };
                                egui::ComboBox::from_id_salt("setup_spk")
                                    .selected_text(truncate_str(sel, 30))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut form.output_device,
                                            String::new(),
                                            "System Default",
                                        );
                                        for dev in &form.available_outputs {
                                            ui.selectable_value(
                                                &mut form.output_device,
                                                dev.clone(),
                                                dev,
                                            );
                                        }
                                    });
                            });
                        }

                        ui.add_space(8.0);
                        ui.checkbox(&mut form.push_to_talk, "Push-to-Talk (hold Space)");
                        ui.checkbox(&mut form.proactive_audio, "Proactive check-ins");
                        ui.checkbox(
                            &mut form.auto_memory_consolidation,
                            "Auto AI memory summary (Flash-Lite)",
                        )
                        .on_hover_text(
                            "Summarizes long chats & extracts durable facts in the background \
                             using lightweight gemini-2.5-flash-lite (separate quota, 10m cooldown).",
                        );

                        ui.add_space(6.0);
                        let log_path = crate::config::active_config_dir().join("boogie.log");
                        ui.label(
                            RichText::new(format!("Log: {}", log_path.display()))
                                .size(10.0)
                                .color(TEXT_MUTED)
                                .family(FontFamily::Monospace),
                        );
                    });
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let save_label = if form.is_settings {
                        "Save & Apply"
                    } else {
                        "Save & Connect"
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(save_label)
                                    .size(13.0)
                                    .color(BG_OBSIDIAN)
                                    .strong(),
                            )
                            .fill(accent)
                            .corner_radius(CornerRadius::same(14))
                            .min_size(Vec2::new(140.0, 36.0)),
                        )
                        .clicked()
                    {
                        let key = form.api_key.trim().to_string();
                        if key.is_empty() {
                            form.error_msg = Some("Please enter your Gemini API key.".to_string());
                        } else {
                            let mut cfg = form.base_config.clone();
                            cfg.gemini_api_key = key;
                            cfg.graphics_quality = Some(form.graphics_quality.as_str().to_string());
                            cfg.color_theme = Some(form.color_theme.as_str().to_string());
                            cfg.voice = Some(form.voice.clone());
                            cfg.assistant_name = if form.assistant_name.trim().is_empty() {
                                None
                            } else {
                                Some(form.assistant_name.trim().to_string())
                            };
                            cfg.user_name = if form.user_name.trim().is_empty() {
                                None
                            } else {
                                Some(form.user_name.trim().to_string())
                            };
                            cfg.input_device = if form.input_device.is_empty() {
                                None
                            } else {
                                Some(form.input_device.clone())
                            };
                            cfg.output_device = if form.output_device.is_empty() {
                                None
                            } else {
                                Some(form.output_device.clone())
                            };
                            cfg.push_to_talk = Some(form.push_to_talk);
                            cfg.proactive_audio = Some(form.proactive_audio);
                            cfg.auto_memory_consolidation = Some(form.auto_memory_consolidation);
                            if !form.is_settings && cfg.dashboard_enabled.is_none() {
                                cfg.dashboard_enabled = Some(false);
                            }
                            match cfg.save() {
                                Ok(_) => submitted_cfg = Some(cfg),
                                Err(e) => form.error_msg = Some(format!("Save failed: {e}")),
                            }
                        }
                    }

                    if form.is_settings {
                        ui.add_space(10.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Cancel").size(12.5).color(TEXT_SOFT),
                                )
                                .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 36),
                                ))
                                .corner_radius(CornerRadius::same(14))
                                .min_size(Vec2::new(92.0, 36.0)),
                            )
                            .clicked()
                        {
                            close_modal = true;
                        }
                    }
                });
            });

        if let Some(cfg) = submitted_cfg {
            self.graphics_quality = cfg.graphics_quality();
            self.color_theme = cfg.color_theme();
            self.current_cfg = Some(cfg.clone());
            self.push_to_talk = cfg.push_to_talk();
            self.ptt_holding = false;
            self.setup_form = None;
            let _ = self.config_tx.try_send(cfg);
        } else if close_modal {
            self.setup_form = None;
        }
    }
}

impl eframe::App for BoogieGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        let dt = if self.last_frame_time > 0.0 {
            (now - self.last_frame_time).clamp(0.001, 0.05) as f32
        } else {
            0.016
        };
        self.last_frame_time = now;
        self.egui_time = now;

        self.process_events();
        if self.shutdown_requested {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        // Push-to-talk Space key tracking (disabled while modal/input active or window unfocused)
        if self.push_to_talk {
            let can_ptt = self.setup_form.is_none()
                && !ctx.wants_keyboard_input()
                && ctx.input(|i| i.viewport().focused.unwrap_or(true));
            let holding = can_ptt && ctx.input(|i| i.key_down(egui::Key::Space));
            if holding != self.ptt_holding
                && let Ok(mut s) = self.shared.try_lock()
            {
                s.ptt_active = holding;
                self.ptt_holding = holding;
            }
        } else if self.ptt_holding
            && let Ok(mut s) = self.shared.try_lock()
        {
            s.ptt_active = false;
            self.ptt_holding = false;
        }

        self.metrics.refresh();
        self.update_animations(ctx, dt);

        // Adaptive repaint cadence by graphics tier (30 / 45 / 60 FPS)
        let frame_ms = theme::quality_frame_interval_ms(self.active_quality());
        ctx.request_repaint_after(std::time::Duration::from_millis(frame_ms));

        let screen = ctx.screen_rect();
        self.handle_frameless_resize(ctx, screen);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG_OBSIDIAN))
            .show(ctx, |ui| {
                let full_rect = ui.max_rect();

                // 1. Full-canvas 3D Liquid Glass Orb + Obsidian Aurora Shader
                self.paint_orb_backdrop(ui, full_rect);

                // 2. Floating Glass Control Dock (bottom)
                let dock_top = self.draw_floating_dock(ui, full_rect);

                // 3. Live Captions (when drawer closed) + Expandable Glass Transcript Drawer
                self.draw_live_captions(ui, full_rect, dock_top);
                self.draw_transcript_drawer(ui, full_rect, dock_top);

                // 4. Frameless Custom Header Bar (top) + Error Banner
                self.draw_frameless_header(ui, full_rect);
                self.draw_error_banner(ui, full_rect);

                // 5. Interactive poke/ripple on the 3D orb region (safely bounded between header & lower panels)
                let orb_hit_y =
                    full_rect.top() + (1.0 - (0.58 + self.drawer_anim * 0.16)) * full_rect.height();
                let orb_hit_r = (full_rect.width().min(full_rect.height())
                    * (0.20 - self.drawer_anim * 0.05))
                    .max(40.0);
                let lower_limit = if self.drawer_open {
                    dock_top - (full_rect.height() * 0.44).clamp(180.0, 380.0) - 18.0
                } else {
                    dock_top - 140.0
                };
                let orb_hit_rect = Rect::from_center_size(
                    Pos2::new(full_rect.center().x, orb_hit_y),
                    Vec2::splat(orb_hit_r * 2.0),
                )
                .intersect(Rect::from_min_max(
                    Pos2::new(full_rect.left(), full_rect.top() + HEADER_H + 4.0),
                    Pos2::new(
                        full_rect.right(),
                        lower_limit.max(full_rect.top() + HEADER_H + 40.0),
                    ),
                ));
                if orb_hit_rect.is_positive() {
                    let orb_resp =
                        ui.interact(orb_hit_rect, ui.id().with("orb_poke"), Sense::click());
                    if orb_resp.clicked() {
                        self.click_pulse = 1.0;
                    }
                }

                // 6. Crisp 1px outer obsidian glass rim
                ui.painter().rect_stroke(
                    full_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(190, 180, 255, 26)),
                    egui::StrokeKind::Inside,
                );
            });

        // 7. Modals (Onboarding setup or Confirmation card)
        self.draw_setup_dialog(ctx);
        self.draw_confirm_dialog(ctx);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(gl), Some(renderer_arc)) = (gl, &self.orb_renderer)
            && let Ok(r) = renderer_arc.lock()
        {
            r.destroy(gl);
        }
    }
}

// --- Custom Vector Icon Painters ---

enum WinBtnKind {
    Settings(bool),
    Minimize,
    Maximize(bool),
    Close,
}

fn draw_memory_tab(ui: &mut egui::Ui, form: &mut SetupForm, max_scroll_h: f32, accent: Color32) {
    ui.horizontal(|ui| {
        let search_w = (ui.available_width() - 68.0).max(180.0);
        ui.add(
            egui::TextEdit::singleline(&mut form.memory_filter)
                .hint_text("Search facts & sessions...")
                .desired_width(search_w),
        );
        if ui
            .add(
                egui::Button::new(RichText::new("Refresh").size(11.0).color(TEXT_SOFT))
                    .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(60.0, 24.0)),
            )
            .clicked()
        {
            form.reload_memories();
        }
    });
    ui.add_space(8.0);

    let filter_folded = crate::memory::fold_vietnamese(form.memory_filter.trim());
    let mut forget_target: Option<(String, String)> = None;
    let mut delete_session_rev_idx: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("boogie_memory_scroll")
        .max_height((max_scroll_h - 68.0).max(180.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            ui.label(
                RichText::new(format!("STORED FACTS ({})", form.memory_rows.len()))
                    .size(10.0)
                    .color(accent)
                    .family(FontFamily::Monospace),
            );
            ui.add_space(4.0);

            let mut shown_facts = 0usize;
            for row in &form.memory_rows {
                if !filter_folded.is_empty() {
                    let hay = crate::memory::fold_vietnamese(&format!(
                        "{} {} {}",
                        row.category, row.key, row.entry.value
                    ));
                    if !hay.contains(&filter_folded) {
                        continue;
                    }
                }
                shown_facts += 1;
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 8))
                    .stroke(Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 18),
                    ))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{}/{}", row.category, row.key))
                                    .size(10.5)
                                    .color(accent)
                                    .family(FontFamily::Monospace),
                            );
                            if !row.entry.updated_at.is_empty() {
                                let d =
                                    row.entry.updated_at.split_whitespace().next().unwrap_or("");
                                ui.label(RichText::new(d).size(9.5).color(TEXT_MUTED));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("×").size(12.0).color(CRIMSON),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .min_size(Vec2::new(20.0, 18.0)),
                                        )
                                        .on_hover_text("Delete this memory")
                                        .clicked()
                                    {
                                        forget_target =
                                            Some((row.category.clone(), row.key.clone()));
                                    }
                                },
                            );
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(&row.entry.value)
                                    .size(12.0)
                                    .color(TEXT_BRIGHT),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(4.0);
            }

            if shown_facts == 0 {
                ui.label(
                    RichText::new("No matching facts stored yet.")
                        .size(11.5)
                        .color(TEXT_MUTED),
                );
            }

            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("PAST SESSIONS ({})", form.recent_sessions.len()))
                    .size(10.0)
                    .color(accent)
                    .family(FontFamily::Monospace),
            );
            ui.add_space(4.0);

            let mut shown_sessions = 0usize;
            for (rev_idx, sess) in form.recent_sessions.iter().rev().enumerate() {
                if !filter_folded.is_empty() {
                    let hay = crate::memory::fold_vietnamese(&format!(
                        "{} {} {}",
                        sess.date,
                        sess.topics.join(" "),
                        sess.summary
                    ));
                    if !hay.contains(&filter_folded) {
                        continue;
                    }
                }
                shown_sessions += 1;
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 7))
                    .stroke(Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 16),
                    ))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let badge = if sess.ai_summarized { "AI · " } else { "" };
                            ui.label(
                                RichText::new(format!("{badge}{}", sess.date))
                                    .size(10.0)
                                    .color(TEXT_SOFT)
                                    .family(FontFamily::Monospace),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("×").size(12.0).color(CRIMSON),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .min_size(Vec2::new(20.0, 18.0)),
                                        )
                                        .on_hover_text("Delete this session summary")
                                        .clicked()
                                    {
                                        delete_session_rev_idx = Some(rev_idx);
                                    }
                                },
                            );
                        });
                        if !sess.topics.is_empty() {
                            ui.label(
                                RichText::new(sess.topics.join(" · "))
                                    .size(10.0)
                                    .color(accent),
                            );
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(truncate_str(&sess.summary, 220))
                                    .size(11.5)
                                    .color(TEXT_BRIGHT),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(4.0);
            }

            if shown_sessions == 0 {
                ui.label(
                    RichText::new("No past sessions recorded yet.")
                        .size(11.5)
                        .color(TEXT_MUTED),
                );
            }
        });

    if let Some((cat, key)) = forget_target {
        let _ = crate::memory::forget_memory(Some(&cat), Some(&key), None);
        form.reload_memories();
    }
    if let Some(rev_idx) = delete_session_rev_idx {
        let _ = crate::memory::delete_session_by_index(rev_idx);
        form.reload_memories();
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let clear_label = if form.confirm_clear_memory {
            "Click again to wipe all memory"
        } else {
            "Clear All Memory"
        };
        if ui
            .add(
                egui::Button::new(RichText::new(clear_label).size(11.0).color(CRIMSON))
                    .fill(Color32::from_rgba_unmultiplied(
                        CRIMSON.r(),
                        CRIMSON.g(),
                        CRIMSON.b(),
                        if form.confirm_clear_memory { 44 } else { 18 },
                    ))
                    .stroke(Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(CRIMSON.r(), CRIMSON.g(), CRIMSON.b(), 95),
                    ))
                    .corner_radius(CornerRadius::same(10))
                    .min_size(Vec2::new(130.0, 26.0)),
            )
            .clicked()
        {
            if form.confirm_clear_memory {
                let _ = crate::memory::clear_all_memory();
                form.reload_memories();
            } else {
                form.confirm_clear_memory = true;
            }
        }
    });
}

fn draw_theme_chip(ui: &mut egui::Ui, pal: &ThemePalette, selected: bool, width: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let resp = resp.on_hover_text(pal.display_name);
    let painter = ui.painter();

    let fill = if selected {
        Color32::from_rgba_unmultiplied(
            pal.ui_secondary.r(),
            pal.ui_secondary.g(),
            pal.ui_secondary.b(),
            36,
        )
    } else if resp.hovered() {
        Color32::from_rgba_unmultiplied(255, 255, 255, 18)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 9)
    };
    let border_col = if selected {
        Color32::from_rgba_unmultiplied(
            pal.ui_secondary.r(),
            pal.ui_secondary.g(),
            pal.ui_secondary.b(),
            195,
        )
    } else if resp.hovered() {
        Color32::from_rgba_unmultiplied(255, 255, 255, 58)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 24)
    };

    painter.rect_filled(rect, CornerRadius::same(10), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0_f32, border_col),
        egui::StrokeKind::Inside,
    );

    // 3 miniature glowing color swatch dots
    let cy = rect.center().y;
    let dot_start_x = rect.left() + 10.0;
    for (idx, &sw) in pal.swatches.iter().enumerate() {
        let cx = dot_start_x + idx as f32 * 6.5;
        painter.circle_filled(Pos2::new(cx, cy), 3.2, sw);
    }

    painter.text(
        Pos2::new(dot_start_x + 21.0, cy),
        Align2::LEFT_CENTER,
        pal.short_name,
        FontId::new(10.5, FontFamily::Proportional),
        if selected { TEXT_BRIGHT } else { TEXT_SOFT },
    );

    resp.clicked()
}

fn draw_win_button(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    kind: WinBtnKind,
    accent: Color32,
) -> bool {
    let mut resp = ui.interact(rect, ui.id().with(id), Sense::click());
    if matches!(kind, WinBtnKind::Settings(_)) {
        resp = resp.on_hover_text("Settings");
    }
    let painter = ui.painter();
    let is_active = matches!(kind, WinBtnKind::Settings(true));

    if resp.hovered() || is_active {
        let hover_bg = match kind {
            WinBtnKind::Close => {
                Color32::from_rgba_unmultiplied(CRIMSON.r(), CRIMSON.g(), CRIMSON.b(), 190)
            }
            WinBtnKind::Settings(true) => {
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36)
            }
            _ => Color32::from_rgba_unmultiplied(255, 255, 255, 22),
        };
        painter.rect_filled(rect, CornerRadius::same(7), hover_bg);
    }

    let c = rect.center();
    let fg = if is_active {
        accent
    } else if resp.hovered() {
        TEXT_BRIGHT
    } else {
        TEXT_SOFT
    };
    let stroke = Stroke::new(1.3_f32, fg);

    match kind {
        WinBtnKind::Settings(_) => {
            painter.circle_stroke(c, 3.6, stroke);
            painter.circle_stroke(c, 1.4, stroke);
            for step in 0..8 {
                let angle = step as f32 * std::f32::consts::FRAC_PI_4;
                let dir = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment([c + dir * 3.8, c + dir * 5.7], stroke);
            }
        }
        WinBtnKind::Minimize => {
            painter.line_segment([c + Vec2::new(-5.0, 0.5), c + Vec2::new(5.0, 0.5)], stroke);
        }
        WinBtnKind::Maximize(is_max) => {
            if is_max {
                let back = Rect::from_center_size(c + Vec2::new(1.5, -1.5), Vec2::splat(7.5));
                let front = Rect::from_center_size(c + Vec2::new(-1.5, 1.5), Vec2::splat(7.5));
                painter.rect_stroke(
                    back,
                    CornerRadius::same(1),
                    stroke,
                    egui::StrokeKind::Inside,
                );
                painter.rect_filled(front, CornerRadius::same(1), BG_OBSIDIAN);
                painter.rect_stroke(
                    front,
                    CornerRadius::same(1),
                    stroke,
                    egui::StrokeKind::Inside,
                );
            } else {
                let sq = Rect::from_center_size(c, Vec2::splat(8.5));
                painter.rect_stroke(sq, CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            }
        }
        WinBtnKind::Close => {
            painter.line_segment([c + Vec2::new(-4.2, -4.2), c + Vec2::new(4.2, 4.2)], stroke);
            painter.line_segment([c + Vec2::new(4.2, -4.2), c + Vec2::new(-4.2, 4.2)], stroke);
        }
    }

    resp.clicked()
}

fn draw_mic_icon(painter: &egui::Painter, c: Pos2, col: Color32, muted: bool) {
    let s = Stroke::new(1.5_f32, col);
    // Capsule body
    let cap = Rect::from_center_size(c + Vec2::new(0.0, -2.5), Vec2::new(6.5, 10.5));
    painter.rect_stroke(cap, CornerRadius::same(3), s, egui::StrokeKind::Inside);
    // U-arc cradle (approximated smoothly)
    let mut arc = Vec::with_capacity(11);
    for i in 0..=10 {
        let a = std::f32::consts::PI * (i as f32 / 10.0);
        arc.push(Pos2::new(c.x + a.cos() * 6.2, c.y - 1.5 + a.sin() * 6.0));
    }
    painter.add(egui::Shape::line(arc, s));
    // Stem + base
    painter.line_segment([Pos2::new(c.x, c.y + 4.5), Pos2::new(c.x, c.y + 8.0)], s);
    painter.line_segment(
        [
            Pos2::new(c.x - 3.8, c.y + 8.0),
            Pos2::new(c.x + 3.8, c.y + 8.0),
        ],
        s,
    );
    if muted {
        painter.line_segment(
            [c + Vec2::new(-7.5, -7.5), c + Vec2::new(7.5, 7.5)],
            Stroke::new(1.8_f32, CRIMSON),
        );
    }
}

fn draw_transcript_icon(painter: &egui::Painter, c: Pos2, col: Color32) {
    let s = Stroke::new(1.4_f32, col);
    painter.line_segment([c + Vec2::new(-6.0, -4.0), c + Vec2::new(6.0, -4.0)], s);
    painter.line_segment([c + Vec2::new(-6.0, 0.0), c + Vec2::new(6.0, 0.0)], s);
    painter.line_segment([c + Vec2::new(-6.0, 4.0), c + Vec2::new(2.0, 4.0)], s);
}

// --- Helpers ---

fn exp_lerp(current: f32, target: f32, speed: f32, dt: f32) -> f32 {
    let factor = 1.0 - (-speed * dt).exp();
    current + (target - current) * factor
}

fn exp_lerp_rgb(current: &mut [f32; 3], target: [f32; 3], speed: f32, dt: f32) {
    let factor = 1.0 - (-speed * dt).exp();
    for i in 0..3 {
        current[i] += (target[i] - current[i]) * factor;
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}

fn append_transcript_chunk(target: &mut String, chunk: &str) {
    if target.is_empty() {
        target.push_str(chunk);
        return;
    }
    let starts_punct = chunk
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '.' | ',' | '!' | '?' | ':' | ';' | '\'' | '’' | ')'));
    if !starts_punct && !target.ends_with(' ') && !target.ends_with('\n') {
        target.push(' ');
    }
    target.push_str(chunk);
}

/// Turns raw debug strings like `computer_settings Object {"action": String("brightness_down")}`
/// into a sleek readable badge: `("Computer Settings", "brightness down")`.
fn format_tool_log(raw: &str) -> (String, String) {
    let (name_part, args_part) = match raw.split_once(' ') {
        Some((n, a)) => (n.trim(), a.trim()),
        None => (raw.trim(), ""),
    };

    let nice_name = name_part
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Extract clean values from JSON `{"k":"v"}` or serde_json Debug output `String("...")`
    let mut values = Vec::new();
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(args_part)
    {
        for (_, v) in map {
            let s = match v {
                serde_json::Value::String(st) => st.replace('_', " "),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            if !s.is_empty() {
                values.push(s);
            }
        }
    } else {
        let mut rest = args_part;
        while let Some(idx) = rest.find("String(\"") {
            let after = &rest[idx + 8..];
            if let Some(end) = after.find("\")") {
                let val = after[..end].replace('_', " ");
                if !val.is_empty() {
                    values.push(val);
                }
                rest = &after[end + 2..];
            } else {
                break;
            }
        }
    }

    let detail = if !values.is_empty() {
        truncate_str(&values.join(" · "), 64)
    } else if args_part == "Object {}" || args_part == "{}" || args_part.is_empty() {
        String::new()
    } else {
        truncate_str(args_part, 56)
    };

    (nice_name, detail)
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars.saturating_sub(1))
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

fn truncate_tail(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let skip = count.saturating_sub(max_chars.saturating_sub(1));
        let start = s.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
        format!("…{}", s[start..].trim_start())
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // Load crisp Windows system fonts (also unlocks full Vietnamese glyph support!)
    let sans_candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\arial.ttf",
    ];
    for path in sans_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("ui_sans".to_owned(), Arc::new(FontData::from_owned(bytes)));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "ui_sans".to_owned());
            break;
        }
    }

    let mono_candidates = [
        r"C:\Windows\Fonts\CascadiaMono.ttf",
        r"C:\Windows\Fonts\consola.ttf",
    ];
    for path in mono_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("ui_mono".to_owned(), Arc::new(FontData::from_owned(bytes)));
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .insert(0, "ui_mono".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);
}

fn apply_obsidian_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_OBSIDIAN;
    v.window_fill = Color32::from_rgb(14, 13, 30);
    v.extreme_bg_color = Color32::from_rgb(9, 8, 22);
    v.faint_bg_color = Color32::from_rgb(16, 15, 34);
    v.override_text_color = Some(TEXT_BRIGHT);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(VIOLET.r(), VIOLET.g(), VIOLET.b(), 80);
    v.widgets.inactive.bg_fill = Color32::from_rgb(20, 18, 40);
    v.widgets.hovered.bg_fill = Color32::from_rgb(30, 26, 56);
    v.widgets.active.bg_fill = Color32::from_rgb(42, 34, 76);
    ctx.set_visuals(v);
}

fn load_app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(APP_LOGO_PNG).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn load_logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let img = image::load_from_memory(APP_LOGO_PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img);
    Some(ctx.load_texture("boogie_app_logo", color_image, egui::TextureOptions::LINEAR))
}

pub fn run_gui(
    event_rx: broadcast::Receiver<AppEvent>,
    shared: Arc<Mutex<SharedState>>,
    confirm_tx: mpsc::Sender<ConfirmResponse>,
    interrupt_tx: mpsc::Sender<()>,
    config_tx: mpsc::Sender<ApiConfig>,
    initial_cfg: Option<ApiConfig>,
    initial_cfg_err: Option<String>,
) -> Result<(), eframe::Error> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0_f32, 800.0_f32])
        .with_min_inner_size([480.0_f32, 540.0_f32])
        .with_maximized(true)
        .with_decorations(false)
        .with_title("Boogie");

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let multisampling = match initial_cfg
        .as_ref()
        .map(ApiConfig::graphics_quality)
        .unwrap_or_default()
    {
        GraphicsQuality::Low => 0,
        GraphicsQuality::Medium => 2,
        GraphicsQuality::High => 4,
    };

    let options = eframe::NativeOptions {
        viewport,
        multisampling,
        ..Default::default()
    };

    eframe::run_native(
        "Boogie",
        options,
        Box::new(move |cc| {
            Ok(Box::new(BoogieGui::new(
                cc,
                event_rx,
                shared,
                confirm_tx,
                interrupt_tx,
                config_tx,
                initial_cfg,
                initial_cfg_err,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_streaming_transcript_chunks_cleanly() {
        let mut s = String::new();
        append_transcript_chunk(&mut s, "I've lowered");
        append_transcript_chunk(&mut s, "the brightness");
        append_transcript_chunk(&mut s, "for");
        append_transcript_chunk(&mut s, "you.");
        append_transcript_chunk(&mut s, "Anything else?");
        assert_eq!(s, "I've lowered the brightness for you. Anything else?");
    }

    #[test]
    fn formats_tool_debug_into_clean_chip() {
        let (title, detail) =
            format_tool_log(r#"computer_settings Object {"action": String("brightness_down")}"#);
        assert_eq!(title, "Computer Settings");
        assert_eq!(detail, "brightness down");
    }
}
