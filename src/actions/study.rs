use base64::Engine as _;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::AppEvent;
use crate::config::{ApiConfig, StudyBrainMode};
use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{encode_rgba_to_jpeg, encode_rgba_to_png, truncate_chars};

use super::{param, tool};

const PRO_MODEL: &str = "gemini-3.1-pro-preview";
const FLASH_MODEL: &str = "gemini-2.5-flash";
// Live-verified fast REST models (avoids retired 2.0-* 404s and slow-hanging preview models).
// If both hit quota/503, callers drop instantaneously into zero-quota Direct Live Vision / Direct Board mode!
const FAST_FALLBACK_MODELS: &[&str] = &[FLASH_MODEL, "gemini-3.1-flash-lite"];
const PRO_RATE_LIMIT_BACKOFF_SECS: u64 = 120;
const STUDY_MAX_SCREEN_DIM: u32 = 1920;
const STUDY_MAX_CLIPBOARD_DIM: u32 = 2200;

pub(crate) const REST_QUOTA_EXHAUSTED_TAG: &str = "REST_QUOTA_EXHAUSTED";

static STUDY_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static FAST_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static PRO_COOLDOWN_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
static PERMA_UNAVAILABLE_MODELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn study_http_client(fast_lane: bool) -> &'static reqwest::Client {
    if fast_lane {
        FAST_HTTP_CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(35))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default()
        })
    } else {
        STUDY_HTTP_CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(65))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default()
        })
    }
}

fn is_model_perma_disabled(model: &str) -> bool {
    PERMA_UNAVAILABLE_MODELS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .ok()
        .is_some_and(|set| set.contains(model))
}

fn mark_model_perma_disabled(model: &str) {
    if let Ok(mut set) = PERMA_UNAVAILABLE_MODELS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
    {
        set.insert(model.to_string());
    }
}

fn is_pro_in_cooldown() -> bool {
    if is_model_perma_disabled(PRO_MODEL) {
        return true;
    }
    let lock = PRO_COOLDOWN_UNTIL.get_or_init(|| Mutex::new(None));
    match lock.lock().ok().and_then(|g| *g) {
        Some(until) => Instant::now() < until,
        None => false,
    }
}

fn mark_pro_rate_limited() {
    let lock = PRO_COOLDOWN_UNTIL.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = lock.lock() {
        *g = Some(Instant::now() + Duration::from_secs(PRO_RATE_LIMIT_BACKOFF_SECS));
    }
}

pub fn register(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    registry.register(
        tool(
            "study_solve",
            "High-precision academic tutor & deep-reasoning solver for ALL subjects from Primary \
             school (Cấp 1), Middle school (Cấp 2), High school (Cấp 3 / THPTQG) up to University \
             (Đại học): Math, Physics, Chemistry, Biology, Programming/IT/SQL, English/Languages, \
             Literature, History, Geography, Economics, Engineering, Medicine, etc. \
             Captures lossless full-resolution PNG of the screen or clipboard snip (Win+Shift+S), \
             runs high-resolution OCR + step-by-step deep thinking + double-check verification \
             so signs (+/-), exponents, fractions, and final answers are accurate. \
             IMPORTANT: ALWAYS speak ONE brief natural sentence aloud FIRST (e.g. 'Đợi mình chút, \
             để mình đọc kỹ đề rồi giải cẩn thận cho bạn nhé') BEFORE emitting this tool call. \
             ALWAYS use this instead of screen_process whenever the user asks to solve, calculate, \
             check homework, answer a quiz/exam, read equations/formulas, debug code on screen, \
             or study any subject.",
            vec![
                (
                    "question",
                    param(
                        "question",
                        "string",
                        "The user's exact request (e.g. 'Giải câu 3 trên màn hình', 'Kiểm tra bài làm của mình', 'Giải thích định luật bảo toàn năng lượng')",
                    ),
                ),
                (
                    "source",
                    param(
                        "source",
                        "string",
                        "Where the problem is: 'screen' (capture lossless full-res screen — default when referring to screen/exercise/photo), 'clipboard' (read exact snipped image from Win+Shift+S clipboard), or 'text' (spoken/text question with no image needed)",
                    ),
                ),
                (
                    "subject",
                    param(
                        "subject",
                        "string",
                        "Optional subject hint: math, physics, chemistry, biology, informatics, english, literature, history, geography, economics, general",
                    ),
                ),
                (
                    "level",
                    param(
                        "level",
                        "string",
                        "Optional education level: auto (default — infer from curriculum/notation), primary (Cấp 1 / lớp 1-5), middle (Cấp 2 / lớp 6-9), high (Cấp 3 / lớp 10-12 / THPTQG), university (Đại học / Cao đẳng)",
                    ),
                ),
                (
                    "mode",
                    param(
                        "mode",
                        "string",
                        "Pedagogy mode: solve (default — verified final answer + clear steps), hint (guided Socratic hints without spoiling final answer), check (find mistakes in user's work & fix them), explain (teach concept/formula/rule clearly)",
                    ),
                ),
                (
                    "monitor",
                    param(
                        "monitor",
                        "integer",
                        "Optional 0-based monitor index when source='screen' (omit to auto-select best monitor)",
                    ),
                ),
                (
                    "language",
                    param(
                        "language",
                        "string",
                        "Language to answer in (e.g. 'Vietnamese', 'English'); defaults to the user's conversation language",
                    ),
                ),
            ],
            vec!["question"],
        ),
        Box::new(move |args| {
            let tx = event_tx.clone();
            Box::pin(study_solve_handler(args, tx))
        }),
    );
}

