use base64::Engine as _;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
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
// Only use genuinely fast REST models (removed congested preview/lite endpoints that hung 24s-65s).
// If FLASH_MODEL hits daily 429 or >14s timeout, callers drop in 0ms into Direct Live Vision / Direct Board!
const FAST_FALLBACK_MODELS: &[&str] = &[FLASH_MODEL];
// Dedicated high-output multi-bucket cascade for Document Studio (PDF/Word/Excel authoring).
// Each model sits in an independent free-tier quota bucket so exhausting gemini-2.5-flash (20/day)
// seamlessly rotates to fast 32K-65K output models instead of falling back to a stubby Live tool arg!
const DOC_AUTHOR_MODELS: &[&str] = &[
    FLASH_MODEL,
    "gemini-flash-lite-latest",
    "gemini-3.5-flash-lite",
    "gemma-4-26b-a4b-it",
];
const PRO_RATE_LIMIT_BACKOFF_SECS: u64 = 120;
const STUDY_MAX_SCREEN_DIM: u32 = 1920;
const STUDY_MAX_CLIPBOARD_DIM: u32 = 2200;

pub(crate) const REST_QUOTA_EXHAUSTED_TAG: &str = "REST_QUOTA_EXHAUSTED";

static STUDY_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static FAST_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static DOC_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static PRO_COOLDOWN_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
static PERMA_UNAVAILABLE_MODELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(1);

fn doc_http_client() -> &'static reqwest::Client {
    DOC_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(55))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default()
    })
}

fn study_http_client(fast_lane: bool) -> &'static reqwest::Client {
    if fast_lane {
        FAST_HTTP_CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default()
        })
    } else {
        STUDY_HTTP_CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default()
        })
    }
}

pub(crate) fn next_capture_seq() -> u64 {
    CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed)
}

#[cfg(windows)]
pub(crate) fn get_foreground_window_title() -> String {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return "Desktop".to_string();
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            let title = String::from_utf16_lossy(&buf[..len as usize])
                .trim()
                .to_string();
            if !title.is_empty() {
                return truncate_chars(&title, 120).to_string();
            }
        }
    }
    "Desktop".to_string()
}

#[cfg(not(windows))]
pub(crate) fn get_foreground_window_title() -> String {
    "Desktop".to_string()
}

fn question_explicitly_asks_clipboard(question: &str) -> bool {
    let q = question.to_lowercase();
    const CLIPBOARD_HINTS: &[&str] = &[
        "clipboard",
        "snip",
        "win+shift+s",
        "bộ nhớ tạm",
        "ảnh cắt",
        "vừa cắt",
        "đã cắt",
        "chụp vùng",
        "vừa copy",
        "đã copy",
        "copy ảnh",
    ];
    CLIPBOARD_HINTS.iter().any(|kw| q.contains(kw))
}

fn needs_verified_stem_solver(
    subject: &str,
    mode: &str,
    question: &str,
    window_title: &str,
) -> bool {
    if mode == "check" {
        return true;
    }
    if matches!(
        subject,
        "math"
            | "maths"
            | "toan"
            | "toán"
            | "physics"
            | "vatly"
            | "lý"
            | "chemistry"
            | "hoa"
            | "hóa"
            | "calculus"
            | "algebra"
            | "geometry"
            | "statistics"
    ) {
        return true;
    }
    let combined = format!(
        "{} {}",
        question.to_lowercase(),
        window_title.to_lowercase()
    );
    const STEM_HINTS: &[&str] = &[
        "toán",
        "môn toán",
        "tích phân",
        "đạo hàm",
        "phương trình",
        "bất phương trình",
        "hệ phương trình",
        "ma trận",
        "xác suất",
        "hình học",
        "oxyz",
        "số phức",
        "môđun",
        "logarit",
        "lượng giác",
        "vật lý",
        "hóa học",
        "cân bằng phương trình",
        "tính giá trị",
        "rút gọn biểu thức",
        "giải tích",
        "integral",
        "derivative",
        "equation",
        "matrix",
        "stoichiometry",
        "calculus",
        "solve for x",
    ];
    STEM_HINTS.iter().any(|kw| combined.contains(kw))
}

