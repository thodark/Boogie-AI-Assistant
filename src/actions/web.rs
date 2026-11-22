use crate::tools::{ToolRegistry, ToolResult};
use crate::util::http_client;
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "web_search",
            "Search the web using DuckDuckGo and return titles, URLs, and snippets.",
            vec![
                ("query", param("query", "string", "Search query")),
                (
                    "num_results",
                    param("num_results", "integer", "Number of results (default 5)"),
                ),
            ],
            vec!["query"],
        ),
        Box::new(|args| Box::pin(web_search(args))),
    );

    registry.register(
        tool(
            "get_weather",
            "Get current weather for a city or location.",
            vec![(
                "location",
                param("location", "string", "City name or coordinates"),
            )],
            vec!["location"],
        ),
        Box::new(|args| Box::pin(get_weather(args))),
    );

    registry.register(
        tool(
            "get_news",
            "Get latest news headlines for a topic.",
            vec![("topic", param("topic", "string", "News topic"))],
            vec![],
        ),
        Box::new(|args| Box::pin(get_news(args))),
    );
}

fn extract_ddg_url(href: &str) -> String {
    if let Some(idx) = href.find("uddg=") {
        let rest = &href[idx + 5..];
        let encoded = rest.split('&').next().unwrap_or(rest);
        if let Ok(decoded) = urlencoding::decode(encoded) {
            return decoded.into_owned();
        }
    }
    if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    }
}

async fn web_search(args: Value) -> ToolResult {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if query.is_empty() {
        return ToolResult::err("No search query provided.");
    }
    let num = args
        .get("num_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    match http_client()
        .get(&url)
        .header("Accept-Language", "en-US,en;q=0.9,vi;q=0.8")
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(html) => {
                let doc = scraper::Html::parse_document(&html);
                let result_sel = scraper::Selector::parse(".result__a").unwrap();
                let snippet_sel = scraper::Selector::parse(".result__snippet").unwrap();

                let entries: Vec<(String, String)> = doc
                    .select(&result_sel)
                    .map(|el| {
                        let title = el.text().collect::<String>().trim().to_string();
                        let href = el.value().attr("href").unwrap_or_default();
                        (title, extract_ddg_url(href))
                    })
                    .collect();

                let snippets: Vec<String> = doc
                    .select(&snippet_sel)
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .collect();

                let mut results = Vec::new();
                for (i, (title, link)) in entries.iter().enumerate().take(num) {
                    let snippet = snippets.get(i).cloned().unwrap_or_default();
                    if link.is_empty() {
                        results.push(format!("{}. {}\n   {}", i + 1, title, snippet));
                    } else {
                        results.push(format!(
                            "{}. {}\n   URL: {}\n   {}",
                            i + 1,
                            title,
                            link,
                            snippet
                        ));
                    }
                }

                if results.is_empty() {
                    ToolResult::ok(
                        "No results found (DuckDuckGo may have rate-limited this request).",
                    )
                } else {
                    ToolResult::ok_untrusted("web", results.join("\n\n"))
                }
            }
            Err(e) => ToolResult::err(format!("read response: {e}")),
        },
        Err(e) => ToolResult::err(format!("search request failed: {e}")),
    }
}

async fn get_weather(args: Value) -> ToolResult {
    let location = args
        .get("location")
        .and_then(|v| v.as_str())
        .unwrap_or("Hanoi")
        .trim();
    let encoded = urlencoding::encode(location);
    let url = format!("https://wttr.in/{encoded}?format=j1");
    match http_client().get(&url).send().await {
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
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("technology");
    let query = format!("{topic} news");
    web_search(serde_json::json!({"query": query, "num_results": 5})).await
}
