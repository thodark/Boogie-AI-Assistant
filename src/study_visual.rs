use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, FontFamily, FontId, Pos2, Rect, RichText,
    Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::AppEvent;
use crate::actions::documents::{
    DocFormat, DocSection, DocTable, DocumentSpec, TaskProgressGuard, save_spec_with_dialog,
};
use crate::actions::study::{
    CapturedImage, REST_QUOTA_EXHAUSTED_TAG, call_study_brain, capture_clipboard_png,
    capture_screen_png, captured_to_jpeg, extract_json_payload, strip_json_nulls,
};
use crate::config::ApiConfig;
use crate::theme::ThemePalette;
use crate::tools::{ToolDeclaration, ToolRegistry, ToolResult};

static NEXT_BOARD_ID: AtomicU64 = AtomicU64::new(1);

const TEXT_BRIGHT: Color32 = Color32::from_rgb(240, 238, 255);
const TEXT_SOFT: Color32 = Color32::from_rgb(174, 168, 210);
const TEXT_MUTED: Color32 = Color32::from_rgb(112, 106, 148);
const MINT: Color32 = Color32::from_rgb(64, 242, 184);
const AMBER: Color32 = Color32::from_rgb(255, 178, 86);
const CRIMSON: Color32 = Color32::from_rgb(255, 86, 122);
const ART_MAX_DIM: u32 = 640;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StudyBoardData {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub model_used: String,
    #[serde(default)]
    pub steps: Vec<StudyStep>,
    #[serde(default)]
    pub blocks: Vec<VisualBlock>,
    #[serde(default)]
    pub art_study: Option<ArtStudyData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StudyStep {
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub highlight: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VisualBlock {
    Graph2d {
        #[serde(default)]
        title: String,
        #[serde(default = "default_neg5")]
        x_min: f32,
        #[serde(default = "default_pos5")]
        x_max: f32,
        #[serde(default = "default_neg5")]
        y_min: f32,
        #[serde(default = "default_pos5")]
        y_max: f32,
        #[serde(default = "default_x_label")]
        x_label: String,
        #[serde(default = "default_y_label")]
        y_label: String,
        #[serde(default)]
        curves: Vec<GraphCurve>,
        #[serde(default)]
        points: Vec<GraphPoint>,
        #[serde(default)]
        vectors: Vec<GraphVector>,
    },
    Table {
        #[serde(default)]
        title: String,
        #[serde(default)]
        headers: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<String>>,
    },
    Diagram {
        #[serde(default)]
        title: String,
        #[serde(default = "default_flowchart")]
        kind: String,
        #[serde(default)]
        nodes: Vec<DiagramNode>,
        #[serde(default)]
        edges: Vec<DiagramEdge>,
    },
    Code {
        #[serde(default)]
        title: String,
        #[serde(default)]
        language: String,
        #[serde(default)]
        lines: Vec<String>,
        #[serde(default)]
        highlight_lines: Vec<usize>,
    },
    FormulaBox {
        #[serde(default)]
        label: String,
        #[serde(default)]
        content: String,
        #[serde(default)]
        note: String,
    },
}

fn default_neg5() -> f32 {
    -5.0
}
fn default_pos5() -> f32 {
    5.0
}
fn default_x_label() -> String {
    "x".to_string()
}
fn default_y_label() -> String {
    "y".to_string()
}
fn default_flowchart() -> String {
    "flowchart".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphCurve {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub expr: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub points: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphPoint {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphVector {
    #[serde(default)]
    pub x0: f32,
    #[serde(default)]
    pub y0: f32,
    #[serde(default)]
    pub x1: f32,
    #[serde(default)]
    pub y1: f32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagramNode {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagramEdge {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ColorSwatch {
    #[serde(default)]
    pub hex: String,
    #[serde(default)]
    pub rgb: [u8; 3],
    #[serde(default)]
    pub percent: f32,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtFocalPoint {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtStudyData {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(skip)]
    pub orig_rgba: Vec<u8>,
    #[serde(skip)]
    pub value_rgba: Vec<u8>,
    #[serde(skip)]
    pub edge_strength: Vec<u8>,
    #[serde(default)]
    pub palette: Vec<ColorSwatch>,
    #[serde(default)]
    pub composition_type: String,
    #[serde(default)]
    pub focal_points: Vec<ArtFocalPoint>,
    #[serde(default)]
    pub brush_steps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawBoardJson {
    #[serde(default)]
    title: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    steps: Vec<StudyStep>,
    #[serde(default)]
    blocks: Vec<VisualBlock>,
    #[serde(default)]
    art_meta: Option<RawArtMeta>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawArtMeta {
    #[serde(default)]
    composition_type: String,
    #[serde(default)]
    focal_points: Vec<ArtFocalPoint>,
    #[serde(default)]
    brush_steps: Vec<String>,
}

impl StudyBoardData {
    /// Converts the active Study Visual lesson into a structured DocumentSpec for 1-click PDF / Word export.
    pub fn to_document_spec(&self) -> DocumentSpec {
        let mut sections = Vec::new();

        if !self.summary.trim().is_empty() {
            sections.push(DocSection {
                heading: "Tóm tắt / Overview".to_string(),
                paragraphs: vec![self.summary.clone()],
                ..Default::default()
            });
        }

        if !self.steps.is_empty() {
            let mut step_sec = DocSection {
                heading: "Các bước giảng bài / Step-by-Step Guide".to_string(),
                ..Default::default()
            };
            for (idx, step) in self.steps.iter().enumerate() {
                let title = if step.heading.trim().is_empty() {
                    format!("Bước {}", idx + 1)
                } else {
                    format!("Bước {}: {}", idx + 1, step.heading.trim())
                };
                let mut line = format!("{title} — {}", step.body.trim());
                if let Some(f) = step
                    .formula
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    line.push_str(&format!("\nCông thức: {f}"));
                }
                if let Some(h) = step
                    .highlight
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    line.push_str(&format!("\nLưu ý: {h}"));
                }
                step_sec.paragraphs.push(line);
            }
            sections.push(step_sec);
        }

        for block in &self.blocks {
            match block {
                VisualBlock::FormulaBox {
                    label,
                    content,
                    note,
                } => {
                    let mut text = content.clone();
                    if !note.trim().is_empty() {
                        text.push_str(&format!(" — {}", note.trim()));
                    }
                    sections.push(DocSection {
                        heading: if label.trim().is_empty() {
                            "Công thức trọng tâm".to_string()
                        } else {
                            label.clone()
                        },
                        callout: Some(text),
                        ..Default::default()
                    });
                }
                VisualBlock::Table {
                    title,
                    headers,
                    rows,
                } => {
                    sections.push(DocSection {
                        heading: if title.trim().is_empty() {
                            "Bảng tổng hợp".to_string()
                        } else {
                            title.clone()
                        },
                        table: Some(DocTable {
                            headers: headers.clone(),
                            rows: rows.clone(),
                        }),
                        ..Default::default()
                    });
                }
                VisualBlock::Diagram {
                    title,
                    kind,
                    nodes,
                    edges,
                } => {
                    let mut bullets = Vec::new();
                    for n in nodes {
                        if n.detail.trim().is_empty() {
                            bullets.push(n.label.clone());
                        } else {
                            bullets.push(format!("{}: {}", n.label, n.detail));
                        }
                    }
                    for e in edges {
                        if e.label.trim().is_empty() {
                            bullets.push(format!("{} → {}", e.from, e.to));
                        } else {
                            bullets.push(format!("{} → {} ({})", e.from, e.to, e.label));
                        }
                    }
                    sections.push(DocSection {
                        heading: if title.trim().is_empty() {
                            format!("Sơ đồ ({kind})")
                        } else {
                            title.clone()
                        },
                        bullets,
                        ..Default::default()
                    });
                }
                VisualBlock::Code {
                    title,
                    language,
                    lines,
                    ..
                } => {
                    sections.push(DocSection {
                        heading: if title.trim().is_empty() {
                            format!("Code ({language})")
                        } else {
                            title.clone()
                        },
                        callout: Some(lines.join("\n")),
                        ..Default::default()
                    });
                }
                VisualBlock::Graph2d {
                    title,
                    curves,
                    points,
                    ..
                } => {
                    let mut bullets = Vec::new();
                    for c in curves {
                        let expr = c.expr.as_deref().unwrap_or("");
                        bullets.push(format!("Đồ thị: {} {}", c.label, expr).trim().to_string());
                    }
                    for p in points {
                        bullets.push(format!("Điểm {} ({:.2}, {:.2})", p.label, p.x, p.y));
                    }
                    if !bullets.is_empty() {
                        sections.push(DocSection {
                            heading: if title.trim().is_empty() {
                                "Đồ thị 2D".to_string()
                            } else {
                                title.clone()
                            },
                            bullets,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        if let Some(art) = &self.art_study {
            let mut art_sec = DocSection {
                heading: format!("Phân tích Mỹ thuật ({})", art.composition_type),
                ..Default::default()
            };
            if !art.palette.is_empty() {
                let sw_list: Vec<String> = art
                    .palette
                    .iter()
                    .map(|s| format!("{} ({:.0}%)", s.hex, s.percent))
                    .collect();
                art_sec.callout = Some(format!("Bảng màu chính: {}", sw_list.join("  ·  ")));
            }
            for step in &art.brush_steps {
                art_sec.bullets.push(step.clone());
            }
            sections.push(art_sec);
        }

        DocumentSpec {
            title: if self.title.trim().is_empty() {
                "Boogie Study Visual Notes".to_string()
            } else {
                self.title.clone()
            },
            subtitle: format!("Exported from Boogie Study Visual ({})", self.model_used),
            subject: self.subject.clone(),
            level: self.level.clone(),
            sections,
            answer_key: Vec::new(),
            sheets: Vec::new(),
        }
    }
}

// ============================================================================
// Tool Registration: `study_board`
// ============================================================================

fn param(_name: &str, ty: &str, desc: &str) -> Value {
    serde_json::json!({ "type": ty, "description": desc })
}

fn tool(name: &str, desc: &str, props: Vec<(&str, Value)>, required: Vec<&str>) -> ToolDeclaration {
    let mut properties = serde_json::Map::new();
    for (k, v) in props {
        properties.insert(k.to_string(), v);
    }
    ToolDeclaration {
        name: name.to_string(),
        description: desc.to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }),
    }
}

pub fn register(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    registry.register(
        tool(
            "study_board",
            "Opens, refreshes, or controls Boogie's interactive 'Study Visual' split-screen teaching board inside the app window. \
             IMPORTANT: ALWAYS speak ONE brief natural sentence aloud FIRST (e.g. 'Đợi mình một xíu, \
             mình đang cập nhật bảng Study Visual cho bạn ngay nhé!') BEFORE emitting action='open'. \
             STICKY TEACHING MODE: Whenever the Study Visual board is already open and the user switches \
             to ANY new question, new concept, new exercise, or follow-up topic (even without repeating \
             'mở bảng' or 'dạy học'), ALWAYS call study_board(action='open', topic=<new topic>) so the \
             board refreshes live to match the new question! \
             Use when the user enters teaching/study mode ('vào chế độ dạy học', 'giảng bài này trên bảng'), \
             wants visual step-by-step explanations, 2D Math/Physics graphs (y=f(x), vectors, points), \
             mindmaps/flowcharts/timelines, comparison tables, code walkthroughs, OR Art Study analysis \
             (analyzing a painting's composition, Rule of Thirds, light/shadow Value Study, color palette HEX codes, \
             and animated Live Sketch Replay from the screen or Win+Shift+S clipboard). \
             Can also step forward ('next_step'), backward ('prev_step'), reveal all ('show_all'), or close ('close').",
            vec![
                (
                    "action",
                    param(
                        "action",
                        "string",
                        "Board action: 'open' (default — build & show visual lesson), 'next_step', 'prev_step', 'show_all', or 'close'",
                    ),
                ),
                (
                    "topic",
                    param(
                        "topic",
                        "string",
                        "What to teach visually (required when action='open'; e.g. 'Giảng hàm bậc 2 y = x^2 - 2x - 3 và vẽ đồ thị', 'Phân tích bố cục, màu sắc và cách vẽ bức tranh trên màn hình')",
                    ),
                ),
                (
                    "subject",
                    param(
                        "subject",
                        "string",
                        "Optional subject hint: math, physics, chemistry, biology, art, informatics, english, literature, history, geography, economics, general",
                    ),
                ),
                (
                    "level",
                    param(
                        "level",
                        "string",
                        "Optional education level: auto (default), primary (Cấp 1), middle (Cấp 2), high (Cấp 3), university (Đại học)",
                    ),
                ),
                (
                    "source",
                    param(
                        "source",
                        "string",
                        "Visual input source: 'text' (default for concept lessons), 'screen' (capture current screen — auto-selected when art_mode=true or referring to screen), or 'clipboard' (Win+Shift+S image)",
                    ),
                ),
                (
                    "art_mode",
                    param(
                        "art_mode",
                        "boolean",
                        "Set true when teaching art / analyzing a painting or drawing on screen/clipboard (activates Rule-of-Thirds grid, Value Study B&W map, real HEX palette extractor, and animated Live Sketch Replay)",
                    ),
                ),
                (
                    "language",
                    param(
                        "language",
                        "string",
                        "Teaching language (e.g. 'Vietnamese', 'English'); defaults to the user's language",
                    ),
                ),
                (
                    "board_json",
                    param(
                        "board_json",
                        "string",
                        "ALWAYS PROVIDE THIS for source='text' lessons (renders INSTANTLY in <50ms with ZERO wait and ZERO REST quota!): \
                         Compact valid JSON string: \
                         {\"title\":\"...\",\"subject\":\"...\",\"level\":\"...\",\"summary\":\"...\",\
                         \"steps\":[{\"heading\":\"...\",\"body\":\"...\",\"formula\":\"\",\"highlight\":\"\"}],\
                         \"blocks\":[{\"type\":\"graph2d|formula_box|table|diagram|code\", ...}]}. \
                         Graph2d curve example: {\"type\":\"graph2d\",\"title\":\"...\",\"x_min\":-5,\"x_max\":5,\"y_min\":-5,\"y_max\":5,\
                         \"curves\":[{\"label\":\"y=x²-2x-3\",\"expr\":\"x^2-2*x-3\",\"color\":\"#38BDF8\"}],\
                         \"points\":[{\"x\":1,\"y\":-4,\"label\":\"I(1,-4)\"}]}. \
                         Omit board_json ONLY when art_mode=true or source='screen'/'clipboard'.",
                    ),
                ),
            ],
            vec![],
        ),
        Box::new(move |args| {
            let tx = event_tx.clone();
            Box::pin(study_board_handler(args, tx))
        }),
    );
}

fn build_board_system_prompt(subject: &str, level: &str, art_mode: bool, language: &str) -> String {
    let art_clause = if art_mode {
        "ART STUDY MODE IS ACTIVE: Analyze the attached painting/image carefully.\n\
         Fill `art_meta` with:\n\
         - `composition_type`: e.g. 'Rule of Thirds / Đường dẫn ánh sáng' or 'Golden Triangle'\n\
         - `focal_points`: 2 to 4 key composition points with normalized coordinates x (0.0..1.0 left-to-right), y (0.0..1.0 top-to-bottom), and short `label`\n\
         - `brush_steps`: 4 to 6 concrete painting/coloring steps (1. Phác nét hình khối tổng thể, 2. Lót mảng sáng tối / Underpainting, 3. Lên tông màu chính, 4. Tả chi tiết & ánh sáng nổi bật...)\n\
         Also populate `steps` explaining composition, lighting/values, color harmony, and brush technique clearly."
    } else {
        "Choose the BEST visual blocks for the topic:\n\
         - Math / Physics / Economics curves -> include a `graph2d` block! Provide clean math expressions like `x^2 - 2*x - 3`, `sin(x)`, `2*x + 1`, `sqrt(x)`, `1/x` inside `curves[].expr` (using `*` for multiplication, `^` for power, variable `x`), plus key `points` (vertex, roots, extrema) and optional `vectors`.\n\
         - Processes / History / Biology / Algorithms / Grammar structure -> include a `diagram` block (`kind`: `'flowchart'`, `'mindmap'`, or `'timeline'`, 4-7 concise nodes with `id`, `label`, `detail`, and `edges` connecting `from` -> `to`).\n\
         - Formulas / Laws / Rules -> include `formula_box` and/or `table` blocks.\n\
         - Programming / CS / SQL -> include a `code` block with `lines` and 1-based `highlight_lines`."
    };

    format!(
        "You are Boogie's Interactive Visual Tutor.\n\
         Output ONLY a single valid JSON object (no markdown fences, no text outside JSON).\n\
         Language: {language}. Subject: {subject}. Level: {level} (adapt strictly to Primary/Middle/High/University).\n\
         {art_clause}\n\n\
         REQUIRED JSON SCHEMA:\n\
         {{\n\
           \"title\": \"short crisp lesson title\",\n\
           \"subject\": \"subject display label\",\n\
           \"level\": \"level display label\",\n\
           \"summary\": \"1-2 sentence big-picture takeaway\",\n\
           \"steps\": [\n\
             {{ \"heading\": \"short step title\", \"body\": \"clear teaching explanation\", \"formula\": \"optional Unicode formula or null\", \"highlight\": \"optional key tip or null\" }}\n\
           ],\n\
           \"blocks\": [\n\
             {{ \"type\": \"graph2d\", \"title\": \"...\", \"x_min\": -5, \"x_max\": 5, \"y_min\": -5, \"y_max\": 5, \"x_label\": \"x\", \"y_label\": \"y\", \"curves\": [{{ \"label\": \"y = x² - 2x - 3\", \"expr\": \"x^2 - 2*x - 3\", \"color\": \"#38BDF8\", \"points\": [] }}], \"points\": [{{ \"x\": 1, \"y\": -4, \"label\": \"I(1, -4)\", \"color\": \"#F59E0B\" }}], \"vectors\": [] }},\n\
             {{ \"type\": \"formula_box\", \"label\": \"...\", \"content\": \"...\", \"note\": \"...\" }},\n\
             {{ \"type\": \"table\", \"title\": \"...\", \"headers\": [\"...\"], \"rows\": [[\"...\"]] }},\n\
             {{ \"type\": \"diagram\", \"title\": \"...\", \"kind\": \"flowchart|mindmap|timeline\", \"nodes\": [{{ \"id\": \"n1\", \"label\": \"...\", \"detail\": \"...\", \"color\": \"#60A5FA\" }}], \"edges\": [{{ \"from\": \"n1\", \"to\": \"n2\", \"label\": \"\" }}] }},\n\
             {{ \"type\": \"code\", \"title\": \"...\", \"language\": \"python\", \"lines\": [\"...\"], \"highlight_lines\": [2] }}\n\
           ],\n\
           \"art_meta\": {{ \"composition_type\": \"...\", \"focal_points\": [{{ \"x\": 0.33, \"y\": 0.33, \"label\": \"Main subject\" }}], \"brush_steps\": [\"...\"] }} or null\n\
         }}\n\
         Keep it CRISP & FAST: 3-4 short steps (1-2 concise sentences each) and 1-2 focused visual blocks (target under 2500 chars total so the board loads in ~2-4 seconds). \
         Use empty string \"\" instead of null for optional text fields. \
         Use clean readable Unicode math (x², √, π, ≤, ≥, ≠, →, Δ) — never raw LaTeX backslashes. \
         Double-check every math sign, vertex, root, and fact before emitting JSON."
    )
}

/// Parses Gemini's StudyBoard JSON leniently: strips explicit `null` values (which otherwise break
/// serde `String`/`Vec` fields) and isolates individual malformed visual blocks so one imperfect
/// block can NEVER throw away the rest of the lesson or display raw JSON text!
fn parse_raw_board_lenient(raw_json: &str) -> Result<RawBoardJson, String> {
    let payload = extract_json_payload(raw_json);
    let mut val: Value =
        serde_json::from_str(payload).map_err(|e| format!("json syntax error: {e}"))?;
    strip_json_nulls(&mut val);

    // Fast path: clean full-struct decode
    if let Ok(parsed) = serde_json::from_value::<RawBoardJson>(val.clone()) {
        return Ok(parsed);
    }

    // Resilient field-by-field recovery (keeps all valid steps & valid blocks)
    let obj = val
        .as_object()
        .ok_or_else(|| "top-level JSON is not an object".to_string())?;
    let str_field = |k: &str| {
        obj.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    let mut steps = Vec::new();
    if let Some(arr) = obj.get("steps").and_then(Value::as_array) {
        for item in arr {
            if let Ok(s) = serde_json::from_value::<StudyStep>(item.clone()) {
                steps.push(s);
            } else if let Some(text) = item.as_str() {
                steps.push(StudyStep {
                    heading: String::new(),
                    body: text.to_string(),
                    formula: None,
                    highlight: None,
                });
            }
        }
    }

    let mut blocks = Vec::new();
    if let Some(arr) = obj.get("blocks").and_then(Value::as_array) {
        for item in arr {
            if let Ok(b) = serde_json::from_value::<VisualBlock>(item.clone()) {
                blocks.push(b);
            } else {
                tracing::warn!("skipping single malformed visual block in StudyBoard JSON");
            }
        }
    }

    let art_meta = obj
        .get("art_meta")
        .and_then(|v| serde_json::from_value::<RawArtMeta>(v.clone()).ok());

    if steps.is_empty() && blocks.is_empty() && str_field("summary").is_empty() {
        return Err("parsed JSON contained no usable lesson fields".to_string());
    }

    Ok(RawBoardJson {
        title: str_field("title"),
        subject: str_field("subject"),
        level: str_field("level"),
        summary: str_field("summary"),
        steps,
        blocks,
        art_meta,
    })
}

fn extract_direct_json_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(_) => Some(v.to_string()),
        _ => None,
    })
}

async fn study_board_handler(args: Value, event_tx: broadcast::Sender<AppEvent>) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("open")
        .trim()
        .to_lowercase();

    match action.as_str() {
        "close" | "hide" | "exit" => {
            let _ = event_tx.send(AppEvent::CloseStudyVisual);
            return ToolResult::ok("Closed the Study Visual teaching board.");
        }
        "next" | "next_step" => {
            let _ = event_tx.send(AppEvent::StudyVisualStep("next".to_string()));
            return ToolResult::ok("Revealed the next step on the Study Visual board.");
        }
        "prev" | "prev_step" | "back" => {
            let _ = event_tx.send(AppEvent::StudyVisualStep("prev".to_string()));
            return ToolResult::ok("Moved back one step on the Study Visual board.");
        }
        "show_all" | "all" => {
            let _ = event_tx.send(AppEvent::StudyVisualStep("all".to_string()));
            return ToolResult::ok("Revealed all steps on the Study Visual board.");
        }
        _ => {}
    }

    let cfg_opt = ApiConfig::load().ok();
    if cfg_opt.as_ref().is_some_and(|c| !c.study_visual_enabled()) {
        return ToolResult::err(
            "Study Visual mode is currently disabled in Settings. Tell the user they can toggle 'Study Visual (Interactive Teaching Board)' ON in Settings anytime, and explain the lesson verbally or via study_solve instead.",
        );
    }

    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if topic.is_empty() {
        return ToolResult::err("topic is required when opening study_board");
    }

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
    let art_mode = args
        .get("art_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| subject == "art" || subject == "painting" || subject == "drawing");

    let default_source = if art_mode { "screen" } else { "text" };
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
        .map(str::to_lowercase)
        .unwrap_or_else(|| default_source.to_string());

    let language = args
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| crate::config::detect_language(&crate::config::load_memory()));

    // INSTANT DIRECT PATH (<50ms, ZERO extra REST API call, ZERO 20/day Free-Tier quota!):
    // When Gemini Live supplies compact `board_json` directly for a text lesson, render immediately!
    if !art_mode
        && source != "screen"
        && source != "clipboard"
        && let Some(raw_direct) = extract_direct_json_arg(&args, "board_json")
    {
        match parse_raw_board_lenient(&raw_direct) {
            Ok(parsed) => {
                let board = StudyBoardData {
                    title: if parsed.title.trim().is_empty() {
                        topic.clone()
                    } else {
                        parsed.title
                    },
                    subject: if parsed.subject.trim().is_empty() {
                        subject
                    } else {
                        parsed.subject
                    },
                    level: parsed.level,
                    summary: parsed.summary,
                    model_used: "gemini-live-direct".to_string(),
                    steps: parsed.steps,
                    blocks: parsed.blocks,
                    art_study: None,
                };
                let step_count = board.steps.len();
                let voice_summary = build_voice_brief(&board);
                let _ = event_tx.send(AppEvent::StudyVisual(Box::new(board)));
                tracing::info!(
                    "study_board rendered instantly via direct board_json ({step_count} steps)"
                );
                return ToolResult::ok(format!(
                    "[STUDY VISUAL BOARD OPENED INSTANTLY ({step_count} steps) — guide the user aloud warmly while referring to what is now visible on their screen. \
                     STICKY TEACHING MODE ACTIVE: whenever the user switches to a new question/topic, call study_board(action='open', topic=<new topic>, board_json=<compact JSON>) again immediately so the board refreshes live!]\n\n{voice_summary}"
                ));
            }
            Err(e) => {
                tracing::warn!(
                    "study_board direct board_json parse failed ({e}); falling back to Study Brain REST"
                );
            }
        }
    }

    let cfg = match cfg_opt {
        Some(c) => c,
        None => match ApiConfig::load() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Cannot load Gemini API config: {e}")),
        },
    };

    let progress_guard = TaskProgressGuard::new(
        event_tx.clone(),
        if art_mode {
            "Study Visual · Art Study"
        } else {
            "Study Visual · Teaching Board"
        },
        "Bước 1/3 · Đang chuẩn bị dữ liệu & nguồn hình ảnh...",
        0.18,
        crate::util::truncate_chars(&topic, 54),
    );

    let captured: Option<CapturedImage> = match source.as_str() {
        "clipboard" => {
            let tx_cap = event_tx.clone();
            match tokio::task::spawn_blocking(move || capture_clipboard_png(Some(&tx_cap))).await {
                Ok(Ok(img)) => Some(img),
                Ok(Err(e)) => return ToolResult::err(e),
                Err(e) => return ToolResult::err(format!("clipboard capture failed: {e}")),
            }
        }
        "screen" => {
            let tx_cap = event_tx.clone();
            match tokio::task::spawn_blocking(move || capture_screen_png(None, Some(&tx_cap))).await
            {
                Ok(Ok(img)) => Some(img),
                Ok(Err(e)) => return ToolResult::err(e),
                Err(e) => return ToolResult::err(format!("screen capture failed: {e}")),
            }
        }
        _ => None,
    };

    progress_guard.update(
        if captured.is_some() {
            "📸 Đã chụp xong! Có thể Alt+Tab — đang dựng bảng Study Visual..."
        } else {
            "Bước 2/3 · Đang dựng đồ thị, sơ đồ & từng bước giảng..."
        },
        0.42,
    );
    let _ = event_tx.send(AppEvent::Log(format!(
        "Preparing Study Visual board ({topic})..."
    )));

    let system_prompt = build_board_system_prompt(&subject, &level, art_mode, &language);
    let user_prompt = if let Some(ref img) = captured {
        format!(
            "Attached image source: {}.\nTeaching request: {}\nBuild a clear, accurate visual teaching board.",
            img.source_desc, topic
        )
    } else {
        format!(
            "Teaching request: {}\nBuild a clear, accurate visual teaching board with step-by-step guidance and appropriate graphs/diagrams/tables.",
            topic
        )
    };

    let (raw_json, used_model) = match call_study_brain(
        &cfg,
        &system_prompt,
        &user_prompt,
        captured.as_ref(),
        true,
        10_240,
        Some(0),
        true,
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            // ZERO-QUOTA RESILIENCE: Never tell the user the board is broken just because
            // secondary REST hit Google's 20/day Free Tier limit!
            if art_mode && let Some(img) = captured {
                tracing::warn!(
                    "study_board Art Study REST unavailable ({e}); rendering local Art Studio + Live Vision!"
                );
                progress_guard.update(
                    "🎨 Đang phân tích ảnh Art Study cục bộ (Value / Sketch / Màu HEX)...",
                    0.85,
                );
                let jpeg_opt = captured_to_jpeg(&img).ok();
                let img_bytes = img.bytes;
                let art_study = tokio::task::spawn_blocking(move || {
                    build_art_study_data(&img_bytes, RawArtMeta::default())
                })
                .await
                .ok()
                .and_then(Result::ok);

                if let Some(art) = art_study {
                    let hexes: Vec<String> =
                        art.palette.iter().take(5).map(|c| c.hex.clone()).collect();
                    let board = StudyBoardData {
                        title: topic.clone(),
                        subject: if subject == "general" { "Art Study".to_string() } else { subject },
                        level,
                        summary: format!(
                            "Phân tích trực quan cục bộ (Rule of Thirds, Value Study, Sketch Replay) · Bảng màu chính: {}",
                            hexes.join(", ")
                        ),
                        model_used: "local-art-engine + live".to_string(),
                        steps: vec![
                            StudyStep {
                                heading: "1. Bố cục & Đường dẫn (Rule of Thirds)".to_string(),
                                body: "Quan sát lưới 1/3 và đường chéo trên tranh để xác định điểm nhấn chính.".to_string(),
                                formula: None,
                                highlight: Some("Bật/tắt lưới Rule of Thirds ở thanh điều khiển phía trên tranh.".to_string()),
                            },
                            StudyStep {
                                heading: "2. Sắc độ sáng - tối (Value Study)".to_string(),
                                body: "Chuyển sang tab Value Study (đen trắng) để kiểm tra độ tương phản giữa vùng sáng mạnh và bóng đổ.".to_string(),
                                formula: None,
                                highlight: None,
                            },
                            StudyStep {
                                heading: "3. Bảng màu & Phác nét (Sketch Replay)".to_string(),
                                body: format!("Các mã màu chủ đạo đã tách trực tiếp từ ảnh: {}. Nhấn Play Sketch Replay để xem các nét phác chính.", hexes.join(", ")),
                                formula: None,
                                highlight: None,
                            },
                        ],
                        blocks: Vec::new(),
                        art_study: Some(art),
                    };
                    let _ = event_tx.send(AppEvent::StudyVisual(Box::new(board)));
                    let msg = format!(
                        "[ART STUDY VISUAL BOARD OPENED LOCALLY with Rule-of-Thirds grid, Value Study, Live Sketch Replay, and extracted HEX palette ({}) — explain the painting's composition, lighting/values, color harmony, and brush steps warmly in {language} using the attached screenshot!]",
                        hexes.join(", ")
                    );
                    if let Some(jpeg) = jpeg_opt {
                        return ToolResult::ok_with_image(msg, jpeg);
                    }
                    return ToolResult::ok(msg);
                }
            } else if let Some(ref img) = captured
                && let Ok(jpeg) = captured_to_jpeg(img)
            {
                tracing::warn!(
                    "study_board screen REST unavailable ({e}); handing screenshot to Live to populate board_json"
                );
                return ToolResult::ok_with_image(
                    format!(
                        "[ACTION REQUIRED — DIRECT BOARD MODE: Secondary REST quota is full. Read the attached crisp screenshot ({}), then IMMEDIATELY call study_board(action=\"open\", topic={topic:?}, source=\"text\", board_json=<compact JSON with title, subject, summary, steps, blocks>) so the Study Visual board renders in <50ms without using REST quota! Do NOT tell the user the board is broken.]",
                        img.source_desc
                    ),
                    jpeg,
                );
            } else if e.contains(REST_QUOTA_EXHAUSTED_TAG) {
                tracing::warn!(
                    "study_board REST quota exhausted; asking Gemini Live to supply direct board_json"
                );
                return ToolResult::ok(format!(
                    "[ACTION REQUIRED — DIRECT BOARD MODE: Secondary REST quota is full for today, so render the board directly right now! IMMEDIATELY call study_board(action=\"open\", topic={topic:?}, subject={subject:?}, source=\"text\", board_json=<compact JSON string with {{\"title\",\"subject\",\"level\",\"summary\",\"steps\":[{{\"heading\",\"body\",\"formula\",\"highlight\"}}],\"blocks\":[...]}}>) in this next step — it renders in <50ms with ZERO REST quota! Do NOT say the board is broken.]"
                ));
            }
            return ToolResult::err(e);
        }
    };
    progress_guard.update(
        if art_mode {
            "Bước 3/3 · Đang phân tích ảnh Art Study (Value / Sketch / Màu HEX)..."
        } else {
            "Bước 3/3 · Đang hiển thị bảng Study Visual..."
        },
        0.86,
    );

    let parsed: RawBoardJson = match parse_raw_board_lenient(&raw_json) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("StudyBoard JSON parse failed ({e}); building graceful fallback board");
            RawBoardJson {
                title: topic.clone(),
                subject: subject.clone(),
                level: level.clone(),
                summary: "Không thể dựng đồ họa đầy đủ lần này — bạn hãy hỏi lại hoặc chuyển sang câu tiếp theo nhé.".to_string(),
                steps: vec![StudyStep {
                    heading: "Nội dung bài giảng".to_string(),
                    body: raw_json.trim().to_string(),
                    formula: None,
                    highlight: None,
                }],
                blocks: Vec::new(),
                art_meta: None,
            }
        }
    };

    let art_study = if let Some(img) = captured
        && (art_mode || parsed.art_meta.is_some())
    {
        let meta = parsed.art_meta.unwrap_or_default();
        let img_bytes = img.bytes;
        match tokio::task::spawn_blocking(move || build_art_study_data(&img_bytes, meta)).await {
            Ok(Ok(data)) => Some(data),
            Ok(Err(e)) => {
                tracing::warn!("Art Study image processing failed: {e}");
                None
            }
            Err(e) => {
                tracing::warn!("Art Study task panicked: {e}");
                None
            }
        }
    } else {
        None
    };

    let board = StudyBoardData {
        title: if parsed.title.trim().is_empty() {
            topic.clone()
        } else {
            parsed.title
        },
        subject: if parsed.subject.trim().is_empty() {
            subject
        } else {
            parsed.subject
        },
        level: parsed.level,
        summary: parsed.summary,
        model_used: used_model.to_string(),
        steps: parsed.steps,
        blocks: parsed.blocks,
        art_study,
    };

    let step_count = board.steps.len();
    let voice_summary = build_voice_brief(&board);
    let _ = event_tx.send(AppEvent::StudyVisual(Box::new(board)));

    ToolResult::ok(format!(
        "[STUDY VISUAL BOARD OPENED via {used_model} ({step_count} steps) — guide the user aloud warmly while referring to what is now visible on their screen. \
         STICKY TEACHING MODE ACTIVE: whenever the user asks a new question, switches to another topic/exercise, or asks to explain a new concept during this study session (even without saying 'mở bảng'), \
         CALL study_board(action='open', topic=<new topic>) again immediately so the visual board updates to the new lesson!]\n\n{voice_summary}"
    ))
}

fn build_voice_brief(board: &StudyBoardData) -> String {
    let mut out = format!("Title: {}\nSummary: {}\n", board.title, board.summary);
    for (i, s) in board.steps.iter().take(6).enumerate() {
        out.push_str(&format!("Step {}: {} — {}\n", i + 1, s.heading, s.body));
    }
    if let Some(art) = &board.art_study {
        let hexes: Vec<&str> = art.palette.iter().take(5).map(|c| c.hex.as_str()).collect();
        out.push_str(&format!(
            "Art Study active: composition='{}', extracted palette=[{}], Live Sketch Replay & Value Study ready.\n",
            art.composition_type,
            hexes.join(", ")
        ));
    }
    out
}

// ============================================================================
// Pure-Rust Art Study Image Processor (Value Study, Sobel Sketch, HEX Palette)
// ============================================================================

fn build_art_study_data(png_bytes: &[u8], meta: RawArtMeta) -> Result<ArtStudyData, String> {
    let dyn_img = image::load_from_memory(png_bytes)
        .map_err(|e| format!("decode captured art image: {e}"))?;
    let (w, h) = (dyn_img.width(), dyn_img.height());
    let resized = if w > ART_MAX_DIM || h > ART_MAX_DIM {
        let scale = (ART_MAX_DIM as f32 / w.max(h) as f32).min(1.0);
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        dyn_img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
    } else {
        dyn_img
    };

    let rgba_img = resized.to_rgba8();
    let (rw, rh) = rgba_img.dimensions();
    let orig_rgba = rgba_img.into_raw();

    let (value_rgba, edge_strength) = compute_value_and_sobel(&orig_rgba, rw, rh);
    let palette = extract_dominant_palette(&orig_rgba, 7);

    Ok(ArtStudyData {
        width: rw,
        height: rh,
        orig_rgba,
        value_rgba,
        edge_strength,
        palette,
        composition_type: if meta.composition_type.trim().is_empty() {
            "Rule of Thirds & Light-Value Study".to_string()
        } else {
            meta.composition_type
        },
        focal_points: meta.focal_points,
        brush_steps: meta.brush_steps,
    })
}

/// Computes grayscale Value Study RGBA + Sobel gradient strength map [0..255] per pixel.
pub fn compute_value_and_sobel(orig_rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
    let pixels = (w as usize) * (h as usize);
    let mut luma = vec![0u8; pixels];
    let mut value_rgba = vec![0u8; pixels * 4];

    for i in 0..pixels {
        let r = orig_rgba[i * 4] as f32;
        let g = orig_rgba[i * 4 + 1] as f32;
        let b = orig_rgba[i * 4 + 2] as f32;
        let y = (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 255.0) as u8;
        luma[i] = y;
        value_rgba[i * 4] = y;
        value_rgba[i * 4 + 1] = y;
        value_rgba[i * 4 + 2] = y;
        value_rgba[i * 4 + 3] = 255;
    }

    let mut edge = vec![0u8; pixels];
    if w >= 3 && h >= 3 {
        let wu = w as usize;
        let hu = h as usize;
        for y in 1..(hu - 1) {
            for x in 1..(wu - 1) {
                let tl = luma[(y - 1) * wu + (x - 1)] as i32;
                let tc = luma[(y - 1) * wu + x] as i32;
                let tr = luma[(y - 1) * wu + (x + 1)] as i32;
                let ml = luma[y * wu + (x - 1)] as i32;
                let mr = luma[y * wu + (x + 1)] as i32;
                let bl = luma[(y + 1) * wu + (x - 1)] as i32;
                let bc = luma[(y + 1) * wu + x] as i32;
                let br = luma[(y + 1) * wu + (x + 1)] as i32;

                let gx = -tl + tr - 2 * ml + 2 * mr - bl + br;
                let gy = -tl - 2 * tc - tr + bl + 2 * bc + br;
                let mag = ((gx * gx + gy * gy) as f32).sqrt() * 0.62;
                edge[y * wu + x] = mag.clamp(0.0, 255.0) as u8;
            }
        }
    }

    (value_rgba, edge)
}

/// Extracts top N visually distinct dominant RGB/HEX color swatches directly from image pixels.
pub fn extract_dominant_palette(orig_rgba: &[u8], max_colors: usize) -> Vec<ColorSwatch> {
    let mut buckets: std::collections::HashMap<u16, (u64, u64, u64, u64)> =
        std::collections::HashMap::new();
    let total_pixels = orig_rgba.len() / 4;
    if total_pixels == 0 {
        return Vec::new();
    }
    let stride = (total_pixels / 40_000).max(1);
    let mut sampled: u64 = 0;

    let mut i = 0usize;
    while i < total_pixels {
        let base = i * 4;
        let r = orig_rgba[base];
        let g = orig_rgba[base + 1];
        let b = orig_rgba[base + 2];
        let a = orig_rgba[base + 3];
        if a >= 128 {
            let key = (((r >> 3) as u16) << 10) | (((g >> 3) as u16) << 5) | ((b >> 3) as u16);
            let e = buckets.entry(key).or_insert((0, 0, 0, 0));
            e.0 += 1;
            e.1 += r as u64;
            e.2 += g as u64;
            e.3 += b as u64;
            sampled += 1;
        }
        i += stride;
    }

    if sampled == 0 {
        return Vec::new();
    }

    let mut ranked: Vec<(u64, [u8; 3])> = buckets
        .into_values()
        .map(|(cnt, rs, gs, bs)| (cnt, [(rs / cnt) as u8, (gs / cnt) as u8, (bs / cnt) as u8]))
        .collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.0));

    let mut chosen: Vec<(u64, [u8; 3])> = Vec::new();
    for (cnt, rgb) in ranked {
        let too_close = chosen.iter().any(|(_, prev)| {
            let dr = rgb[0] as i32 - prev[0] as i32;
            let dg = rgb[1] as i32 - prev[1] as i32;
            let db = rgb[2] as i32 - prev[2] as i32;
            (dr * dr + dg * dg + db * db) < 42 * 42
        });
        if !too_close {
            chosen.push((cnt, rgb));
            if chosen.len() >= max_colors {
                break;
            }
        }
    }

    chosen
        .into_iter()
        .enumerate()
        .map(|(idx, (cnt, rgb))| {
            let pct = (cnt as f32 / sampled as f32) * 100.0;
            let luma = 0.2126 * rgb[0] as f32 + 0.7152 * rgb[1] as f32 + 0.0722 * rgb[2] as f32;
            let role = if idx == 0 {
                "Dominant"
            } else if luma < 65.0 {
                "Shadow"
            } else if luma > 205.0 {
                "Highlight"
            } else {
                "Midtone"
            };
            ColorSwatch {
                hex: format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]),
                rgb,
                percent: pct,
                role: role.to_string(),
            }
        })
        .collect()
}

/// Generates an animated Live Sketch Replay RGBA frame for `progress` in `[0.0..1.0]`.
pub fn render_sketch_replay_frame(art: &ArtStudyData, progress: f32) -> Vec<u8> {
    let p = progress.clamp(0.0, 1.0);
    let w = art.width as usize;
    let h = art.height as usize;
    let pixels = w * h;
    let mut out = vec![0u8; pixels * 4];

    let paper = [245.0_f32, 241.0, 232.0];
    let ink = [26.0_f32, 28.0, 42.0];

    let line_phase = (p / 0.74).clamp(0.0, 1.0);
    let color_glaze = ((p - 0.72) / 0.28).clamp(0.0, 1.0);
    let threshold = 185.0 * (1.0 - line_phase) + 18.0;

    for y in 0..h {
        let ny = y as f32 / (h.max(1) as f32);
        let sweep_gate = (line_phase * 1.35 - ny * 0.35).clamp(0.0, 1.0);

        for x in 0..w {
            let idx = y * w + x;
            let e = art.edge_strength.get(idx).copied().unwrap_or(0) as f32;
            let stroke_strength = if sweep_gate <= 0.01 {
                0.0
            } else {
                let over = ((e - threshold) / 48.0).clamp(0.0, 1.0);
                (over * sweep_gate * 1.15).clamp(0.0, 1.0)
            };

            let mut r = paper[0] + (ink[0] - paper[0]) * stroke_strength;
            let mut g = paper[1] + (ink[1] - paper[1]) * stroke_strength;
            let mut b = paper[2] + (ink[2] - paper[2]) * stroke_strength;

            if color_glaze > 0.001 && idx * 4 + 2 < art.orig_rgba.len() {
                let or = art.orig_rgba[idx * 4] as f32;
                let og = art.orig_rgba[idx * 4 + 1] as f32;
                let ob = art.orig_rgba[idx * 4 + 2] as f32;
                r += (or - r) * color_glaze;
                g += (og - g) * color_glaze;
                b += (ob - b) * color_glaze;
            }

            let base = idx * 4;
            out[base] = r.clamp(0.0, 255.0) as u8;
            out[base + 1] = g.clamp(0.0, 255.0) as u8;
            out[base + 2] = b.clamp(0.0, 255.0) as u8;
            out[base + 3] = 255;
        }
    }

    out
}

// ============================================================================
// Lightweight Safe Pure-Rust Math Expression Evaluator (for smooth 2D graphs)
// ============================================================================

pub fn eval_math_expr(expr: &str, x_val: f64) -> Option<f64> {
    let cleaned = expr
        .trim()
        .replace('−', "-")
        .replace(['×', '·'], "*")
        .replace('÷', "/")
        .replace("²", "^2")
        .replace("³", "^3")
        .replace("π", "pi");
    let rhs = cleaned
        .split_once('=')
        .map(|(_, r)| r.trim())
        .unwrap_or(cleaned.as_str());
    let mut parser = ExprParser::new(rhs, x_val);
    let val = parser.parse_add_sub()?;
    parser.skip_ws();
    if parser.pos < parser.bytes.len() || !val.is_finite() {
        None
    } else {
        Some(val)
    }
}

struct ExprParser<'a> {
    bytes: &'a [u8],
    pos: usize,
    x: f64,
}

impl<'a> ExprParser<'a> {
    fn new(s: &'a str, x: f64) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
            x,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }

    fn parse_add_sub(&mut self) -> Option<f64> {
        let mut lhs = self.parse_mul_div()?;
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    lhs += self.parse_mul_div()?;
                }
                Some(b'-') => {
                    self.pos += 1;
                    lhs -= self.parse_mul_div()?;
                }
                _ => break,
            }
        }
        Some(lhs)
    }

    fn parse_mul_div(&mut self) -> Option<f64> {
        let mut lhs = self.parse_pow()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.pos += 1;
                    lhs *= self.parse_pow()?;
                }
                Some(b'/') => {
                    self.pos += 1;
                    let rhs = self.parse_pow()?;
                    if rhs.abs() < 1e-12 {
                        return None;
                    }
                    lhs /= rhs;
                }
                Some(b'(') | Some(b'x') | Some(b'X') => {
                    lhs *= self.parse_pow()?;
                }
                _ => break,
            }
        }
        Some(lhs)
    }

    fn parse_pow(&mut self) -> Option<f64> {
        let base = self.parse_unary()?;
        if self.peek() == Some(b'^') {
            self.pos += 1;
            let exp = self.parse_pow()?;
            Some(base.powf(exp))
        } else {
            Some(base)
        }
    }

    fn parse_unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(b'+') => {
                self.pos += 1;
                self.parse_unary()
            }
            Some(b'-') => {
                self.pos += 1;
                Some(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Option<f64> {
        match self.peek()? {
            b'(' => {
                self.pos += 1;
                let v = self.parse_add_sub()?;
                if self.peek() == Some(b')') {
                    self.pos += 1;
                    Some(v)
                } else {
                    None
                }
            }
            b'0'..=b'9' | b'.' => {
                let start = self.pos;
                while self.pos < self.bytes.len()
                    && (self.bytes[self.pos].is_ascii_digit() || self.bytes[self.pos] == b'.')
                {
                    self.pos += 1;
                }
                let s = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
                s.parse::<f64>().ok()
            }
            b'a'..=b'z' | b'A'..=b'Z' => {
                let start = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_alphabetic() {
                    self.pos += 1;
                }
                let ident = std::str::from_utf8(&self.bytes[start..self.pos])
                    .ok()?
                    .to_ascii_lowercase();
                if self.peek() == Some(b'(') {
                    self.pos += 1;
                    let arg = self.parse_add_sub()?;
                    if self.peek() != Some(b')') {
                        return None;
                    }
                    self.pos += 1;
                    match ident.as_str() {
                        "sin" => Some(arg.sin()),
                        "cos" => Some(arg.cos()),
                        "tan" => Some(arg.tan()),
                        "sqrt" => {
                            if arg >= 0.0 {
                                Some(arg.sqrt())
                            } else {
                                None
                            }
                        }
                        "abs" => Some(arg.abs()),
                        "ln" => {
                            if arg > 0.0 {
                                Some(arg.ln())
                            } else {
                                None
                            }
                        }
                        "log" | "log10" => {
                            if arg > 0.0 {
                                Some(arg.log10())
                            } else {
                                None
                            }
                        }
                        "exp" => Some(arg.exp()),
                        _ => None,
                    }
                } else {
                    match ident.as_str() {
                        "x" | "t" => Some(self.x),
                        "pi" => Some(std::f64::consts::PI),
                        "e" => Some(std::f64::consts::E),
                        _ => None,
                    }
                }
            }
            _ => None,
        }
    }
}

// ============================================================================
// Interactive `egui` Study Visual Teaching Panel
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtViewMode {
    OriginalGrid,
    ValueStudy,
    SketchReplay,
}

pub struct StudyBoardState {
    pub data: StudyBoardData,
    board_id: u64,
    visible_steps: usize,
    art_mode: ArtViewMode,
    show_thirds: bool,
    show_diagonals: bool,
    show_focals: bool,
    replay_progress: f32,
    replay_playing: bool,
    replay_speed: f32,
    tex_orig: Option<TextureHandle>,
    tex_value: Option<TextureHandle>,
    tex_sketch: Option<TextureHandle>,
    last_sketch_bucket: i32,
    export_status: Arc<StdMutex<Option<String>>>,
    copied_toast: Option<(String, f64)>,
}

impl StudyBoardState {
    pub fn new(data: StudyBoardData) -> Self {
        let init_steps = if data.steps.len() > 1 {
            1
        } else {
            data.steps.len()
        };
        Self {
            data,
            board_id: NEXT_BOARD_ID.fetch_add(1, Ordering::Relaxed),
            visible_steps: init_steps,
            art_mode: ArtViewMode::OriginalGrid,
            show_thirds: true,
            show_diagonals: false,
            show_focals: true,
            replay_progress: 0.0,
            replay_playing: false,
            replay_speed: 0.22,
            tex_orig: None,
            tex_value: None,
            tex_sketch: None,
            last_sketch_bucket: -1,
            export_status: Arc::new(StdMutex::new(None)),
            copied_toast: None,
        }
    }

    pub fn apply_step_command(&mut self, cmd: &str) {
        let total = self.data.steps.len();
        if total == 0 {
            return;
        }
        match cmd {
            "next" => self.visible_steps = (self.visible_steps + 1).min(total),
            "prev" => self.visible_steps = self.visible_steps.saturating_sub(1).max(1),
            "all" => self.visible_steps = total,
            _ => {}
        }
    }

    fn trigger_export(&self, format: DocFormat) {
        let spec = self.data.to_document_spec();
        let status_slot = Arc::clone(&self.export_status);
        if let Ok(mut g) = status_slot.lock() {
            *g = Some(format!("Choose where to save {}...", format.display_name()));
        }
        std::thread::spawn(move || {
            let msg = match save_spec_with_dialog(&spec, format, true) {
                Ok(Some(path)) => format!("Saved: {}", path.display()),
                Ok(None) => "Export cancelled.".to_string(),
                Err(e) => format!("Export error: {e}"),
            };
            if let Ok(mut g) = status_slot.lock() {
                *g = Some(msg);
            }
        });
    }

    /// Renders the full frosted-glass interactive Study Visual teaching board inside `rect`.
    /// Returns `true` when the user clicks the Close `×` button.
    pub fn draw_panel(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        pal: &ThemePalette,
        anim: f32,
        now: f64,
        dt: f32,
    ) -> bool {
        if anim < 0.04 {
            return false;
        }
        let accent = pal.ui_secondary;
        let primary = pal.ui_primary;
        let painter = ui.painter();

        painter.rect_filled(
            rect,
            CornerRadius::same(18),
            Color32::from_rgba_unmultiplied(11, 11, 28, (anim * 234.0) as u8),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(18),
            Stroke::new(
                1.1_f32,
                Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    (anim * 92.0) as u8,
                ),
            ),
            egui::StrokeKind::Inside,
        );
        painter.line_segment(
            [
                Pos2::new(rect.left() + 28.0, rect.top() + 1.0),
                Pos2::new(rect.right() - 28.0, rect.top() + 1.0),
            ],
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    (anim * 110.0) as u8,
                ),
            ),
        );

        if anim < 0.35 {
            return false;
        }

        if self.art_mode == ArtViewMode::SketchReplay && self.replay_playing {
            self.replay_progress += dt * self.replay_speed;
            if self.replay_progress >= 1.0 {
                self.replay_progress = 1.0;
                self.replay_playing = false;
            }
        }

        let mut close_clicked = false;
        let inner = rect.shrink2(Vec2::new(16.0, 12.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));

        // --- Header Row ---
        child.horizontal(|ui| {
            ui.label(
                RichText::new("✦ STUDY VISUAL")
                    .size(10.5)
                    .color(accent)
                    .family(FontFamily::Monospace)
                    .strong(),
            );
            if !self.data.subject.trim().is_empty() {
                draw_pill(ui, &self.data.subject, primary);
            }
            if !self.data.level.trim().is_empty() && !self.data.level.eq_ignore_ascii_case("auto") {
                draw_pill(ui, &self.data.level, MINT);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("×").size(14.0).color(TEXT_SOFT))
                            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(26.0, 24.0)),
                    )
                    .on_hover_text("Close Study Visual")
                    .clicked()
                {
                    close_clicked = true;
                }
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::Button::new(RichText::new("Word").size(10.5).color(TEXT_BRIGHT))
                            .fill(Color32::from_rgba_unmultiplied(
                                primary.r(),
                                primary.g(),
                                primary.b(),
                                42,
                            ))
                            .stroke(Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(
                                    primary.r(),
                                    primary.g(),
                                    primary.b(),
                                    120,
                                ),
                            ))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(50.0, 24.0)),
                    )
                    .on_hover_text("Export Study Board as Word (.docx) — Save As dialog")
                    .clicked()
                {
                    self.trigger_export(DocFormat::Docx);
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("PDF").size(10.5).color(TEXT_BRIGHT))
                            .fill(Color32::from_rgba_unmultiplied(
                                accent.r(),
                                accent.g(),
                                accent.b(),
                                38,
                            ))
                            .stroke(Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    120,
                                ),
                            ))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(44.0, 24.0)),
                    )
                    .on_hover_text("Export Study Board as PDF — Save As dialog")
                    .clicked()
                {
                    self.trigger_export(DocFormat::Pdf);
                }
            });
        });

        child.add_space(4.0);
        child.label(
            RichText::new(&self.data.title)
                .size(16.5)
                .color(TEXT_BRIGHT)
                .strong(),
        );
        if !self.data.summary.trim().is_empty() {
            child.label(
                RichText::new(&self.data.summary)
                    .size(12.0)
                    .color(TEXT_SOFT),
            );
        }

        if let Some(status) = self.export_status.lock().ok().and_then(|g| g.clone()) {
            child.add_space(4.0);
            child.label(
                RichText::new(status)
                    .size(10.5)
                    .color(MINT)
                    .family(FontFamily::Monospace),
            );
        }

        child.add_space(6.0);
        child.separator();
        child.add_space(6.0);

        // --- Scrollable Visual Canvas + Step-by-Step Guide (resets scroll cleanly per board_id) ---
        let avail_h = child.available_height().max(120.0);
        egui::ScrollArea::vertical()
            .id_salt(("boogie_study_visual_scroll", self.board_id))
            .max_height(avail_h)
            .auto_shrink([false; 2])
            .show(&mut child, |ui| {
                ui.set_min_width(ui.available_width());

                if self.data.art_study.is_some() {
                    self.draw_art_study_section(ui, accent, now);
                    ui.add_space(12.0);
                }

                for (idx, block) in self.data.blocks.iter().enumerate() {
                    match block {
                        VisualBlock::Graph2d {
                            title,
                            x_min,
                            x_max,
                            y_min,
                            y_max,
                            x_label,
                            y_label,
                            curves,
                            points,
                            vectors,
                        } => {
                            draw_graph_2d(
                                ui, title, *x_min, *x_max, *y_min, *y_max, x_label, y_label,
                                curves, points, vectors, accent,
                            );
                        }
                        VisualBlock::FormulaBox {
                            label,
                            content,
                            note,
                        } => {
                            draw_formula_box(ui, label, content, note, accent);
                        }
                        VisualBlock::Table {
                            title,
                            headers,
                            rows,
                        } => {
                            draw_visual_table(
                                ui,
                                self.board_id,
                                idx,
                                title,
                                headers,
                                rows,
                                primary,
                            );
                        }
                        VisualBlock::Diagram {
                            title,
                            kind,
                            nodes,
                            edges,
                        } => {
                            draw_visual_diagram(ui, title, kind, nodes, edges, accent, primary);
                        }
                        VisualBlock::Code {
                            title,
                            language,
                            lines,
                            highlight_lines,
                        } => {
                            draw_code_block(ui, title, language, lines, highlight_lines, accent);
                        }
                    }
                    ui.add_space(10.0);
                }

                if !self.data.steps.is_empty() {
                    let total = self.data.steps.len();
                    let shown = self.visible_steps.clamp(1, total);

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("STEP-BY-STEP ({shown}/{total})"))
                                .size(10.5)
                                .color(accent)
                                .family(FontFamily::Monospace)
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if shown < total && ui.small_button("Show All").clicked() {
                                self.visible_steps = total;
                            }
                            if shown < total
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Next →")
                                                .size(11.0)
                                                .color(Color32::from_rgb(8, 10, 22))
                                                .strong(),
                                        )
                                        .fill(accent)
                                        .corner_radius(CornerRadius::same(8))
                                        .min_size(Vec2::new(64.0, 22.0)),
                                    )
                                    .clicked()
                            {
                                self.visible_steps = (self.visible_steps + 1).min(total);
                            }
                            if shown > 1 && ui.small_button("← Prev").clicked() {
                                self.visible_steps = self.visible_steps.saturating_sub(1).max(1);
                            }
                        });
                    });
                    ui.add_space(6.0);

                    for (idx, step) in self.data.steps.iter().take(shown).enumerate() {
                        let is_latest = idx + 1 == shown;
                        let border_a = if is_latest { 140 } else { 42 };
                        egui::Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(18, 20, 44, 195))
                            .stroke(Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    border_a,
                                ),
                            ))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(egui::Margin::symmetric(12, 9))
                            .show(ui, |ui| {
                                ui.set_width((ui.available_width() - 4.0).max(160.0));
                                let head = if step.heading.trim().is_empty() {
                                    format!("Step {}", idx + 1)
                                } else {
                                    format!("{}. {}", idx + 1, step.heading.trim())
                                };
                                ui.label(
                                    RichText::new(head)
                                        .size(13.0)
                                        .color(if is_latest { accent } else { TEXT_BRIGHT })
                                        .strong(),
                                );
                                if !step.body.trim().is_empty() {
                                    ui.add_space(3.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(step.body.trim())
                                                .size(12.5)
                                                .color(TEXT_BRIGHT),
                                        )
                                        .selectable(true)
                                        .wrap(),
                                    );
                                }
                                if let Some(f) = step
                                    .formula
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    ui.add_space(5.0);
                                    egui::Frame::new()
                                        .fill(Color32::from_rgba_unmultiplied(10, 14, 30, 220))
                                        .stroke(Stroke::new(
                                            1.0_f32,
                                            Color32::from_rgba_unmultiplied(
                                                MINT.r(),
                                                MINT.g(),
                                                MINT.b(),
                                                80,
                                            ),
                                        ))
                                        .corner_radius(CornerRadius::same(8))
                                        .inner_margin(egui::Margin::symmetric(10, 6))
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(f)
                                                        .size(12.5)
                                                        .color(MINT)
                                                        .family(FontFamily::Monospace),
                                                )
                                                .selectable(true)
                                                .wrap(),
                                            );
                                        });
                                }
                                if let Some(h) = step
                                    .highlight
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(format!("💡 {h}")).size(11.5).color(AMBER),
                                    );
                                }
                            });
                        ui.add_space(6.0);
                    }
                }
            });

        close_clicked
    }

    fn draw_art_study_section(&mut self, ui: &mut egui::Ui, accent: Color32, now: f64) {
        let Some(art) = self.data.art_study.as_ref() else {
            return;
        };
        let (w, h) = (art.width as usize, art.height as usize);
        if w == 0 || h == 0 || art.orig_rgba.len() != w * h * 4 {
            return;
        }

        if self.tex_orig.is_none() {
            let ci = ColorImage::from_rgba_unmultiplied([w, h], &art.orig_rgba);
            self.tex_orig = Some(ui.ctx().load_texture(
                format!("boogie_art_orig_{}", self.board_id),
                ci,
                TextureOptions::LINEAR,
            ));
        }
        if self.tex_value.is_none() && art.value_rgba.len() == w * h * 4 {
            let ci = ColorImage::from_rgba_unmultiplied([w, h], &art.value_rgba);
            self.tex_value = Some(ui.ctx().load_texture(
                format!("boogie_art_value_{}", self.board_id),
                ci,
                TextureOptions::LINEAR,
            ));
        }

        if self.art_mode == ArtViewMode::SketchReplay {
            let bucket = (self.replay_progress * 65.0).round() as i32;
            if bucket != self.last_sketch_bucket || self.tex_sketch.is_none() {
                let frame = render_sketch_replay_frame(art, self.replay_progress);
                let ci = ColorImage::from_rgba_unmultiplied([w, h], &frame);
                self.tex_sketch = Some(ui.ctx().load_texture(
                    format!("boogie_art_sketch_{}", self.board_id),
                    ci,
                    TextureOptions::LINEAR,
                ));
                self.last_sketch_bucket = bucket;
            }
        }

        let comp_label = art.composition_type.clone();
        let focal_points = art.focal_points.clone();
        let palette = art.palette.clone();
        let brush_steps = art.brush_steps.clone();
        let aspect = w as f32 / (h as f32).max(1.0);

        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(15, 16, 36, 210))
            .stroke(Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 85),
            ))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("🎨 ART STUDY STUDIO")
                            .size(10.5)
                            .color(accent)
                            .family(FontFamily::Monospace)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format!("· {comp_label}"))
                            .size(11.0)
                            .color(TEXT_SOFT),
                    );
                });
                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    for (mode, label) in [
                        (ArtViewMode::OriginalGrid, "Original + Guides"),
                        (ArtViewMode::ValueStudy, "Value Study (B&W)"),
                        (ArtViewMode::SketchReplay, "Live Sketch Replay"),
                    ] {
                        let active = self.art_mode == mode;
                        let fill = if active {
                            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 44)
                        } else {
                            Color32::from_rgba_unmultiplied(255, 255, 255, 10)
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(label)
                                        .size(11.0)
                                        .color(if active { TEXT_BRIGHT } else { TEXT_SOFT })
                                        .strong(),
                                )
                                .fill(fill)
                                .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.art_mode = mode;
                            if mode == ArtViewMode::SketchReplay && self.replay_progress >= 0.99 {
                                self.replay_progress = 0.0;
                                self.replay_playing = true;
                            } else if mode == ArtViewMode::SketchReplay
                                && self.replay_progress <= 0.01
                            {
                                self.replay_playing = true;
                            }
                        }
                    }
                });

                ui.add_space(6.0);

                if self.art_mode == ArtViewMode::SketchReplay {
                    ui.horizontal(|ui| {
                        let play_txt = if self.replay_playing { "Pause" } else { "Play" };
                        if ui.small_button(play_txt).clicked() {
                            if self.replay_progress >= 0.99 {
                                self.replay_progress = 0.0;
                            }
                            self.replay_playing = !self.replay_playing;
                        }
                        if ui.small_button("Replay").clicked() {
                            self.replay_progress = 0.0;
                            self.replay_playing = true;
                        }
                        ui.add(
                            egui::Slider::new(&mut self.replay_progress, 0.0..=1.0)
                                .text("Sketch → Color"),
                        );
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.show_thirds, "Rule of Thirds");
                        ui.checkbox(&mut self.show_diagonals, "Diagonals");
                        ui.checkbox(&mut self.show_focals, "Focal Points");
                    });
                }

                ui.add_space(6.0);

                let max_w = ui.available_width().clamp(220.0, 680.0);
                let draw_h = (max_w / aspect).clamp(160.0, 320.0);
                let draw_w = (draw_h * aspect).min(max_w);
                let (img_rect, _) =
                    ui.allocate_exact_size(Vec2::new(draw_w, draw_h), Sense::hover());

                let active_tex = match self.art_mode {
                    ArtViewMode::OriginalGrid => self.tex_orig.as_ref(),
                    ArtViewMode::ValueStudy => self.tex_value.as_ref().or(self.tex_orig.as_ref()),
                    ArtViewMode::SketchReplay => {
                        self.tex_sketch.as_ref().or(self.tex_orig.as_ref())
                    }
                };

                let painter = ui.painter();
                if let Some(tex) = active_tex {
                    painter.image(
                        tex.id(),
                        img_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                painter.rect_stroke(
                    img_rect,
                    CornerRadius::same(6),
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 45)),
                    egui::StrokeKind::Inside,
                );

                if self.art_mode != ArtViewMode::SketchReplay {
                    if self.show_thirds {
                        let grid_stroke = Stroke::new(
                            1.0_f32,
                            Color32::from_rgba_unmultiplied(255, 224, 130, 155),
                        );
                        for frac in [1.0 / 3.0, 2.0 / 3.0] {
                            let x = img_rect.left() + img_rect.width() * frac;
                            let y = img_rect.top() + img_rect.height() * frac;
                            painter.line_segment(
                                [
                                    Pos2::new(x, img_rect.top()),
                                    Pos2::new(x, img_rect.bottom()),
                                ],
                                grid_stroke,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(img_rect.left(), y),
                                    Pos2::new(img_rect.right(), y),
                                ],
                                grid_stroke,
                            );
                        }
                    }
                    if self.show_diagonals {
                        let diag_stroke = Stroke::new(
                            1.0_f32,
                            Color32::from_rgba_unmultiplied(110, 220, 255, 125),
                        );
                        painter.line_segment(
                            [img_rect.left_top(), img_rect.right_bottom()],
                            diag_stroke,
                        );
                        painter.line_segment(
                            [img_rect.right_top(), img_rect.left_bottom()],
                            diag_stroke,
                        );
                    }
                    if self.show_focals {
                        for fp in &focal_points {
                            let fx = img_rect.left() + img_rect.width() * fp.x.clamp(0.03, 0.97);
                            let fy = img_rect.top() + img_rect.height() * fp.y.clamp(0.03, 0.97);
                            let p = Pos2::new(fx, fy);
                            painter.circle_stroke(p, 9.0, Stroke::new(1.8_f32, AMBER));
                            painter.circle_filled(p, 3.0, CRIMSON);
                            if !fp.label.trim().is_empty() {
                                painter.text(
                                    p + Vec2::new(12.0, -4.0),
                                    Align2::LEFT_CENTER,
                                    fp.label.trim(),
                                    FontId::new(10.5, FontFamily::Proportional),
                                    Color32::WHITE,
                                );
                            }
                        }
                    }
                }

                if !palette.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("EXTRACTED PALETTE (click to copy HEX)")
                                .size(9.5)
                                .color(TEXT_MUTED)
                                .family(FontFamily::Monospace),
                        );
                        if let Some((ref hex, until)) = self.copied_toast
                            && now < until
                        {
                            ui.label(
                                RichText::new(format!("Copied {hex}!"))
                                    .size(10.0)
                                    .color(MINT),
                            );
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for sw in &palette {
                            let c = Color32::from_rgb(sw.rgb[0], sw.rgb[1], sw.rgb[2]);
                            let (r, resp) =
                                ui.allocate_exact_size(Vec2::new(78.0, 34.0), Sense::click());
                            let p = ui.painter();
                            p.rect_filled(
                                r,
                                CornerRadius::same(8),
                                Color32::from_rgba_unmultiplied(24, 24, 48, 220),
                            );
                            let sw_rect = Rect::from_min_size(
                                Pos2::new(r.left() + 5.0, r.top() + 5.0),
                                Vec2::new(16.0, 24.0),
                            );
                            p.rect_filled(sw_rect, CornerRadius::same(4), c);
                            p.text(
                                Pos2::new(sw_rect.right() + 6.0, r.top() + 11.0),
                                Align2::LEFT_CENTER,
                                &sw.hex,
                                FontId::new(9.5, FontFamily::Monospace),
                                TEXT_BRIGHT,
                            );
                            p.text(
                                Pos2::new(sw_rect.right() + 6.0, r.top() + 24.0),
                                Align2::LEFT_CENTER,
                                format!("{} {:.0}%", sw.role, sw.percent),
                                FontId::new(8.5, FontFamily::Proportional),
                                TEXT_MUTED,
                            );
                            if resp.on_hover_text("Click to copy HEX color").clicked() {
                                ui.ctx().copy_text(sw.hex.clone());
                                self.copied_toast = Some((sw.hex.clone(), now + 2.5));
                            }
                        }
                    });
                }

                if !brush_steps.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("PAINTING & COLORING WORKFLOW")
                            .size(9.5)
                            .color(accent)
                            .family(FontFamily::Monospace),
                    );
                    for (idx, bs) in brush_steps.iter().enumerate() {
                        ui.label(
                            RichText::new(format!("{}. {}", idx + 1, bs))
                                .size(11.5)
                                .color(TEXT_SOFT),
                        );
                    }
                }
            });
    }
}

