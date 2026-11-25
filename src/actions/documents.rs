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
    CapturedImage, REST_QUOTA_EXHAUSTED_TAG, call_document_author, capture_clipboard_png,
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
    pub doc_type: String,
    #[serde(default)]
    pub duration: String,
    #[serde(default)]
    pub total_points: String,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub sections: Vec<DocSection>,
    #[serde(default)]
    pub transcripts: Vec<TranscriptItem>,
    #[serde(default)]
    pub answer_key: Vec<AnswerItem>,
    #[serde(default)]
    pub sheets: Vec<ExcelSheet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranscriptItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocSection {
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub passage_title: String,
    #[serde(default)]
    pub passage: Option<String>,
    #[serde(default)]
    pub code_block: Option<String>,
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
    #[serde(default)]
    pub answer_lines: u8,
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
             STRICT RESEARCH-THEN-CONFIRM WORKFLOW: \
             (1) For ANY exam/test/study-guide subject whose exact section structure was not already specified by the user, \
             FIRST call `web_search` to research the real exam format of that subject. \
             (2) Briefly share the proposed sections (or ask openly if school/lecturer-specific) and ask the user if they \
             want to add/adjust any section, question count, AND confirm the required file format ('pdf', 'docx', or 'xlsx') \
             BEFORE calling this tool! \
             (3) Once confirmed, ALWAYS speak ONE brief friendly sentence aloud FIRST, then call this tool passing the \
             researched + user-confirmed section breakdown in `exam_structure` (leaving `spec_json` empty). \
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
                    "exam_structure",
                    param(
                        "exam_structure",
                        "string",
                        "Researched & user-confirmed section breakdown and custom requirements (e.g. 'Phần 1: Trắc nghiệm lý thuyết 12 câu (4đ); Phần 2: Bài tập tính toán PERT/CPM 2 câu (3đ); Phần 3: Tình huống thực tế 1 câu (3đ); nhấn mạnh chương quản lý rủi ro'). Pass this whenever structure was researched or customized with the user!",
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
                        "LEAVE EMPTY on normal calls so Boogie's dedicated long-form Document Author Engine writes the full exam/study guide with passages, transcripts, and verified answers. ONLY pass a complete spec_json if a prior tool result explicitly asked for COMPLETE DIRECT DOCUMENT MODE.",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubjectProfile {
    EnglishExam,
    Math,
    NaturalScience,
    Informatics,
    Literature,
    HumanitiesEcon,
    General,
}

fn detect_subject_profile(topic: &str, subject: &str) -> SubjectProfile {
    let combined = format!("{} {}", topic.to_lowercase(), subject.to_lowercase());
    let has_any = |kws: &[&str]| kws.iter().any(|k| combined.contains(k));

    if has_any(&[
        "vstep",
        "ielts",
        "toeic",
        "toefl",
        "tiếng anh",
        "tieng anh",
        "english",
        "ngoại ngữ",
        "reading comprehension",
        "listening comprehension",
        "grammar",
        "vocabulary",
    ]) {
        SubjectProfile::EnglishExam
    } else if has_any(&[
        "toán",
        "toan",
        "math",
        "calculus",
        "giải tích",
        "đại số",
        "hình học",
        "xác suất",
        "thống kê",
        "lượng giác",
        "algebra",
        "geometry",
        "statistics",
    ]) {
        SubjectProfile::Math
    } else if has_any(&[
        "vật lý",
        "vật lí",
        "vat ly",
        "physics",
        "hóa học",
        "hoá học",
        "hoa hoc",
        "chemistry",
        "sinh học",
        "sinh hoc",
        "biology",
        "khoa học tự nhiên",
        "khtn",
    ]) {
        SubjectProfile::NaturalScience
    } else if has_any(&[
        "tin học",
        "tin hoc",
        "informatics",
        "lập trình",
        "programming",
        "python",
        "rust",
        "java",
        "c++",
        "c#",
        "javascript",
        "typescript",
        "sql",
        "cơ sở dữ liệu",
        "database",
        "thuật toán",
        "algorithm",
        "cấu trúc dữ liệu",
        "mạng máy tính",
        "computer science",
    ]) {
        SubjectProfile::Informatics
    } else if has_any(&[
        "ngữ văn",
        "ngu van",
        "văn học",
        "van hoc",
        "literature",
        "tập làm văn",
        "nghị luận",
        "đọc hiểu văn bản",
    ]) {
        SubjectProfile::Literature
    } else if has_any(&[
        "lịch sử",
        "lich su",
        "history",
        "địa lý",
        "địa lí",
        "dia ly",
        "geography",
        "gdcd",
        "kinh tế",
        "kinh te",
        "economics",
        "pháp luật",
        "law",
        "triết học",
        "philosophy",
        "tâm lý",
        "psychology",
        "marketing",
        "quản trị",
        "kế toán",
        "tài chính",
        "finance",
    ]) {
        SubjectProfile::HumanitiesEcon
    } else {
        SubjectProfile::General
    }
}

fn is_exam_intent(topic: &str) -> bool {
    let lower = topic.to_lowercase();
    const EXAM_HINTS: &[&str] = &[
        "đề thi",
        "de thi",
        "đề kiểm tra",
        "de kiem tra",
        "kiểm tra",
        "bài test",
        "đề thử",
        "đề luyện",
        "đề minh họa",
        "vstep",
        "ielts",
        "toeic",
        "thpt",
        "học kì",
        "học kỳ",
        "giữa kì",
        "giữa kỳ",
        "cuối kì",
        "cuối kỳ",
        "midterm",
        "final exam",
        "mock test",
        "practice test",
        "exam",
        "quiz",
        "test",
        "trắc nghiệm",
        "bài tập",
        "phiếu bài tập",
        "worksheet",
    ];
    EXAM_HINTS.iter().any(|k| lower.contains(k))
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

fn contains_placeholder_stub(text: &str) -> Option<&'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.ends_with("..") && !trimmed.ends_with("...") && !trimmed.contains("___") {
        return Some("trailing '..' truncation");
    }
    const LAZY_MARKERS: &[&str] = &[
        "insert passage",
        "insert text here",
        "insert reading",
        "insert audio",
        "to be filled",
        "similar questions",
        "more questions here",
        "add more questions",
        "[tbd]",
        "(tbd)",
        "lorem ipsum",
    ];
    for m in LAZY_MARKERS {
        if lower.contains(m) {
            return Some("placeholder text");
        }
    }
    if (lower.starts_with("passage excerpt:") || lower.starts_with("excerpt:"))
        && word_count(trimmed) < 65
    {
        return Some("truncated passage excerpt");
    }
    None
}

/// Validates a generated `DocumentSpec` before opening Save As so Boogie NEVER outputs a 2-question
/// placeholder stub when the user asked for a real complete exam or study guide.
pub(crate) fn validate_document_spec(
    spec: &DocumentSpec,
    format: DocFormat,
    topic: &str,
    requested_questions: Option<u64>,
    include_answers: bool,
) -> Vec<String> {
    let mut issues = Vec::new();

    if format == DocFormat::Xlsx {
        if spec.sheets.is_empty() {
            issues.push("Excel workbook has 0 sheets; generate 1-3 populated worksheets with headers, rows, and formulas.".to_string());
        } else {
            let best_rows = spec.sheets.iter().map(|s| s.rows.len()).max().unwrap_or(0);
            if best_rows < 3 {
                issues.push(format!(
                    "Excel workbook only has {best_rows} data row(s); include at least 5 realistic data rows plus summary formula rows (=SUM / =AVERAGE)."
                ));
            }
        }
        return issues;
    }

    if spec.sections.is_empty() {
        issues.push("Document has 0 sections; populate complete sections with full content.".to_string());
        return issues;
    }

    let total_questions: usize = spec.sections.iter().map(|s| s.questions.len()).sum();
    let exam_mode = is_exam_intent(topic) || spec.doc_type.eq_ignore_ascii_case("exam");

    if let Some(req) = requested_questions
        && req >= 4
    {
        let min_required = ((req as f32 * 0.80).ceil() as usize).max(4);
        if total_questions < min_required {
            issues.push(format!(
                "Requested {req} questions/tasks, but document only contains {total_questions}. Write ALL {req} complete, distinct questions/tasks — do NOT abbreviate or skip!"
            ));
        }
    } else if exam_mode && total_questions < 8 {
        issues.push(format!(
            "This is a full exam/test request, but document only has {total_questions} question(s). Write a complete, realistic exam with at least 12-18 concrete questions/tasks across all requested sections."
        ));
    }

    let transcript_words: usize = spec
        .transcripts
        .iter()
        .map(|t| word_count(&t.content))
        .sum();

    for sec in &spec.sections {
        let heading_lower = sec.heading.to_lowercase();
        let sec_prose_words = sec.passage.as_deref().map(word_count).unwrap_or(0)
            + sec.paragraphs.iter().map(|p| word_count(p)).sum::<usize>()
            + sec.bullets.iter().map(|b| word_count(b)).sum::<usize>();

        // Scan for lazy placeholder stubs
        if let Some(p) = sec.passage.as_deref()
            && let Some(reason) = contains_placeholder_stub(p)
        {
            issues.push(format!(
                "Section '{}' passage contains {reason}; write the FULL complete passage text.",
                sec.heading
            ));
        }
        for p in &sec.paragraphs {
            if let Some(reason) = contains_placeholder_stub(p) {
                issues.push(format!(
                    "Section '{}' paragraph contains {reason}; replace with full real content.",
                    sec.heading
                ));
                break;
            }
        }
        for q in &sec.questions {
            if let Some(reason) = contains_placeholder_stub(&q.prompt) {
                issues.push(format!(
                    "Question '{}' in '{}' contains {reason}; write a complete self-contained prompt.",
                    q.number, sec.heading
                ));
                break;
            }
        }

        // Reading / Đọc hiểu MUST include a real complete reading passage!
        if (heading_lower.contains("reading") || heading_lower.contains("đọc hiểu"))
            && sec_prose_words < 95
        {
            issues.push(format!(
                "Section '{}' is a Reading section but only has {sec_prose_words} words of text; put a COMPLETE 220-350 word reading passage inside `passage` so every question can be answered directly.",
                sec.heading
            ));
        }

        // Listening sections on a written PDF/Word exam MUST provide real audio transcripts!
        if (heading_lower.contains("listening") || heading_lower.contains("nghe hiểu"))
            && !sec.questions.is_empty()
            && transcript_words < 60
            && sec_prose_words < 90
        {
            issues.push(format!(
                "Section '{}' has {} Listening question(s) but NO audio transcript/script; populate top-level `transcripts` with complete natural dialogues/monologues matching every Listening question so students can self-study on PDF/Word.",
                sec.heading,
                sec.questions.len()
            ));
        }

        // Writing / Speaking / Làm văn MUST have concrete prompts (not just task names!)
        if (heading_lower.contains("writing")
            || heading_lower.contains("speaking")
            || heading_lower.contains("làm văn")
            || heading_lower.contains("tự luận"))
            && sec.questions.is_empty()
            && sec_prose_words < 45
        {
            issues.push(format!(
                "Section '{}' only lists bare task labels ({sec_prose_words} words) with 0 concrete prompts; add full actionable `questions` with detailed scenarios, instructions, bullet points, and word limits.",
                sec.heading
            ));
        }
    }

    if include_answers && total_questions >= 4 {
        let min_ans = (total_questions * 75) / 100;
        if spec.answer_key.len() < min_ans {
            issues.push(format!(
                "Document has {total_questions} questions/tasks but `answer_key` only covers {}. Provide verified answers/explanations (and sample outlines/essays for Writing/Speaking/Essay) for ALL questions.",
                spec.answer_key.len()
            ));
        }
    }

    issues
}

fn spec_richness_score(spec: &DocumentSpec, issues_len: usize) -> i64 {
    let q_count: usize = spec.sections.iter().map(|s| s.questions.len()).sum();
    let total_words: usize = spec
        .sections
        .iter()
        .map(|s| {
            s.passage.as_deref().map(word_count).unwrap_or(0)
                + s.paragraphs.iter().map(|p| word_count(p)).sum::<usize>()
                + s.questions
                    .iter()
                    .map(|q| word_count(&q.prompt))
                    .sum::<usize>()
        })
        .sum::<usize>()
        + spec
            .transcripts
            .iter()
            .map(|t| word_count(&t.content))
            .sum::<usize>();
    (q_count as i64 * 120) + (spec.answer_key.len() as i64 * 40) + (total_words as i64)
        - (issues_len as i64 * 800)
}

#[allow(clippy::too_many_arguments)]
fn build_doc_system_prompt(
    format: DocFormat,
    topic: &str,
    subject: &str,
    level: &str,
    question_count: Option<u64>,
    exam_structure: Option<&str>,
    include_answers: bool,
    language: &str,
) -> String {
    let exam_mode = is_exam_intent(topic);
    let profile = detect_subject_profile(topic, subject);

    let q_rule = match question_count {
        Some(n) if n > 0 => format!(
            "STRICT QUESTION COUNT: Include EXACTLY {n} complete, well-crafted, non-repetitive questions/tasks with clear sequential numbering. NEVER stop early or abbreviate!"
        ),
        _ if exam_mode => "STRICT EXAM LENGTH: Since no exact count was specified, craft a FULL, realistic practice exam with 15 to 20 complete, distinct questions/tasks covering every requested skill/topic — NEVER a tiny 2-5 question stub!".to_string(),
        _ => "Include rich, thorough structured sections, worked examples, summary tables, and practice exercises suited to the request.".to_string(),
    };

    let ans_rule = if include_answers {
        "Populate `answer_key` with verified, double-checked answers and concise explanations for EVERY question (plus complete outlines or model responses for Writing / Speaking / Essay / Literature prompts)."
    } else {
        "Leave `answer_key` as an empty array []."
    };

    let subject_blueprint = match profile {
        SubjectProfile::EnglishExam => {
            "SUBJECT BLUEPRINT — ENGLISH / FOREIGN LANGUAGE EXAM (VSTEP / IELTS / TOEIC / THPTQG / School English):\n\
             - Set `doc_type` to \"exam\", and fill realistic `duration` (e.g. \"60 minutes\" or \"120 minutes\") and `total_points`.\n\
             - Keep ALL English exam content, headings, directions, and question labels (`Question 1`, `Task 1`, `Part 1`) in crisp, natural English (explanations in `answer_key` may use the target language).\n\
             - READING SECTIONS: ALWAYS place a COMPLETE, original academic reading passage of 220–350 words (multi-paragraph with [1], [2], [3] markers) inside `section.passage` + `section.passage_title`, then write comprehension questions (main idea, detail, vocabulary-in-context, reference, inference) strictly grounded in that passage.\n\
             - LISTENING SECTIONS (when a 4-skill exam like VSTEP/IELTS/TOEIC is requested): ALWAYS write COMPLETE, realistic spoken dialogues/monologues inside top-level `transcripts` (one `TranscriptItem` per Listening Part, labelled clearly e.g. \"Part 1 (Questions 1–4): Short Announcements & Dialogues\") so every Listening question is 100% solvable on paper! Add a 1-sentence direction in the Listening section noting that the Audio Scripts appear at the end of the test for self-study.\n\
             - WRITING SECTIONS: NEVER just list 'Task 1: Email'. Write FULL concrete exam prompts as `questions` (with `choices: []` and `answer_lines: 8` for Task 1, `answer_lines: 12` for Task 2): exact scenario, sender/recipient, 3–4 bullet requirements, and word limit (e.g. 120 words / 250 words). Include Band B2+/C1 outlines & full model responses in `answer_key`!\n\
             - SPEAKING SECTIONS: Write FULL concrete prompts as `questions` (`choices: []`, `answer_lines: 4`): Part 1 with 4–6 concrete interview questions across 2 topics; Part 2 with a real situation + 3 explicit options to weigh; Part 3 with a clear topic statement + 3 mind-map supporting points + 2–3 follow-up questions. Include strong sample answers & key expressions in `answer_key`!\n\
             - GRAMMAR / VOCABULARY / CLOZE: Use clear `____` blanks, unambiguous distractors, and verified single correct answers."
        }
        SubjectProfile::Math => {
            "SUBJECT BLUEPRINT — MATHEMATICS (Grade 1 → University):\n\
             - Progress cleanly from Foundation (Nhận biết) → Understanding (Thông hiểu) → Application (Vận dụng) → Challenge (Vận dụng cao).\n\
             - Use crisp Unicode math (`x²`, `x³`, `√`, `∫`, `∑`, `π`, `≤`, `≥`, `≠`, `⇒`, `⇔`, `Δ`, `∠`, `⊥`, `∥`, `∈`, `ℝ`) — NEVER raw LaTeX backslashes.\n\
             - Every problem MUST be 100% self-contained with explicit coordinates, lengths, equations, and domain conditions (never reference a missing figure!).\n\
             - For essay/proof questions, set `choices: []` and `answer_lines: 6`. Double-check EVERY algebraic sign and numeric result before emitting `answer_key`!"
        }
        SubjectProfile::NaturalScience => {
            "SUBJECT BLUEPRINT — PHYSICS / CHEMISTRY / BIOLOGY:\n\
             - Put given physical constants or atomic masses (`g = 10 m/s²`, `M(Fe) = 56, M(O) = 16, M(H) = 1, M(C) = 12`) in `instructions` or a section `callout`.\n\
             - Use proper Unicode chemical subscripts & physics symbols (`H₂SO₄`, `Fe₃O₄`, `CO₂`, `Ω`, `μ`, `λ`, `Δ`, `→`).\n\
             - Ensure every numerical answer has consistent SI units, balanced chemical equations, and verified step-by-step formula derivations in `answer_key`."
        }
        SubjectProfile::Informatics => {
            "SUBJECT BLUEPRINT — INFORMATICS / COMPUTER SCIENCE / PROGRAMMING / SQL:\n\
             - Put complete, syntactically valid code snippets or SQL schemas inside `section.code_block` (and sample relational tables inside `section.table` for SQL exams!).\n\
             - Mix conceptual MCQs, trace-the-output questions, bug-finding questions, complexity analysis, and 1–2 hands-on coding tasks (`choices: []`, `answer_lines: 8`).\n\
             - Include exact verified outputs and clean reference code solutions in `answer_key`."
        }
        SubjectProfile::Literature => {
            "SUBJECT BLUEPRINT — LITERATURE / NGỮ VĂN:\n\
             - Phần Đọc hiểu: ALWAYS include a COMPLETE literary or informational excerpt (150–280 words, citing author/source in `passage_title`) inside `section.passage`, followed by 4 graded comprehension questions (`answer_lines: 4`).\n\
             - Phần Làm văn: Write concrete Nghị luận xã hội (200 chữ) and/or Nghị luận văn học prompts (`choices: []`, `answer_lines: 10`) with clear grading rubric & detailed outline (`Dàn ý chi tiết`) in `answer_key`."
        }
        SubjectProfile::HumanitiesEcon => {
            "SUBJECT BLUEPRINT — HISTORY / GEOGRAPHY / ECONOMICS / LAW / UNIVERSITY HUMANITIES:\n\
             - Include real structured data tables (`section.table` for Geography/Economics/Statistics) or primary-source excerpts/case studies (`section.passage` for Law/History/Management) when relevant.\n\
             - Verify all historical dates, economic formulas, and legal principles; provide clear rubrics and structured arguments in `answer_key` for essay prompts (`answer_lines: 6`)."
        }
        SubjectProfile::General => {
            "SUBJECT BLUEPRINT — GENERAL ACADEMIC / STUDY GUIDE / WORKSHEET:\n\
             - Organize logically with clear headings, concise explanations, structured comparison tables or formula callouts where helpful, and complete, self-contained questions/exercises.\n\
             - Use `answer_lines: 5` on open-ended/short-answer questions so learners can write directly on printed copies."
        }
    };

    let format_schema = match format {
        DocFormat::Xlsx => {
            "TARGET FORMAT: EXCEL WORKBOOK (.xlsx).\n\
             Set `doc_type` to \"spreadsheet\" and populate `sheets` with 1 to 3 practical worksheets. Each sheet has:\n\
             - `name` (short sheet tab name <= 28 chars, no / \\ ? * : [ ])\n\
             - `headers` (array of column header strings)\n\
             - `rows` (2D array of at least 6–15 realistic string cells; use plain numeric strings like \"125000\" for numbers and REAL Excel formulas starting with \"=\" like \"=SUM(B2:B10)\", \"=AVERAGE(C2:C10)\", \"=B2*C2\" whenever totals/averages/calculations make sense!)\n\
             - `notes` (short helpful notes shown below the table)\n\
             Also fill `title`, `subtitle`, `subject`, `level` cleanly."
        }
        DocFormat::Pdf | DocFormat::Docx => {
            "TARGET FORMAT: PRINT-READY DOCUMENT (PDF / Word).\n\
             Populate `doc_type` (\"exam\", \"study_guide\", \"worksheet\", or \"notes\"), `duration`, `total_points`, `instructions` (2–3 crisp rules when exam), `sections`, `transcripts` (whenever Listening audio scripts are needed), and `answer_key`.\n\
             Each section can contain:\n\
             - `heading` (clear section heading)\n\
             - `passage_title` (optional title/source above a reading passage or literary excerpt)\n\
             - `passage` (optional FULL multi-paragraph reading passage / excerpt / case study string, or null)\n\
             - `code_block` (optional formatted code / SQL snippet string, or null)\n\
             - `paragraphs` (concise directions or explanatory paragraphs)\n\
             - `bullets` (key points / formulas / rules)\n\
             - `callout` (optional highlighted formula / constants / key rule box, or null)\n\
             - `table` (optional { \"headers\": [...], \"rows\": [[...], ...] }, or null)\n\
             - `questions` (array of { \"number\": \"Question 1\" or \"Câu 1\", \"prompt\": \"...\", \"choices\": [\"A. ...\", \"B. ...\", \"C. ...\", \"D. ...\"], \"points\": \"0.5 pts\" or \"0.5 đ\", \"answer_lines\": 0 } — set `choices: []` and `answer_lines: 4..12` for written/essay/short-answer questions!)"
        }
    };

    let structure_block = match exam_structure.map(str::trim).filter(|s| !s.is_empty()) {
        Some(st) => format!(
            "\nMANDATORY RESEARCHED & USER-CONFIRMED EXAM STRUCTURE (FOLLOW EVERY SECTION FAITHFULLY — DO NOT DROP OR MERGE ANY SECTION):\n{st}\n"
        ),
        None => String::new(),
    };

    format!(
        "You are Boogie's Document Studio — an elite academic exam & study-material author across ALL subjects.\n\
         Output ONLY a single valid JSON object (no markdown fences, no commentary outside JSON).\n\
         Target language: {language} (note: for English/foreign-language exams, write the exam passages, prompts, and options in English!).\n\
         Subject: {subject}. Education level: {level} (adapt depth strictly to Primary/Middle/High/University/CEFR B1-C1 when specified).\n\
         {q_rule}\n\
         {ans_rule}{structure_block}\n\
         {subject_blueprint}\n\n\
         {format_schema}\n\n\
         REQUIRED JSON SCHEMA:\n\
         {{\n\
           \"title\": \"string\",\n\
           \"subtitle\": \"string\",\n\
           \"subject\": \"string\",\n\
           \"level\": \"string\",\n\
           \"doc_type\": \"exam | study_guide | worksheet | notes | spreadsheet\",\n\
           \"duration\": \"string (e.g. '60 phút' or '120 minutes', or '')\",\n\
           \"total_points\": \"string (e.g. '10 điểm' or '100 pts', or '')\",\n\
           \"instructions\": [\"string\"],\n\
           \"sections\": [\n\
             {{\n\
               \"heading\": \"string\",\n\
               \"passage_title\": \"string\",\n\
               \"passage\": \"string or null\",\n\
               \"code_block\": \"string or null\",\n\
               \"paragraphs\": [\"string\"],\n\
               \"bullets\": [\"string\"],\n\
               \"callout\": \"string or null\",\n\
               \"table\": {{ \"headers\": [\"string\"], \"rows\": [[\"string\"]] }} or null,\n\
               \"questions\": [\n\
                 {{ \"number\": \"string\", \"prompt\": \"string\", \"choices\": [\"string\"], \"points\": \"string\", \"answer_lines\": 0 }}\n\
               ]\n\
             }}\n\
           ],\n\
           \"transcripts\": [\n\
             {{ \"title\": \"string\", \"content\": \"string\" }}\n\
           ],\n\
           \"answer_key\": [\n\
             {{ \"number\": \"string\", \"answer\": \"string\", \"explanation\": \"string\" }}\n\
           ],\n\
           \"sheets\": [\n\
             {{ \"name\": \"string\", \"headers\": [\"string\"], \"rows\": [[\"string\"]], \"notes\": [\"string\"] }}\n\
           ]\n\
         }}\n\
         ABSOLUTE ZERO-PLACEHOLDER RULE:\n\
         - NEVER output truncated sentences ending in '..' or '...', NEVER write 'Passage excerpt:', 'TBD', 'similar questions', or bare task labels without full concrete prompts.\n\
         - EVERY single question MUST be 100% real, complete, verified, and directly solvable from this document alone."
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
    let exam_structure = args
        .get("exam_structure")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
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

    // If direct spec_json was passed AND passes our Quality Gate, use it immediately (0 REST).
    // If direct spec_json is a lazy stub (e.g. 2/20 questions or placeholder '..'), automatically
    // upgrade through our dedicated 4-model Document Author Engine instead of exporting a broken PDF!
    let mut prevalidated_direct: Option<(DocumentSpec, &'static str)> = None;
    let mut direct_issues: Vec<String> = Vec::new();
    if let Some(direct_raw) = direct_spec_raw {
        if let Ok(parsed) = parse_document_spec_lenient(&direct_raw) {
            let issues =
                validate_document_spec(&parsed, format, &topic, question_count, include_answers);
            if issues.is_empty() {
                tracing::info!(
                    "generate_document using verified direct spec_json (zero REST quota)"
                );
                prevalidated_direct = Some((parsed, "gemini-live-direct"));
            } else {
                tracing::warn!(
                    "direct spec_json failed quality gate ({} issue(s)); upgrading via Document Author Engine: {}",
                    issues.len(),
                    issues.join(" ; ")
                );
                direct_issues = issues;
            }
        } else {
            tracing::warn!("direct spec_json was invalid JSON; upgrading via Document Author Engine");
        }
    }

    let (mut spec, used_model) = if let Some(ready) = prevalidated_direct {
        ready
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
            "Drafting complete {} document...",
            format.display_name()
        )));

        let system_prompt = build_doc_system_prompt(
            format,
            &topic,
            &subject,
            &level,
            question_count,
            exam_structure.as_deref(),
            include_answers,
            &language,
        );
        let structure_note = match exam_structure.as_deref() {
            Some(st) => format!("\nConfirmed exam structure & user additions:\n{st}"),
            None => String::new(),
        };
        let user_prompt = if let Some(ref img) = captured {
            format!(
                "Attached reference image ({}).\nCreate a COMPLETE, full-length, publication-grade {} document for: {}\nTitle hint: {}{}\nRemember: write EVERY passage, transcript, prompt, question, and answer in full — ZERO placeholders or ellipses!",
                img.source_desc,
                format.display_name(),
                topic,
                title_hint
                    .as_deref()
                    .unwrap_or("(choose an appropriate clear title)"),
                structure_note
            )
        } else {
            format!(
                "Create a COMPLETE, full-length, publication-grade {} document for: {}\nTitle hint: {}{}\nRemember: write EVERY passage, transcript, prompt, question, and answer in full — ZERO placeholders or ellipses!",
                format.display_name(),
                topic,
                title_hint
                    .as_deref()
                    .unwrap_or("(choose an appropriate clear title)"),
                structure_note
            )
        };

        status_guard.update(
            if captured.is_some() {
                "📸 Đã chụp xong! Có thể Alt+Tab — đang soạn đầy đủ nội dung..."
            } else {
                "Bước 2/4 · Đang soạn đầy đủ nội dung, bài đọc & đáp án..."
            },
            0.35,
        );

        let (raw_json, first_model) = match call_document_author(
            &cfg,
            &system_prompt,
            &user_prompt,
            captured.as_ref(),
            24_576,
        )
        .await
        {
            Ok(res) => res,
            Err(e) => {
                if e.contains(REST_QUOTA_EXHAUSTED_TAG) {
                    tracing::warn!(
                        "generate_document all 4 REST author models exhausted; requesting complete direct spec_json"
                    );
                    let schema_hint = if format == DocFormat::Xlsx {
                        r#"{"title":"...","subtitle":"...","doc_type":"spreadsheet","sheets":[{"name":"Sheet1","headers":["..."],"rows":[["..."]],"notes":[]}]}"#
                    } else {
                        r#"{"title":"...","subtitle":"...","subject":"...","level":"...","doc_type":"exam","duration":"60 phút","total_points":"10 điểm","sections":[{"heading":"...","passage_title":"","passage":null,"paragraphs":["..."],"questions":[{"number":"Câu 1","prompt":"...","choices":["A. ...","B. ...","C. ...","D. ..."],"points":"0.5 đ","answer_lines":0}]}],"transcripts":[],"answer_key":[{"number":"Câu 1","answer":"A","explanation":"..."}]}"#
                    };
                    let extra_warn = if direct_issues.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " Previous draft was REJECTED because: {}.",
                            direct_issues.join(" | ")
                        )
                    };
                    return ToolResult::ok(format!(
                        "[ACTION REQUIRED — COMPLETE DIRECT DOCUMENT MODE: All secondary REST models are at daily quota.{extra_warn} IMMEDIATELY call generate_document(topic={topic:?}, format={raw_fmt:?}, spec_json=<COMPLETE JSON matching {schema_hint}>) with ALL requested questions, full passages/prompts, and ZERO '..' placeholders — Boogie will validate, open Save As, and render the {} file locally!]",
                        format.display_name()
                    ));
                }
                return ToolResult::err(e);
            }
        };

        let mut best_spec = match parse_document_spec_lenient(&raw_json) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("DocumentSpec JSON parse failed ({e}); triggering clean JSON regeneration");
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
        let mut best_model = first_model;

        // Quality Gate & Auto-Repair: if draft is missing questions, lacks reading passages,
        // lacks listening transcripts, or contains lazy '..' stubs, automatically repair ONCE
        // before popping up Save As!
        let issues = validate_document_spec(
            &best_spec,
            format,
            &topic,
            question_count,
            include_answers,
        );
        if !issues.is_empty() {
            tracing::warn!(
                "generate_document quality gate flagged {} issue(s) on first draft ({best_model}): {}",
                issues.len(),
                issues.join(" | ")
            );
            status_guard.update(
                "Bước 2/4 · Đang kiểm duyệt & bổ sung đủ câu hỏi, bài đọc và đáp án...",
                0.58,
            );
            let repair_prompt = format!(
                "{user_prompt}\n\n\
                 CRITICAL QUALITY AUDIT FAILED ON PREVIOUS DRAFT — FIX EVERY ISSUE BELOW IN ONE COMPLETE JSON REGENERATION:\n\
                 - {}\n\n\
                 Do NOT leave ANY placeholder, truncated '..' text, empty Writing/Speaking heading, missing Reading passage, or missing Listening transcript. Output the FULL complete JSON object now.",
                issues.join("\n- ")
            );
            if let Ok((repaired_raw, repaired_model)) = call_document_author(
                &cfg,
                &system_prompt,
                &repair_prompt,
                captured.as_ref(),
                24_576,
            )
            .await
                && let Ok(repaired_spec) = parse_document_spec_lenient(&repaired_raw)
            {
                let repaired_issues = validate_document_spec(
                    &repaired_spec,
                    format,
                    &topic,
                    question_count,
                    include_answers,
                );
                let old_score = spec_richness_score(&best_spec, issues.len());
                let new_score = spec_richness_score(&repaired_spec, repaired_issues.len());
                tracing::info!(
                    "generate_document auto-repair via {repaired_model}: issues {} -> {}, score {} -> {}",
                    issues.len(),
                    repaired_issues.len(),
                    old_score,
                    new_score
                );
                if new_score >= old_score {
                    best_spec = repaired_spec;
                    best_model = repaired_model;
                }
            }
        }

        (best_spec, best_model)
    };

    // If a newer generate_document call superseded this one while waiting on the API, abort cleanly
    // so an older Save As dialog NEVER pops up out of order!
    if ACTIVE_DOC_GEN.load(Ordering::SeqCst) != my_gen {
        tracing::info!(
            "generate_document (gen={my_gen}) superseded by newer request — skipping Save As"
        );
        return ToolResult::ok("Superseded by a newer document request.".to_string());
    }

    if spec.title.trim().is_empty() {
        spec.title = title_hint.unwrap_or_else(|| "Boogie_Document".to_string());
    }
    if spec.doc_type.trim().is_empty() && is_exam_intent(&topic) {
        spec.doc_type = "exam".to_string();
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
            tracing::error!(
                "generate_document render failed ({}): {e}",
                format.display_name()
            );
            let _ = event_tx.send(AppEvent::Log(format!(
                "Failed to render {}: {e}",
                format.display_name()
            )));
            return ToolResult::err(format!("Failed to render {}: {e}", format.display_name()));
        }
        Err(e) => {
            tracing::error!("generate_document render task panicked: {e}");
            return ToolResult::err(format!("Document render task failed: {e}"));
        }
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

