use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::AppEvent;
use crate::config::{ApiConfig, StudyBrainMode};
use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{encode_rgba_to_png, truncate_chars};

use super::{param, tool};

const PRO_MODEL: &str = "gemini-2.5-pro";
const FLASH_MODEL: &str = "gemini-2.5-flash";
const PRO_RATE_LIMIT_BACKOFF_SECS: u64 = 120;
const STUDY_MAX_PNG_DIM: u32 = 2560;

static STUDY_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static PRO_COOLDOWN_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn study_http_client() -> &'static reqwest::Client {
    STUDY_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(48))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default()
    })
}

fn is_pro_in_cooldown() -> bool {
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

struct CapturedImage {
    png_bytes: Vec<u8>,
    width: u32,
    height: u32,
    source_desc: String,
}

fn capture_clipboard_png() -> Result<CapturedImage, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;
    let img = cb
        .get_image()
        .map_err(|_| "No image found in clipboard. Ask the user to snip the problem with Win+Shift+S first, or switch source to 'screen'.".to_string())?;
    let w = img.width as u32;
    let h = img.height as u32;
    let (png, out_w, out_h) = encode_rgba_to_png(&img.bytes, w, h, STUDY_MAX_PNG_DIM)?;
    Ok(CapturedImage {
        png_bytes: png,
        width: out_w,
        height: out_h,
        source_desc: format!("clipboard snip ({out_w}x{out_h} lossless PNG)"),
    })
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
fn find_boogie_window_center() -> Option<(i32, i32)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
    use windows::core::PCWSTR;

    let title: Vec<u16> = OsStr::new("Boogie")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hwnd = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())).ok()?;
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        Some(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2))
    }
}

fn capture_screen_png(monitor_opt: Option<usize>) -> Result<CapturedImage, String> {
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
    let (orig_w, orig_h) = (img.width(), img.height());
    let (png, out_w, out_h) = encode_rgba_to_png(img.as_raw(), orig_w, orig_h, STUDY_MAX_PNG_DIM)?;
    Ok(CapturedImage {
        png_bytes: png,
        width: out_w,
        height: out_h,
        source_desc: format!("monitor {idx} ({out_w}x{out_h} lossless PNG)"),
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
         3. CLEAN OUTPUT FORMAT (in {language}, plain readable Unicode math like x², √, π, ≤, ≥, ≠, → — avoid messy raw LaTeX backslashes):\n\
            - **Đề bài / Problem:** (short verbatim transcription of the problem read — omit if purely conceptual)\n\
            - **Đáp án / Answer:** (crystal-clear final answer or option letter, placed right near the top — unless mode is 'hint')\n\
            - **Cách giải / Steps:** (concise, numbered, easy-to-follow steps)\n\
            - **Kiểm tra lại / Verification:** (1 short line confirming substitution / signs / units / logic check)\n\
         Keep the response crisp, well-structured, and ready both to be read on screen AND summarized aloud."
    )
}

fn select_model_chain(mode: StudyBrainMode) -> Vec<&'static str> {
    match mode {
        StudyBrainMode::Pro => vec![PRO_MODEL, FLASH_MODEL],
        StudyBrainMode::Flash => vec![FLASH_MODEL],
        StudyBrainMode::Auto => {
            if is_pro_in_cooldown() {
                vec![FLASH_MODEL]
            } else {
                vec![PRO_MODEL, FLASH_MODEL]
            }
        }
    }
}

async fn call_gemini_study(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    image: Option<&CapturedImage>,
) -> Result<String, (u16, String)> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
    );

    let mut parts = Vec::new();
    if let Some(img) = image {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&img.png_bytes);
        parts.push(json!({
            "inlineData": {
                "mimeType": "image/png",
                "data": b64
            }
        }));
    }
    parts.push(json!({ "text": user_prompt }));

    // Try with high media resolution + thinking budget first; if API rejects optional config with 400, retry cleanly.
    let gen_configs = [
        json!({
            "temperature": 0.1,
            "maxOutputTokens": 4096,
            "mediaResolution": "MEDIA_RESOLUTION_HIGH",
            "thinkingConfig": { "thinkingBudget": 8192 }
        }),
        json!({
            "temperature": 0.1,
            "maxOutputTokens": 4096,
            "thinkingConfig": { "thinkingBudget": 8192 }
        }),
        json!({
            "temperature": 0.1,
            "maxOutputTokens": 4096
        }),
    ];

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

        let resp = match study_http_client().post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => return Err((0, format!("network/timeout error: {e}"))),
        };

        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();

        if status == 200 {
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
                "study_solve ({model}) 400 on attempt {}: {err_detail} — retrying simpler config",
                attempt + 1
            );
            continue;
        }
        return Err(last_err);
    }

    Err(last_err)
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
    let api_key = cfg.api_key().to_string();
    if api_key.is_empty() {
        return ToolResult::err("Gemini API key is missing.");
    }

    let language = args
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| crate::config::detect_language(&crate::config::load_memory()));

    // 1. Capture lossless high-resolution image when requested
    let captured: Option<CapturedImage> = match source.as_str() {
        "text" | "none" => None,
        "clipboard" => match tokio::task::spawn_blocking(capture_clipboard_png).await {
            Ok(Ok(img)) => Some(img),
            Ok(Err(e)) => return ToolResult::err(e),
            Err(e) => return ToolResult::err(format!("clipboard capture task failed: {e}")),
        },
        _ => match tokio::task::spawn_blocking(move || capture_screen_png(monitor)).await {
            Ok(Ok(img)) => Some(img),
            Ok(Err(e)) => return ToolResult::err(e),
            Err(e) => return ToolResult::err(format!("screen capture task failed: {e}")),
        },
    };

    if let Some(ref img) = captured {
        tracing::info!(
            "study_solve captured {} ({} KB, {}x{})",
            img.source_desc,
            img.png_bytes.len() / 1024,
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

    // 2. Execute deep reasoning via configured Study Brain model cascade (Pro -> Flash fallback)
    let models = select_model_chain(cfg.study_brain_mode());
    let mut final_answer: Option<(String, &str)> = None;
    let mut last_err = String::new();

    for model in models {
        tracing::info!(
            "study_solve invoking {model} (subject={subject}, level={level}, mode={mode}, source={source})"
        );
        match call_gemini_study(
            &api_key,
            model,
            &system_prompt,
            &user_prompt,
            captured.as_ref(),
        )
        .await
        {
            Ok(ans) => {
                final_answer = Some((ans, model));
                break;
            }
            Err((status, msg)) => {
                tracing::warn!("study_solve {model} failed (HTTP {status}): {msg}");
                if model == PRO_MODEL && (status == 429 || status == 503 || status == 404) {
                    mark_pro_rate_limited();
                }
                last_err = format!("{model} (HTTP {status}): {msg}");
            }
        }
    }

    let Some((answer, used_model)) = final_answer else {
        return ToolResult::err(format!(
            "Study Brain could not complete the request ({last_err}). Try again or switch Study Brain to Flash in Settings."
        ));
    };

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
        assert_eq!(select_model_chain(StudyBrainMode::Flash), vec![FLASH_MODEL]);
        assert_eq!(
            select_model_chain(StudyBrainMode::Pro),
            vec![PRO_MODEL, FLASH_MODEL]
        );
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