pub(crate) struct CapturedImage {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
    pub source_desc: String,
}

pub(crate) fn capture_clipboard_png(
    notify_tx: Option<&broadcast::Sender<AppEvent>>,
) -> Result<CapturedImage, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;
    let img = cb
        .get_image()
        .map_err(|_| "No image found in clipboard. Ask the user to snip the problem with Win+Shift+S first, or switch source to 'screen'.".to_string())?;
    if let Some(tx) = notify_tx {
        let _ = tx.send(AppEvent::PlayChime(crate::ChimeKind::ScreenCaptured));
    }
    let w = img.width as u32;
    let h = img.height as u32;
    // Small clipboard snips stay lossless PNG; large snips use ultra-crisp Q95 JPEG for speed
    if (w as u64) * (h as u64) <= 900_000 {
        let (png, out_w, out_h) = encode_rgba_to_png(&img.bytes, w, h, STUDY_MAX_CLIPBOARD_DIM)?;
        Ok(CapturedImage {
            bytes: png,
            mime_type: "image/png",
            width: out_w,
            height: out_h,
            source_desc: format!("clipboard snip ({out_w}x{out_h} lossless PNG)"),
        })
    } else {
        let (jpg, out_w, out_h) =
            encode_rgba_to_jpeg(&img.bytes, w, h, STUDY_MAX_CLIPBOARD_DIM, 95)?;
        Ok(CapturedImage {
            bytes: jpg,
            mime_type: "image/jpeg",
            width: out_w,
            height: out_h,
            source_desc: format!("clipboard snip ({out_w}x{out_h} Q95 JPEG)"),
        })
    }
}

fn pick_best_monitor(screens: &[screenshots::Screen], requested: Option<usize>) -> usize {
    if let Some(idx) = requested
        && idx < screens.len()
    {
        return idx;
    }
    if screens.len() <= 1 {
        return 0;
    }
    // If multiple monitors exist, prefer a monitor that is NOT where Boogie's own window sits
    #[cfg(windows)]
    {
        if let Some((bx, by)) = find_boogie_window_center() {
            for (i, scr) in screens.iter().enumerate() {
                let di = scr.display_info;
                let in_this_monitor = bx >= di.x
                    && bx < di.x + di.width as i32
                    && by >= di.y
                    && by < di.y + di.height as i32;
                if !in_this_monitor {
                    return i;
                }
            }
        }
    }
    0
}

#[cfg(windows)]
pub(crate) fn bring_boogie_window_to_front() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, HWND_NOTOPMOST, HWND_TOPMOST, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        SetForegroundWindow, SetWindowPos, ShowWindow,
    };
    use windows::core::PCWSTR;

    let versioned_title = format!("Boogie v{}", env!("CARGO_PKG_VERSION"));
    for candidate in [versioned_title.as_str(), "Boogie"] {
        let title: Vec<u16> = OsStr::new(candidate)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            if let Ok(hwnd) = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) {
                let _ = ShowWindow(hwnd, SW_SHOW);
                // Briefly toggle topmost so Windows reliably surfaces Boogie & Save As above Chrome/VS Code
                let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
                let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
                let _ = SetForegroundWindow(hwnd);
                return;
            }
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn bring_boogie_window_to_front() {}

#[cfg(windows)]
fn find_boogie_window_center() -> Option<(i32, i32)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
    use windows::core::PCWSTR;

    let versioned_title = format!("Boogie v{}", env!("CARGO_PKG_VERSION"));
    for candidate in [versioned_title.as_str(), "Boogie"] {
        let title: Vec<u16> = OsStr::new(candidate)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            if let Ok(hwnd) = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_ok() {
                    return Some(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2));
                }
            }
        }
    }
    None
}