// ============================================================================
// Visual Block Painters (2D Graphs, Formulas, Diagrams, Tables, Code)
// ============================================================================

fn parse_hex_color(raw: Option<&str>, fallback: Color32) -> Color32 {
    let Some(s) = raw.map(str::trim) else {
        return fallback;
    };
    let hex = s.trim_start_matches('#');
    if hex.len() == 6
        && let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
        )
    {
        return Color32::from_rgb(r, g, b);
    }
    fallback
}

fn draw_pill(ui: &mut egui::Ui, label: &str, col: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            col.r(),
            col.g(),
            col.b(),
            28,
        ))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 95),
        ))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(9.5).color(col).strong());
        });
}

#[allow(clippy::too_many_arguments)]
fn draw_graph_2d(
    ui: &mut egui::Ui,
    title: &str,
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
    x_label: &str,
    y_label: &str,
    curves: &[GraphCurve],
    points: &[GraphPoint],
    vectors: &[GraphVector],
    accent: Color32,
) {
    let x0 = x_min.min(x_max - 0.1);
    let x1 = x_max.max(x0 + 0.1);
    let y0 = y_min.min(y_max - 0.1);
    let y1 = y_max.max(y0 + 0.1);

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(12, 14, 32, 220))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 75),
        ))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            if !title.trim().is_empty() {
                ui.label(
                    RichText::new(format!("📈 {}", title.trim()))
                        .size(12.0)
                        .color(accent)
                        .strong(),
                );
                ui.add_space(4.0);
            }

            let plot_w = ui.available_width().clamp(240.0, 680.0);
            let plot_h = 230.0_f32;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(plot_w, plot_h), Sense::hover());
            let inner = rect.shrink2(Vec2::new(28.0, 18.0));
            let painter = ui.painter_at(rect);

            painter.rect_filled(rect, CornerRadius::same(10), Color32::from_rgb(8, 10, 24));

            let to_screen = |wx: f32, wy: f32| -> Pos2 {
                let tx = (wx - x0) / (x1 - x0);
                let ty = (wy - y0) / (y1 - y0);
                Pos2::new(
                    inner.left() + tx * inner.width(),
                    inner.bottom() - ty * inner.height(),
                )
            };

            let grid_col = Color32::from_rgba_unmultiplied(140, 150, 210, 28);
            let x_step = nice_grid_step(x1 - x0);
            let y_step = nice_grid_step(y1 - y0);

            let mut gx = (x0 / x_step).ceil() * x_step;
            while gx <= x1 + 1e-4 {
                let sx = to_screen(gx, y0).x;
                painter.line_segment(
                    [Pos2::new(sx, inner.top()), Pos2::new(sx, inner.bottom())],
                    Stroke::new(1.0_f32, grid_col),
                );
                if gx.abs() > 1e-3 {
                    painter.text(
                        Pos2::new(sx, inner.bottom() + 9.0),
                        Align2::CENTER_CENTER,
                        format_axis_num(gx),
                        FontId::new(8.5, FontFamily::Monospace),
                        TEXT_MUTED,
                    );
                }
                gx += x_step;
            }
            let mut gy = (y0 / y_step).ceil() * y_step;
            while gy <= y1 + 1e-4 {
                let sy = to_screen(x0, gy).y;
                painter.line_segment(
                    [Pos2::new(inner.left(), sy), Pos2::new(inner.right(), sy)],
                    Stroke::new(1.0_f32, grid_col),
                );
                if gy.abs() > 1e-3 {
                    painter.text(
                        Pos2::new(inner.left() - 14.0, sy),
                        Align2::CENTER_CENTER,
                        format_axis_num(gy),
                        FontId::new(8.5, FontFamily::Monospace),
                        TEXT_MUTED,
                    );
                }
                gy += y_step;
            }

            let axis_col = Color32::from_rgba_unmultiplied(210, 215, 255, 130);
            if y0 <= 0.0 && y1 >= 0.0 {
                let sy = to_screen(x0, 0.0).y;
                painter.line_segment(
                    [Pos2::new(inner.left(), sy), Pos2::new(inner.right(), sy)],
                    Stroke::new(1.4_f32, axis_col),
                );
                painter.text(
                    Pos2::new(inner.right() + 9.0, sy),
                    Align2::CENTER_CENTER,
                    x_label,
                    FontId::new(10.0, FontFamily::Monospace),
                    TEXT_SOFT,
                );
            }
            if x0 <= 0.0 && x1 >= 0.0 {
                let sx = to_screen(0.0, y0).x;
                painter.line_segment(
                    [Pos2::new(sx, inner.top()), Pos2::new(sx, inner.bottom())],
                    Stroke::new(1.4_f32, axis_col),
                );
                painter.text(
                    Pos2::new(sx, inner.top() - 9.0),
                    Align2::CENTER_CENTER,
                    y_label,
                    FontId::new(10.0, FontFamily::Monospace),
                    TEXT_SOFT,
                );
            }

            let palette_cycle = [
                accent,
                MINT,
                AMBER,
                Color32::from_rgb(244, 114, 182),
                Color32::from_rgb(167, 139, 250),
            ];
            let mut legend_items = Vec::new();
            let y_span = (y1 - y0).max(1.0);

            for (idx, curve) in curves.iter().enumerate() {
                let col = parse_hex_color(
                    curve.color.as_deref(),
                    palette_cycle[idx % palette_cycle.len()],
                );
                if !curve.label.trim().is_empty() {
                    legend_items.push((curve.label.clone(), col));
                }

                let mut segments: Vec<Vec<Pos2>> = Vec::new();
                let mut current: Vec<Pos2> = Vec::new();

                if let Some(expr) = curve.expr.as_deref().filter(|s| !s.trim().is_empty()) {
                    let samples = 180usize;
                    let mut prev_y: Option<f32> = None;
                    for i in 0..=samples {
                        let wx = x0 + (x1 - x0) * (i as f32 / samples as f32);
                        if let Some(wy64) = eval_math_expr(expr, wx as f64) {
                            let wy = wy64 as f32;
                            let out_of_bounds = wy < y0 - y_span * 1.5 || wy > y1 + y_span * 1.5;
                            let jump = prev_y.is_some_and(|py| (wy - py).abs() > y_span * 0.85);
                            if out_of_bounds || jump {
                                if current.len() >= 2 {
                                    segments.push(std::mem::take(&mut current));
                                } else {
                                    current.clear();
                                }
                                prev_y = None;
                            } else {
                                current.push(to_screen(wx, wy));
                                prev_y = Some(wy);
                            }
                        } else {
                            if current.len() >= 2 {
                                segments.push(std::mem::take(&mut current));
                            } else {
                                current.clear();
                            }
                            prev_y = None;
                        }
                    }
                } else {
                    for &[wx, wy] in &curve.points {
                        if wx.is_finite() && wy.is_finite() {
                            current.push(to_screen(wx, wy));
                        }
                    }
                }

                if current.len() >= 2 {
                    segments.push(current);
                }
                for seg in segments {
                    painter.add(egui::Shape::line(seg, Stroke::new(2.2_f32, col)));
                }
            }

            for v in vectors {
                let col = parse_hex_color(v.color.as_deref(), AMBER);
                let p0 = to_screen(v.x0, v.y0);
                let p1 = to_screen(v.x1, v.y1);
                draw_arrow(&painter, p0, p1, Stroke::new(2.0_f32, col));
                if !v.label.trim().is_empty() {
                    let mid = Pos2::new((p0.x + p1.x) * 0.5, (p0.y + p1.y) * 0.5 - 10.0);
                    painter.text(
                        mid,
                        Align2::CENTER_CENTER,
                        v.label.trim(),
                        FontId::new(10.0, FontFamily::Proportional),
                        col,
                    );
                }
            }

            for pt in points {
                let col = parse_hex_color(pt.color.as_deref(), AMBER);
                let sp = to_screen(pt.x, pt.y);
                painter.circle_filled(sp, 4.2, col);
                painter.circle_stroke(sp, 6.2, Stroke::new(1.0_f32, Color32::WHITE));
                if !pt.label.trim().is_empty() {
                    painter.text(
                        sp + Vec2::new(8.0, -8.0),
                        Align2::LEFT_CENTER,
                        pt.label.trim(),
                        FontId::new(10.0, FontFamily::Proportional),
                        TEXT_BRIGHT,
                    );
                }
            }

            if !legend_items.is_empty() {
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    for (lbl, col) in legend_items {
                        ui.label(RichText::new("━━").size(10.0).color(col).strong());
                        ui.label(RichText::new(lbl).size(10.5).color(TEXT_SOFT));
                        ui.add_space(8.0);
                    }
                });
            }
        });
}

