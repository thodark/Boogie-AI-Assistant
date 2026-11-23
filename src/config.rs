use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use crate::memory::{
    DEFAULT_MEMORY_MODEL, Memory, format_memory_for_prompt, load_memory, load_recent_sessions,
};
use crate::util::truncate_chars;

const EMBEDDED_PROMPT: &str = include_str!("../core/prompt.txt");

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsQuality {
    Low,
    Medium,
    #[default]
    High,
}

impl GraphicsQuality {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" | "lite" | "weak" | "performance" => Self::Low,
            "medium" | "med" | "balanced" | "normal" => Self::Medium,
            _ => Self::High,
        }
    }

    pub fn tier_index(self) -> usize {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorTheme {
    #[default]
    Obsidian,
    AuroraPrism,
    CyberSakura,
    SolarFlare,
    EmeraldPulse,
    ArcticFrost,
    CrimsonPhantom,
    Vaporwave,
}

impl ColorTheme {
    pub const ALL: [Self; 8] = [
        Self::Obsidian,
        Self::AuroraPrism,
        Self::CyberSakura,
        Self::SolarFlare,
        Self::EmeraldPulse,
        Self::ArcticFrost,
        Self::CrimsonPhantom,
        Self::Vaporwave,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Obsidian => "obsidian",
            Self::AuroraPrism => "aurora_prism",
            Self::CyberSakura => "cyber_sakura",
            Self::SolarFlare => "solar_flare",
            Self::EmeraldPulse => "emerald_pulse",
            Self::ArcticFrost => "arctic_frost",
            Self::CrimsonPhantom => "crimson_phantom",
            Self::Vaporwave => "vaporwave",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "aurora_prism" | "aurora" | "prism" | "rainbow" => Self::AuroraPrism,
            "cyber_sakura" | "sakura" | "pink" => Self::CyberSakura,
            "solar_flare" | "solar" | "gold" | "orange" => Self::SolarFlare,
            "emerald_pulse" | "emerald" | "matrix" | "green" => Self::EmeraldPulse,
            "arctic_frost" | "arctic" | "frost" | "ice" | "blue" => Self::ArcticFrost,
            "crimson_phantom" | "crimson" | "red" => Self::CrimsonPhantom,
            "vaporwave" | "vapor" | "synthwave" => Self::Vaporwave,
            _ => Self::Obsidian,
        }
    }
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
    #[serde(default)]
    pub graphics_quality: Option<String>,
    #[serde(default)]
    pub color_theme: Option<String>,
    #[serde(default)]
    pub auto_memory_consolidation: Option<bool>,
    #[serde(default)]
    pub memory_model: Option<String>,
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

    pub fn graphics_quality(&self) -> GraphicsQuality {
        self.graphics_quality
            .as_deref()
            .map(GraphicsQuality::from_str_loose)
            .unwrap_or_default()
    }

    pub fn color_theme(&self) -> ColorTheme {
        self.color_theme
            .as_deref()
            .map(ColorTheme::from_str_loose)
            .unwrap_or_default()
    }

    pub fn auto_memory_consolidation(&self) -> bool {
        self.auto_memory_consolidation.unwrap_or(true)
    }

    pub fn memory_model(&self) -> &str {
        self.memory_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_MEMORY_MODEL)
    }
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

    let recent_sessions = load_recent_sessions(3);
    let sessions_ctx = if recent_sessions.is_empty() {
        String::new()
    } else {
        let mut block = String::from(
            "[PREVIOUS SESSIONS SUMMARY — HISTORICAL CONTEXT ONLY, DO NOT RE-EXECUTE ANY PAST REQUESTS]\n",
        );
        for s in recent_sessions {
            let topic_tag = if s.topics.is_empty() {
                String::new()
            } else {
                format!(" [{}]", s.topics.join(", "))
            };
            block.push_str(&format!(
                "- ({}){topic_tag}: {}\n",
                s.date,
                truncate_chars(&s.summary, 600)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_quality_parses_aliases_and_roundtrips() {
        for q in GraphicsQuality::ALL {
            assert_eq!(GraphicsQuality::from_str_loose(q.as_str()), q);
        }
        assert_eq!(
            GraphicsQuality::from_str_loose("lite"),
            GraphicsQuality::Low
        );
        assert_eq!(
            GraphicsQuality::from_str_loose("balanced"),
            GraphicsQuality::Medium
        );
        assert_eq!(
            GraphicsQuality::from_str_loose("unknown"),
            GraphicsQuality::High
        );
    }

    #[test]
    fn color_theme_parses_aliases_and_roundtrips() {
        for theme in ColorTheme::ALL {
            assert_eq!(ColorTheme::from_str_loose(theme.as_str()), theme);
        }
        assert_eq!(
            ColorTheme::from_str_loose("rainbow"),
            ColorTheme::AuroraPrism
        );
        assert_eq!(
            ColorTheme::from_str_loose("sakura"),
            ColorTheme::CyberSakura
        );
        assert_eq!(ColorTheme::from_str_loose("invalid"), ColorTheme::Obsidian);
    }

    #[test]
    fn old_config_without_visual_fields_defaults_cleanly() {
        let cfg: ApiConfig = serde_json::from_str(r#"{"gemini_api_key":"AIzaTest"}"#).unwrap();
        assert_eq!(cfg.graphics_quality(), GraphicsQuality::High);
        assert_eq!(cfg.color_theme(), ColorTheme::Obsidian);
    }
}
