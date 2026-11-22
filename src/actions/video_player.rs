use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{is_blocked_executable, validate_public_http_url};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "video_player",
            "Play, pause, or stop media. Supports local media files, public http/https URLs, \
             YouTube links, or search queries (opens in default media player or browser). \
             For pause/stop, sends OS media control keys.",
            vec![
                (
                    "action",
                    param("action", "string", "play | stop | pause (default: play)"),
                ),
                (
                    "source",
                    param(
                        "source",
                        "string",
                        "File path, URL, YouTube link, or search query (required for play)",
                    ),
                ),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(video_player(args))),
    );
}

fn is_url(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

async fn video_player(args: Value) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("play")
        .to_lowercase();
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    match action.as_str() {
        "stop" => {
            #[cfg(windows)]
            {
                let _ = tokio::task::spawn_blocking(|| {
                    super::computer_control::send_media_key(true);
                })
                .await;
                ToolResult::ok("Sent OS media stop key.")
            }
            #[cfg(not(windows))]
            ToolResult::err("Media stop control is only supported on Windows.")
        }
        "pause" => {
            #[cfg(windows)]
            {
                let _ = tokio::task::spawn_blocking(|| {
                    super::computer_control::send_media_key(false);
                })
                .await;
                ToolResult::ok("Sent OS media play/pause key.")
            }
            #[cfg(not(windows))]
            ToolResult::err("Media pause control is only supported on Windows.")
        }

        _ => {
            if source.is_empty() {
                return ToolResult::err("No source specified.");
            }

            if is_url(&source) {
                let parsed = match validate_public_http_url(&source) {
                    Ok(u) => u,
                    Err(e) => return ToolResult::err(e),
                };
                return match open::that(parsed.as_str()) {
                    Ok(_) => ToolResult::ok(format!("Playing: {}", parsed.as_str())),
                    Err(e) => ToolResult::err(format!("Failed to open: {e}")),
                };
            }

            let path = std::path::Path::new(&source);
            if path.exists() && path.is_file() {
                if is_blocked_executable(&source) {
                    return ToolResult::err(format!(
                        "Refused to open executable/script file via video_player: {source}"
                    ));
                }
                return match open::that(path) {
                    Ok(_) => ToolResult::ok(format!("Playing local media: {}", path.display())),
                    Err(e) => ToolResult::err(format!("Failed to play: {e}")),
                };
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
