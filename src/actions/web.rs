use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "web_search",
            "Search the web using DuckDuckGo",
            vec![
                ("query", param("query", "string", "Search query")),
                ("num_results", param("num_results", "integer", "Number of results (default 5)")),
            ],
            vec!["query"],
        ),
        Box::new(|args| Box::pin(web_search(args))),
    );

    registry.register(
        tool(
            "get_weather",
            "Get weather for a location",
            vec![("location", param("location", "string", "City name or coordinates"))],
            vec!["location"],
        ),
        Box::new(|args| Box::pin(get_weather(args))),
    );

    registry.register(
        tool(
            "get_news",
            "Get latest news headlines",
            vec![("topic", param("topic", "string", "News topic"))],
            vec![],
        ),
        Box::new(|args| Box::pin(get_news(args))),
    );
}

async fn web_search(args: Value) -> ToolResult {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
    let _num = args.get("num_results").and_then(|v| v.as_u64()).unwrap_or(5);

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    match reqwest::get(&url).await {
        Ok(resp) => match resp.text().await {
            Ok(html) => {
                let doc = scraper::Html::parse_document(&html);
                let result_sel = scraper::Selector::parse(".result__a").unwrap();
                let snippet_sel = scraper::Selector::parse(".result__snippet").unwrap();

                let titles: Vec<String> = doc.select(&result_sel)
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .collect();
                let snippets: Vec<String> = doc.select(&snippet_sel)
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .collect();

                let mut results = Vec::new();
                for (i, title) in titles.iter().enumerate().take(_num as usize) {
                    let snippet = snippets.get(i).cloned().unwrap_or_default();
                    results.push(format!("{}. {}\n   {}", i + 1, title, snippet));
                }

                if results.is_empty() {
                    ToolResult::ok("No results found.")
                } else {
                    ToolResult::ok(results.join("\n\n"))
                }
            }
            Err(e) => ToolResult::err(format!("read response: {e}")),
        },
        Err(e) => ToolResult::err(format!("search request failed: {e}")),
    }
}

async fn get_weather(args: Value) -> ToolResult {
    let location = args.get("location").and_then(|v| v.as_str()).unwrap_or("Hanoi");
    let url = format!("https://wttr.in/{location}?format=j1");
    match reqwest::get(&url).await {
        Ok(resp) => match resp.json::<Value>().await {
            Ok(data) => {
                let current = &data["current_condition"][0];
                let temp = current["temp_C"].as_str().unwrap_or("?");
                let desc = current["weatherDesc"][0]["value"].as_str().unwrap_or("?");
                let humidity = current["humidity"].as_str().unwrap_or("?");
                let wind = current["windspeedKmph"].as_str().unwrap_or("?");
                ToolResult::ok(format!(
                    "{location}: {temp}°C, {desc}, humidity {humidity}%, wind {wind} km/h"
                ))
            }
            Err(e) => ToolResult::err(format!("parse weather: {e}")),
        },
        Err(e) => ToolResult::err(format!("weather request: {e}")),
    }
}

async fn get_news(args: Value) -> ToolResult {
    let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("technology");
    let query = format!("{topic} news");
    web_search(serde_json::json!({"query": query, "num_results": 5})).await
}

mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                b' ' => out.push('+'),
                _ => {
                    out.push('%');
                    out.push_str(&format!("{b:02X}"));
                }
            }
        }
        out
    }
}