fn nice_grid_step(span: f32) -> f32 {
    let raw = (span / 6.0).max(0.1);
    let mag = 10.0_f32.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = if norm < 1.5 {
        1.0
    } else if norm < 3.5 {
        2.0
    } else if norm < 7.5 {
        5.0
    } else {
        10.0
    };
    (step * mag).max(0.1)
}

fn format_axis_num(v: f32) -> String {
    if (v - v.round()).abs() < 0.01 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

fn draw_arrow(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    painter.line_segment([from, to], stroke);
    let dir = (to - from).normalized();
    if dir.length_sq() < 0.5 {
        return;
    }
    let perp = Vec2::new(-dir.y, dir.x);
    let tip_len = 8.0;
    let wing = 4.2;
    let a = to - dir * tip_len + perp * wing;
    let b = to - dir * tip_len - perp * wing;
    painter.line_segment([to, a], stroke);
    painter.line_segment([to, b], stroke);
}

fn draw_formula_box(ui: &mut egui::Ui, label: &str, content: &str, note: &str, accent: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            22,
        ))
        .stroke(Stroke::new(
            1.1_f32,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 110),
        ))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width((ui.available_width() - 4.0).max(160.0));
            if !label.trim().is_empty() {
                ui.label(
                    RichText::new(label.trim().to_uppercase())
                        .size(9.5)
                        .color(accent)
                        .family(FontFamily::Monospace)
                        .strong(),
                );
                ui.add_space(3.0);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(content.trim())
                        .size(14.0)
                        .color(TEXT_BRIGHT)
                        .family(FontFamily::Monospace)
                        .strong(),
                )
                .selectable(true)
                .wrap(),
            );
            if !note.trim().is_empty() {
                ui.add_space(3.0);
                ui.label(RichText::new(note.trim()).size(11.5).color(TEXT_SOFT));
            }
        });
}