pub(crate) fn capture_screen_png(
    monitor_opt: Option<usize>,
    notify_tx: Option<&broadcast::Sender<AppEvent>>,
) -> Result<CapturedImage, String> {
    let screens = screenshots::Screen::all().map_err(|e| format!("screen list failed: {e}"))?;
    if screens.is_empty() {
        return Err("No monitors detected.".to_string());
    }
    let idx = pick_best_monitor(&screens, monitor_opt);
    let screen = screens
        .get(idx)
        .ok_or_else(|| format!("Monitor {idx} not found ({} available)", screens.len()))?;
    let img = screen
        .capture()
        .map_err(|e| format!("screen capture failed: {e}"))?;
    // Fire shutter beep IMMEDIATELY (~25ms) once raw screen pixels are safely in RAM
    // BEFORE image encoding — so the user can Alt+Tab right away!
    if let Some(tx) = notify_tx {
        let _ = tx.send(AppEvent::PlayChime(crate::ChimeKind::ScreenCaptured));
    }
    let (orig_w, orig_h) = (img.width(), img.height());
    // High-resolution Q95 JPEG: sharp text & math signs at ~6-8x smaller payload and ~50x faster encode than PNG!
    let (jpg, out_w, out_h) =
        encode_rgba_to_jpeg(img.as_raw(), orig_w, orig_h, STUDY_MAX_SCREEN_DIM, 95)?;
    Ok(CapturedImage {
        bytes: jpg,
        mime_type: "image/jpeg",
        width: out_w,
        height: out_h,
        source_desc: format!("monitor {idx} ({out_w}x{out_h} crisp Q95 JPEG)"),
    })
}

fn build_study_system_prompt(subject: &str, level: &str, mode: &str, language: &str) -> String {
    let level_guidance = match level {
        "primary" => {
            "Education level: PRIMARY SCHOOL (Cấp 1, grades 1-5). Use ONLY elementary methods (bar models / sơ đồ đoạn thẳng, unit method, simple arithmetic, clear age-appropriate wording). NEVER use algebra systems, quadratic formulas, or calculus."
        }
        "middle" => {
            "Education level: MIDDLE SCHOOL (Cấp 2, grades 6-9). Use middle-school algebra, geometry (Thales, similarity, circles), and foundational physics/chemistry/biology/humanities."
        }
        "high" => {
            "Education level: HIGH SCHOOL (Cấp 3, grades 10-12 / national exam THPTQG). Use high-school calculus, vectors, Oxyz, stoichiometry, physics laws, IELTS/exam grammar, and rigorous exam-ready methods."
        }
        "university" => {
            "Education level: UNIVERSITY / COLLEGE (Đại học). Use full undergraduate rigor: linear algebra, multivariable calculus, differential equations, probability/statistics, algorithms/CS/SQL, economics, engineering, medicine, or academic humanities."
        }
        _ => {
            "Education level: AUTO-DETECT from the problem's notation and curriculum (Primary Cấp 1, Middle Cấp 2, High Cấp 3, or University Đại học). Match your solving method strictly to that grade level — e.g., never use calculus on an elementary math problem."
        }
    };

    let mode_guidance = match mode {
        "hint" => {
            "Mode: SOCRATIC TUTOR (HINT). Do NOT spoil the final answer unless explicitly asked. Give 2-3 clear guided hints pointing out the exact theorem/formula and first step so the student solves it themselves."
        }
        "check" => {
            "Mode: CHECK STUDENT WORK. Read both the problem and the student's attempted solution carefully. Verify every single algebraic sign, formula, unit, and logical step. State clearly whether their final answer is RIGHT or WRONG; if wrong, pinpoint the exact line/sign mistake and show the corrected step & final result."
        }
        "explain" => {
            "Mode: CONCEPT EXPLANATION. Teach the underlying concept, formula, theorem, grammar rule, or intuition clearly with a concrete example suited to the student's level."
        }
        _ => {
            "Mode: SOLVE & VERIFY. Provide the exact verified final answer AND clear, concise step-by-step reasoning."
        }
    };

    format!(
        "You are Boogie's Deep Study Brain — an elite, ultra-accurate academic tutor across ALL subjects \
         from Primary school (Cấp 1) through University (Đại học).\n\
         Target reply language: {language}.\n\
         Subject focus: {subject}.\n\
         {level_guidance}\n\
         {mode_guidance}\n\n\
         CRITICAL ACCURACY & ANTI-HALLUCINATION PROTOCOL:\n\
         1. STRICT VISUAL OCR FIRST (when an image is attached):\n\
            - Read EVERY symbol, minus vs plus sign (- vs +), decimal vs comma, exponent (x² vs x³), \
              subscript (H₂SO₄), fraction bar, square-root scope, integral/sum bounds, inequalities (<, ≤, >, ≥), \
              units, diagram labels, and multiple-choice options (A/B/C/D) carefully.\n\
            - If the image only shows the Boogie AI dark orb app window covering the screen, say clearly that \
              Boogie's window is covering the exercise and ask the user to bring the exercise window to front \
              or snip it with Win+Shift+S.\n\
            - If any digit, minus sign, exponent, or line is genuinely unreadable/cut off/blurry, NEVER guess it. \
              Explicitly state which exact part is unclear and ask the user to zoom in or snip with Win+Shift+S.\n\
         2. INDEPENDENT DOUBLE-CHECK BEFORE ANSWERING:\n\
            - For Math/Physics/Chemistry/Economics: solve step by step, THEN verify by substituting your answer \
              back into the original equation, checking domain/conditions, signs, and units (or checking all \
              multiple-choice options A/B/C/D). If verification disagrees, fix the calculation before outputting.\n\
            - For Programming/CS/SQL: trace execution line-by-line on edge cases and check syntax/complexity.\n\
            - For English/Languages/Literature/History/Geography: verify grammar rules, tense/context, exact \
              historical facts, and textual evidence.\n\
         3. ULTRA-CONCISE CLEAN OUTPUT FORMAT (in {language}, plain readable Unicode math like x², √, π, ≤, ≥, ≠, → — avoid raw LaTeX backslashes; keep total reply crisp and under ~120 words unless user explicitly asks for full long proof):
            - **Đề bài:** (1 short line of the exact problem/question read)
            - **Đáp án:** (crystal-clear final answer or option letter A/B/C/D right on top — unless mode is 'hint')
            - **Cách giải:** (3 to 5 super tight, numbered steps — no filler)
            - **Kiểm tra:** (1 short line confirming sign/substitution check)"
    )
}

