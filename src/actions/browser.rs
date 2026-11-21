use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "open_url",
            "Open a URL in the default browser (native, with user's profile and logins)",
            vec![("url", param("url", "string", "URL to open"))],
            vec!["url"],
        ),
        Box::new(|args| Box::pin(open_url(args))),
    );

    registry.register(
        tool(
            "web_scrape",
            "Scrape text content from a URL",
            vec![("url", param("url", "string", "URL to scrape"))],
            vec!["url"],
        ),
        Box::new(|args| Box::pin(web_scrape(args))),
    );

    registry.register(
        tool(
            "browser_control",
            "Controls the web browser: open an address, search, click, type, \
             scroll, read the page, manage tabs, go back and forward. \
             THIS TOOL OPENS THE BROWSER ITSELF. Never call open_app for the \
             browser first. Actions: go_to, search, click, type, scroll, get_text, \
             get_url, press, screenshot, new_tab, close_tab, back, forward, reload, close.",
            vec![
                ("action", param("action", "string", "go_to|search|click|type|scroll|get_text|get_url|press|screenshot|new_tab|close_tab|back|forward|reload|close")),
                ("url", param("url", "string", "URL for go_to/new_tab")),
                ("query", param("query", "string", "Search query")),
                ("selector", param("selector", "string", "CSS selector for click/type")),
                ("text", param("text", "string", "Text for type/click-by-text/press key")),
                ("direction", param("direction", "string", "scroll direction: up/down (default: down)")),
                ("amount", param("amount", "string", "scroll amount in pixels (default: 500)")),
            ],
            vec!["action"],
        ),
        Box::new(|args| Box::pin(browser_control(args))),
    );
}

async fn open_url(args: Value) -> ToolResult {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
    match open::that(url) {
        Ok(_) => ToolResult::ok(format!("Opened {url} in default browser.")),
        Err(e) => ToolResult::err(format!("Failed to open {url}: {e}")),
    }
}

async fn web_scrape(args: Value) -> ToolResult {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .build()
        .unwrap_or_default();
    match client.get(url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(html) => {
                let doc = scraper::Html::parse_document(&html);
                let sel = scraper::Selector::parse("body").unwrap();
                let text: String = doc
                    .select(&sel)
                    .flat_map(|el| el.text())
                    .collect::<Vec<_>>()
                    .join(" ");
                let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let truncated = if clean.len() > 8000 {
                    format!("{}...(truncated)", &clean[..8000])
                } else {
                    clean
                };
                ToolResult::ok(truncated)
            }
            Err(e) => ToolResult::err(format!("Read body: {e}")),
        },
        Err(e) => ToolResult::err(format!("Fetch: {e}")),
    }
}

use std::sync::LazyLock;

struct BrowserState {
    page: Option<chromiumoxide::Page>,
    _browser: Option<chromiumoxide::Browser>,
    _handle: Option<tokio::task::JoinHandle<()>>,
}

static BROWSER: LazyLock<Arc<Mutex<BrowserState>>> = LazyLock::new(|| {
    Arc::new(Mutex::new(BrowserState {
        page: None,
        _browser: None,
        _handle: None,
    }))
});

async fn ensure_browser() -> Result<(), String> {
    let mut state = BROWSER.lock().await;
    if state.page.is_some() {
        return Ok(());
    }

    let launch = chromiumoxide::BrowserConfig::builder()
        .no_sandbox()
        .window_size(1920, 1080)
        .arg("--disable-blink-features=AutomationControlled")
        .build()
        .map_err(|e| format!("browser config: {e}"))?;

    let (browser, mut handler) = chromiumoxide::Browser::launch(launch)
        .await
        .map_err(|e| format!("browser launch: {e}"))?;

    let handle = tokio::spawn(async move {
        while let Some(_event) = handler.next().await {}
    });

    let page = browser
        .new_page("about:blank")
        .await
        .map_err(|e| format!("new page: {e}"))?;

    state.page = Some(page);
    state._browser = Some(browser);
    state._handle = Some(handle);

    Ok(())
}

use futures_util::StreamExt;

