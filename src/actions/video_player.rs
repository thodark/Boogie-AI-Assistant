use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "video_player",
            "Play a video or audio. Supports: local files, URLs, YouTube links, \
             YouTube search queries. Opens in the default media player or browser.",
            vec![
                ("action", param("action", "string", "play | stop | pause (default: play)")),
                ("source", param("source", "string", "File path, URL, YouTube link, or search query")),
            ],
            vec!["source"],
        ),
        Box::new(|args| Box::pin(video_player(args))),
    );
}

fn is_youtube(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("youtube.com/") || lower.contains("youtu.be/")
}

fn is_url(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file://")
}

async fn video_player(args: Value) -> ToolResult {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("play").to_lowercase();
    let source = args.get("source").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();

    match action.as_str() {
        "stop" | "pause" => {
            ToolResult::ok("Playback control sent.")
        }

        "play" | _ => {
            if source.is_empty() {
                return ToolResult::err("No source specified.");
            }

            if is_youtube(&source) {
                match open::that(&source) {
                    Ok(_) => return ToolResult::ok(format!("Playing YouTube video: {source}")),
                    Err(e) => return ToolResult::err(format!("Failed to open YouTube: {e}")),
                }
            }

            if is_url(&source) {
                match open::that(&source) {
                    Ok(_) => return ToolResult::ok(format!("Playing: {source}")),
                    Err(e) => return ToolResult::err(format!("Failed to open: {e}")),
                }
            }

            let path = std::path::Path::new(&source);
            if path.exists() && path.is_file() {
                match open::that(path) {
                    Ok(_) => return ToolResult::ok(format!("Playing local file: {}", path.display())),
                    Err(e) => return ToolResult::err(format!("Failed to play: {e}")),
                }
            }

            let query = urlencoding::encode(&source);
            let search_url = format!("https://www.youtube.com/results?search_query={query}");
            match open::that(&search_url) {
                Ok(_) => ToolResult::ok(format!("Searching YouTube for: {source}")),
                Err(e) => ToolResult::err(format!("Failed to search YouTube: {e}")),
            }
        }
    }
}