fn draw_visual_table(
    ui: &mut egui::Ui,
    board_id: u64,
    idx: usize,
    title: &str,
    headers: &[String],
    rows: &[Vec<String>],
    primary: Color32,
) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(15, 16, 36, 205))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(primary.r(), primary.g(), primary.b(), 72),
        ))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            if !title.trim().is_empty() {
                ui.label(
                    RichText::new(title.trim())
                        .size(12.0)
                        .color(TEXT_BRIGHT)
                        .strong(),
                );
                ui.add_space(5.0);
            }
            egui::Grid::new(format!("study_visual_tbl_{board_id}_{idx}"))
                .striped(true)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    if !headers.is_empty() {
                        for h in headers {
                            ui.label(RichText::new(h.trim()).size(11.5).color(MINT).strong());
                        }
                        ui.end_row();
                    }
                    for row in rows {
                        for cell in row {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(cell.trim()).size(11.5).color(TEXT_BRIGHT),
                                )
                                .wrap(),
                            );
                        }
                        ui.end_row();
                    }
                });
        });
}

fn draw_visual_diagram(
    ui: &mut egui::Ui,
    title: &str,
    kind: &str,
    nodes: &[DiagramNode],
    edges: &[DiagramEdge],
    accent: Color32,
    primary: Color32,
) {
    if nodes.is_empty() {
        return;
    }

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 15, 34, 210))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 70),
        ))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            let heading = if title.trim().is_empty() {
                format!("Diagram ({kind})")
            } else {
                format!("{title} · {kind}")
            };
            ui.label(RichText::new(heading).size(11.5).color(accent).strong());
            ui.add_space(6.0);

            let width = ui.available_width().clamp(240.0, 680.0);
            let is_mindmap = kind.eq_ignore_ascii_case("mindmap") && nodes.len() >= 3;
            let height = if is_mindmap {
                235.0
            } else {
                (nodes.len() as f32 * 56.0 + 16.0).clamp(110.0, 340.0)
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let painter = ui.painter_at(rect);

            let mut node_rects: Vec<(String, Rect)> = Vec::with_capacity(nodes.len());

            if is_mindmap {
                let center = rect.center();
                let root_r = Rect::from_center_size(center, Vec2::new(136.0, 42.0));
                node_rects.push((nodes[0].id.clone(), root_r));
                let outer = nodes.len() - 1;
                let rx = (rect.width() * 0.34).max(95.0);
                let ry = (rect.height() * 0.36).max(66.0);
                for (i, n) in nodes.iter().skip(1).enumerate() {
                    let ang = std::f32::consts::TAU * (i as f32 / outer.max(1) as f32)
                        - std::f32::consts::FRAC_PI_2;
                    let pos = Pos2::new(center.x + ang.cos() * rx, center.y + ang.sin() * ry);
                    let nr = Rect::from_center_size(pos, Vec2::new(132.0, 38.0));
                    node_rects.push((n.id.clone(), nr));
                }
            } else {
                let n = nodes.len();
                let step_y = rect.height() / n.max(1) as f32;
                let box_w = (rect.width() * 0.78).clamp(180.0, 420.0);
                let box_h = (step_y - 14.0).clamp(32.0, 46.0);
                for (i, node) in nodes.iter().enumerate() {
                    let cy = rect.top() + (i as f32 + 0.5) * step_y;
                    let nr = Rect::from_center_size(
                        Pos2::new(rect.center().x, cy),
                        Vec2::new(box_w, box_h),
                    );
                    node_rects.push((node.id.clone(), nr));
                }
            }

            let edge_stroke = Stroke::new(
                1.4_f32,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 160),
            );
            if edges.is_empty() && !is_mindmap {
                for pair in node_rects.windows(2) {
                    let a = Pos2::new(pair[0].1.center().x, pair[0].1.bottom());
                    let b = Pos2::new(pair[1].1.center().x, pair[1].1.top());
                    draw_arrow(&painter, a, b, edge_stroke);
                }
            } else if edges.is_empty() && is_mindmap {
                let root_c = node_rects[0].1.center();
                for (_, r) in node_rects.iter().skip(1) {
                    painter.line_segment([root_c, r.center()], edge_stroke);
                }
            } else {
                for e in edges {
                    let from_r = node_rects
                        .iter()
                        .find(|(id, _)| id == &e.from)
                        .map(|(_, r)| *r);
                    let to_r = node_rects
                        .iter()
                        .find(|(id, _)| id == &e.to)
                        .map(|(_, r)| *r);
                    if let (Some(fr), Some(tr)) = (from_r, to_r) {
                        let (p0, p1) = if is_mindmap {
                            (fr.center(), tr.center())
                        } else {
                            (
                                Pos2::new(fr.center().x, fr.bottom()),
                                Pos2::new(tr.center().x, tr.top()),
                            )
                        };
                        draw_arrow(&painter, p0, p1, edge_stroke);
                        if !e.label.trim().is_empty() {
                            let mid = Pos2::new((p0.x + p1.x) * 0.5 + 8.0, (p0.y + p1.y) * 0.5);
                            painter.text(
                                mid,
                                Align2::LEFT_CENTER,
                                e.label.trim(),
                                FontId::new(9.5, FontFamily::Proportional),
                                AMBER,
                            );
                        }
                    }
                }
            }

            for (idx, (node, (_, nr))) in nodes.iter().zip(node_rects.iter()).enumerate() {
                let col = parse_hex_color(
                    node.color.as_deref(),
                    if idx == 0 { accent } else { primary },
                );
                painter.rect_filled(
                    *nr,
                    CornerRadius::same(10),
                    Color32::from_rgba_unmultiplied(19, 22, 46, 240),
                );
                painter.rect_stroke(
                    *nr,
                    CornerRadius::same(10),
                    Stroke::new(
                        1.2_f32,
                        Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 165),
                    ),
                    egui::StrokeKind::Inside,
                );
                let text = if node.detail.trim().is_empty() || is_mindmap {
                    crate::util::truncate_chars(node.label.trim(), 34).to_string()
                } else {
                    format!(
                        "{} — {}",
                        crate::util::truncate_chars(node.label.trim(), 24),
                        crate::util::truncate_chars(node.detail.trim(), 36)
                    )
                };
                painter.text(
                    nr.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::new(11.0, FontFamily::Proportional),
                    TEXT_BRIGHT,
                );
            }
        });
}