pub(crate) fn captured_to_jpeg(img: &CapturedImage) -> Result<Vec<u8>, String> {
    if img.mime_type == "image/jpeg" {
        return Ok(img.bytes.clone());
    }
    let dyn_img = image::load_from_memory(&img.bytes)
        .map_err(|e| format!("decode image for live vision: {e}"))?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let (jpg, _, _) = encode_rgba_to_jpeg(rgba.as_raw(), w, h, STUDY_MAX_SCREEN_DIM, 95)?;
    Ok(jpg)
}

fn model_supports_thinking(model: &str) -> bool {
    model.contains("2.5") || model.contains("3.") || model.contains("3-")
}

fn is_quota_exhausted_for_session(status: u16, msg: &str) -> bool {
    if status == 404 {
        return true;
    }
    if status == 429 {
        let lower = msg.to_ascii_lowercase();
        return lower.contains("limit: 0")
            || lower.contains("per day")
            || lower.contains("perday")
            || lower.contains("free_tier_requests")
            || lower.contains("daily");
    }
    false
}

fn select_fast_model_chain() -> Vec<&'static str> {
    FAST_FALLBACK_MODELS
        .iter()
        .copied()
        .filter(|m| !is_model_perma_disabled(m))
        .collect()
}

fn select_model_chain(mode: StudyBrainMode) -> Vec<&'static str> {
    let mut chain = Vec::new();
    if mode == StudyBrainMode::Pro && !is_pro_in_cooldown() {
        chain.push(PRO_MODEL);
    }
    // Auto & Flash use fast multi-model cascade directly so free-tier keys never waste 3-4s on
    // a 429 limit:0 Pro attempt, and automatically rotate across free-tier model buckets if one
    // hits its 20 req/day limit.
    for &m in FAST_FALLBACK_MODELS {
        if !is_model_perma_disabled(m) {
            chain.push(m);
        }
    }
    chain
}

