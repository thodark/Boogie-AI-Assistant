use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use docx_rs::{
    AlignmentType, BreakType, Docx, Paragraph, Run, RunFonts, Shading, Table, TableCell, TableRow,
    WidthType,
};
use rust_xlsxwriter::{Color as XlsxColor, Format, FormatAlign, FormatBorder, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::AppEvent;
use crate::actions::study::{
    CapturedImage, REST_QUOTA_EXHAUSTED_TAG, call_study_brain, capture_clipboard_png,
    capture_screen_png, extract_json_payload, strip_json_nulls,
};
use crate::config::ApiConfig;
use crate::tools::{ToolRegistry, ToolResult};

use super::{param, tool};

static TEMP_DOC_COUNTER: AtomicU64 = AtomicU64::new(1);
static ACTIVE_DOC_GEN: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TaskProgressGuard {
    tx: broadcast::Sender<AppEvent>,
    title: String,
    detail: String,
}

impl TaskProgressGuard {
    pub(crate) fn new(
        tx: broadcast::Sender<AppEvent>,
        title: impl Into<String>,
        stage: impl Into<String>,
        progress: f32,
        detail: impl Into<String>,
    ) -> Self {
        let title = title.into();
        let detail = detail.into();
        let _ = tx.send(AppEvent::TaskProgress(Some(crate::TaskProgress {
            title: title.clone(),
            stage: stage.into(),
            progress: progress.clamp(0.0, 1.0),
            detail: detail.clone(),
        })));
        Self { tx, title, detail }
    }

    pub(crate) fn update(&self, stage: impl Into<String>, progress: f32) {
        let _ = self
            .tx
            .send(AppEvent::TaskProgress(Some(crate::TaskProgress {
                title: self.title.clone(),
                stage: stage.into(),
                progress: progress.clamp(0.0, 1.0),
                detail: self.detail.clone(),
            })));
    }
}

impl Drop for TaskProgressGuard {
    fn drop(&mut self) {
        let _ = self.tx.send(AppEvent::TaskProgress(None));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocFormat {
    Pdf,
    Docx,
    Xlsx,
}

impl DocFormat {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pdf" | ".pdf" => Ok(Self::Pdf),
            "docx" | ".docx" | "word" | "doc" | ".doc" => Ok(Self::Docx),
            "xlsx" | ".xlsx" | "excel" | "xls" | ".xls" | "spreadsheet" | "sheet" => Ok(Self::Xlsx),
            "pptx" | ".pptx" | "ppt" | ".ppt" | "powerpoint" | "slides" | "slide" => Err(
                "PowerPoint (.pptx) is intentionally not supported. Please choose PDF (.pdf), Word (.docx), or Excel (.xlsx)."
                    .to_string(),
            ),
            other => Err(format!(
                "Unsupported document format '{other}'. Supported formats are: pdf, docx (Word), xlsx (Excel)."
            )),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
        }
    }

    pub fn filter_label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF Document",
            Self::Docx => "Word Document",
            Self::Xlsx => "Excel Workbook",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx => "Word (.docx)",
            Self::Xlsx => "Excel (.xlsx)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentSpec {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub sections: Vec<DocSection>,
    #[serde(default)]
    pub answer_key: Vec<AnswerItem>,
    #[serde(default)]
    pub sheets: Vec<ExcelSheet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocSection {
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub paragraphs: Vec<String>,
    #[serde(default)]
    pub bullets: Vec<String>,
    #[serde(default)]
    pub callout: Option<String>,
    #[serde(default)]
    pub table: Option<DocTable>,
    #[serde(default)]
    pub questions: Vec<ExamQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocTable {
    #[serde(default)]
    pub headers: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExamQuestion {
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub points: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnswerItem {
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExcelSheet {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub headers: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
    #[serde(default)]
    pub notes: Vec<String>,
}

pub fn default_boogie_save_dir() -> PathBuf {
    if let Some(docs) = dirs::document_dir() {
        let boogie_dir = docs.join("Boogie");
        let _ = std::fs::create_dir_all(&boogie_dir);
        if boogie_dir.is_dir() {
            return boogie_dir;
        }
        return docs;
    }
    dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Sanitizes a user/AI-provided filename for Windows NTFS and enforces a default extension when needed.
pub fn sanitize_filename(raw: &str, required_ext: Option<&str>) -> String {
    let mut cleaned: String = raw
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => ' ',
            other => other,
        })
        .collect();

    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }
    cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() {
        cleaned = "Boogie_Document".to_string();
    }
    // Cap stem length safely so full Windows path never overflows
    if cleaned.chars().count() > 90 {
        cleaned = cleaned
            .chars()
            .take(90)
            .collect::<String>()
            .trim()
            .to_string();
    }

    if let Some(ext) = required_ext {
        let ext_clean = ext.trim_start_matches('.');
        if !ext_clean.is_empty() {
            let suffix = format!(".{ext_clean}");
            if !cleaned
                .to_ascii_lowercase()
                .ends_with(&suffix.to_ascii_lowercase())
            {
                cleaned.push_str(&suffix);
            }
        }
    }
    cleaned
}

/// Opens the MANDATORY native Windows Save As dialog so the user explicitly picks
/// the destination folder and filename before Boogie writes any file to disk.
pub fn pick_save_path(
    title: &str,
    default_filename: &str,
    filter_name: &str,
    extensions: &[&str],
    initial_dir: Option<&Path>,
    allow_all_files: bool,
) -> Result<Option<PathBuf>, String> {
    let start_dir = initial_dir
        .filter(|d| d.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(default_boogie_save_dir);

    let primary_ext = extensions
        .first()
        .copied()
        .filter(|e| !e.is_empty() && *e != "*");
    let safe_name = sanitize_filename(default_filename, primary_ext);

    let mut dialog = rfd::FileDialog::new()
        .set_title(title)
        .set_directory(&start_dir)
        .set_file_name(&safe_name);

    if !extensions.is_empty() && extensions[0] != "*" {
        dialog = dialog.add_filter(filter_name, extensions);
    }
    if allow_all_files {
        dialog = dialog.add_filter("All Files", &["*"]);
    }

    // Bring Boogie window to foreground right before opening Save As so Windows places the
    // picker visibly on top rather than hidden behind Chrome/Word/other apps.
    crate::actions::study::bring_boogie_window_to_front();

    let Some(mut chosen) = dialog.save_file() else {
        return Ok(None);
    };

    if let Some(ext) = primary_ext
        && !allow_all_files
    {
        let current_ext = chosen
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let matches_allowed = current_ext
            .as_deref()
            .is_some_and(|cur| extensions.iter().any(|e| e.eq_ignore_ascii_case(cur)));
        if !matches_allowed {
            let mut os_name = chosen.file_name().unwrap_or_default().to_os_string();
            os_name.push(format!(".{ext}"));
            chosen.set_file_name(os_name);
        }
    }

    let chosen_str = chosen.to_string_lossy();
    if crate::confirm::is_sensitive_write_path(&chosen_str) {
        return Err(format!(
            "Refused to save into a protected Windows system path ({chosen_str}). Please choose a normal folder such as Documents or Desktop."
        ));
    }

    Ok(Some(chosen))
}

pub fn register(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    registry.register(
        tool(
            "generate_document",
            "Document Studio: create structured study guides (đề cương ôn thi), practice exams with answer keys, \
             lecture notes, comparison tables, or spreadsheets with formulas in PDF (.pdf), Word (.docx), or Excel (.xlsx). \
             PowerPoint (.pptx) is NOT supported. \
             STRICT PRE-FLIGHT REQUIREMENT: NEVER call this tool until the user has explicitly specified BOTH \
             the topic AND the file format ('pdf', 'docx'/Word, or 'xlsx'/Excel). If the user omitted the file format \
             (e.g. only said 'tạo đề ôn tập' without saying PDF/Word/Excel), DO NOT guess — ask them 1 crisp question \
             confirming which format they want (and question count if relevant) BEFORE calling this tool! \
             IMPORTANT: Once confirmed, ALWAYS speak ONE brief friendly sentence aloud FIRST (e.g. 'Ok đợi mình vài giây, \
             mình đang soạn đề cho bạn ngay đây!') BEFORE emitting this tool call. \
             NEVER re-call this tool for an older topic that was already saved; pass ONLY the user's NEWEST requested topic. \
             ALWAYS pops up the native Windows Save As dialog so the user picks the exact folder & filename before saving.",
            vec![
                (
                    "topic",
                    param(
                        "topic",
                        "string",
                        "Detailed description of what document to generate (e.g. 'Đề cương ôn thi Toán 12 chương Khảo sát hàm số gồm 10 câu có đáp án', 'Bảng quản lý chi tiêu tháng Excel có công thức tính tổng')",
                    ),
                ),
                (
                    "format",
                    param(
                        "format",
                        "string",
                        "Output format: 'pdf', 'docx' (Word), or 'xlsx' (Excel). Do NOT pass 'pptx'.",
                    ),
                ),
                (
                    "title",
                    param(
                        "title",
                        "string",
                        "Optional clean document title / suggested filename",
                    ),
                ),
                (
                    "subject",
                    param(
                        "subject",
                        "string",
                        "Optional subject hint (e.g. math, physics, english, history, informatics, finance, general)",
                    ),
                ),
                (
                    "level",
                    param(
                        "level",
                        "string",
                        "Optional academic level: auto (default), primary (Cấp 1), middle (Cấp 2), high (Cấp 3), university (Đại học)",
                    ),
                ),
                (
                    "question_count",
                    param(
                        "question_count",
                        "integer",
                        "Optional number of questions/exercises to include when generating an exam or study guide",
                    ),
                ),
                (
                    "include_answers",
                    param(
                        "include_answers",
                        "boolean",
                        "Whether to include an Answer Key & step-by-step solutions section (default: true)",
                    ),
                ),
                (
                    "source",
                    param(
                        "source",
                        "string",
                        "Where source material comes from: 'text' (default — generate from topic), 'screen' (read current screen slide/doc), or 'clipboard' (read Win+Shift+S image)",
                    ),
                ),
                (
                    "language",
                    param(
                        "language",
                        "string",
                        "Document language (e.g. 'Vietnamese', 'English'); defaults to the user's conversation language",
                    ),
                ),
                (
                    "auto_open",
                    param(
                        "auto_open",
                        "boolean",
                        "Open the saved document automatically right after saving (default: true)",
                    ),
                ),
                (
                    "spec_json",
                    param(
                        "spec_json",
                        "string",
                        "Optional direct DocumentSpec JSON string (for zero-REST-quota fallback or pre-structured content): \
                         {\"title\",\"subtitle\",\"subject\",\"level\",\"sections\":[{\"heading\",\"paragraphs\",\"bullets\",\"callout\",\"table\",\"questions\"}],\"answer_key\":[{\"number\",\"answer\",\"explanation\"}],\"sheets\":[{\"name\",\"headers\",\"rows\",\"notes\"}]}.",
                    ),
                ),
            ],
            vec!["topic", "format"],
        ),
        Box::new(move |args| {
            let tx = event_tx.clone();
            Box::pin(generate_document_handler(args, tx))
        }),
    );
}

fn build_doc_system_prompt(
    format: DocFormat,
    subject: &str,
    level: &str,
    question_count: Option<u64>,
    include_answers: bool,
    language: &str,
) -> String {
    let q_rule = match question_count {
        Some(n) if n > 0 => format!(
            "Include EXACTLY {n} well-crafted, non-repetitive questions/exercises with clear numbering."
        ),
        _ => "Include an appropriate number of structured sections, examples, or exercises for the request.".to_string(),
    };
    let ans_rule = if include_answers {
        "Populate `answer_key` with verified, double-checked answers and concise step-by-step explanations for every question."
    } else {
        "Leave `answer_key` as an empty array []."
    };

    let format_schema = match format {
        DocFormat::Xlsx => {
            "TARGET FORMAT: EXCEL WORKBOOK (.xlsx).\n\
             Populate `sheets` with 1 to 3 clean, practical worksheets. Each sheet has:\n\
             - `name` (short sheet tab name <= 28 chars, no / \\ ? * : [ ])\n\
             - `headers` (array of column header strings)\n\
             - `rows` (2D array of string cells; use plain numeric strings like \"125000\" for numbers and real Excel formulas starting with \"=\" like \"=SUM(B2:B10)\" or \"=AVERAGE(C2:C10)\" whenever totals/averages/calculations make sense!)\n\
             - `notes` (optional short notes shown below the table)\n\
             Also fill `title`, `subtitle`, `subject`, `level` cleanly."
        }
        DocFormat::Pdf | DocFormat::Docx => {
            "TARGET FORMAT: PRINT-READY DOCUMENT (PDF / Word).\n\
             Populate `sections` richly with:\n\
             - `heading` (clear section heading)\n\
             - `paragraphs` (concise well-written paragraphs)\n\
             - `bullets` (key points / formulas / rules)\n\
             - `callout` (optional highlighted formula / theorem / important tip string, or null)\n\
             - `table` (optional { \"headers\": [...], \"rows\": [[...], ...] } for summary tables / comparison tables, or null)\n\
             - `questions` (array of { \"number\": \"Câu 1\", \"prompt\": \"...\", \"choices\": [\"A. ...\", \"B. ...\", \"C. ...\", \"D. ...\"], \"points\": \"1.0 đ\" } — `choices` can be empty [] for essay/short-answer questions)\n\
             Use clean readable Unicode math (x², x³, √, π, ≤, ≥, ≠, →, ∫, ∑, Δ, H₂SO₄) instead of raw LaTeX backslashes."
        }
    };

    format!(
        "You are Boogie's Document Studio — an elite academic & productivity author.\n\
         Output ONLY a single valid JSON object (no markdown fences, no commentary outside JSON).\n\
         Target language: {language}.\n\
         Subject: {subject}. Education level: {level} (adapt depth strictly to Primary/Middle/High/University when specified).\n\
         {q_rule}\n\
         {ans_rule}\n\
         {format_schema}\n\n\
         REQUIRED JSON SCHEMA:\n\
         {{\n\
           \"title\": \"string\",\n\
           \"subtitle\": \"string\",\n\
           \"subject\": \"string\",\n\
           \"level\": \"string\",\n\
           \"sections\": [\n\
             {{\n\
               \"heading\": \"string\",\n\
               \"paragraphs\": [\"string\"],\n\
               \"bullets\": [\"string\"],\n\
               \"callout\": \"string or null\",\n\
               \"table\": {{ \"headers\": [\"string\"], \"rows\": [[\"string\"]] }} or null,\n\
               \"questions\": [\n\
                 {{ \"number\": \"string\", \"prompt\": \"string\", \"choices\": [\"string\"], \"points\": \"string\" }}\n\
               ]\n\
             }}\n\
           ],\n\
           \"answer_key\": [\n\
             {{ \"number\": \"string\", \"answer\": \"string\", \"explanation\": \"string\" }}\n\
           ],\n\
           \"sheets\": [\n\
             {{ \"name\": \"string\", \"headers\": [\"string\"], \"rows\": [[\"string\"]], \"notes\": [\"string\"] }}\n\
           ]\n\
         }}\n\
         CRITICAL ACCURACY & CONCISENESS: Double-check every calculation, sign (+/-), unit, historical fact, and Excel formula reference before emitting JSON. Keep each question prompt and answer explanation crisp and concise so the entire JSON fits cleanly without truncation."
    )
}

fn parse_document_spec_lenient(raw_json: &str) -> Result<DocumentSpec, String> {
    let payload = extract_json_payload(raw_json);
    let mut val: Value =
        serde_json::from_str(payload).map_err(|e| format!("json syntax error: {e}"))?;
    strip_json_nulls(&mut val);
    serde_json::from_value(val).map_err(|e| format!("schema decode error: {e}"))
}

async fn generate_document_handler(
    args: Value,
    event_tx: broadcast::Sender<AppEvent>,
) -> ToolResult {
    let my_gen = ACTIVE_DOC_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if topic.is_empty() {
        return ToolResult::err("topic is required for generate_document");
    }

    let raw_fmt = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw_fmt.is_empty()
        || matches!(
            raw_fmt.to_ascii_lowercase().as_str(),
            "auto" | "any" | "unspecified" | "unknown"
        )
    {
        return ToolResult::err(
            "Missing required file format. Ask the user briefly whether they want PDF (.pdf), Word (.docx), or Excel (.xlsx) before calling generate_document.",
        );
    }
    let format = match DocFormat::parse(raw_fmt) {
        Ok(f) => f,
        Err(e) => return ToolResult::err(e),
    };

    let status_guard = TaskProgressGuard::new(
        event_tx.clone(),
        format!("Document Studio · {}", format.display_name()),
        "Bước 1/4 · Chuẩn bị yêu cầu & nguồn dữ liệu...",
        0.15,
        crate::util::truncate_chars(&topic, 54),
    );

    let title_hint = args
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let subject = args
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("general")
        .trim()
        .to_string();
    let level = args
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .trim()
        .to_string();
    let question_count = args.get("question_count").and_then(|v| v.as_u64());
    let include_answers = args
        .get("include_answers")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("text")
        .trim()
        .to_lowercase();
    let auto_open = args
        .get("auto_open")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let cfg = match ApiConfig::load() {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Cannot load Gemini API config: {e}")),
    };

    let language = args
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| crate::config::detect_language(&crate::config::load_memory()));

    let direct_spec_raw: Option<String> = args.get("spec_json").and_then(|v| match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(_) => Some(v.to_string()),
        _ => None,
    });

    let (raw_json, used_model) = if let Some(direct_raw) = direct_spec_raw {
        tracing::info!("generate_document using direct spec_json (zero REST quota)");
        (direct_raw, "gemini-live-direct")
    } else {
        let captured: Option<CapturedImage> = match source.as_str() {
            "clipboard" => {
                let tx_cap = event_tx.clone();
                match tokio::task::spawn_blocking(move || capture_clipboard_png(Some(&tx_cap)))
                    .await
                {
                    Ok(Ok(img)) => Some(img),
                    Ok(Err(e)) => return ToolResult::err(e),
                    Err(e) => return ToolResult::err(format!("clipboard capture failed: {e}")),
                }
            }
            "screen" => {
                let tx_cap = event_tx.clone();
                match tokio::task::spawn_blocking(move || capture_screen_png(None, Some(&tx_cap)))
                    .await
                {
                    Ok(Ok(img)) => Some(img),
                    Ok(Err(e)) => return ToolResult::err(e),
                    Err(e) => return ToolResult::err(format!("screen capture failed: {e}")),
                }
            }
            _ => None,
        };

        let _ = event_tx.send(AppEvent::Log(format!(
            "Drafting {} document...",
            format.display_name()
        )));

        let system_prompt = build_doc_system_prompt(
            format,
            &subject,
            &level,
            question_count,
            include_answers,
            &language,
        );
        let user_prompt = if let Some(ref img) = captured {
            format!(
                "Attached reference image ({}).\nCreate a complete, polished {} document for: {}\nTitle hint: {}",
                img.source_desc,
                format.display_name(),
                topic,
                title_hint
                    .as_deref()
                    .unwrap_or("(choose an appropriate clear title)")
            )
        } else {
            format!(
                "Create a complete, polished {} document for: {}\nTitle hint: {}",
                format.display_name(),
                topic,
                title_hint
                    .as_deref()
                    .unwrap_or("(choose an appropriate clear title)")
            )
        };

        status_guard.update(
            if captured.is_some() {
                "📸 Đã chụp xong! Có thể Alt+Tab — đang soạn nội dung..."
            } else {
                "Bước 2/4 · Đang soạn nội dung, câu hỏi & đáp án..."
            },
            0.38,
        );

        // Fast lane (multi-model free-tier cascade, thinkingBudget=0, maxOutputTokens=16384):
        // generates complete exams/outlines in ~3-7s with ZERO mid-JSON truncation!
        match call_study_brain(
            &cfg,
            &system_prompt,
            &user_prompt,
            captured.as_ref(),
            true,
            16_384,
            Some(0),
            true,
        )
        .await
        {
            Ok(res) => res,
            Err(e) => {
                if e.contains(REST_QUOTA_EXHAUSTED_TAG) {
                    tracing::warn!(
                        "generate_document REST quota exhausted; asking Gemini Live to pass direct spec_json"
                    );
                    let schema_hint = if format == DocFormat::Xlsx {
                        r#"{"title":"...","subtitle":"...","sheets":[{"name":"Sheet1","headers":["..."],"rows":[["..."]],"notes":[]}]}"#
                    } else {
                        r#"{"title":"...","subtitle":"...","subject":"...","level":"...","sections":[{"heading":"...","paragraphs":["..."],"bullets":["..."],"questions":[{"number":"Câu 1","prompt":"...","choices":["A. ...","B. ...","C. ...","D. ..."],"points":"1 đ"}]}],"answer_key":[{"number":"Câu 1","answer":"A","explanation":"..."}]}"#
                    };
                    return ToolResult::ok(format!(
                        "[ACTION REQUIRED — DIRECT DOCUMENT MODE: Secondary REST quota is full today, so supply the document content yourself! IMMEDIATELY call generate_document(topic={topic:?}, format={raw_fmt:?}, spec_json=<valid JSON matching {schema_hint}>) — Boogie will pop up Save As and render the {} file locally with ZERO REST quota! Do NOT tell the user it failed.]",
                        format.display_name()
                    ));
                }
                return ToolResult::err(e);
            }
        }
    };

    // If a newer generate_document call superseded this one while waiting on the API, abort cleanly
    // so an older Save As dialog NEVER pops up out of order!
    if ACTIVE_DOC_GEN.load(Ordering::SeqCst) != my_gen {
        tracing::info!(
            "generate_document (gen={my_gen}) superseded by newer request — skipping Save As"
        );
        return ToolResult::ok("Superseded by a newer document request.".to_string());
    }

    let mut spec: DocumentSpec = match parse_document_spec_lenient(&raw_json) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("DocumentSpec JSON parse failed ({e}); using fallback text document");
            DocumentSpec {
                title: title_hint
                    .clone()
                    .unwrap_or_else(|| "Boogie Study Document".to_string()),
                subtitle: topic.clone(),
                subject: subject.clone(),
                level: level.clone(),
                sections: vec![DocSection {
                    heading: "Content".to_string(),
                    paragraphs: vec![raw_json.trim().to_string()],
                    ..Default::default()
                }],
                ..Default::default()
            }
        }
    };

    if spec.title.trim().is_empty() {
        spec.title = title_hint.unwrap_or_else(|| "Boogie_Document".to_string());
    }

    // Mandatory Native Save As Picker — NEVER save silently without user choosing folder & filename
    status_guard.update(
        format!(
            "Bước 3/4 · Đang mở hộp thoại Save As cho '{}' — hãy chọn nơi lưu!",
            crate::util::truncate_chars(&spec.title, 36)
        ),
        0.78,
    );
    let _ = event_tx.send(AppEvent::Log(
        "Waiting for you to pick a save location...".to_string(),
    ));
    let suggested_name = sanitize_filename(&spec.title, Some(format.extension()));
    let picker_title = format!("Boogie — Save {} As", format.display_name());
    let filter_label = format.filter_label();
    let ext = format.extension();

    let chosen_opt = match tokio::task::spawn_blocking(move || {
        pick_save_path(
            &picker_title,
            &suggested_name,
            filter_label,
            &[ext],
            None,
            false,
        )
    })
    .await
    {
        Ok(Ok(opt)) => opt,
        Ok(Err(e)) => return ToolResult::err(e),
        Err(e) => return ToolResult::err(format!("Save dialog task failed: {e}")),
    };

    let Some(chosen_path) = chosen_opt else {
        crate::confirm::write_audit_log("generate_document", "CANCELLED_USER", &spec.title);
        let _ = event_tx.send(AppEvent::Log("Document save cancelled".to_string()));
        return ToolResult::ok(
            "User cancelled the Save As dialog. No document was saved.".to_string(),
        );
    };

    status_guard.update(
        format!("Bước 4/4 · Đang xuất file {}...", format.display_name()),
        0.92,
    );
    let render_path = chosen_path.clone();
    let render_spec = spec.clone();
    let render_res =
        tokio::task::spawn_blocking(move || render_document(&render_spec, format, &render_path))
            .await;

    match render_res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return ToolResult::err(format!("Failed to render {}: {e}", format.display_name()));
        }
        Err(e) => return ToolResult::err(format!("Document render task failed: {e}")),
    }

    let path_str = chosen_path.display().to_string();
    crate::confirm::write_audit_log("generate_document", "SAVED", &path_str);

    if auto_open {
        let _ = open::that(&chosen_path);
    }

    let summary_log = format!(
        "Study [{used_model}]: Saved {} — {}\nPath: {}",
        format.display_name(),
        spec.title,
        path_str
    );
    let _ = event_tx.send(AppEvent::Log(summary_log));

    ToolResult::ok(format!(
        "Successfully generated {} '{}' via {} and saved to: {}{}",
        format.display_name(),
        spec.title,
        used_model,
        path_str,
        if auto_open {
            " (opened automatically)."
        } else {
            "."
        }
    ))
}