fn mono_fonts() -> RunFonts {
    RunFonts::new()
        .ascii("Consolas")
        .hi_ansi("Consolas")
        .east_asia("Consolas")
        .cs("Consolas")
}

fn is_english_document(spec: &DocumentSpec) -> bool {
    let subj = spec.subject.to_lowercase();
    let title = spec.title.to_lowercase();
    if subj.contains("english")
        || subj.contains("vstep")
        || subj.contains("ielts")
        || subj.contains("toeic")
        || title.contains("vstep")
        || title.contains("ielts")
        || title.contains("toeic")
    {
        return true;
    }
    // Check if majority of section headings are English exam headings
    let en_headings = spec
        .sections
        .iter()
        .filter(|s| {
            let h = s.heading.to_lowercase();
            h.contains("section ")
                || h.contains("reading")
                || h.contains("listening")
                || h.contains("writing")
                || h.contains("speaking")
                || h.contains("grammar")
                || h.contains("part ")
        })
        .count();
    en_headings >= 2
}

fn is_exam_spec(spec: &DocumentSpec) -> bool {
    if spec.doc_type.eq_ignore_ascii_case("exam") {
        return true;
    }
    let total_q: usize = spec.sections.iter().map(|s| s.questions.len()).sum();
    total_q >= 4 && is_exam_intent(&format!("{} {}", spec.title, spec.subtitle))
}