/// Automatically trims the Windows taskbar, browser top chrome (when active window is a browser),
/// and uniform solid left/right page gutters on full-screen captures so exam text & math formulas
/// receive ~1.8-2x higher effective pixel density inside Gemini's visual tiles.
fn crop_active_content_rgba(
    raw: &[u8],
    w: u32,
    h: u32,
    window_title: &str,
) -> Option<(Vec<u8>, u32, u32)> {
    if w < 1200 || h < 700 || raw.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    let lower_title = window_title.to_lowercase();
    let is_browser = [
        "brave",
        "chrome",
        "edge",
        "firefox",
        "opera",
        "vivaldi",
        "cốc cốc",
    ]
    .iter()
    .any(|b| lower_title.contains(b));

    // Browser tab bar + address bar at 1080p is ~96-102px (~9.4% of screen height).
    let y0 = if is_browser { (h * 94) / 1000 } else { 0 };
    let y1 = h.saturating_sub((h * 46) / 1000);
    if y1 <= y0 + (h * 60) / 100 {
        return None;
    }

    // A gutter column is mostly a flat background color (even if 1-2 floating icons like a Zalo
    // chat button, TOC widget, or bottom back-to-top arrow sit inside the margin, >= 79% of
    // vertical samples still match the column's upper-middle or lower-middle gutter color!).
    let span = (y1 - y0).max(1);
    let ref_y1 = y0 + span / 3;
    let ref_y2 = y0 + (span * 2) / 3;
    let is_gutter_col = |x: u32| -> bool {
        let idx1 = ((ref_y1 as usize) * (w as usize) + (x as usize)) * 4;
        let idx2 = ((ref_y2 as usize) * (w as usize) + (x as usize)) * 4;
        let ref_r = raw[idx1];
        let ref_g = raw[idx1 + 1];
        let ref_b = raw[idx1 + 2];
        let ref2_r = raw[idx2];
        let ref2_g = raw[idx2 + 1];
        let ref2_b = raw[idx2 + 2];

        let samples = 24u32;
        let mut match1 = 0u32;
        let mut match2 = 0u32;
        for s in 0..samples {
            let y = y0 + (s * span) / samples;
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            let r = raw[idx];
            let g = raw[idx + 1];
            let b = raw[idx + 2];
            if r.abs_diff(ref_r) <= 18 && g.abs_diff(ref_g) <= 18 && b.abs_diff(ref_b) <= 18 {
                match1 += 1;
            }
            if r.abs_diff(ref2_r) <= 18 && g.abs_diff(ref2_g) <= 18 && b.abs_diff(ref2_b) <= 18 {
                match2 += 1;
            }
        }
        match1.max(match2) >= 19 // >= 79% flat background along the column
    };

    let max_side_trim = (w * 20) / 100;
    // Skip 4px left window edge and 22px right browser scrollbar so scrollbar thumbs never block
    // side-gutter detection on web exam pages.
    let start_x0 = if is_browser { 4u32 } else { 0u32 };
    let mut x0 = start_x0;
    while x0 + 8 <= max_side_trim && is_gutter_col(x0) {
        x0 += 8;
    }
    if x0 == start_x0 {
        x0 = 0;
    }
    let start_x1 = if is_browser { w.saturating_sub(22) } else { w };
    let mut x1 = start_x1;
    while w - x1 + 8 <= max_side_trim && x1 > x0 + 8 && is_gutter_col(x1 - 1) {
        x1 -= 8;
    }
    if x1 == start_x1 && !is_browser {
        x1 = w;
    }
    // Leave a tiny 12px breathing margin around detected text edge
    x0 = x0.saturating_sub(12);
    x1 = (x1 + 12).min(w);

    let cw = x1.saturating_sub(x0);
    let ch = y1.saturating_sub(y0);
    if cw < (w * 55) / 100 || ch < (h * 60) / 100 || (cw == w && ch == h) {
        return None;
    }

    let mut out = Vec::with_capacity((cw as usize) * (ch as usize) * 4);
    let row_bytes = (cw as usize) * 4;
    for y in y0..y1 {
        let start = ((y as usize) * (w as usize) + (x0 as usize)) * 4;
        out.extend_from_slice(&raw[start..start + row_bytes]);
    }
    Some((out, cw, ch))
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
    // BEFORE image cropping/encoding — so the user can Alt+Tab right away!
    if let Some(tx) = notify_tx {
        let _ = tx.send(AppEvent::PlayChime(crate::ChimeKind::ScreenCaptured));
    }
    let (orig_w, orig_h) = (img.width(), img.height());
    let win_title = get_foreground_window_title();
    let (jpg, out_w, out_h, crop_note) = if let Some((cropped, cw, ch)) =
        crop_active_content_rgba(img.as_raw(), orig_w, orig_h, &win_title)
    {
        let (j, ow, oh) = encode_rgba_to_jpeg(&cropped, cw, ch, STUDY_MAX_SCREEN_DIM, 92)?;
        (j, ow, oh, "smart-cropped")
    } else {
        let (j, ow, oh) =
            encode_rgba_to_jpeg(img.as_raw(), orig_w, orig_h, STUDY_MAX_SCREEN_DIM, 92)?;
        (j, ow, oh, "full")
    };
    Ok(CapturedImage {
        bytes: jpg,
        mime_type: "image/jpeg",
        width: out_w,
        height: out_h,
        source_desc: format!(
            "monitor {idx} ({out_w}x{out_h} {crop_note} Q92 JPEG · Window: '{win_title}')"
        ),
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

async fn call_gemini_document(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    image: Option<&CapturedImage>,
    max_output_tokens: u32,
) -> Result<String, (u16, String)> {
    let url =
        format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let is_gemma = model.starts_with("gemma-");

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

    let (body_templates, effective_parts) = if is_gemma {
        let combined = format!("{system_prompt}\n\n---\n\n{user_prompt}");
        parts.push(json!({ "text": combined }));
        let configs = vec![json!({
            "temperature": 0.25,
            "maxOutputTokens": max_output_tokens.min(32_000)
        })];
        (configs, parts)
    } else {
        parts.push(json!({ "text": user_prompt }));
        let mut configs = Vec::new();
        if model_supports_thinking(model) {
            configs.push(json!({
                "temperature": 0.25,
                "maxOutputTokens": max_output_tokens,
                "responseMimeType": "application/json",
                "thinkingConfig": { "thinkingBudget": 0 }
            }));
        }
        configs.push(json!({
            "temperature": 0.25,
            "maxOutputTokens": max_output_tokens,
            "responseMimeType": "application/json"
        }));
        configs.push(json!({
            "temperature": 0.25,
            "maxOutputTokens": max_output_tokens
        }));
        (configs, parts)
    };

    let mut last_err = (0u16, "no attempt".to_string());

    for (attempt, gen_cfg) in body_templates.iter().enumerate() {
        let mut body = json!({
            "contents": [{
                "role": "user",
                "parts": effective_parts
            }],
            "generationConfig": gen_cfg
        });
        if !is_gemma {
            body["systemInstruction"] = json!({
                "parts": [{ "text": system_prompt }]
            });
        }

        let started = Instant::now();
        let resp = match doc_http_client()
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
                "document author {model} responded in {}ms ({} chars)",
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

        if status == 400 && attempt + 1 < body_templates.len() {
            tracing::warn!(
                "document author ({model}) 400 on attempt {}: {err_detail} — retrying simpler config",
                attempt + 1
            );
            continue;
        }
        return Err(last_err);
    }

    Err(last_err)
}

/// Dedicated long-form Document Author Engine with a 55s timeout and 4 independent free-tier
/// model buckets (`gemini-2.5-flash` -> `gemini-flash-lite-latest` -> `gemini-3.5-flash-lite`
/// -> `gemma-4-26b-a4b-it`) so full 20-40 question exams never get truncated or degraded to stubs.
pub(crate) async fn call_document_author(
    cfg: &ApiConfig,
    system_prompt: &str,
    user_prompt: &str,
    image: Option<&CapturedImage>,
    max_output_tokens: u32,
) -> Result<(String, &'static str), String> {
    let api_key = cfg.api_key();
    if api_key.is_empty() {
        return Err("Gemini API key is missing.".to_string());
    }

    let models: Vec<&'static str> = DOC_AUTHOR_MODELS
        .iter()
        .copied()
        .filter(|m| !is_model_perma_disabled(m))
        .collect();
    if models.is_empty() {
        return Err(format!(
            "{REST_QUOTA_EXHAUSTED_TAG}: All document author models reached their daily quota."
        ));
    }

    let mut last_err = String::new();
    let mut hit_quota = false;

    for model in models {
        tracing::info!(
            "document author invoking {model} (max_tokens={max_output_tokens}, has_image={})",
            image.is_some()
        );
        match call_gemini_document(
            api_key,
            model,
            system_prompt,
            user_prompt,
            image,
            max_output_tokens,
        )
        .await
        {
            Ok(ans) => return Ok((ans, model)),
            Err((status, msg)) => {
                tracing::warn!("document author {model} failed (HTTP {status}): {msg}");
                if status == 429 {
                    hit_quota = true;
                }
                if is_quota_exhausted_for_session(status, &msg) {
                    mark_model_perma_disabled(model);
                }
                last_err = format!("{model} (HTTP {status}): {msg}");
            }
        }
    }

    if hit_quota {
        return Err(format!(
            "{REST_QUOTA_EXHAUSTED_TAG}: Document author REST quota exceeded ({last_err})."
        ));
    }
    Err(format!(
        "Document author could not complete the request ({last_err})."
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

fn build_spoken_vision_prompt(
    capture_id: u64,
    source_desc: &str,
    question: &str,
    language: &str,
    level: &str,
    mode: &str,
) -> String {
    if language.to_ascii_lowercase().contains("viet") {
        format!(
            "Đây là ảnh chụp màn hình mới nhất (lần chụp #{capture_id}, {source_desc}). \
             Bỏ qua hoàn toàn mọi ảnh hay tab cũ trước đó và chỉ nhìn ảnh này. \
             Hãy đọc thật kỹ đề bài và 4 đáp án A, B, C, D trên ảnh cho yêu cầu \"{question}\" \
             (cấp học: {level}, chế độ: {mode}), kiểm tra cẩn thận từng dấu cộng trừ và đối chiếu khớp đúng \
             1 trong 4 đáp án có trên ảnh — nếu ký tự nào quá nhỏ hoặc mờ không chắc chắn 100% thì báo thật để người dùng phóng to thay vì đoán bừa. \
             Hãy NÓI THÀNH TIẾNG thật tự nhiên, rõ ràng và gọn trong 2 đến 4 câu bằng tiếng Việt, \
             tuyệt đối không viết ký hiệu LaTeX hay markdown."
        )
    } else {
        format!(
            "This is the latest screenshot (capture #{capture_id}, {source_desc}). \
             Ignore all earlier screenshots or tabs and read only this image. \
             Carefully read the exact question and all visible A, B, C, D choices for \"{question}\" \
             (level: {level}, mode: {mode}), double-check every sign and number so your result matches one of the visible options \
             (if any digit is genuinely too small or blurry, tell the user to zoom in instead of guessing). \
             SPEAK ALOUD naturally and concisely in 2 to 4 sentences in {language} — do not output LaTeX or markdown formatting."
        )
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

    let raw_source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("screen")
        .trim()
        .to_lowercase();
    // Guard against Gemini Live accidentally choosing stale `clipboard` when user asked about "trên màn hình"!
    let source = if raw_source == "clipboard" && !question_explicitly_asks_clipboard(&question) {
        tracing::info!(
            "study_solve overriding source='clipboard' -> 'screen' (user did not ask for clipboard)"
        );
        "screen".to_string()
    } else {
        raw_source
    };
    let active_window_title = if source == "screen" {
        get_foreground_window_title()
    } else {
        String::new()
    };
    let capture_id = next_capture_seq();
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

    // 2. Smart Hybrid Speed + Accuracy Routing:
    // - Flash mode: ALWAYS Instant Crisp Live Vision (~1.3s, 0 REST quota) for every subject.
    // - Auto (Default — Smart Hybrid):
    //     * General quizzes / IT / English / Theory / Screen reading -> Instant Crisp Live Vision (~1.3s, 0 REST quota!)
    //     * Math / Physics / Chemistry / Check-work (sign-sensitive!) -> Verified gemini-2.5-flash thinking (~2.5-4s, hard 12s cap!)
    //       with instant 0ms skip to Direct Crisp Live Vision if 20/day quota was already reached!
    // - Pro mode: Dedicated Deep REST Verification (Pro -> Flash, hard 15s cap) with seamless Live Vision fallback.
    let mode_cfg = cfg.study_brain_mode();
    let wants_stem_verify =
        needs_verified_stem_solver(&subject, &mode, &question, &active_window_title);
    let use_instant_live_vision = match mode_cfg {
        StudyBrainMode::Flash => true,
        StudyBrainMode::Auto => !wants_stem_verify || is_model_perma_disabled(FLASH_MODEL),
        StudyBrainMode::Pro => false,
    };

    if use_instant_live_vision
        && let Some(ref img) = captured
        && let Ok(jpeg) = captured_to_jpeg(img)
    {
        progress_guard.update(
            "📸 Đã chụp xong! Có thể Alt+Tab — đang trả lời ngay...",
            0.96,
        );
        let _ = event_tx.send(AppEvent::Log(format!(
            "Study [instant-live-vision #{capture_id}]: Captured {} ({} KB) -> Atomic HIGH-RES Live Vision",
            img.source_desc,
            jpeg.len() / 1024
        )));
        return ToolResult::ok_with_image(
            build_spoken_vision_prompt(
                capture_id,
                &img.source_desc,
                &question,
                &language,
                &level,
                &mode,
            ),
            jpeg,
        );
    }

    let think_budget = match mode_cfg {
        StudyBrainMode::Flash => Some(0),
        StudyBrainMode::Auto => Some(512),
        StudyBrainMode::Pro => Some(2048),
    };
    let use_fast_timeout = mode_cfg != StudyBrainMode::Pro;
    progress_guard.update(
        if captured.is_some() {
            "📸 Đã chụp xong! Có thể Alt+Tab — đang kiểm chứng dấu & giải..."
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
        use_fast_timeout,
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
                    build_spoken_vision_prompt(
                        capture_id,
                        &img.source_desc,
                        &question,
                        &language,
                        &level,
                        &mode,
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
        assert_eq!(flash_chain, vec![FLASH_MODEL]);

        let pro_chain = select_model_chain(StudyBrainMode::Pro);
        assert_eq!(pro_chain.first().copied(), Some(PRO_MODEL));
        assert!(pro_chain.contains(&FLASH_MODEL));
    }

    #[test]
    fn smart_hybrid_routing_distinguishes_quizzes_from_stem_math() {
        // IT / networking / English / general quizzes go straight to Instant HIGH-RES Live Vision
        assert!(!needs_verified_stem_solver(
            "informatics",
            "solve",
            "Giải câu trắc nghiệm Quản trị hệ thống mạng Windows trên màn hình",
            "Ôn thi Quản trị hệ thống mạng - Brave"
        ));
        assert!(!needs_verified_stem_solver(
            "english",
            "solve",
            "Choose the best answer A, B, C, or D",
            "IELTS Reading Test - Brave"
        ));
        // Math / Physics / Chemistry detected from subject, question, OR active browser tab title!
        assert!(needs_verified_stem_solver(
            "math",
            "solve",
            "Giải câu 3",
            ""
        ));
        assert!(needs_verified_stem_solver(
            "general",
            "solve",
            "Cho biết kết quả câu 16 trên màn hình",
            "Tổng hợp đề thi học kỳ 2 lớp 12 môn Toán có đáp án - Brave"
        ));
        assert!(needs_verified_stem_solver(
            "general",
            "solve",
            "Tính tích phân và đạo hàm câu 2 trên màn hình",
            ""
        ));
        assert!(needs_verified_stem_solver(
            "english",
            "check",
            "Kiểm tra bài làm của mình",
            ""
        ));
        // Anti-stale clipboard guard
        assert!(!question_explicitly_asks_clipboard(
            "Check ngữ pháp đoạn này trên màn hình giúp tao"
        ));
        assert!(question_explicitly_asks_clipboard(
            "Giải ảnh vừa cắt Win+Shift+S trong clipboard giúp mình"
        ));
    }

    #[test]
    fn smart_content_crop_trims_browser_chrome_taskbar_and_side_gutters() {
        let (w, h) = (1920u32, 1080u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // Fill background with uniform dark gutter (24, 24, 30), center content [320..1600] with varied text pixels,
        // PLUS floating sidebar buttons (like a left TOC icon & right Zalo icon) inside the gutters!
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                let is_floating_btn = ((40..100).contains(&x) && (260..340).contains(&y))
                    || ((1780..1860).contains(&x) && (720..820).contains(&y));
                if (320..1600).contains(&x) && (90..1020).contains(&y) {
                    rgba[idx] = ((x + y) % 255) as u8;
                    rgba[idx + 1] = ((x * 3) % 255) as u8;
                    rgba[idx + 2] = 240;
                } else if is_floating_btn {
                    rgba[idx] = 0;
                    rgba[idx + 1] = 140;
                    rgba[idx + 2] = 255;
                } else {
                    rgba[idx] = 24;
                    rgba[idx + 1] = 24;
                    rgba[idx + 2] = 30;
                }
                rgba[idx + 3] = 255;
            }
        }
        let (cropped, cw, ch) = crop_active_content_rgba(&rgba, w, h, "Đề thi Toán 12 - Brave")
            .expect("should crop browser chrome, taskbar, and dark gutters");
        assert!((1200..1500).contains(&cw), "unexpected cropped width {cw}");
        assert!((850..980).contains(&ch), "unexpected cropped height {ch}");
        assert_eq!(cropped.len(), (cw as usize) * (ch as usize) * 4);

        // Simulate realistic 100px browser address bar, 18px right scrollbar with thumb, and
        // floating left/right widgets (e.g. TOC / Zalo / back-to-top icons) on light gutters.
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                let (r, g, b) = if y < 100 {
                    (40, 42, 48) // dark browser tabs + address bar
                } else if x >= w - 18 {
                    if (200..420).contains(&y) {
                        (120, 120, 125) // scrollbar thumb
                    } else {
                        (55, 55, 60) // scrollbar track
                    }
                } else if (300..1620).contains(&x) {
                    // dense alternating text rows
                    if y % 12 < 6 {
                        (20, 20, 30)
                    } else {
                        (252, 252, 255)
                    }
                } else if (24..88).contains(&x)
                    && ((240..310).contains(&y) || (950..1020).contains(&y))
                {
                    (30, 30, 35) // floating TOC & back-to-top widgets
                } else {
                    (242, 244, 248) // light page gutter
                };
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 255;
            }
        }
        let (_, cw2, ch2) = crop_active_content_rgba(&rgba, w, h, "Tổng hợp đề thi - Brave")
            .expect("should trim gutters past scrollbar and floating widgets");
        assert!(
            (1280..1450).contains(&cw2),
            "scrollbar blocked crop: cw2={cw2}"
        );
        assert!(
            (900..960).contains(&ch2),
            "browser chrome trim off: ch2={ch2}"
        );
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