/// Renders a `DocumentSpec` into the requested format (`Pdf`, `Docx`, or `Xlsx`) at `output_path`.
pub fn render_document(
    spec: &DocumentSpec,
    format: DocFormat,
    output_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent folder failed: {e}"))?;
    }
    match format {
        DocFormat::Docx => render_docx(spec, output_path),
        DocFormat::Xlsx => render_xlsx(spec, output_path),
        DocFormat::Pdf => render_pdf(spec, output_path),
    }
}

/// Interactive helper for the Study Visual UI Export buttons (always opens native Save As dialog).
pub fn save_spec_with_dialog(
    spec: &DocumentSpec,
    format: DocFormat,
    auto_open: bool,
) -> Result<Option<PathBuf>, String> {
    let title = if spec.title.trim().is_empty() {
        "Boogie_Study_Notes"
    } else {
        spec.title.trim()
    };
    let suggested = sanitize_filename(title, Some(format.extension()));
    let picker_title = format!("Boogie — Export {} As", format.display_name());
    let Some(chosen) = pick_save_path(
        &picker_title,
        &suggested,
        format.filter_label(),
        &[format.extension()],
        None,
        false,
    )?
    else {
        crate::confirm::write_audit_log("study_visual_export", "CANCELLED_USER", title);
        return Ok(None);
    };

    render_document(spec, format, &chosen)?;
    crate::confirm::write_audit_log(
        "study_visual_export",
        "SAVED",
        &chosen.display().to_string(),
    );
    if auto_open {
        let _ = open::that(&chosen);
    }
    Ok(Some(chosen))
}

