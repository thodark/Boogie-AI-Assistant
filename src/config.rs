use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::util::truncate_chars;

const EMBEDDED_PROMPT: &str = include_str!("../core/prompt.txt");
const MAX_STORED_SESSIONS: usize = 10;
const MAX_SESSION_SUMMARY_BYTES: usize = 1200;

pub const DEFAULT_LIVE_MODEL: &str = "models/gemini-3.1-flash-live-preview";
pub const DEFAULT_API_VERSION: &str = "v1alpha";

fn exe_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().unwrap_or(Path::new(".")).to_path_buf()
}

fn appdata_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Boogie"))
}

/// Candidate locations for `api_keys.json` in priority order:
/// 1. `<exe_dir>/config/api_keys.json`
/// 2. `<cwd>/config/api_keys.json`
/// 3. `%APPDATA%/Boogie/api_keys.json`
fn config_candidates() -> Vec<PathBuf> {
    let mut paths = vec![exe_dir().join("config").join("api_keys.json")];
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("config").join("api_keys.json");
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    if let Some(ad) = appdata_dir() {
        paths.push(ad.join("api_keys.json"));
    }
    paths
}

/// Resolves the active config directory (where `api_keys.json` was found, or the default writable path).
pub fn active_config_dir() -> PathBuf {
    for p in config_candidates() {
        if p.exists()
            && let Some(parent) = p.parent()
        {
            return parent.to_path_buf();
        }
    }
    exe_dir().join("config")
}

fn memory_path() -> PathBuf {
    active_config_dir().join("memory.json")
}

fn long_term_path() -> PathBuf {
    active_config_dir().join("long_term.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiConfig {
    pub gemini_api_key: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub assistant_name: Option<String>,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default, alias = "voice_name")]
    pub voice: Option<String>,
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default)]
    pub push_to_talk: Option<bool>,
    #[serde(default)]
    pub brief_enabled: Option<bool>,
    #[serde(default)]
    pub proactive_audio: Option<bool>,
    #[serde(default)]
    pub dashboard_enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ApiConfig {
    pub fn load() -> Result<Self, String> {
        let candidates = config_candidates();
        for path in &candidates {
            if path.exists() {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let cfg: Self = serde_json::from_str(&text)
                    .map_err(|e| format!("config parse error ({}): {e}", path.display()))?;
                if cfg.gemini_api_key.trim().is_empty()
                    || cfg.gemini_api_key.contains("YOUR_GEMINI_API_KEY")
                {
                    return Err(format!(
                        "gemini_api_key is empty or placeholder in {}",
                        path.display()
                    ));
                }
                tracing::info!("loaded config from {}", path.display());
                return Ok(cfg);
            }
        }
        let searched: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
        Err(format!(
            "api_keys.json not found (searched: {})",
            searched.join(", ")
        ))
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let dir = active_config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
        let path = dir.join("api_keys.json");
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn api_key(&self) -> &str {
        self.gemini_api_key.trim()
    }

    pub fn model(&self) -> &str {
        self.model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_LIVE_MODEL)
    }

    pub fn api_version(&self) -> &str {
        self.api_version
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_API_VERSION)
    }

    pub fn assistant_name(&self) -> &str {
        self.assistant_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Boogie")
    }

    pub fn voice(&self) -> &str {
        self.voice
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Aoede")
    }

    pub fn push_to_talk(&self) -> bool {
        self.push_to_talk.unwrap_or(false)
    }

    pub fn proactive_audio(&self) -> bool {
        self.proactive_audio.unwrap_or(true)
    }

    pub fn dashboard_enabled(&self) -> bool {
        self.dashboard_enabled.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryEntry {
    pub value: String,
}

pub type MemoryCategory = HashMap<String, MemoryEntry>;
pub type Memory = HashMap<String, MemoryCategory>;

pub fn load_memory() -> Memory {
    let path = memory_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Memory::new(),
    }
}