fn format_question_label(raw_num: &str, fallback_idx: usize, english: bool) -> String {
    let cleaned = raw_num
        .trim()
        .trim_end_matches(['.', ':', ')'])
        .trim();
    if cleaned.is_empty() {
        if english {
            format!("Question {}", fallback_idx + 1)
        } else {
            format!("Câu {}", fallback_idx + 1)
        }
    } else if cleaned.chars().all(|c| c.is_ascii_digit()) {
        if english {
            format!("Question {cleaned}")
        } else {
            format!("Câu {cleaned}")
        }
    } else {
        cleaned.to_string()
    }
}

/// Renders a crisp Vietnamese-UTF8 Word `.docx` document in pure Rust.
pub fn render_docx(spec: &DocumentSpec, output_path: &Path) -> Result<(), String> {
    let mut doc = Docx::new();
    let english = is_english_document(spec);
    let exam_mode = is_exam_spec(spec);

    // Title
    let title_text = if spec.title.trim().is_empty() {
        "Study Document"
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
    if !spec.duration.trim().is_empty() {
        meta_parts.push(if english {
            format!("Time: {}", spec.duration.trim())
        } else {
            format!("Thời gian: {}", spec.duration.trim())
        });
    }
    if !spec.total_points.trim().is_empty() {
        meta_parts.push(spec.total_points.trim().to_string());
    }
    if meta_parts.is_empty() {
        meta_parts.push(chrono::Local::now().format("%Y-%m-%d").to_string());
    }

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
        "475569",
        true,
    )));

    if exam_mode {
        let info_line = if english {
            "Full Name: ____________________________    ID / Class: ________________    Score: _______"
        } else {
            "Họ và tên: ____________________________    Lớp / SBD: ________________    Điểm: _______"
        };
        let info_cell = TableCell::new()
            .shading(Shading::new().fill("F8FAFC"))
            .width(9200, WidthType::Dxa)
            .add_paragraph(Paragraph::new().add_run(styled_run(info_line, 20, "334155", false)));
        doc = doc.add_table(Table::new(vec![TableRow::new(vec![info_cell])]));
    }

    if !spec.instructions.is_empty() {
        for inst in &spec.instructions {
            if !inst.trim().is_empty() {
                doc = doc.add_paragraph(
                    Paragraph::new()
                        .indent(Some(240), None, None, None)
                        .add_run(styled_run(&format!("▸  {}", inst.trim()), 20, "475569", false)),
                );
            }
        }
    }
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

        if let Some(passage) = section
            .passage
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let mut cell = TableCell::new()
                .shading(Shading::new().fill("F8FAFC"))
                .width(9200, WidthType::Dxa);
            if !section.passage_title.trim().is_empty() {
                cell = cell.add_paragraph(Paragraph::new().add_run(styled_run(
                    section.passage_title.trim(),
                    22,
                    "0369A1",
                    true,
                )));
            }
            for line in passage.split('\n') {
                if !line.trim().is_empty() {
                    cell = cell.add_paragraph(Paragraph::new().add_run(styled_run(
                        line.trim(),
                        21,
                        "0F172A",
                        false,
                    )));
                }
            }
            doc = doc.add_table(Table::new(vec![TableRow::new(vec![cell])]));
            doc = doc.add_paragraph(Paragraph::new());
        }

        if let Some(code) = section
            .code_block
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let mut cell = TableCell::new()
                .shading(Shading::new().fill("F1F5F9"))
                .width(9200, WidthType::Dxa);
            for line in code.lines() {
                let run = Run::new()
                    .add_text(line)
                    .size(19)
                    .color("0F172A")
                    .fonts(mono_fonts());
                cell = cell.add_paragraph(Paragraph::new().add_run(run));
            }
            doc = doc.add_table(Table::new(vec![TableRow::new(vec![cell])]));
            doc = doc.add_paragraph(Paragraph::new());
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
            let num = format_question_label(&q.number, q_idx, english);
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
            if q.choices.is_empty() && q.answer_lines > 0 {
                let line_count = q.answer_lines.min(12);
                for _ in 0..line_count {
                    doc = doc.add_paragraph(
                        Paragraph::new().indent(Some(240), None, None, None).add_run(
                            styled_run(
                                "____________________________________________________________________________",
                                20,
                                "CBD5E1",
                                false,
                            ),
                        ),
                    );
                }
            }
        }

        doc = doc.add_paragraph(Paragraph::new());
    }

    if !spec.transcripts.is_empty() {
        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::Page)));
        let tr_title = if english {
            "LISTENING TRANSCRIPTS (AUDIO SCRIPTS)"
        } else {
            "KỊCH BẢN BÀI NGHE (LISTENING TRANSCRIPTS)"
        };
        doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
            tr_title,
            30,
            "0369A1",
            true,
        )));
        doc = doc.add_paragraph(Paragraph::new());
        for t in &spec.transcripts {
            if !t.title.trim().is_empty() {
                doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
                    t.title.trim(),
                    23,
                    "1E3A8A",
                    true,
                )));
            }
            for line in t.content.lines() {
                if !line.trim().is_empty() {
                    doc = doc.add_paragraph(
                        Paragraph::new()
                            .indent(Some(240), None, None, None)
                            .add_run(styled_run(line.trim(), 21, "1E293B", false)),
                    );
                }
            }
            doc = doc.add_paragraph(Paragraph::new());
        }
    }

    if !spec.answer_key.is_empty() {
        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::Page)));
        let ak_title = if english {
            "ANSWER KEY & DETAILED EXPLANATIONS"
        } else {
            "ĐÁP ÁN & LỜI GIẢI CHI TIẾT"
        };
        doc = doc.add_paragraph(Paragraph::new().add_run(styled_run(
            ak_title,
            30,
            "1E3A8A",
            true,
        )));
        doc = doc.add_paragraph(Paragraph::new());

        for (idx, item) in spec.answer_key.iter().enumerate() {
            let num = format_question_label(&item.number, idx, english);
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

/// Extracts a compact MCQ choice token (e.g. "A", "B", "C", "D") for the quick-lookup grid.
fn compact_answer_badge(answer: &str) -> Option<String> {
    let t = answer.trim();
    if t.is_empty() {
        return None;
    }
    let chars: Vec<char> = t.chars().collect();
    if chars.len() == 1 && matches!(chars[0], 'A'..='F' | 'a'..='f') {
        return Some(chars[0].to_ascii_uppercase().to_string());
    }
    if chars.len() >= 2
        && matches!(chars[0], 'A'..='F' | 'a'..='f')
        && matches!(chars[1], '.' | ')' | ':' | ' ')
    {
        return Some(chars[0].to_ascii_uppercase().to_string());
    }
    if t.chars().count() <= 14 && !t.contains('\n') {
        return Some(t.to_string());
    }
    None
}

/// Builds a crisp, Vietnamese-UTF8, print-ready A4 HTML document for headless Chromium PDF printing.
pub fn build_document_html(spec: &DocumentSpec) -> String {
    let title = if spec.title.trim().is_empty() {
        "Study Document"
    } else {
        spec.title.trim()
    };
    let english = is_english_document(spec);
    let exam_mode = is_exam_spec(spec);
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
    if !spec.duration.trim().is_empty() {
        let lbl = if english { "Time" } else { "Thời gian" };
        body.push_str(&format!(
            "<span class=\"pill pill-warm\">{lbl}: {}</span>",
            escape_html(spec.duration.trim())
        ));
    }
    if !spec.total_points.trim().is_empty() {
        body.push_str(&format!(
            "<span class=\"pill pill-muted\">{}</span>",
            escape_html(spec.total_points.trim())
        ));
    }
    body.push_str("</div>");
    body.push_str(&format!("<h1>{}</h1>", escape_html(title)));
    if !spec.subtitle.trim().is_empty() {
        body.push_str(&format!(
            "<p class=\"subtitle\">{}</p>",
            escape_html(spec.subtitle.trim())
        ));
    }

    if exam_mode {
        if english {
            body.push_str(
                "<div class=\"candidate-strip\">\
                 <div class=\"cand-field cand-wide\"><span>Full Name:</span><span class=\"cand-line\"></span></div>\
                 <div class=\"cand-field\"><span>ID / Class:</span><span class=\"cand-line\"></span></div>\
                 <div class=\"cand-field cand-score\"><span>Score:</span><span class=\"cand-line\"></span></div>\
                 </div>",
            );
        } else {
            body.push_str(
                "<div class=\"candidate-strip\">\
                 <div class=\"cand-field cand-wide\"><span>Họ và tên:</span><span class=\"cand-line\"></span></div>\
                 <div class=\"cand-field\"><span>Lớp / SBD:</span><span class=\"cand-line\"></span></div>\
                 <div class=\"cand-field cand-score\"><span>Điểm:</span><span class=\"cand-line\"></span></div>\
                 </div>",
            );
        }
    }

    if !spec.instructions.is_empty() {
        let inst_label = if english {
            "Instructions:"
        } else {
            "Lưu ý làm bài:"
        };
        body.push_str("<div class=\"instructions-box\">");
        body.push_str(&format!("<strong>{inst_label}</strong><ul>"));
        for item in &spec.instructions {
            if !item.trim().is_empty() {
                body.push_str(&format!("<li>{}</li>", escape_html(item.trim())));
            }
        }
        body.push_str("</ul></div>");
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
        if let Some(passage) = sec
            .passage
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            body.push_str("<div class=\"passage-box\">");
            if !sec.passage_title.trim().is_empty() {
                body.push_str(&format!(
                    "<div class=\"passage-title\">{}</div>",
                    escape_html(sec.passage_title.trim())
                ));
            }
            for para in passage.split("\n\n") {
                if !para.trim().is_empty() {
                    body.push_str(&format!(
                        "<p>{}</p>",
                        escape_html(para.trim()).replace('\n', "<br/>")
                    ));
                }
            }
            body.push_str("</div>");
        }
        if let Some(code) = sec
            .code_block
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            body.push_str(&format!(
                "<pre class=\"code-box\"><code>{}</code></pre>",
                escape_html(code)
            ));
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
            let key_lbl = if english {
                "Key Note:"
            } else {
                "Trọng tâm:"
            };
            body.push_str(&format!(
                "<div class=\"callout\"><strong>★ {key_lbl}</strong> {}</div>",
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
            let num = format_question_label(&q.number, q_idx, english);
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
                // Switch to single-column choices automatically when options are long sentences
                let max_choice_len = q
                    .choices
                    .iter()
                    .map(|c| c.chars().count())
                    .max()
                    .unwrap_or(0);
                let grid_cls = if max_choice_len > 48 {
                    "choices choices-1col"
                } else {
                    "choices"
                };
                body.push_str(&format!("<div class=\"{grid_cls}\">"));
                for c in &q.choices {
                    if !c.trim().is_empty() {
                        body.push_str(&format!(
                            "<div class=\"choice\">{}</div>",
                            escape_html(c.trim())
                        ));
                    }
                }
                body.push_str("</div>");
            } else if q.answer_lines > 0 {
                body.push_str("<div class=\"answer-lines\">");
                for _ in 0..q.answer_lines.min(14) {
                    body.push_str("<div class=\"write-line\"></div>");
                }
                body.push_str("</div>");
            }
            body.push_str("</div>");
        }
        body.push_str("</section>");
    }

    if !spec.transcripts.is_empty() {
        body.push_str("<section class=\"transcript-section\">");
        let tr_heading = if english {
            "LISTENING TRANSCRIPTS (AUDIO SCRIPTS FOR SELF-STUDY)"
        } else {
            "KỊCH BẢN BÀI NGHE (LISTENING TRANSCRIPTS)"
        };
        body.push_str(&format!("<h2>{tr_heading}</h2>"));
        for tr in &spec.transcripts {
            body.push_str("<div class=\"transcript-card\">");
            if !tr.title.trim().is_empty() {
                body.push_str(&format!(
                    "<div class=\"tr-title\">{}</div>",
                    escape_html(tr.title.trim())
                ));
            }
            body.push_str(&format!(
                "<div class=\"tr-body\">{}</div>",
                escape_html(tr.content.trim()).replace('\n', "<br/>")
            ));
            body.push_str("</div>");
        }
        body.push_str("</section>");
    }

    if !spec.answer_key.is_empty() {
        body.push_str("<section class=\"answer-key\">");
        let ak_heading = if english {
            "ANSWER KEY &amp; DETAILED EXPLANATIONS"
        } else {
            "ĐÁP ÁN &amp; LỜI GIẢI CHI TIẾT"
        };
        body.push_str(&format!("<h2>{ak_heading}</h2>"));

        // Compact quick-lookup answer grid for short/MCQ items
        let quick_items: Vec<(String, String)> = spec
            .answer_key
            .iter()
            .enumerate()
            .filter_map(|(idx, a)| {
                let badge = compact_answer_badge(&a.answer)?;
                let short_num = format_question_label(&a.number, idx, english)
                    .replace("Question ", "Q")
                    .replace("Câu ", "");
                Some((short_num, badge))
            })
            .collect();
        if quick_items.len() >= 4 {
            let quick_title = if english {
                "Quick Answer Grid"
            } else {
                "Bảng tra nhanh đáp án"
            };
            body.push_str(&format!(
                "<div class=\"quick-grid-wrap\"><div class=\"quick-grid-title\">{quick_title}</div><div class=\"quick-grid\">"
            ));
            for (k, v) in &quick_items {
                body.push_str(&format!(
                    "<div class=\"qg-cell\"><span class=\"qg-num\">{}</span><span class=\"qg-ans\">{}</span></div>",
                    escape_html(k),
                    escape_html(v)
                ));
            }
            body.push_str("</div></div>");
        }

        for (idx, a) in spec.answer_key.iter().enumerate() {
            let num = format_question_label(&a.number, idx, english);
            body.push_str("<div class=\"ans-card\">");
            body.push_str(&format!(
                "<div class=\"ans-head\"><strong>{}:</strong> <span class=\"ans-val\">{}</span></div>",
                escape_html(&num),
                escape_html(a.answer.trim()).replace('\n', "<br/>")
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

    let page_word = if english { "Page " } else { "Trang " };
    let footer_title = crate::util::truncate_chars(title, 54)
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let html_lang = if english { "en" } else { "vi" };

    format!(
        r#"<!DOCTYPE html>
<html lang="{html_lang}">
<head>
<meta charset="UTF-8" />
<title>{title_esc}</title>
<style>
  @page {{
    size: A4;
    margin: 15mm 15mm 18mm 15mm;
    @bottom-left {{
      content: "{footer_title} · {date_str}";
      font-family: 'Segoe UI', Arial, sans-serif;
      font-size: 8pt;
      color: #94A3B8;
    }}
    @bottom-right {{
      content: "{page_word}" counter(page) " / " counter(pages);
      font-family: 'Segoe UI', Arial, sans-serif;
      font-size: 8pt;
      color: #64748B;
      font-weight: 600;
    }}
  }}
  * {{ box-sizing: border-box; }}
  body {{
    font-family: 'Segoe UI', 'Cambria Math', Calibri, Arial, sans-serif;
    color: #0F172A;
    font-size: 10.5pt;
    line-height: 1.55;
    margin: 0;
  }}
  .doc-header {{
    border-bottom: 2.5px solid #1E3A8A;
    padding-bottom: 12px;
    margin-bottom: 16px;
  }}
  .badge-row {{ display: flex; flex-wrap: wrap; gap: 7px; margin-bottom: 7px; }}
  .pill {{
    display: inline-block;
    font-size: 8.5pt;
    font-weight: 600;
    padding: 2px 9px;
    border-radius: 999px;
  }}
  .pill-primary {{ background: #DBEAFE; color: #1E3A8A; }}
  .pill-accent {{ background: #D1FAE5; color: #065F46; }}
  .pill-warm {{ background: #FEF3C7; color: #92400E; }}
  .pill-muted {{ background: #F1F5F9; color: #475569; }}
  h1 {{ margin: 4px 0 4px; font-size: 19pt; color: #1E3A8A; line-height: 1.25; letter-spacing: -0.01em; }}
  .subtitle {{ margin: 0; color: #475569; font-size: 10pt; }}
  .candidate-strip {{
    display: flex;
    gap: 14px;
    margin-top: 10px;
    padding: 8px 11px;
    background: #F8FAFC;
    border: 1px solid #CBD5E1;
    border-radius: 6px;
    font-size: 9.5pt;
    color: #334155;
  }}
  .cand-field {{ display: flex; align-items: baseline; gap: 6px; flex: 1; white-space: nowrap; }}
  .cand-wide {{ flex: 1.7; }}
  .cand-score {{ flex: 0.7; }}
  .cand-line {{ flex: 1; border-bottom: 1px dotted #94A3B8; min-width: 40px; height: 12px; }}
  .instructions-box {{
    margin-top: 9px;
    padding: 7px 11px;
    background: #FFFBEB;
    border-left: 3.5px solid #D97706;
    border-radius: 5px;
    font-size: 9.5pt;
    color: #78350F;
  }}
  .instructions-box ul {{ margin: 3px 0 0 16px; }}
  .instructions-box li {{ margin-bottom: 2px; }}
  .doc-section {{ margin-bottom: 15px; }}
  h2 {{
    font-size: 12.5pt;
    color: #1D4ED8;
    border-left: 4px solid #2563EB;
    padding: 2px 0 2px 9px;
    margin: 14px 0 8px;
    page-break-after: avoid;
  }}
  p {{ margin: 5px 0 8px; }}
  ul {{ margin: 4px 0 10px 18px; padding: 0; }}
  li {{ margin-bottom: 4px; }}
  .passage-box {{
    background: #F8FAFC;
    border: 1px solid #CBD5E1;
    border-left: 4px solid #0284C7;
    border-radius: 6px;
    padding: 11px 14px;
    margin: 9px 0 12px;
    font-size: 10.2pt;
    line-height: 1.65;
    text-align: justify;
    color: #0F172A;
  }}
  .passage-title {{
    font-weight: 700;
    color: #0369A1;
    font-size: 10.5pt;
    margin-bottom: 6px;
    text-align: left;
  }}
  .code-box {{
    background: #F1F5F9;
    border: 1px solid #CBD5E1;
    border-left: 4px solid #475569;
    border-radius: 6px;
    padding: 9px 12px;
    margin: 9px 0 12px;
    font-family: Consolas, 'Cascadia Mono', 'Courier New', monospace;
    font-size: 9.2pt;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    color: #0F172A;
    break-inside: avoid;
  }}
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
    font-size: 9.8pt;
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
    margin: 9px 0;
    padding: 8px 11px;
    background: #FFFFFF;
    border: 1px solid #E2E8F0;
    border-radius: 7px;
    break-inside: avoid;
  }}
  .q-num {{ font-weight: 700; color: #1E3A8A; }}
  .q-pts {{ color: #64748B; font-size: 9pt; font-weight: 600; }}
  .choices {{
    margin-top: 6px;
    padding-left: 14px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px 14px;
  }}
  .choices-1col {{ grid-template-columns: 1fr; }}
  .choice {{ color: #1E293B; }}
  .answer-lines {{ margin-top: 8px; }}
  .write-line {{ height: 22px; border-bottom: 1px dashed #CBD5E1; }}
  .transcript-section {{ page-break-before: always; }}
  .transcript-card {{
    margin: 9px 0;
    padding: 9px 12px;
    background: #F0F9FF;
    border: 1px solid #BAE6FD;
    border-left: 4px solid #0284C7;
    border-radius: 6px;
    font-size: 9.8pt;
    line-height: 1.58;
    break-inside: avoid;
  }}
  .tr-title {{ font-weight: 700; color: #0369A1; margin-bottom: 4px; }}
  .tr-body {{ color: #1E293B; }}
  .answer-key {{ page-break-before: always; }}
  .quick-grid-wrap {{
    margin: 8px 0 14px;
    padding: 9px 11px;
    background: #F8FAFC;
    border: 1px solid #CBD5E1;
    border-radius: 7px;
    break-inside: avoid;
  }}
  .quick-grid-title {{ font-weight: 700; font-size: 9.5pt; color: #1E3A8A; margin-bottom: 6px; }}
  .quick-grid {{
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 5px;
  }}
  .qg-cell {{
    display: flex;
    justify-content: space-between;
    padding: 3px 7px;
    background: #FFFFFF;
    border: 1px solid #E2E8F0;
    border-radius: 4px;
    font-size: 9pt;
  }}
  .qg-num {{ font-weight: 600; color: #475569; }}
  .qg-ans {{ font-weight: 700; color: #047857; }}
  .ans-card {{
    margin: 7px 0;
    padding: 7px 11px;
    border-left: 3px solid #059669;
    background: #F0FDF4;
    border-radius: 5px;
    break-inside: avoid;
  }}
  .ans-val {{ color: #047857; font-weight: 700; }}
  .ans-exp {{ margin-top: 3px; color: #334155; font-size: 9.8pt; }}
</style>
</head>
<body>
{body}
</body>
</html>"#,
        html_lang = html_lang,
        title_esc = escape_html(title),
        footer_title = footer_title,
        date_str = date_str,
        page_word = page_word,
        body = body,
    )
}

/// Returns installed Chromium-family browsers in best reliability order for headless PDF printing:
/// Google Chrome first (fast ~1.4s, no Edge StartupBoost/FirstLaunch relaunch quirk),
/// then Microsoft Edge (preinstalled on every Windows 10/11 PC), then Brave as final fallback.
pub fn find_chromium_binaries() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"),
    ];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let base = PathBuf::from(local);
        candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
        candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
        candidates.push(base.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
    }
    let mut found = Vec::new();
    for path in candidates {
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

fn wait_for_valid_pdf(path: &Path, max_wait: std::time::Duration) -> Option<Vec<u8>> {
    let deadline = std::time::Instant::now() + max_wait;
    let mut last_len: Option<usize> = None;
    loop {
        if let Ok(bytes) = std::fs::read(path)
            && bytes.len() >= 100
            && bytes.starts_with(b"%PDF-")
        {
            if last_len == Some(bytes.len())
                || bytes.windows(5).rev().take(64).any(|w| w == b"%%EOF")
            {
                return Some(bytes);
            }
            last_len = Some(bytes.len());
        }
        if std::time::Instant::now() >= deadline {
            if let Ok(bytes) = std::fs::read(path)
                && bytes.len() >= 100
                && bytes.starts_with(b"%PDF-")
            {
                return Some(bytes);
            }
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

fn try_render_pdf_with_browser(
    browser: &Path,
    html_url: &str,
    temp_pdf: &Path,
    profile_dir: &Path,
) -> Result<Vec<u8>, String> {
    let _ = std::fs::remove_file(temp_pdf);

    let mut cmd = std::process::Command::new(browser);
    cmd.args([
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-extensions",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-sync",
        "--disable-features=msEdgeSidebarV2,EdgeShoppingAssistant,msEdgeStartupBoost",
        "--no-first-run",
        "--no-default-browser-check",
        "--no-pdf-header-footer",
        &format!("--user-data-dir={}", profile_dir.display()),
        &format!("--print-to-pdf={}", temp_pdf.display()),
        html_url,
    ])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to launch {}: {e}", browser.display()))?;

    // Hard 16s timeout so a busy/already-running browser (e.g. Brave) never hangs PDF export.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(16);
    let mut exit_code: Option<i32> = None;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_code = status.code();
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("wait failed: {e}"));
            }
        }
    }

    let stderr_text = child
        .stderr
        .take()
        .and_then(|mut s| {
            use std::io::Read as _;
            let mut buf = String::new();
            s.read_to_string(&mut buf).ok().map(|_| buf)
        })
        .unwrap_or_default();

    // Microsoft Edge on Windows can re-launch a detached child on first profile boot and exit the
    // parent process ~2-3s before the PDF file finishes flushing. Wait up to 6s BEFORE deleting
    // the temp HTML or giving up!
    let wait_budget = if timed_out {
        std::time::Duration::from_millis(200)
    } else {
        std::time::Duration::from_secs(6)
    };
    let pdf_bytes = wait_for_valid_pdf(temp_pdf, wait_budget);

    let _ = std::fs::remove_file(temp_pdf);
    let _ = std::fs::remove_dir_all(profile_dir);

    match pdf_bytes {
        Some(bytes) => Ok(bytes),
        None => Err(format!(
            "{} did not produce a valid PDF (timeout={timed_out}, exit={exit_code:?}): {}",
            browser
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("browser"),
            crate::util::truncate_chars(stderr_text.trim(), 200)
        )),
    }
}

/// Renders a crisp UTF-8 Vietnamese PDF using installed Chromium browsers (Chrome → Edge → Brave).
/// Always prints to a clean ASCII temp PDF first, waits for Edge detached-process flushes, and
/// then writes to the user's chosen Unicode/OneDrive destination with clear lock-error reporting.
pub fn render_pdf(spec: &DocumentSpec, output_path: &Path) -> Result<(), String> {
    let browsers = find_chromium_binaries();
    if browsers.is_empty() {
        return Err(
            "No Chrome / Microsoft Edge / Brave browser found for PDF printing. Please save as Word (.docx) instead."
                .to_string(),
        );
    }

    let seq = TEMP_DOC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir();
    let html_path = temp_dir.join(format!("boogie_doc_{pid}_{seq}.html"));

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

    let html_url = match url::Url::from_file_path(&html_path) {
        Ok(u) => u.to_string(),
        Err(_) => {
            let _ = std::fs::remove_file(&html_path);
            return Err("failed to convert temp html path to file:// URL".to_string());
        }
    };

    let mut errors = Vec::new();
    let mut rendered_bytes: Option<Vec<u8>> = None;

    for (idx, browser) in browsers.iter().enumerate() {
        let temp_pdf = temp_dir.join(format!("boogie_pdf_out_{pid}_{seq}_{idx}.pdf"));
        let profile_dir = temp_dir.join(format!("boogie_pdf_profile_{pid}_{seq}_{idx}"));
        match try_render_pdf_with_browser(browser, &html_url, &temp_pdf, &profile_dir) {
            Ok(bytes) => {
                rendered_bytes = Some(bytes);
                break;
            }
            Err(e) => {
                tracing::warn!("render_pdf attempt #{} failed: {e}", idx + 1);
                errors.push(e);
            }
        }
    }

    let _ = std::fs::remove_file(&html_path);

    let pdf_bytes = rendered_bytes.ok_or_else(|| {
        format!(
            "PDF printer failed across {} browser(s): {}",
            browsers.len(),
            errors.join(" | ")
        )
    })?;

    if let Err(e) = std::fs::write(&abs_out, &pdf_bytes) {
        let locked_hint = if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.raw_os_error() == Some(32)
            || e.raw_os_error() == Some(33)
        {
            " (file may currently be open in Foxit/Edge/PDF Reader — close it or pick a new filename)"
        } else {
            ""
        };
        return Err(format!(
            "Could not save PDF to '{}': {e}{locked_hint}",
            abs_out.display()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> DocumentSpec {
        DocumentSpec {
            title: "Đề cương ôn tập Toán 12 — Khảo sát hàm số".to_string(),
            subtitle: "Tổng hợp công thức & câu hỏi trắc nghiệm mẫu có đáp án".to_string(),
            subject: "Toán học".to_string(),
            level: "Cấp 3 (Lớp 12)".to_string(),
            doc_type: "exam".to_string(),
            duration: "45 phút".to_string(),
            total_points: "10 điểm".to_string(),
            instructions: vec!["Không sử dụng tài liệu trong quá trình làm bài.".to_string()],
            sections: vec![DocSection {
                heading: "I. Kiến thức trọng tâm".to_string(),
                passage_title: "Tóm tắt lý thuyết".to_string(),
                passage: Some(
                    "Đạo hàm cho biết chiều biến thiên của hàm số trên từng khoảng xác định."
                        .to_string(),
                ),
                code_block: Some("def deriv(x): return 3*x*x - 3".to_string()),
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
                questions: vec![
                    ExamQuestion {
                        number: "Câu 1".to_string(),
                        prompt: "Hàm số y = x³ - 3x + 1 nghịch biến trên khoảng nào?".to_string(),
                        choices: vec![
                            "A. (-1; 1)".to_string(),
                            "B. (-∞; -1)".to_string(),
                            "C. (1; +∞)".to_string(),
                            "D. ℝ".to_string(),
                        ],
                        points: "0.5 đ".to_string(),
                        answer_lines: 0,
                    },
                    ExamQuestion {
                        number: "Câu 2".to_string(),
                        prompt: "Tìm tất cả cực trị của hàm số y = x³ - 3x + 1.".to_string(),
                        choices: vec![],
                        points: "1.5 đ".to_string(),
                        answer_lines: 4,
                    },
                ],
            }],
            transcripts: vec![TranscriptItem {
                title: "Audio Script Sample".to_string(),
                content: "Speaker A: Remember the math workshop begins at 9 AM sharp.".to_string(),
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
        assert!(html.contains("ĐÁP ÁN &amp; LỜI GIẢI CHI TIẾT"));
        assert!(html.contains("candidate-strip"));
        assert!(html.contains("passage-box"));
        assert!(html.contains("code-box"));
        assert!(html.contains("write-line"));
        assert!(html.contains("transcript-section"));
        assert!(!html.contains("Boogie Document Studio"));
    }

    #[test]
    fn quality_gate_rejects_stubby_vstep_exam_and_approves_complete_exam() {
        // Reproduces the exact 2-question placeholder VSTEP stub so it can NEVER slip past Save As
        let stub = DocumentSpec {
            title: "ĐỀ THI VSTEP B2 NÂNG CAO".to_string(),
            subject: "English".to_string(),
            level: "B2".to_string(),
            doc_type: "exam".to_string(),
            sections: vec![
                DocSection {
                    heading: "Section 1: Listening Comprehension".to_string(),
                    paragraphs: vec!["Directions: answer questions based on audio.".to_string()],
                    questions: vec![ExamQuestion {
                        number: "Câu 1".to_string(),
                        prompt: "What does the speaker imply about the upcoming event?".to_string(),
                        choices: vec!["A. Postponed".to_string(), "B. Cancelled".to_string()],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                DocSection {
                    heading: "Section 2: Reading Comprehension".to_string(),
                    paragraphs: vec![
                        "Passage excerpt: Technology has fundamentally altered the pedagogical landscape..".to_string(),
                    ],
                    questions: vec![ExamQuestion {
                        number: "Câu 2".to_string(),
                        prompt: "The word fundamentally is closest in meaning to..?".to_string(),
                        choices: vec!["A. slightly".to_string(), "B. basically".to_string()],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                DocSection {
                    heading: "Section 3: Writing".to_string(),
                    paragraphs: vec!["Task 1: Email writing (formal). Task 2: Essay writing.".to_string()],
                    ..Default::default()
                },
                DocSection {
                    heading: "Section 4: Speaking".to_string(),
                    paragraphs: vec!["Part 1: Social Interaction. Part 2: Solution Discussion.".to_string()],
                    ..Default::default()
                },
            ],
            answer_key: vec![AnswerItem {
                number: "Câu 1".to_string(),
                answer: "A".to_string(),
                explanation: "...".to_string(),
            }],
            ..Default::default()
        };

        let issues = validate_document_spec(
            &stub,
            DocFormat::Pdf,
            "Đề thi VSTEP B2 đầy đủ 4 kỹ năng",
            Some(20),
            true,
        );
        assert!(
            issues.len() >= 5,
            "quality gate must catch shortfall + placeholder + missing passage + missing transcript + empty writing/speaking, got: {issues:?}"
        );
        assert!(issues.iter().any(|i| i.contains("Requested 20")));
        assert!(issues.iter().any(|i| i.contains("Reading")));
        assert!(issues.iter().any(|i| i.contains("transcript")));
        assert!(issues.iter().any(|i| i.contains("Writing")));
        assert!(issues.iter().any(|i| i.contains("Speaking")));

        // Verify subject profile detector handles all major subjects
        assert_eq!(
            detect_subject_profile("Đề thi VSTEP B2 4 kỹ năng", "English"),
            SubjectProfile::EnglishExam
        );
        assert_eq!(
            detect_subject_profile("Đề kiểm tra Giải tích lớp 12", "math"),
            SubjectProfile::Math
        );
        assert_eq!(
            detect_subject_profile("Đề ôn tập Hóa học hữu cơ 11", ""),
            SubjectProfile::NaturalScience
        );
        assert_eq!(
            detect_subject_profile("Đề thi SQL và lập trình Python", "IT"),
            SubjectProfile::Informatics
        );
        assert_eq!(
            detect_subject_profile("Đề kiểm tra Ngữ văn 12 đọc hiểu và nghị luận", ""),
            SubjectProfile::Literature
        );
        assert_eq!(
            detect_subject_profile("Đề ôn tập Lịch sử Việt Nam 1945-1975", ""),
            SubjectProfile::HumanitiesEcon
        );
    }

    #[test]
    fn renders_valid_pdf_when_chromium_is_installed() {
        if find_chromium_binaries().is_empty() {
            return;
        }
        let pdf_path = std::env::temp_dir().join(format!(
            "ĐỀ THI THỬ TIẾNG ANH B2 VSTEP_{}.pdf",
            std::process::id()
        ));
        render_pdf(&sample_spec(), &pdf_path)
            .expect("pdf render via headless Chrome/Edge should succeed with Unicode filename");
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
