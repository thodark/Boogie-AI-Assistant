use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "youtube_video",
            "YouTube: play a song/video by search query, get trending videos, or get video info. \
             For play: searches YouTube and opens the first non-Shorts video result. \
             For trending: returns top trending videos by region.",
            vec![
                ("action", param("action", "string", "play | trending | get_info (default: play)")),
                ("query", param("query", "string", "Search query for play action")),
                ("region", param("region", "string", "Country code for trending (default: VN)")),
                ("url", param("url", "string", "Video URL for get_info")),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(youtube_video(args))),
    );
}

async fn youtube_video(args: Value) -> ToolResult {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("play");
    match action {
        "play" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
            if query.is_empty() {
                return ToolResult::err("No search query provided");
            }
            play_video(query).await
        }
        "trending" => {
            let region = args.get("region").and_then(|v| v.as_str()).unwrap_or("VN");
            trending(region).await
        }
        "get_info" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            if url.is_empty() {
                return ToolResult::err("No video URL provided");
            }
            get_info(url).await
        }
        _ => ToolResult::err(format!("Unknown action '{action}'. Use: play, trending, get_info")),
    }
}

async fn play_video(query: &str) -> ToolResult {
    let encoded = urlencoding::encode(query);
    let search_url = format!(
        "https://www.youtube.com/results?search_query={encoded}&sp=EgIQAQ%3D%3D"
    );

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("http client: {e}"))
        .unwrap_or_default();

    let html = match client.get(&search_url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("Failed to read response: {e}")),
        },
        Err(e) => return ToolResult::err(format!("Failed to fetch YouTube: {e}")),
    };

    let re = match regex::Regex::new(r#""videoId":"([A-Za-z0-9_-]{11})""#) {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("regex: {e}")),
    };

    let mut seen = std::collections::HashSet::new();
    for cap in re.captures_iter(&html) {
        let vid = cap.get(1).unwrap().as_str();
        if !seen.insert(vid.to_string()) {
            continue;
        }
        if html.contains(&format!("/shorts/{vid}")) {
            continue;
        }
        let url = format!("https://www.youtube.com/watch?v={vid}");
        open_url_platform(&url);
        return ToolResult::ok(format!("Playing: {query}\n{url}"));
    }

    open_url_platform(&search_url);
    ToolResult::ok(format!("No direct video found. Opened search results for: {query}"))
}

async fn trending(region: &str) -> ToolResult {
    let url = format!("https://www.youtube.com/feed/trending?gl={region}");

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .unwrap_or_default();

    let html = match client.get(&url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("read: {e}")),
        },
        Err(e) => return ToolResult::err(format!("fetch: {e}")),
    };

    let title_re = regex::Regex::new(r#""title":\{"runs":\[\{"text":"([^"]+)"\}\]"#).unwrap();
    let channel_re = regex::Regex::new(r#""ownerText":\{"runs":\[\{"text":"([^"]+)""#).unwrap();
    let vid_re = regex::Regex::new(r#""videoId":"([A-Za-z0-9_-]{11})""#).unwrap();

    let titles: Vec<&str> = title_re.captures_iter(&html).map(|c| c.get(1).unwrap().as_str()).collect();
    let channels: Vec<&str> = channel_re.captures_iter(&html).map(|c| c.get(1).unwrap().as_str()).collect();
    let vids: Vec<&str> = vid_re.captures_iter(&html).map(|c| c.get(1).unwrap().as_str()).collect();

    if titles.is_empty() {
        return ToolResult::ok(format!("No trending videos found for region {region}"));
    }

    let mut lines = Vec::new();
    let count = titles.len().min(8);
    for i in 0..count {
        let title = titles[i];
        let channel = channels.get(i).unwrap_or(&"");
        let vid = vids.get(i).map(|v| format!(" (https://youtube.com/watch?v={v})")).unwrap_or_default();
        lines.push(format!("{}. {} — {}{}", i + 1, title, channel, vid));
    }

    ToolResult::ok(format!("Trending in {}:\n{}", region, lines.join("\n")))
}

async fn get_info(url: &str) -> ToolResult {
    let vid = extract_video_id(url).unwrap_or_default();
    if vid.is_empty() {
        return ToolResult::err("Could not extract video ID from URL");
    }

    let page_url = format!("https://www.youtube.com/watch?v={vid}");
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .unwrap_or_default();

    let html = match client.get(&page_url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("read: {e}")),
        },
        Err(e) => return ToolResult::err(format!("fetch: {e}")),
    };

    let title = extract_meta(&html, r#""title":"([^"]+)""#).unwrap_or_default();
    let channel = extract_meta(&html, r#""ownerChannelName":"([^"]+)""#).unwrap_or_default();
    let views = extract_meta(&html, r#""viewCount":"(\d+)""#).unwrap_or_default();
    let length = extract_meta(&html, r#""lengthSeconds":"(\d+)""#).unwrap_or_default();

    let duration = if let Ok(secs) = length.parse::<u64>() {
        format!("{}:{:02}", secs / 60, secs % 60)
    } else {
        length
    };

    ToolResult::ok(format!(
        "Title: {title}\nChannel: {channel}\nViews: {views}\nDuration: {duration}\nURL: {page_url}"
    ))
}

fn extract_video_id(url: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?:v=|youtu\.be/|/embed/|/v/)([A-Za-z0-9_-]{11})").ok()?;
    re.captures(url).map(|c| c.get(1).unwrap().as_str().to_string())
}

fn extract_meta(html: &str, pattern: &str) -> Option<String> {
    let re = regex::Regex::new(pattern).ok()?;
    re.captures(html).map(|c| c.get(1).unwrap().as_str().to_string())
}

fn open_url_platform(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = open::that(url);
    }
}