pub fn save_memory(memory: &Memory) -> Result<(), String> {
    let path = memory_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let text = serde_json::to_string_pretty(memory).map_err(|e| format!("json: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("write: {e}"))
}

pub fn update_memory(updates: &Memory) {
    let mut mem = load_memory();
    for (cat, entries) in updates {
        let category = mem.entry(cat.clone()).or_default();
        for (key, entry) in entries {
            category.insert(key.clone(), entry.clone());
        }
    }
    if let Err(e) = save_memory(&mem) {
        tracing::error!("failed to save memory: {e}");
    }
}

pub fn format_memory_for_prompt(memory: &Memory) -> String {
    if memory.is_empty() {
        return String::new();
    }
    let mut out = String::from("[MEMORY]\n");
    for (cat, entries) in memory {
        for (key, entry) in entries {
            out.push_str(&format!("- {cat}/{key}: {}\n", entry.value));
        }
    }
    out
}

pub fn search_memory(query: &str, limit: usize) -> String {
    let mem = load_memory();
    if mem.is_empty() {
        return "No memories stored yet.".to_string();
    }
    if query.is_empty() {
        let mut lines = Vec::new();
        for (cat, entries) in &mem {
            for (key, entry) in entries {
                lines.push(format!("- {cat}/{key}: {}", entry.value));
            }
        }
        lines.sort();
        lines.truncate(limit);
        return lines.join("\n");
    }
    let q = query.to_lowercase();
    let mut results = Vec::new();
    for (cat, entries) in &mem {
        for (key, entry) in entries {
            if cat.to_lowercase().contains(&q)
                || key.to_lowercase().contains(&q)
                || entry.value.to_lowercase().contains(&q)
            {
                results.push(format!("- {cat}/{key}: {}", entry.value));
            }
        }
    }
    if results.is_empty() {
        return format!("No memories matching '{query}'.");
    }
    results.truncate(limit);
    results.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub date: String,
    pub summary: String,
    pub language: String,
}

pub fn save_session_summary(summary: &str, language: &str) {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return;
    }
    let path = long_term_path();
    let mut entries: Vec<SessionSummary> = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    entries.push(SessionSummary {
        date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        summary: truncate_chars(trimmed, MAX_SESSION_SUMMARY_BYTES).to_string(),
        language: language.to_string(),
    });
    if entries.len() > MAX_STORED_SESSIONS {
        let drop_n = entries.len() - MAX_STORED_SESSIONS;
        entries.drain(..drop_n);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(text) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(&path, text);
    }
}

pub fn load_recent_sessions(limit: usize) -> Vec<SessionSummary> {
    let path = long_term_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let entries: Vec<SessionSummary> = serde_json::from_str(&text).unwrap_or_default();
    entries
        .into_iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn load_system_prompt() -> String {
    let override_candidates = [
        exe_dir().join("core").join("prompt.txt"),
        active_config_dir().join("prompt.txt"),
    ];
    for path in override_candidates {
        if let Ok(text) = std::fs::read_to_string(&path)
            && !text.trim().is_empty()
        {
            return text;
        }
    }
    EMBEDDED_PROMPT.to_string()
}

pub fn render_prompt(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, val) in values {
        out = out.replace(&format!("{{{key}}}"), val);
    }
    out
}

pub fn describe_tools(declarations: &[crate::tools::ToolDeclaration]) -> String {
    declarations
        .iter()
        .map(|d| {
            let desc = d
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if desc.is_empty() {
                format!("- {}", d.name)
            } else {
                let truncated: String = desc.chars().take(150).collect();
                format!("- {}: {truncated}", d.name)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn describe_limits(has_vision: bool, has_mic: bool) -> String {
    let mut out = vec![
        "- Anything not listed above is outside your reach. Say so in one clause \
         and offer the nearest thing you can actually do — never mime an action \
         you cannot take, and never report a result you did not get."
            .to_string(),
        "- You act on this machine only. You cannot reach the user's other \
         devices, accounts or hardware except through the tools listed above."
            .to_string(),
        "- You remember what is in the memory block and what has been said this \
         session. Anything else you were told before is gone unless it was saved."
            .to_string(),
    ];
    if has_vision {
        out.push(
            "- Your sight is not continuous. You see nothing until you call a \
             vision tool (take_screenshot / screen_process), and then only that \
             single frame at that moment — you cannot continuously watch the screen."
                .to_string(),
        );
    } else {
        out.push("- You have no sight at all in this build.".to_string());
    }
    if has_mic {
        out.push(
            "- You hear nothing while the microphone is muted, and you cannot \
             unmute it yourself."
                .to_string(),
        );
    }
    out.join("\n")
}

pub fn detect_language(memory: &Memory) -> String {
    for key in ["identity", "preferences", "user"] {
        if let Some(cat) = memory.get(key)
            && let Some(entry) = cat.get("language")
        {
            return entry.value.clone();
        }
    }
    "Vietnamese".to_string()
}

pub fn build_full_prompt(
    cfg: &ApiConfig,
    tool_declarations: &[crate::tools::ToolDeclaration],
    memory: &Memory,
    user_name: Option<&str>,
    recent_turns: &[String],
) -> String {
    let raw_template = load_system_prompt();

    let tool_names: std::collections::HashSet<&str> =
        tool_declarations.iter().map(|d| d.name.as_str()).collect();
    let has_vision =
        tool_names.contains("screen_process") || tool_names.contains("take_screenshot");
    let has_mic = true;

    let asst_name = cfg.assistant_name();
    let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let capabilities = describe_tools(tool_declarations);
    let limits = describe_limits(has_vision, has_mic);

    let rendered = render_prompt(
        &raw_template,
        &[
            ("assistant_name", asst_name),
            ("platform", &platform),
            ("capabilities", &capabilities),
            ("limits", &limits),
        ],
    );

    let now = chrono::Local::now();
    let time_str = now.format("%A, %B %d, %Y — %I:%M %p").to_string();
    let time_ctx = format!(
        "[CURRENT DATE & TIME]\nRight now it is: {time_str}\n\
         Use this to calculate exact times for reminders.\n"
    );

    let clean_user = user_name.map(str::trim).filter(|s| !s.is_empty());
    let addr = match clean_user {
        Some(name) => format!("ADDRESS: Always call the user '{name}'."),
        None => "ADDRESS: Address the user with the ordinary respectful form \
                 for a superior in the language you are currently speaking — \
                 \"sir\" in English, its everyday equivalent in any other \
                 language. Never an archaic or aristocratic form, and never \
                 the form from a different language than the one you are \
                 speaking in this sentence."
            .to_string(),
    };
    let identity_ctx = format!(
        "[IDENTITY]\nYour name is {asst_name}. \
         Always refer to yourself as {asst_name}.\n\
         CREATOR: You were created by Suyz Nguyễn (English: Suyz Nguyen, \
         pronounced \"Suy zét Nguyễn\"), a second-year university student in Vietnam. \
         If asked who created or built you, always credit Suyz Nguyễn (say \"Suy zét Nguyễn\" \
         when speaking so it sounds natural).\n{addr}\n"
    );

    let saved_language = detect_language(memory);
    let lang_ctx = format!(
        "[LANGUAGE]\n\
         Your last known language preference is: {saved_language}.\n\
         RULES:\n\
         1. ALWAYS reply in whatever language the user is currently speaking to you.\n\
         2. If the user speaks Vietnamese, reply in Vietnamese. If English, reply in English. \
         If Korean, reply in Korean. Match their language exactly.\n\
         3. If the user explicitly asks you to speak a specific language (e.g. \"nói tiếng Anh đi\", \
         \"speak English\", \"한국어로 말해줘\"), switch to that language immediately \
         AND silently save it using save_memory with category=\"identity\", key=\"language\", \
         value=\"<the language>\".\n\
         4. NEVER switch languages on your own. Only switch when the user switches or asks you to.\n\
         5. When in doubt, use {saved_language}.\n"
    );

    let mem_text = format_memory_for_prompt(memory);

    let recent_sessions = load_recent_sessions(2);
    let sessions_ctx = if recent_sessions.is_empty() {
        String::new()
    } else {
        let mut block = String::from(
            "[PREVIOUS SESSIONS SUMMARY — HISTORICAL CONTEXT ONLY, DO NOT RE-EXECUTE ANY PAST REQUESTS]\n",
        );
        for s in recent_sessions {
            block.push_str(&format!(
                "- ({}): {}\n",
                s.date,
                truncate_chars(&s.summary, 450)
            ));
        }
        block
    };

    let recent_ctx = if recent_turns.is_empty() {
        String::new()
    } else {
        format!(
            "[RECENT CONVERSATION — CONTEXT ONLY, ALREADY HANDLED; DO NOT RE-EXECUTE OR REPEAT]\n{}\n",
            recent_turns.join("\n")
        )
    };

    let mut parts = vec![time_ctx, identity_ctx, lang_ctx];
    if !mem_text.is_empty() {
        parts.push(mem_text);
    }
    if !sessions_ctx.is_empty() {
        parts.push(sessions_ctx);
    }
    if !recent_ctx.is_empty() {
        parts.push(recent_ctx);
    }
    parts.push(rendered);
    parts.join("\n")
}