async fn browser_control(args: Value) -> ToolResult {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();

    if action == "go_to" || action == "search" || action == "new_tab" {
        let url = if action == "search" {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
            if query.is_empty() {
                return ToolResult::err("No search query provided.");
            }
            format!("https://www.google.com/search?q={}", urlencoding::encode(query))
        } else {
            let raw = args.get("url").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
            if raw.is_empty() {
                return ToolResult::err("No URL provided.");
            }
            if !raw.starts_with("http://") && !raw.starts_with("https://") {
                format!("https://{raw}")
            } else {
                raw
            }
        };

        let state = BROWSER.lock().await;
        if state.page.is_none() {
            drop(state);
            match open::that(&url) {
                Ok(_) => return ToolResult::ok(format!("Opened {url} in default browser (native).")),
                Err(e) => return ToolResult::err(format!("Failed to open: {e}")),
            }
        }

        let page = state.page.as_ref().unwrap();
        if action == "new_tab" {
            if let Some(browser) = &state._browser {
                match browser.new_page(&url).await {
                    Ok(_p) => return ToolResult::ok(format!("New tab opened: {url}")),
                    Err(e) => return ToolResult::err(format!("New tab: {e}")),
                }
            }
        }

        match page.goto(&url).await {
            Ok(_) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let current = page.url().await.ok().flatten().map(|u| u.to_string()).unwrap_or_default();
                ToolResult::ok(format!("Navigated to {current}"))
            }
            Err(e) => ToolResult::err(format!("Navigate: {e}")),
        }
    } else if action == "close" {
        let mut state = BROWSER.lock().await;
        state.page = None;
        state._browser = None;
        state._handle = None;
        ToolResult::ok("Browser closed.")
    } else {
        if let Err(e) = ensure_browser().await {
            return ToolResult::err(e);
        }

        let state = BROWSER.lock().await;
        let page = state.page.as_ref().unwrap();

        match action.as_str() {
            "click" => {
                let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or_default();
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();

                if !selector.is_empty() {
                    match page.find_element(selector).await {
                        Ok(el) => {
                            let _ = el.click().await;
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            ToolResult::ok(format!("Clicked element: {selector}"))
                        }
                        Err(e) => ToolResult::err(format!("Element not found '{selector}': {e}")),
                    }
                } else if !text.is_empty() {
                    let js = format!(
                        r#"(() => {{
                            const els = [...document.querySelectorAll('a, button, [role="button"], input[type="submit"]')];
                            const el = els.find(e => e.textContent.trim().toLowerCase().includes('{}'));
                            if (el) {{ el.click(); return 'clicked'; }}
                            return 'not found';
                        }})()"#,
                        text.to_lowercase().replace('\'', "\\'")
                    );
                    match page.evaluate(js).await {
                        Ok(val) => {
                            let result = val.into_value::<String>().unwrap_or_default();
                            if result == "clicked" {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                ToolResult::ok(format!("Clicked element with text: {text}"))
                            } else {
                                ToolResult::err(format!("No clickable element found with text: {text}"))
                            }
                        }
                        Err(e) => ToolResult::err(format!("Click by text: {e}")),
                    }
                } else {
                    ToolResult::err("Provide either 'selector' or 'text' for click.")
                }
            }

            "type" => {
                let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or_default();
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                if selector.is_empty() {
                    return ToolResult::err("No selector provided for type.");
                }
                match page.find_element(selector).await {
                    Ok(el) => {
                        let _ = el.click().await;
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        match el.type_str(text).await {
                            Ok(_) => ToolResult::ok(format!("Typed into {selector}")),
                            Err(e) => ToolResult::err(format!("Type: {e}")),
                        }
                    }
                    Err(e) => ToolResult::err(format!("Element not found '{selector}': {e}")),
                }
            }

            "scroll" => {
                let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("down");
                let amount: i32 = args.get("amount").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(500);
                let delta = if direction == "up" { -amount } else { amount };
                let js = format!("window.scrollBy(0, {delta})");
                let _ = page.evaluate(js).await;
                ToolResult::ok(format!("Scrolled {direction} {amount}px"))
            }

            "get_text" => {
                let js = "document.body.innerText".to_string();
                match page.evaluate(js).await {
                    Ok(val) => {
                        let text = val.into_value::<String>().unwrap_or_default();
                        let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        let truncated = if clean.len() > 6000 { format!("{}...(truncated)", &clean[..6000]) } else { clean };
                        ToolResult::ok(truncated)
                    }
                    Err(e) => ToolResult::err(format!("Get text: {e}")),
                }
            }

            "get_url" => {
                match page.url().await {
                    Ok(Some(url)) => ToolResult::ok(url.to_string()),
                    Ok(None) => ToolResult::ok("about:blank".to_string()),
                    Err(e) => ToolResult::err(format!("Get URL: {e}")),
                }
            }

            "press" => {
                let key = args.get("text").and_then(|v| v.as_str()).unwrap_or("Enter");
                let js = format!(
                    r#"document.dispatchEvent(new KeyboardEvent('keydown', {{key: '{}', bubbles: true}}));
                       document.dispatchEvent(new KeyboardEvent('keyup', {{key: '{}', bubbles: true}}))"#,
                    key, key
                );
                match page.evaluate(js).await {
                    Ok(_) => ToolResult::ok(format!("Pressed {key}")),
                    Err(e) => ToolResult::err(format!("Press: {e}")),
                }
            }

            "screenshot" => {
                match page.screenshot(chromiumoxide::page::ScreenshotParams::default()).await {
                    Ok(bytes) => {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        ToolResult::ok(format!("Screenshot taken ({} bytes, base64 encoded). data:image/png;base64,{}", bytes.len(), &b64[..b64.len().min(100)]))
                    }
                    Err(e) => ToolResult::err(format!("Screenshot: {e}")),
                }
            }

            "close_tab" => {
                let _ = page.execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default()).await;
                ToolResult::ok("Tab closed.")
            }

            "back" => {
                let js = "window.history.back()";
                let _ = page.evaluate(js).await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                ToolResult::ok("Navigated back.")
            }

            "forward" => {
                let js = "window.history.forward()";
                let _ = page.evaluate(js).await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                ToolResult::ok("Navigated forward.")
            }

            "reload" => {
                let _ = page.reload().await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                ToolResult::ok("Page reloaded.")
            }

            other => ToolResult::err(format!("Unknown browser action: '{other}'")),
        }
    }
}