#[allow(clippy::too_many_arguments)]
async fn call_gemini_study(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    image: Option<&CapturedImage>,
    json_mode: bool,
    max_output_tokens: u32,
    thinking_budget: Option<u32>,
    fast_lane: bool,
) -> Result<String, (u16, String)> {
    // Pass API key via x-goog-api-key header (never in URL) so error logs can never leak keys.
    let url =
        format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");

    let mut parts = Vec::new();
    if let Some(img) = image {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&img.bytes);
        parts.push(json!({
            "inlineData": {
                "mimeType": img.mime_type,
                "data": b64
            }
        }));
    }
    parts.push(json!({ "text": user_prompt }));

    // Build graceful config cascade: full (high media + thinking) -> thinking -> basic.
    // Note: Keep thinkingBudget modest relative to maxOutputTokens so JSON outputs never truncate!
    let mut gen_configs = Vec::new();
    if let Some(budget) = thinking_budget {
        if image.is_some() {
            let mut c = json!({
                "temperature": 0.15,
                "maxOutputTokens": max_output_tokens,
                "mediaResolution": "MEDIA_RESOLUTION_HIGH",
                "thinkingConfig": { "thinkingBudget": budget }
            });
            if json_mode {
                c["responseMimeType"] = json!("application/json");
            }
            gen_configs.push(c);
        }
        let mut c = json!({
            "temperature": 0.15,
            "maxOutputTokens": max_output_tokens,
            "thinkingConfig": { "thinkingBudget": budget }
        });
        if json_mode {
            c["responseMimeType"] = json!("application/json");
        }
        gen_configs.push(c);
    }
    let mut cfg_basic = json!({
        "temperature": 0.15,
        "maxOutputTokens": max_output_tokens
    });
    if json_mode {
        cfg_basic["responseMimeType"] = json!("application/json");
    }
    gen_configs.push(cfg_basic);

    let mut last_err = (0u16, "no attempt".to_string());

    for (attempt, gen_cfg) in gen_configs.iter().enumerate() {
        let body = json!({
            "systemInstruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "role": "user",
                "parts": parts
            }],
            "generationConfig": gen_cfg
        });

        let started = Instant::now();
        let resp = match study_http_client(fast_lane)
            .post(&url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let kind = if e.is_timeout() {
                    "request timed out"
                } else if e.is_connect() {
                    "connection failed"
                } else {
                    "network error"
                };
                return Err((0, format!("{kind} calling {model}")));
            }
        };

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();

        if status == 200 {
            tracing::info!(
                "study brain {model} responded in {}ms ({} chars)",
                started.elapsed().as_millis(),
                text.len()
            );
            let parsed: Value = serde_json::from_str(&text)
                .map_err(|e| (200, format!("json decode failed: {e}")))?;
            if let Some(answer) = extract_answer_text(&parsed) {
                return Ok(answer);
            }
            return Err((200, "model returned empty response".to_string()));
        }

        let err_detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| truncate_chars(&text, 200).to_string());

        last_err = (status, err_detail.clone());

        // Only strip optional config fields if server returned 400 Invalid Argument
        if status == 400 && attempt + 1 < gen_configs.len() {
            tracing::warn!(
                "study brain ({model}) 400 on attempt {}: {err_detail} — retrying simpler config",
                attempt + 1
            );
            continue;
        }
        return Err(last_err);
    }

    Err(last_err)
}

/// Shared Deep Study Brain cascade (Pro -> Flash fallback with rate-limit cooldown & 404 skip).
/// When `fast_lane` is true (used by interactive `study_board` & `generate_document`), prefers
/// `gemini-2.5-flash` immediately with zero-thinking budget so responses arrive in ~2–6 seconds.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn call_study_brain(
    cfg: &ApiConfig,
    system_prompt: &str,
    user_prompt: &str,
    image: Option<&CapturedImage>,
    json_mode: bool,
    max_output_tokens: u32,
    thinking_budget: Option<u32>,
    fast_lane: bool,
) -> Result<(String, &'static str), String> {
    let api_key = cfg.api_key();
    if api_key.is_empty() {
        return Err("Gemini API key is missing.".to_string());
    }

    let models = if fast_lane {
        select_fast_model_chain()
    } else {
        select_model_chain(cfg.study_brain_mode())
    };
    if models.is_empty() {
        return Err(format!(
            "{REST_QUOTA_EXHAUSTED_TAG}: All Gemini REST models have reached their daily quota for this session."
        ));
    }

    let mut last_err = String::new();
    let mut hit_quota = false;

    for model in models {
        let effective_think = thinking_budget.filter(|_| model_supports_thinking(model));
        tracing::info!(
            "study brain invoking {model} (json_mode={json_mode}, fast_lane={fast_lane}, think={:?})",
            effective_think
        );
        match call_gemini_study(
            api_key,
            model,
            system_prompt,
            user_prompt,
            image,
            json_mode,
            max_output_tokens,
            effective_think,
            fast_lane,
        )
        .await
        {
            Ok(ans) => return Ok((ans, model)),
            Err((status, msg)) => {
                tracing::warn!("study brain {model} failed (HTTP {status}): {msg}");
                if status == 429 {
                    hit_quota = true;
                }
                if is_quota_exhausted_for_session(status, &msg) {
                    mark_model_perma_disabled(model);
                } else if model == PRO_MODEL && (status == 429 || status == 503) {
                    mark_pro_rate_limited();
                }
                last_err = format!("{model} (HTTP {status}): {msg}");
            }
        }
    }

    if hit_quota {
        return Err(format!(
            "{REST_QUOTA_EXHAUSTED_TAG}: Study Brain REST quota exceeded ({last_err})."
        ));
    }
    Err(format!(
        "Study Brain could not complete the request ({last_err}). Try again or switch Study Brain to Flash Direct Vision in Settings."
    ))
}

