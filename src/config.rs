use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn app_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().unwrap_or(Path::new(".")).to_path_buf()
}

fn config_path() -> PathBuf {
    app_dir().join("config").join("api_keys.json")
}

fn memory_path() -> PathBuf {
    app_dir().join("config").join("memory.json")
}

fn long_term_path() -> PathBuf {
    app_dir().join("config").join("long_term.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiConfig {
    pub gemini_api_key: String,
    #[serde(default)]
    pub assistant_name: Option<String>,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default, alias = "voice_name")]
    pub voice: Option<String>,
    #[serde(default)]
    pub push_to_talk: Option<bool>,
    #[serde(default)]
    pub brief_enabled: Option<bool>,
    #[serde(default)]
    pub proactive_audio: Option<bool>,
    #[serde(default)]
    pub media_resolution: Option<String>,
    #[serde(default)]
    pub thinking_enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ApiConfig {
    pub fn load() -> Result<Self, String> {
        let path = config_path();
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("config parse error: {e}"))
    }

    pub fn api_key(&self) -> &str {
        &self.gemini_api_key
    }

    pub fn assistant_name(&self) -> &str {
        self.assistant_name.as_deref().unwrap_or("Boogie")
    }

    pub fn voice(&self) -> &str {
        self.voice.as_deref().unwrap_or("Aoede")
    }

    pub fn brief_enabled(&self) -> bool {
        self.brief_enabled.unwrap_or(true)
    }

    pub fn push_to_talk(&self) -> bool {
        self.push_to_talk.unwrap_or(false)
    }

    pub fn proactive_audio(&self) -> bool {
        self.proactive_audio.unwrap_or(true)
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
    let path = long_term_path();
    let mut entries: Vec<SessionSummary> = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    entries.push(SessionSummary {
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        summary: summary.to_string(),
        language: language.to_string(),
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(text) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(&path, text);
    }
}

pub fn pop_last_session() -> Option<SessionSummary> {
    let path = long_term_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let mut entries: Vec<SessionSummary> = serde_json::from_str(&text).ok()?;
    let last = entries.pop()?;
    if let Ok(text) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(&path, text);
    }
    Some(last)
}

pub fn load_system_prompt() -> String {
    let path = app_dir().join("core").join("prompt.txt");
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            "You are Boogie, a personal AI assistant. \
             Be concise, direct, and always use the provided tools to complete tasks. \
             Never simulate or guess results — always call the appropriate tool."
                .to_string()
        }
    }
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
            let desc = d.description.split_whitespace().collect::<Vec<_>>().join(" ");
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
             vision tool, and then only that single frame at that moment — you \
             cannot watch, monitor or notice something changing on screen."
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

fn detect_language(memory: &Memory) -> String {
    for key in ["identity", "preferences", "user"] {
        if let Some(cat) = memory.get(key) {
            if let Some(entry) = cat.get("language") {
                return entry.value.clone();
            }
        }
    }
    "Vietnamese".to_string()
}

pub fn build_full_prompt(
    cfg: &ApiConfig,
    tool_declarations: &[crate::tools::ToolDeclaration],
    memory: &Memory,
    user_name: Option<&str>,
) -> String {
    let raw_template = load_system_prompt();

    let tool_names: std::collections::HashSet<&str> =
        tool_declarations.iter().map(|d| d.name.as_str()).collect();
    let has_vision = tool_names.contains("screen_process");
    let has_mic = true;

    let asst_name = cfg.assistant_name();
    let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let capabilities = describe_tools(tool_declarations);
    let limits = describe_limits(has_vision, has_mic);

    let rendered = render_prompt(&raw_template, &[
        ("assistant_name", asst_name),
        ("platform", &platform),
        ("capabilities", &capabilities),
        ("limits", &limits),
    ]);

    let now = chrono::Local::now();
    let time_str = now.format("%A, %B %d, %Y — %I:%M %p").to_string();
    let time_ctx = format!(
        "[CURRENT DATE & TIME]\nRight now it is: {time_str}\n\
         Use this to calculate exact times for reminders.\n"
    );

    let addr = match user_name {
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
         Always refer to yourself as {asst_name}.\n{addr}\n"
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

    let mut parts = vec![time_ctx, identity_ctx, lang_ctx];
    if !mem_text.is_empty() {
        parts.push(mem_text);
    }
    parts.push(rendered);
    parts.join("\n")
}