fn draw_code_block(
    ui: &mut egui::Ui,
    title: &str,
    language: &str,
    lines: &[String],
    highlight_lines: &[usize],
    accent: Color32,
) {
    egui::Frame::new()
        .fill(Color32::from_rgb(10, 11, 24))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 75),
        ))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width((ui.available_width() - 4.0).max(160.0));
            ui.horizontal(|ui| {
                let lbl = if title.trim().is_empty() {
                    format!("CODE · {}", language.to_uppercase())
                } else {
                    format!("{} · {}", title.trim(), language.to_uppercase())
                };
                ui.label(
                    RichText::new(lbl)
                        .size(10.0)
                        .color(accent)
                        .family(FontFamily::Monospace),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(lines.join("\n"));
                    }
                });
            });
            ui.add_space(4.0);
            for (idx, line) in lines.iter().enumerate() {
                let line_no = idx + 1;
                let hl = highlight_lines.contains(&line_no);
                let bg = if hl {
                    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 32)
                } else {
                    Color32::TRANSPARENT
                };
                egui::Frame::new().fill(bg).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{line_no:>2} "))
                                .size(11.5)
                                .color(if hl { accent } else { TEXT_MUTED })
                                .family(FontFamily::Monospace),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(line)
                                    .size(11.5)
                                    .color(if hl { MINT } else { TEXT_BRIGHT })
                                    .family(FontFamily::Monospace),
                            )
                            .selectable(true),
                        );
                    });
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_evaluator_handles_polynomials_trig_and_implicit_mul() {
        let v1 = eval_math_expr("y = x^2 - 2*x - 3", 1.0).unwrap();
        assert!((v1 - (-4.0)).abs() < 1e-6);

        let v2 = eval_math_expr("2x + 1", 3.0).unwrap();
        assert!((v2 - 7.0).abs() < 1e-6);

        let v3 = eval_math_expr("sin(pi / 2)", 0.0).unwrap();
        assert!((v3 - 1.0).abs() < 1e-6);

        assert!(eval_math_expr("1 / x", 0.0).is_none());
    }

    #[test]
    fn art_study_extracts_palette_sobel_edges_and_sketch_replay() {
        let mut rgba = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let idx = (y * 16 + x) * 4;
                if x < 8 {
                    rgba[idx] = 220;
                    rgba[idx + 1] = 30;
                    rgba[idx + 2] = 40;
                } else {
                    rgba[idx] = 30;
                    rgba[idx + 1] = 210;
                    rgba[idx + 2] = 235;
                }
                rgba[idx + 3] = 255;
            }
        }
        let (val, edge) = compute_value_and_sobel(&rgba, 16, 16);
        assert_eq!(val.len(), rgba.len());
        assert!(edge[8 * 16 + 7] > 40 || edge[8 * 16 + 8] > 40);

        let palette = extract_dominant_palette(&rgba, 4);
        assert_eq!(palette.len(), 2);

        let art = ArtStudyData {
            width: 16,
            height: 16,
            orig_rgba: rgba,
            value_rgba: val,
            edge_strength: edge,
            palette,
            ..Default::default()
        };
        let frame_mid = render_sketch_replay_frame(&art, 0.5);
        assert_eq!(frame_mid.len(), 16 * 16 * 4);
        let frame_end = render_sketch_replay_frame(&art, 1.0);
        assert_eq!(frame_end.len(), 16 * 16 * 4);
    }

    #[test]
    fn board_converts_cleanly_to_document_spec() {
        let board = StudyBoardData {
            title: "Khảo sát hàm bậc 2".to_string(),
            subject: "Toán học".to_string(),
            level: "Cấp 3".to_string(),
            summary: "Đồ thị Parabol và cách tìm đỉnh".to_string(),
            model_used: "gemini-2.5-flash".to_string(),
            steps: vec![StudyStep {
                heading: "Tìm tọa độ đỉnh".to_string(),
                body: "x = -b/(2a) = 1, y = -4".to_string(),
                formula: Some("I(1, -4)".to_string()),
                highlight: Some("a > 0 nên bề lõm quay lên".to_string()),
            }],
            blocks: vec![VisualBlock::FormulaBox {
                label: "Đỉnh Parabol".to_string(),
                content: "x_I = -b / (2a)".to_string(),
                note: "Áp dụng cho y = ax² + bx + c".to_string(),
            }],
            art_study: None,
        };
        let spec = board.to_document_spec();
        assert_eq!(spec.title, "Khảo sát hàm bậc 2");
        assert!(spec.sections.len() >= 3);
    }

    #[test]
    fn lenient_board_parser_handles_explicit_nulls_and_bad_block() {
        let raw = r#"{
            "title": "Vòng lặp While và For",
            "subject": "Tin học",
            "level": null,
            "summary": "So sánh nhanh While và For",
            "steps": [
                { "heading": "While là gì?", "body": "Lặp khi điều kiện đúng", "formula": null, "highlight": null }
            ],
            "blocks": [
                { "type": "code", "title": null, "language": "python", "lines": ["while i < n:", "    i += 1"], "highlight_lines": [1] },
                { "type": "unknown_widget", "foo": 123 }
            ],
            "art_meta": null
        }"#;
        let parsed = parse_raw_board_lenient(raw).expect("null fields must not break decode");
        assert_eq!(parsed.title, "Vòng lặp While và For");
        assert_eq!(parsed.steps.len(), 1);
        assert_eq!(parsed.blocks.len(), 1);
    }

    #[test]
    fn direct_board_json_extracts_both_string_and_object() {
        let as_str = serde_json::json!({
            "board_json": r#"{"title":"SQL DISTINCT","summary":"Loại bỏ bản ghi trùng","steps":[{"heading":"Cú pháp","body":"SELECT DISTINCT col FROM tbl"}]}"#
        });
        let raw_s = extract_direct_json_arg(&as_str, "board_json").unwrap();
        let parsed_s = parse_raw_board_lenient(&raw_s).unwrap();
        assert_eq!(parsed_s.title, "SQL DISTINCT");
        assert_eq!(parsed_s.steps.len(), 1);

        let as_obj = serde_json::json!({
            "board_json": {
                "title": "Hàm bậc 2",
                "summary": "Đồ thị Parabol",
                "blocks": [
                    { "type": "graph2d", "title": "Parabol", "curves": [{ "label": "y=x^2", "expr": "x^2" }] }
                ]
            }
        });
        let raw_o = extract_direct_json_arg(&as_obj, "board_json").unwrap();
        let parsed_o = parse_raw_board_lenient(&raw_o).unwrap();
        assert_eq!(parsed_o.title, "Hàm bậc 2");
        assert_eq!(parsed_o.blocks.len(), 1);
    }

    #[tokio::test]
    async fn study_board_handler_direct_path_and_navigation_work_without_network() {
        let (tx, mut rx) = broadcast::channel(16);

        // 1. Direct board_json renders immediately (<50ms) and emits AppEvent::StudyVisual
        let started = std::time::Instant::now();
        let res = study_board_handler(
            serde_json::json!({
                "action": "open",
                "topic": "Hàm bậc 2 y = x^2 - 2x - 3",
                "subject": "math",
                "source": "text",
                "board_json": {
                    "title": "Khảo sát y = x² - 2x - 3",
                    "subject": "Toán học",
                    "level": "Cấp 3",
                    "summary": "Parabol có đỉnh I(1, -4) và 2 nghiệm x = -1, x = 3",
                    "steps": [
                        { "heading": "Tìm đỉnh", "body": "x = -b/(2a) = 1 -> y = -4", "formula": "I(1, -4)" },
                        { "heading": "Tìm giao trục Ox", "body": "x² - 2x - 3 = 0 -> x = -1 hoặc x = 3" }
                    ],
                    "blocks": [
                        {
                            "type": "graph2d",
                            "title": "Đồ thị y = x² - 2x - 3",
                            "x_min": -3.0, "x_max": 5.0, "y_min": -5.0, "y_max": 5.0,
                            "curves": [{ "label": "y = x² - 2x - 3", "expr": "x^2 - 2*x - 3" }],
                            "points": [{ "x": 1.0, "y": -4.0, "label": "I(1,-4)" }]
                        }
                    ]
                }
            }),
            tx.clone(),
        )
        .await;
        assert!(
            !res.is_error,
            "direct board_json should succeed: {}",
            res.output
        );
        assert!(
            started.elapsed().as_millis() < 500,
            "direct board render must be instant"
        );
        match rx.try_recv() {
            Ok(AppEvent::StudyVisual(board)) => {
                assert_eq!(board.title, "Khảo sát y = x² - 2x - 3");
                assert_eq!(board.model_used, "gemini-live-direct");
                assert_eq!(board.steps.len(), 2);
                assert_eq!(board.blocks.len(), 1);
            }
            other => panic!("expected AppEvent::StudyVisual, got {other:?}"),
        }

        // 2. Voice navigation actions (next_step / prev_step / show_all / close)
        let r_next =
            study_board_handler(serde_json::json!({"action": "next_step"}), tx.clone()).await;
        assert!(!r_next.is_error);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::StudyVisualStep(s)) if s == "next"));

        let r_prev =
            study_board_handler(serde_json::json!({"action": "prev_step"}), tx.clone()).await;
        assert!(!r_prev.is_error);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::StudyVisualStep(s)) if s == "prev"));

        let r_all =
            study_board_handler(serde_json::json!({"action": "show_all"}), tx.clone()).await;
        assert!(!r_all.is_error);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::StudyVisualStep(s)) if s == "all"));

        let r_close = study_board_handler(serde_json::json!({"action": "close"}), tx).await;
        assert!(!r_close.is_error);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseStudyVisual)));
    }
}