/// Strips optional markdown ```json ... ``` fences and locates the outermost JSON object.
pub(crate) fn extract_json_payload(raw: &str) -> &str {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_start())
        .unwrap_or(trimmed);
    let stripped = stripped
        .strip_suffix("```")
        .map(|s| s.trim_end())
        .unwrap_or(stripped);
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && start <= end
    {
        return &stripped[start..=end];
    }
    stripped
}

/// Recursively strips explicit `null` values from JSON objects and arrays so `#[serde(default)]`
/// fields never fail deserialization with `invalid type: null, expected a string/sequence`.
pub(crate) fn strip_json_nulls(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                strip_json_nulls(v);
            }
        }
        Value::Array(arr) => {
            arr.retain(|v| !v.is_null());
            for v in arr.iter_mut() {
                strip_json_nulls(v);
            }
        }
        _ => {}
    }
}

/// Extracts non-thought answer text from Gemini REST `generateContent` JSON.
fn extract_answer_text(resp: &Value) -> Option<String> {
    let parts = resp.pointer("/candidates/0/content/parts")?.as_array()?;
    let mut chunks = Vec::new();
    for part in parts {
        if part.get("thought").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
            let trimmed = t.trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed);
            }
        }
    }
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n\n"))
    }
}

async fn study_solve_handler(args: Value, event_tx: broadcast::Sender<AppEvent>) -> ToolResult {
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if question.is_empty() {
        return ToolResult::err("question is required for study_solve");
    }

    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("screen")
        .trim()
        .to_lowercase();
    let subject = args
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("general")
        .trim()
        .to_lowercase();
    let level = args
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .trim()
        .to_lowercase();
    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("solve")
        .trim()
        .to_lowercase();
    let monitor = args
        .get("monitor")
        .and_then(|v| v.as_u64())
        .map(|m| m as usize);

    let cfg = match ApiConfig::load() {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Cannot load Gemini API config: {e}")),
    };
    if cfg.api_key().is_empty() {
        return ToolResult::err("Gemini API key is missing.");
    }

    let language = args
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| crate::config::detect_language(&crate::config::load_memory()));

    // 1. Capture high-resolution screen/clipboard image (shutter beep fires at ~25ms before encode!)
    let progress_guard = crate::actions::documents::TaskProgressGuard::new(
        event_tx.clone(),
        "Study Brain",
        "Bước 1/3 · Đang chụp màn hình...",
        0.20,
        truncate_chars(&question, 54),
    );
    let captured: Option<CapturedImage> = match source.as_str() {
        "text" | "none" => None,
        "clipboard" => {
            let tx_cap = event_tx.clone();
            match tokio::task::spawn_blocking(move || capture_clipboard_png(Some(&tx_cap))).await {
                Ok(Ok(img)) => Some(img),
                Ok(Err(e)) => return ToolResult::err(e),
                Err(e) => return ToolResult::err(format!("clipboard capture task failed: {e}")),
            }
        }
        _ => {
            let tx_cap = event_tx.clone();
            match tokio::task::spawn_blocking(move || capture_screen_png(monitor, Some(&tx_cap)))
                .await
            {
                Ok(Ok(img)) => Some(img),
                Ok(Err(e)) => return ToolResult::err(e),
                Err(e) => return ToolResult::err(format!("screen capture task failed: {e}")),
            }
        }
    };

    if let Some(ref img) = captured {
        tracing::info!(
            "study_solve captured {} ({} KB, {}x{})",
            img.source_desc,
            img.bytes.len() / 1024,
            img.width,
            img.height
        );
    }

    let system_prompt = build_study_system_prompt(&subject, &level, &mode, &language);
    let user_prompt = if let Some(ref img) = captured {
        format!(
            "Attached image source: {}.\nUser request: {}\n\
             Carefully transcribe every sign, number, exponent, subscript, and option before solving, \
             then double-check your final result.",
            img.source_desc, question
        )
    } else {
        format!(
            "User study request: {}\nSolve/explain accurately step by step and verify your final result.",
            question
        )
    };

    // 2. Execute reasoning via configured Study Brain mode
    // - Flash Direct Vision: passes crisp 1920px Q95 JPEG directly into Gemini Live (~1.2s total, ZERO REST quota!)
    // - Auto (Verified Fast Thinking): thinkingBudget = 512 on multi-model REST cascade (~1.8-3s), with
    //   automatic zero-quota fallback to Direct Crisp Live Vision if all REST buckets hit 20/day limit!
    // - Pro 3.1 Deep Thinking: thinkingBudget = 2048 on Pro -> fast cascade -> Direct Crisp Live Vision fallback
    let mode_cfg = cfg.study_brain_mode();
    if mode_cfg == StudyBrainMode::Flash
        && let Some(ref img) = captured
        && let Ok(jpeg) = captured_to_jpeg(img)
    {
        progress_guard.update("⚡ Đã gửi ảnh sắc nét trực tiếp cho Boogie Live!", 0.96);
        let _ = event_tx.send(AppEvent::Log(format!(
            "Study [live-vision-fast]: Captured {} ({} KB) -> Direct Live Vision",
            img.source_desc,
            jpeg.len() / 1024
        )));
        return ToolResult::ok_with_image(
            format!(
                "[DIRECT CRISP LIVE VISION ({}) — answer immediately! Carefully read every digit, sign (+/-), exponent, subscript, and option A/B/C/D in the attached high-res screenshot, double-check your calculation silently, then give the exact final answer and 2-3 tight steps in {language} matching level={level}, mode={mode}. Request: {question}]",
                img.source_desc
            ),
            jpeg,
        );
    }

    let think_budget = match mode_cfg {
        StudyBrainMode::Flash => Some(0),
        StudyBrainMode::Auto => Some(512),
        StudyBrainMode::Pro => Some(2048),
    };
    progress_guard.update(
        if captured.is_some() {
            "📸 Đã chụp xong! Có thể Alt+Tab — đang đọc dấu & giải..."
        } else {
            "Bước 2/3 · Đang giải & kiểm chứng đáp án..."
        },
        0.52,
    );
    let (answer, used_model) = match call_study_brain(
        &cfg,
        &system_prompt,
        &user_prompt,
        captured.as_ref(),
        false,
        4096,
        think_budget,
        false,
    )
    .await
    {
        Ok(res) => res,
        Err(err) => {
            // Zero-quota resilience: NEVER fail a study question just because REST hit 20/day!
            // Fall back seamlessly to Gemini Live's native crisp vision or direct reasoning.
            if let Some(ref img) = captured
                && let Ok(jpeg) = captured_to_jpeg(img)
            {
                tracing::warn!(
                    "study_solve REST unavailable ({err}); seamlessly falling back to Direct Crisp Live Vision"
                );
                progress_guard.update("⚡ Chuyển sang Live Vision sắc nét trực tiếp...", 0.94);
                let _ = event_tx.send(AppEvent::Log(format!(
                    "Study [live-vision-fallback]: Captured {} -> Direct Live Vision",
                    img.source_desc
                )));
                return ToolResult::ok_with_image(
                    format!(
                        "[DIRECT CRISP LIVE VISION FALLBACK ({}) — secondary REST solver is temporarily at quota, so YOU solve directly from this high-res screenshot! Do NOT say the tool failed. Carefully inspect every digit, sign (+/-), exponent, and option A/B/C/D, double-check your answer silently, and give a clear concise solution in {language} (level={level}, mode={mode}). Request: {question}]",
                        img.source_desc
                    ),
                    jpeg,
                );
            }
            if err.contains(REST_QUOTA_EXHAUSTED_TAG) {
                tracing::warn!(
                    "study_solve text REST quota exhausted; delegating cleanly to Gemini Live"
                );
                return ToolResult::ok(format!(
                    "[DIRECT LIVE SOLVE — secondary REST solver reached daily quota, so solve this directly yourself in {language} (level={level}, mode={mode}) without mentioning any error. Double-check every sign and calculation. Request: {question}]"
                ));
            }
            return ToolResult::err(err);
        }
    };
    progress_guard.update("Bước 3/3 · Hoàn tất lời giải!", 0.96);

    // 3. Push full crisp visual solution directly into Boogie's Conversation Drawer (with Copy button!)
    let _ = event_tx.send(AppEvent::Log(format!("Study [{used_model}]: {answer}")));

    // 4. Return verified solution to Gemini Live so it speaks the exact confirmed answer clearly
    ToolResult::ok(format!(
        "[VERIFIED STUDY SOLUTION via {used_model} — trust these verified numbers, signs & steps; \
         do NOT alter the final answer; the full written solution is already displayed in the user's conversation drawer]\n\n{answer}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_non_thought_answer_parts() {
        let resp = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "Internal scratchpad: checking -2x + 4 = 0..." },
                        { "text": "**Đáp án:** x = 2\n**Cách giải:** -2x = -4 → x = 2." }
                    ]
                }
            }]
        });
        let extracted = extract_answer_text(&resp).unwrap();
        assert!(!extracted.contains("Internal scratchpad"));
        assert!(extracted.contains("x = 2"));
    }

    #[test]
    fn model_chain_respects_study_brain_mode() {
        let flash_chain = select_model_chain(StudyBrainMode::Flash);
        assert_eq!(flash_chain.first().copied(), Some(FLASH_MODEL));
        assert!(flash_chain.contains(&"gemini-3.1-flash-lite"));

        let pro_chain = select_model_chain(StudyBrainMode::Pro);
        assert_eq!(pro_chain.first().copied(), Some(PRO_MODEL));
        assert!(pro_chain.contains(&FLASH_MODEL));
    }

    #[test]
    fn daily_quota_and_404_detection_works() {
        assert!(is_quota_exhausted_for_session(404, "model not found"));
        assert!(is_quota_exhausted_for_session(
            429,
            "Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 20, requests per day"
        ));
        assert!(is_quota_exhausted_for_session(
            429,
            "Quota exceeded, limit: 0 for gemini-3.1-pro-preview"
        ));
        // Temporary per-minute burst should NOT permanently disable a model
        assert!(!is_quota_exhausted_for_session(
            429,
            "Resource exhausted: requests per minute limit exceeded, retry in 10s"
        ));
        assert!(model_supports_thinking("gemini-2.5-flash"));
        assert!(model_supports_thinking("gemini-3.1-flash-lite"));
        assert!(!model_supports_thinking("gemini-2.0-flash"));
    }

    #[test]
    fn captured_image_converts_png_and_jpeg_cleanly_for_live_vision() {
        // Deterministic test (works on unlocked desktops, locked screens, and headless CI alike)
        let mut rgba = vec![0u8; 64 * 48 * 4];
        for i in 0..(64 * 48) {
            rgba[i * 4] = (i % 255) as u8;
            rgba[i * 4 + 1] = 120;
            rgba[i * 4 + 2] = 210;
            rgba[i * 4 + 3] = 255;
        }
        let (png, w, h) = encode_rgba_to_png(&rgba, 64, 48, STUDY_MAX_SCREEN_DIM).unwrap();
        let cap_png = CapturedImage {
            bytes: png,
            mime_type: "image/png",
            width: w,
            height: h,
            source_desc: "synthetic clipboard snip".to_string(),
        };
        let converted_jpg = captured_to_jpeg(&cap_png).expect("PNG -> JPEG conversion must work");
        assert!(converted_jpg.starts_with(&[0xFF, 0xD8]));

        let cap_jpg = CapturedImage {
            bytes: converted_jpg.clone(),
            mime_type: "image/jpeg",
            width: w,
            height: h,
            source_desc: "synthetic screen".to_string(),
        };
        let passthrough = captured_to_jpeg(&cap_jpg).expect("JPEG passthrough must work");
        assert_eq!(passthrough, converted_jpg);

        // When running on an unlocked interactive desktop, also verify live capture + shutter chime.
        if std::env::var("CI").is_err() {
            let (tx, mut rx) = broadcast::channel(8);
            if let Ok(img) = capture_screen_png(None, Some(&tx)) {
                assert_eq!(img.mime_type, "image/jpeg");
                assert!(img.bytes.starts_with(&[0xFF, 0xD8]));
                assert!(matches!(
                    rx.try_recv(),
                    Ok(AppEvent::PlayChime(crate::ChimeKind::ScreenCaptured))
                ));
            }
        }
    }

    #[test]
    fn study_prompt_adapts_to_primary_and_university_levels() {
        let primary = build_study_system_prompt("math", "primary", "solve", "Vietnamese");
        assert!(primary.contains("PRIMARY SCHOOL"));
        assert!(primary.contains("NEVER use algebra systems"));

        let uni = build_study_system_prompt("math", "university", "check", "Vietnamese");
        assert!(uni.contains("UNIVERSITY"));
        assert!(uni.contains("CHECK STUDENT WORK"));
    }
}
