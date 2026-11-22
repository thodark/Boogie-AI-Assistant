use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{http_client, truncate_chars};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "background_monitor",
            "Monitor topics for news. Actions: add (start monitoring a topic), \
             remove (stop monitoring), list (show all monitored topics), \
             check (run all pending checks now — normally runs automatically).",
            vec![
                (
                    "action",
                    param(
                        "action",
                        "string",
                        "add | remove | list | check (default: add)",
                    ),
                ),
                (
                    "topic",
                    param("topic", "string", "Topic to monitor (for add/remove)"),
                ),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(background_monitor(args))),
    );
}

const BLOCKED: &[&str] = &[
    "bitcoin",
    "ethereum",
    "dogecoin",
    "solana",
    "binance",
    "nft",
    "blockchain",
    "defi",
    "altcoin",
    "memecoin",
    "coin",
    "token",
    "crypto",
    "kripto",
    "cripto",
    "krypto",
    "cryptocurrency",
];

fn is_blocked(topic: &str) -> bool {
    let lower = topic.to_lowercase();
    BLOCKED.iter().any(|w| lower.contains(w))
}

fn slug(topic: &str) -> String {
    let lower = topic.to_lowercase();
    let s: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = s.trim_matches('_');
    truncate_chars(trimmed, 40)
        .trim_end_matches('_')
        .to_string()
}

fn title_hash(title: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    title.hash(&mut h);
    format!("{:016x}", h.finish())[..12].to_string()
}

fn monitors_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".boogie")
        .join("monitors.json")
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MonitorEntry {
    topic: String,
    added: String,
    last_check: String,
    last_hash: String,
}

fn load_monitors() -> HashMap<String, MonitorEntry> {
    let path = monitors_path();
    if !path.exists() {
        return HashMap::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_monitors(monitors: &HashMap<String, MonitorEntry>) {
    let path = monitors_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(monitors) {
        let _ = std::fs::write(&path, json);
    }
}

async fn ddg_news_search(topic: &str, max: usize) -> Vec<(String, String)> {
    let query = format!("{topic} news");
    let encoded = urlencoding::encode(&query);
    let url = format!("https://html.duckduckgo.com/html/?q={encoded}");

    let html = match http_client().get(&url).send().await {
        Ok(resp) => resp.text().await.unwrap_or_default(),
        Err(_) => return Vec::new(),
    };

    let doc = scraper::Html::parse_document(&html);
    let title_sel = scraper::Selector::parse(".result__a").unwrap();
    let snippet_sel = scraper::Selector::parse(".result__snippet").unwrap();

    let titles: Vec<String> = doc
        .select(&title_sel)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .collect();
    let snippets: Vec<String> = doc
        .select(&snippet_sel)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .collect();

    titles
        .into_iter()
        .zip(snippets.into_iter().chain(std::iter::repeat(String::new())))
        .take(max)
        .collect()
}

async fn background_monitor(args: Value) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("add")
        .to_lowercase();
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    match action.as_str() {
        "add" => {
            if topic.is_empty() {
                return ToolResult::err("Please specify a topic to monitor.");
            }
            if is_blocked(&topic) {
                return ToolResult::err("I don't monitor crypto or financial topics.");
            }
            let mut monitors = load_monitors();
            let s = slug(&topic);
            if monitors.contains_key(&s) {
                return ToolResult::ok(format!("Already monitoring: {}", monitors[&s].topic));
            }
            monitors.insert(
                s,
                MonitorEntry {
                    topic: topic.clone(),
                    added: chrono::Local::now().format("%Y-%m-%d").to_string(),
                    last_check: String::new(),
                    last_hash: String::new(),
                },
            );
            save_monitors(&monitors);
            ToolResult::ok(format!("Now monitoring: {topic}"))
        }

        "remove" => {
            if topic.is_empty() {
                return ToolResult::err("Please specify a topic to stop monitoring.");
            }
            let mut monitors = load_monitors();
            let s = slug(&topic);
            if let Some(entry) = monitors.remove(&s) {
                save_monitors(&monitors);
                return ToolResult::ok(format!("Stopped monitoring: {}", entry.topic));
            }
            let lower = topic.to_lowercase();
            let key = monitors
                .iter()
                .find(|(_, v)| v.topic.to_lowercase().contains(&lower))
                .map(|(k, _)| k.clone());
            if let Some(k) = key {
                let entry = monitors.remove(&k).unwrap();
                save_monitors(&monitors);
                return ToolResult::ok(format!("Stopped monitoring: {}", entry.topic));
            }
            ToolResult::err(format!("Not found in monitored topics: {topic}"))
        }

        "list" => {
            let monitors = load_monitors();
            if monitors.is_empty() {
                return ToolResult::ok("No topics being monitored.");
            }
            let topics: Vec<&str> = monitors.values().map(|v| v.topic.as_str()).collect();
            ToolResult::ok(format!("Monitoring: {}", topics.join(", ")))
        }

        "check" => {
            let alerts = check_all_monitors().await;
            if alerts.is_empty() {
                ToolResult::ok("All topics checked — no new headlines.")
            } else {
                ToolResult::ok_untrusted("web", alerts.join("\n\n"))
            }
        }

        _ => ToolResult::err(format!(
            "Unknown action: {action}. Use add, remove, list, or check."
        )),
    }
}

pub async fn check_all_monitors() -> Vec<String> {
    let mut monitors = load_monitors();
    if monitors.is_empty() {
        return Vec::new();
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut alerts = Vec::new();
    let mut changed = false;

    for data in monitors.values_mut() {
        if data.last_check == today {
            continue;
        }

        let results = ddg_news_search(&data.topic, 5).await;
        data.last_check = today.clone();
        changed = true;

        if results.is_empty() {
            continue;
        }

        let (title, snippet) = &results[0];
        if title.is_empty() {
            continue;
        }

        let h = title_hash(title);
        if h == data.last_hash {
            continue;
        }
        data.last_hash = h;

        let truncated = truncate_chars(snippet, 150);
        alerts.push(format!(
            "[MONITOR_ALERT] {}\nHeadline: {}\n{}",
            data.topic, title, truncated
        ));
    }

    if changed {
        save_monitors(&monitors);
    }

    alerts
}