fn ui_fonts() -> RunFonts {
    RunFonts::new()
        .ascii("Segoe UI")
        .hi_ansi("Segoe UI")
        .east_asia("Segoe UI")
        .cs("Segoe UI")
}

fn styled_run(text: &str, half_pts: usize, color: &str, bold: bool) -> Run {
    let mut r = Run::new()
        .add_text(text)
        .size(half_pts)
        .color(color)
        .fonts(ui_fonts());
    if bold {
        r = r.bold();
    }
    r
}

/// Renders a crisp Vietnamese-UTF8 Word `.docx` document in pure Rust.
pub fn render_docx(spec: &DocumentSpec, output_path: &Path) -> Result<(), String> {
    let mut doc = Docx::new();

    // Title
    let title_text = if spec.title.trim().is_empty() {
        "Boogie Study Document"
    } else {
        spec.title.trim()
    };
    doc = doc.add_paragraph(
        Paragraph::new()
            .align(AlignmentType::Left)
            .add_run(styled_run(title_text, 40, "1E3A8A", true)),
    );

    // Subtitle & metadata pill line
    let mut meta_parts = Vec::new();
    if !spec.subject.trim().is_empty() && !spec.subject.eq_ignore_ascii_case("general") {
        meta_parts.push(spec.subject.trim().to_string());
    }
    if !spec.level.trim().is_empty() && !spec.level.eq_ignore_ascii_case("auto") {
        meta_parts.push(spec.level.trim().to_string());
    }
    meta_parts.push(chrono::Local::now().format("%Y-%m-%d").to_string());

    if !spec.subtitle.trim().is_empty() {
        doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
            spec.subtitle.trim(),
            22,
            "475569",
            false,
        )));
    }
    doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
        &meta_parts.join("  •  "),
        19,
        "64748B",
        false,
    )));
    doc = doc.add_paragraph(Paragraph::new());

    for section in &spec.sections {
        if !section.heading.trim().is_empty() {
            doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
                section.heading.trim(),
                28,
                "1D4ED8",
                true,
            )));
        }

        for para in &section.paragraphs {
            if !para.trim().is_empty() {
                doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
                    para.trim(),
                    22,
                    "1E293B",
                    false,
                )));
            }
        }

        for bullet in &section.bullets {
            if !bullet.trim().is_empty() {
                let text = format!("•  {}", bullet.trim());
                doc = doc.add_paragraph(
                    Paragraph::new()
                        .indent(Some(360), None, None, None)
                        .add_run(styled_run(&text, 22, "1E293B", false)),
                );
            }
        }

        if let Some(callout) = section
            .callout
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let cell = TableCell::new()
                .shading(Shading::new().fill("EFF6FF"))
                .width(9000, WidthType::Dxa)
                .add_paragraph(
                    Paragraph::new()
                        .add_run(styled_run("★  ", 22, "1D4ED8", true))
                        .add_run(styled_run(callout, 22, "1E3A8A", false)),
                );
            doc = doc.add_table(Table::new(vec![TableRow::new(vec![cell])]));
            doc = doc.add_paragraph(Paragraph::new());
        }

        if let Some(tbl) = &section.table
            && (!tbl.headers.is_empty() || !tbl.rows.is_empty())
        {
            let col_count = tbl
                .headers
                .len()
                .max(tbl.rows.iter().map(Vec::len).max().unwrap_or(1))
                .max(1);
            let col_w = (9000 / col_count).max(900);
            let mut table_rows = Vec::new();

            if !tbl.headers.is_empty() {
                let header_cells: Vec<TableCell> = tbl
                    .headers
                    .iter()
                    .map(|h| {
                        TableCell::new()
                            .shading(Shading::new().fill("1E3A8A"))
                            .width(col_w, WidthType::Dxa)
                            .add_paragraph(Paragraph::new().add_run(styled_run(
                                h.trim(),
                                20,
                                "FFFFFF",
                                true,
                            )))
                    })
                    .collect();
                table_rows.push(TableRow::new(header_cells));
            }

            for (r_idx, row) in tbl.rows.iter().enumerate() {
                let bg = if r_idx % 2 == 1 { "F8FAFC" } else { "FFFFFF" };
                let cells: Vec<TableCell> = (0..col_count)
                    .map(|c_idx| {
                        let val = row.get(c_idx).map(String::as_str).unwrap_or("").trim();
                        TableCell::new()
                            .shading(Shading::new().fill(bg))
                            .width(col_w, WidthType::Dxa)
                            .add_paragraph(
                                Paragraph::new().add_run(styled_run(val, 20, "1E293B", false)),
                            )
                    })
                    .collect();
                table_rows.push(TableRow::new(cells));
            }

            if !table_rows.is_empty() {
                doc = doc.add_table(Table::new(table_rows));
                doc = doc.add_paragraph(Paragraph::new());
            }
        }

        for (q_idx, q) in section.questions.iter().enumerate() {
            let num = if q.number.trim().is_empty() {
                format!("Câu {}", q_idx + 1)
            } else {
                q.number.trim().to_string()
            };
            let mut p = Paragraph::new()
                .add_run(styled_run(&format!("{num}. "), 22, "0F172A", true))
                .add_run(styled_run(q.prompt.trim(), 22, "1E293B", false));
            if !q.points.trim().is_empty() {
                p = p.add_run(styled_run(
                    &format!("  ({})", q.points.trim()),
                    19,
                    "64748B",
                    false,
                ));
            }
            doc = doc.add_paragraph(p);

            for choice in &q.choices {
                if !choice.trim().is_empty() {
                    doc = doc.add_paragraph(
                        Paragraph::new()
                            .indent(Some(420), None, None, None)
                            .add_run(styled_run(choice.trim(), 21, "334155", false)),
                    );
                }
            }
        }

        doc = doc.add_paragraph(Paragraph::new());
    }

    if !spec.answer_key.is_empty() {
        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::Page)));
        doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
            "ĐÁP ÁN & LỜI GIẢI CHI TIẾT / ANSWER KEY",
            30,
            "1E3A8A",
            true,
        )));
        doc = doc.add_paragraph(Paragraph::new());

        for (idx, item) in spec.answer_key.iter().enumerate() {
            let num = if item.number.trim().is_empty() {
                format!("Câu {}", idx + 1)
            } else {
                item.number.trim().to_string()
            };
            doc = doc.add_paragraph(
                Paragraph::new()
                    .add_run(styled_run(&format!("{num}: "), 22, "0F172A", true))
                    .add_run(styled_run(item.answer.trim(), 22, "047857", true)),
            );
            if !item.explanation.trim().is_empty() {
                doc = doc.add_paragraph(
                    Paragraph::new()
                        .indent(Some(360), None, None, None)
                        .add_run(styled_run(item.explanation.trim(), 20, "334155", false)),
                );
            }
        }
    }

    let file = std::fs::File::create(output_path).map_err(|e| format!("create docx file: {e}"))?;
    doc.build()
        .pack(file)
        .map_err(|e| format!("pack docx zip: {e}"))?;
    Ok(())
}

fn sanitize_sheet_name(raw: &str, fallback_idx: usize) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
        .take(30)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        format!("Sheet{}", fallback_idx + 1)
    } else {
        trimmed.to_string()
    }
}

/// Renders a styled multi-sheet Excel `.xlsx` workbook in pure Rust (with real formulas & autofit).
pub fn render_xlsx(spec: &DocumentSpec, output_path: &Path) -> Result<(), String> {
    let mut wb = Workbook::new();

    let header_fmt = Format::new()
        .set_bold()
        .set_font_name("Segoe UI")
        .set_font_size(11.0)
        .set_font_color(XlsxColor::White)
        .set_background_color(XlsxColor::RGB(0x1E3A8A))
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
        .set_border(FormatBorder::Thin)
        .set_text_wrap();

    let cell_even_fmt = Format::new()
        .set_font_name("Segoe UI")
        .set_font_size(10.5)
        .set_border(FormatBorder::Thin)
        .set_border_color(XlsxColor::RGB(0xCBD5E1))
        .set_text_wrap();

    let cell_odd_fmt = Format::new()
        .set_font_name("Segoe UI")
        .set_font_size(10.5)
        .set_background_color(XlsxColor::RGB(0xF8FAFC))
        .set_border(FormatBorder::Thin)
        .set_border_color(XlsxColor::RGB(0xCBD5E1))
        .set_text_wrap();

    let note_fmt = Format::new()
        .set_italic()
        .set_font_name("Segoe UI")
        .set_font_size(10.0)
        .set_font_color(XlsxColor::RGB(0x475569));

    let mut sheets = spec.sheets.clone();
    if sheets.is_empty() {
        sheets = synthesize_sheets_from_sections(spec);
    }

    let mut used_names = std::collections::HashSet::new();
    for (s_idx, sheet) in sheets.iter().enumerate() {
        let mut name = sanitize_sheet_name(&sheet.name, s_idx);
        if !used_names.insert(name.to_ascii_lowercase()) {
            name = format!("{}_{}", &name[..name.len().min(25)], s_idx + 1);
            used_names.insert(name.to_ascii_lowercase());
        }

        let ws = wb.add_worksheet();
        ws.set_name(&name)
            .map_err(|e| format!("set sheet name '{name}': {e}"))?;

        let mut row_cursor: u32 = 0;

        if !sheet.headers.is_empty() {
            for (c_idx, h) in sheet.headers.iter().enumerate() {
                ws.write_string_with_format(row_cursor, c_idx as u16, h.trim(), &header_fmt)
                    .map_err(|e| format!("write header: {e}"))?;
            }
            let _ = ws.set_row_height(row_cursor, 24.0);
            let _ = ws.set_freeze_panes(1, 0);
            row_cursor += 1;
        }

        for (r_idx, row) in sheet.rows.iter().enumerate() {
            let fmt = if r_idx % 2 == 1 {
                &cell_odd_fmt
            } else {
                &cell_even_fmt
            };
            for (c_idx, raw_val) in row.iter().enumerate() {
                let val = raw_val.trim();
                let col = c_idx as u16;
                if val.starts_with('=') && val.len() > 1 {
                    ws.write_formula_with_format(row_cursor, col, val, fmt)
                        .map_err(|e| format!("write formula: {e}"))?;
                } else if let Ok(num) = val.parse::<f64>()
                    && num.is_finite()
                {
                    ws.write_number_with_format(row_cursor, col, num, fmt)
                        .map_err(|e| format!("write number: {e}"))?;
                } else {
                    ws.write_string_with_format(row_cursor, col, val, fmt)
                        .map_err(|e| format!("write string: {e}"))?;
                }
            }
            row_cursor += 1;
        }

        if !sheet.notes.is_empty() {
            row_cursor += 1;
            for note in &sheet.notes {
                if !note.trim().is_empty() {
                    ws.write_string_with_format(row_cursor, 0, note.trim(), &note_fmt)
                        .map_err(|e| format!("write note: {e}"))?;
                    row_cursor += 1;
                }
            }
        }

        ws.autofit();
    }

    wb.save(output_path)
        .map_err(|e| format!("save xlsx workbook: {e}"))?;
    Ok(())
}

fn synthesize_sheets_from_sections(spec: &DocumentSpec) -> Vec<ExcelSheet> {
    let mut out = Vec::new();
    let mut summary_rows = Vec::new();

    for sec in &spec.sections {
        if let Some(tbl) = &sec.table
            && (!tbl.headers.is_empty() || !tbl.rows.is_empty())
        {
            out.push(ExcelSheet {
                name: if sec.heading.trim().is_empty() {
                    format!("Table_{}", out.len() + 1)
                } else {
                    sec.heading.clone()
                },
                headers: tbl.headers.clone(),
                rows: tbl.rows.clone(),
                notes: Vec::new(),
            });
        }
        for p in &sec.paragraphs {
            summary_rows.push(vec![sec.heading.clone(), p.clone()]);
        }
        for b in &sec.bullets {
            summary_rows.push(vec![sec.heading.clone(), format!("• {b}")]);
        }
        for q in &sec.questions {
            let choices = if q.choices.is_empty() {
                String::new()
            } else {
                format!(" | {}", q.choices.join(" ; "))
            };
            summary_rows.push(vec![q.number.clone(), format!("{}{}", q.prompt, choices)]);
        }
    }

    if !summary_rows.is_empty() || out.is_empty() {
        out.insert(
            0,
            ExcelSheet {
                name: "Overview".to_string(),
                headers: vec!["Section / Item".to_string(), "Content".to_string()],
                rows: if summary_rows.is_empty() {
                    vec![vec![spec.title.clone(), spec.subtitle.clone()]]
                } else {
                    summary_rows
                },
                notes: Vec::new(),
            },
        );
    }

    if !spec.answer_key.is_empty() {
        out.push(ExcelSheet {
            name: "Answer_Key".to_string(),
            headers: vec![
                "Question".to_string(),
                "Answer".to_string(),
                "Explanation".to_string(),
            ],
            rows: spec
                .answer_key
                .iter()
                .map(|a| vec![a.number.clone(), a.answer.clone(), a.explanation.clone()])
                .collect(),
            notes: Vec::new(),
        });
    }

    out
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Builds a crisp, Vietnamese-UTF8, print-ready A4 HTML document for headless Chromium PDF printing.
pub fn build_document_html(spec: &DocumentSpec) -> String {
    let title = if spec.title.trim().is_empty() {
        "Boogie Study Document"
    } else {
        spec.title.trim()
    };
    let date_str = chrono::Local::now().format("%Y-%m-%d").to_string();

    let mut body = String::new();
    body.push_str("<header class=\"doc-header\">");
    body.push_str("<div class=\"badge-row\">");
    if !spec.subject.trim().is_empty() && !spec.subject.eq_ignore_ascii_case("general") {
        body.push_str(&format!(
            "<span class=\"pill pill-primary\">{}</span>",
            escape_html(spec.subject.trim())
        ));
    }
    if !spec.level.trim().is_empty() && !spec.level.eq_ignore_ascii_case("auto") {
        body.push_str(&format!(
            "<span class=\"pill pill-accent\">{}</span>",
            escape_html(spec.level.trim())
        ));
    }
    body.push_str(&format!(
        "<span class=\"pill pill-muted\">Boogie Document Studio · {}</span>",
        escape_html(&date_str)
    ));
    body.push_str("</div>");
    body.push_str(&format!("<h1>{}</h1>", escape_html(title)));
    if !spec.subtitle.trim().is_empty() {
        body.push_str(&format!(
            "<p class=\"subtitle\">{}</p>",
            escape_html(spec.subtitle.trim())
        ));
    }
    body.push_str("</header>");

    for sec in &spec.sections {
        body.push_str("<section class=\"doc-section\">");
        if !sec.heading.trim().is_empty() {
            body.push_str(&format!("<h2>{}</h2>", escape_html(sec.heading.trim())));
        }
        for p in &sec.paragraphs {
            if !p.trim().is_empty() {
                body.push_str(&format!(
                    "<p>{}</p>",
                    escape_html(p.trim()).replace('\n', "<br/>")
                ));
            }
        }
        if !sec.bullets.is_empty() {
            body.push_str("<ul>");
            for b in &sec.bullets {
                if !b.trim().is_empty() {
                    body.push_str(&format!("<li>{}</li>", escape_html(b.trim())));
                }
            }
            body.push_str("</ul>");
        }
        if let Some(callout) = sec
            .callout
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            body.push_str(&format!(
                "<div class=\"callout\"><strong>★ Key Point:</strong> {}</div>",
                escape_html(callout).replace('\n', "<br/>")
            ));
        }
        if let Some(tbl) = &sec.table
            && (!tbl.headers.is_empty() || !tbl.rows.is_empty())
        {
            body.push_str("<table>");
            if !tbl.headers.is_empty() {
                body.push_str("<thead><tr>");
                for h in &tbl.headers {
                    body.push_str(&format!("<th>{}</th>", escape_html(h.trim())));
                }
                body.push_str("</tr></thead>");
            }
            body.push_str("<tbody>");
            for row in &tbl.rows {
                body.push_str("<tr>");
                for cell in row {
                    body.push_str(&format!(
                        "<td>{}</td>",
                        escape_html(cell.trim()).replace('\n', "<br/>")
                    ));
                }
                body.push_str("</tr>");
            }
            body.push_str("</tbody></table>");
        }
        for (q_idx, q) in sec.questions.iter().enumerate() {
            let num = if q.number.trim().is_empty() {
                format!("Câu {}", q_idx + 1)
            } else {
                q.number.trim().to_string()
            };
            body.push_str("<div class=\"question\">");
            body.push_str("<div class=\"q-head\">");
            body.push_str(&format!(
                "<span class=\"q-num\">{}.</span> {}",
                escape_html(&num),
                escape_html(q.prompt.trim()).replace('\n', "<br/>")
            ));
            if !q.points.trim().is_empty() {
                body.push_str(&format!(
                    " <span class=\"q-pts\">({})</span>",
                    escape_html(q.points.trim())
                ));
            }
            body.push_str("</div>");
            if !q.choices.is_empty() {
                body.push_str("<div class=\"choices\">");
                for c in &q.choices {
                    if !c.trim().is_empty() {
                        body.push_str(&format!(
                            "<div class=\"choice\">{}</div>",
                            escape_html(c.trim())
                        ));
                    }
                }
                body.push_str("</div>");
            }
            body.push_str("</div>");
        }
        body.push_str("</section>");
    }

    if !spec.answer_key.is_empty() {
        body.push_str("<section class=\"answer-key\">");
        body.push_str("<h2>ĐÁP ÁN &amp; LỜI GIẢI CHI TIẾT / ANSWER KEY</h2>");
        for (idx, a) in spec.answer_key.iter().enumerate() {
            let num = if a.number.trim().is_empty() {
                format!("Câu {}", idx + 1)
            } else {
                a.number.trim().to_string()
            };
            body.push_str("<div class=\"ans-card\">");
            body.push_str(&format!(
                "<div class=\"ans-head\"><strong>{}:</strong> <span class=\"ans-val\">{}</span></div>",
                escape_html(&num),
                escape_html(a.answer.trim())
            ));
            if !a.explanation.trim().is_empty() {
                body.push_str(&format!(
                    "<div class=\"ans-exp\">{}</div>",
                    escape_html(a.explanation.trim()).replace('\n', "<br/>")
                ));
            }
            body.push_str("</div>");
        }
        body.push_str("</section>");
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="vi">
<head>
<meta charset="UTF-8" />
<title>{title_esc}</title>
<style>
  @page {{ size: A4; margin: 16mm 16mm 18mm 16mm; }}
  * {{ box-sizing: border-box; }}
  body {{
    font-family: 'Segoe UI', 'Cambria Math', Calibri, Arial, sans-serif;
    color: #0F172A;
    font-size: 11pt;
    line-height: 1.55;
    margin: 0;
  }}
  .doc-header {{
    border-bottom: 2.5px solid #1E3A8A;
    padding-bottom: 12px;
    margin-bottom: 18px;
  }}
  .badge-row {{ display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 8px; }}
  .pill {{
    display: inline-block;
    font-size: 8.5pt;
    font-weight: 600;
    padding: 2px 9px;
    border-radius: 999px;
  }}
  .pill-primary {{ background: #DBEAFE; color: #1E3A8A; }}
  .pill-accent {{ background: #D1FAE5; color: #065F46; }}
  .pill-muted {{ background: #F1F5F9; color: #475569; }}
  h1 {{ margin: 4px 0 4px; font-size: 20pt; color: #1E3A8A; line-height: 1.25; }}
  .subtitle {{ margin: 0; color: #475569; font-size: 10.5pt; }}
  .doc-section {{ margin-bottom: 16px; }}
  h2 {{
    font-size: 13pt;
    color: #1D4ED8;
    border-left: 4px solid #3B82F6;
    padding-left: 9px;
    margin: 14px 0 8px;
    page-break-after: avoid;
  }}
  p {{ margin: 5px 0 8px; }}
  ul {{ margin: 4px 0 10px 18px; padding: 0; }}
  li {{ margin-bottom: 4px; }}
  .callout {{
    background: #EFF6FF;
    border-left: 4px solid #2563EB;
    padding: 9px 12px;
    border-radius: 6px;
    margin: 10px 0;
    color: #1E3A8A;
    break-inside: avoid;
  }}
  table {{
    width: 100%;
    border-collapse: collapse;
    margin: 10px 0 14px;
    font-size: 10pt;
  }}
  th {{
    background: #1E3A8A;
    color: #FFFFFF;
    text-align: left;
    padding: 7px 9px;
    border: 1px solid #1E3A8A;
  }}
  td {{
    padding: 6px 9px;
    border: 1px solid #CBD5E1;
    vertical-align: top;
  }}
  tbody tr:nth-child(even) {{ background: #F8FAFC; }}
  .question {{
    margin: 10px 0;
    padding: 8px 11px;
    background: #F8FAFC;
    border: 1px solid #E2E8F0;
    border-radius: 7px;
    break-inside: avoid;
  }}
  .q-num {{ font-weight: 700; color: #1E3A8A; }}
  .q-pts {{ color: #64748B; font-size: 9.5pt; }}
  .choices {{
    margin-top: 6px;
    padding-left: 16px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px 14px;
  }}
  .choice {{ color: #1E293B; }}
  .answer-key {{ page-break-before: always; }}
  .ans-card {{
    margin: 7px 0;
    padding: 7px 11px;
    border-left: 3px solid #059669;
    background: #F0FDF4;
    border-radius: 5px;
    break-inside: avoid;
  }}
  .ans-val {{ color: #047857; font-weight: 700; }}
  .ans-exp {{ margin-top: 3px; color: #334155; font-size: 10pt; }}
</style>
</head>
<body>
{body}
</body>
</html>"#,
        title_esc = escape_html(title),
        body = body,
    )
}

pub fn find_chromium_binary() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"),
    ];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let base = PathBuf::from(local);
        candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
        candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
        candidates.push(base.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Renders a crisp UTF-8 Vietnamese PDF using Windows' built-in Edge/Chrome headless print engine.
pub fn render_pdf(spec: &DocumentSpec, output_path: &Path) -> Result<(), String> {
    let browser = find_chromium_binary().ok_or_else(|| {
        "No Microsoft Edge / Chrome / Brave browser found for PDF printing. Please save as Word (.docx) instead."
            .to_string()
    })?;

    let seq = TEMP_DOC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir();
    let html_path = temp_dir.join(format!("boogie_doc_{pid}_{seq}.html"));
    let profile_dir = temp_dir.join(format!("boogie_pdf_profile_{pid}_{seq}"));

    let html = build_document_html(spec);
    std::fs::write(&html_path, html.as_bytes())
        .map_err(|e| format!("write temp html for pdf: {e}"))?;

    let abs_out = if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(output_path)
    };

    let html_url = url::Url::from_file_path(&html_path)
        .map_err(|_| "failed to convert temp html path to file:// URL".to_string())?
        .to_string();

    let mut cmd = std::process::Command::new(&browser);
    cmd.args([
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-extensions",
        "--no-first-run",
        "--no-default-browser-check",
        "--no-pdf-header-footer",
        &format!("--user-data-dir={}", profile_dir.display()),
        &format!("--print-to-pdf={}", abs_out.display()),
        &html_url,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to launch headless PDF printer: {e}"));

    let _ = std::fs::remove_file(&html_path);
    let _ = std::fs::remove_dir_all(&profile_dir);

    let out = output?;
    if !abs_out.is_file() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "PDF printer did not produce output file (exit {:?}): {}",
            out.status.code(),
            crate::util::truncate_chars(stderr.trim(), 220)
        ));
    }

    let head = std::fs::read(&abs_out).map_err(|e| format!("verify pdf output: {e}"))?;
    if !head.starts_with(b"%PDF-") {
        return Err("Generated PDF file header is invalid.".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> DocumentSpec {
        DocumentSpec {
            title: "Đề cương ôn tập Toán 12 — Khảo sát hàm số".to_string(),
            subtitle: "Tổng hợp công thức & 2 câu trắc nghiệm mẫu có đáp án".to_string(),
            subject: "Toán học".to_string(),
            level: "Cấp 3 (Lớp 12)".to_string(),
            sections: vec![DocSection {
                heading: "I. Kiến thức trọng tâm".to_string(),
                paragraphs: vec![
                    "Cho hàm số y = f(x) xác định trên khoảng K. Nếu f'(x) > 0 với mọi x ∈ K thì hàm số đồng biến trên K."
                        .to_string(),
                ],
                bullets: vec![
                    "Cực trị: f'(x₀) = 0 và f'(x) đổi dấu qua x₀.".to_string(),
                    "Tiệm cận đứng: lim(x→a) f(x) = ±∞.".to_string(),
                ],
                callout: Some("Mẹo nhanh: Hàm bậc 3 y = ax³ + bx² + cx + d có 2 cực trị ⇔ b² - 3ac > 0.".to_string()),
                table: Some(DocTable {
                    headers: vec!["Dạng hàm số".to_string(), "Đạo hàm y'".to_string()],
                    rows: vec![
                        vec!["y = xⁿ".to_string(), "y' = n·xⁿ⁻¹".to_string()],
                        vec!["y = sin(x)".to_string(), "y' = cos(x)".to_string()],
                    ],
                }),
                questions: vec![ExamQuestion {
                    number: "Câu 1".to_string(),
                    prompt: "Hàm số y = x³ - 3x + 1 nghịch biến trên khoảng nào?".to_string(),
                    choices: vec![
                        "A. (-1; 1)".to_string(),
                        "B. (-∞; -1)".to_string(),
                        "C. (1; +∞)".to_string(),
                        "D. ℝ".to_string(),
                    ],
                    points: "0.5 đ".to_string(),
                }],
            }],
            answer_key: vec![AnswerItem {
                number: "Câu 1".to_string(),
                answer: "A. (-1; 1)".to_string(),
                explanation: "y' = 3x² - 3 < 0 ⇔ -1 < x < 1.".to_string(),
            }],
            sheets: vec![ExcelSheet {
                name: "Chi_Tieu".to_string(),
                headers: vec!["Hạng mục".to_string(), "Số tiền (VNĐ)".to_string()],
                rows: vec![
                    vec!["Sách vở".to_string(), "150000".to_string()],
                    vec!["Ăn uống".to_string(), "450000".to_string()],
                    vec!["Tổng cộng".to_string(), "=SUM(B2:B3)".to_string()],
                ],
                notes: vec!["Công thức Excel tự tính tổng ở ô B4.".to_string()],
            }],
        }
    }

    #[test]
    fn format_parser_supports_pdf_docx_xlsx_and_rejects_pptx() {
        assert_eq!(DocFormat::parse("pdf").unwrap(), DocFormat::Pdf);
        assert_eq!(DocFormat::parse("Word").unwrap(), DocFormat::Docx);
        assert_eq!(DocFormat::parse("xlsx").unwrap(), DocFormat::Xlsx);
        assert_eq!(DocFormat::parse("excel").unwrap(), DocFormat::Xlsx);
        let err = DocFormat::parse("pptx").unwrap_err();
        assert!(err.contains("not supported"));
    }

    #[test]
    fn sanitize_filename_strips_windows_illegal_chars_and_appends_ext() {
        assert_eq!(
            sanitize_filename("Đề thi: Toán/Lý? 2026", Some("pdf")),
            "Đề thi_ Toán_Lý_ 2026.pdf"
        );
        assert_eq!(sanitize_filename("notes.docx", Some("docx")), "notes.docx");
    }

    #[test]
    fn renders_valid_docx_and_xlsx_with_vietnamese_utf8() {
        let spec = sample_spec();
        let tmp = std::env::temp_dir();
        let docx_path = tmp.join(format!("boogie_test_{}.docx", std::process::id()));
        let xlsx_path = tmp.join(format!("boogie_test_{}.xlsx", std::process::id()));

        render_docx(&spec, &docx_path).expect("docx render should succeed");
        let docx_bytes = std::fs::read(&docx_path).unwrap();
        assert!(docx_bytes.starts_with(b"PK"), "docx must be valid zip");
        assert!(docx_bytes.len() > 1000);
        let _ = std::fs::remove_file(&docx_path);

        render_xlsx(&spec, &xlsx_path).expect("xlsx render should succeed");
        let xlsx_bytes = std::fs::read(&xlsx_path).unwrap();
        assert!(xlsx_bytes.starts_with(b"PK"), "xlsx must be valid zip");
        assert!(xlsx_bytes.len() > 1000);
        let _ = std::fs::remove_file(&xlsx_path);
    }

    #[test]
    fn html_builder_escapes_math_inequalities_and_keeps_vietnamese() {
        let html = build_document_html(&sample_spec());
        assert!(html.contains("Đề cương ôn tập Toán 12"));
        assert!(html.contains("&lt; 0"));
        assert!(html.contains("ANSWER KEY"));
    }

    #[test]
    fn renders_valid_pdf_when_chromium_is_installed() {
        if find_chromium_binary().is_none() {
            return;
        }
        let pdf_path = std::env::temp_dir().join(format!("boogie_test_{}.pdf", std::process::id()));
        render_pdf(&sample_spec(), &pdf_path)
            .expect("pdf render via headless Edge/Chrome should succeed");
        let bytes = std::fs::read(&pdf_path).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(bytes.len() > 2000);
        let _ = std::fs::remove_file(&pdf_path);
    }

    #[tokio::test]
    async fn generate_document_enforces_required_format_and_rejects_pptx() {
        let (tx, _rx) = broadcast::channel(8);

        let missing_fmt = generate_document_handler(
            serde_json::json!({ "topic": "Đề ôn tập Toán 10" }),
            tx.clone(),
        )
        .await;
        assert!(missing_fmt.is_error);
        assert!(missing_fmt.output.contains("Missing required file format"));

        let auto_fmt = generate_document_handler(
            serde_json::json!({ "topic": "Đề ôn tập Anh 7", "format": "auto" }),
            tx.clone(),
        )
        .await;
        assert!(auto_fmt.is_error);
        assert!(auto_fmt.output.contains("Missing required file format"));

        let pptx_fmt = generate_document_handler(
            serde_json::json!({ "topic": "Slide thuyết trình", "format": "pptx" }),
            tx,
        )
        .await;
        assert!(pptx_fmt.is_error);
        assert!(pptx_fmt.output.contains("not supported"));
    }

    #[test]
    fn lenient_document_spec_parser_strips_nulls_and_decodes_cleanly() {
        let raw = r#"```json
        {
          "title": "Đề Tiếng Anh 7",
          "subtitle": null,
          "subject": "English",
          "level": "Middle",
          "sections": [
            {
              "heading": "I. Multiple Choice",
              "paragraphs": [],
              "bullets": null,
              "callout": null,
              "table": null,
              "questions": [
                { "number": "Q1", "prompt": "She ___ to school every day.", "choices": ["A. go", "B. goes"], "points": "1 pt" }
              ]
            }
          ],
          "answer_key": [{ "number": "Q1", "answer": "B", "explanation": "Third-person singular present simple." }],
          "sheets": null
        }
        ```"#;
        let spec = parse_document_spec_lenient(raw).expect("lenient spec parse should succeed");
        assert_eq!(spec.title, "Đề Tiếng Anh 7");
        assert_eq!(spec.sections.len(), 1);
        assert_eq!(spec.sections[0].questions.len(), 1);
        assert_eq!(spec.answer_key.len(), 1);
    }
}
