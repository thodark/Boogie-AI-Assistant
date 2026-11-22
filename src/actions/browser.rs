use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{http_client, reencode_bytes_to_jpeg, truncate_chars, validate_public_http_url};
use futures_util::StreamExt;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::Mutex;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "open_url",
            "PRIMARY tool to open any website or URL (e.g. youtube.com, google.com, facebook.com) \
             immediately in the user's real browser with their profile and logins. \
             ALWAYS use this when the user asks to open a website or 'open Brave/Chrome and go to <site>'. \
             Optional 'browser' parameter lets you target a specific browser (e.g. 'brave', 'chrome', 'edge').",
            vec![
                ("url", param("url", "string", "URL or domain to open (e.g. youtube.com)")),
                (
                    "browser",
                    param(
                        "browser",
                        "string",
                        "Optional specific browser (e.g. 'brave', 'chrome', 'edge', 'firefox'). Leave empty for default browser.",
                    ),
                ),
            ],
            vec!["url"],
        ),
        Box::new(|args| Box::pin(open_url(args))),
    );

    registry.register(
        tool(
            "web_scrape",
            "Fetch and extract readable text content from a public http/https URL.",
            vec![("url", param("url", "string", "URL to scrape"))],
            vec!["url"],
        ),
        Box::new(|args| Box::pin(web_scrape(args))),
    );

    registry.register(
        tool(
            "browser_control",
            "Automated Chromium CDP interaction ONLY for multi-step page interaction (clicking elements, \
             filling forms, scrolling, reading dynamic JS pages, taking page screenshots). \
             DO NOT use this just to open a website for the user — use open_url instead (faster and uses the user's real browser). \
             Actions: go_to, search, click, type, scroll, get_text, get_url, press, screenshot, \
             new_tab, close_tab, back, forward, reload, close.",
            vec![
                (
                    "action",
                    param(
                        "action",
                        "string",
                        "go_to|search|click|type|scroll|get_text|get_url|press|screenshot|new_tab|close_tab|back|forward|reload|close",
                    ),
                ),
                ("url", param("url", "string", "URL for go_to/new_tab")),
                ("query", param("query", "string", "Search query")),
                ("selector", param("selector", "string", "CSS selector for click/type")),
                ("text", param("text", "string", "Text for type/click-by-text/press key")),
                (
                    "direction",
                    param("direction", "string", "scroll direction: up/down (default: down)"),
                ),
                (
                    "amount",
                    param("amount", "string", "scroll amount in pixels (default: 500)"),
                ),
            ],
            vec!["action"],
        ),
        Box::new(|args| Box::pin(browser_control(args))),
    );
}

async fn open_url(args: Value) -> ToolResult {
    let raw = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if raw.is_empty() {
        return ToolResult::err("No URL provided.");
    }
    let normalized = if !raw.starts_with("http://") && !raw.starts_with("https://") {
        format!("https://{raw}")
    } else {
        raw.to_string()
    };
    let parsed = match validate_public_http_url(&normalized) {
        Ok(u) => u,
        Err(e) => return ToolResult::err(e),
    };

    let browser = args
        .get("browser")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let url_str = parsed.to_string();
    tokio::task::spawn_blocking(move || {
        if let Some(browser_name) = browser
            && let Some(exe) = super::open_app::find_browser_exe(&browser_name)
        {
            match std::process::Command::new(&exe)
                .arg(&url_str)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    return ToolResult::ok(format!("Opened {url_str} in {browser_name}."));
                }
                Err(e) => {
                    tracing::warn!("failed to launch {browser_name} with url: {e}");
                }
            }
        }
        match open::that(&url_str) {
            Ok(_) => ToolResult::ok(format!("Opened {url_str} in default browser.")),
            Err(e) => ToolResult::err(format!("Failed to open {url_str}: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("open_url task failed: {e}")))
}

async fn web_scrape(args: Value) -> ToolResult {
    let url_raw = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
    let parsed = match validate_public_http_url(url_raw) {
        Ok(u) => u,
        Err(e) => return ToolResult::err(e),
    };

    match http_client().get(parsed.as_str()).send().await {
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
                let body = if clean.len() > 8000 {
                    format!("{}...(truncated)", truncate_chars(&clean, 8000))
                } else {
                    clean
                };
                ToolResult::ok_untrusted("web", body)
            }
            Err(e) => ToolResult::err(format!("Read body: {e}")),
        },
        Err(e) => ToolResult::err(format!("Fetch: {e}")),
    }
}

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

    // Verify existing page is still alive (handles user closing the browser window)
    if let Some(existing_page) = &state.page {
        let alive = tokio::time::timeout(Duration::from_secs(2), existing_page.url())
            .await
            .is_ok_and(|r| r.is_ok());

        if alive {
            return Ok(());
        }
        tracing::warn!("existing Chromium page was closed or unresponsive; relaunching");
        state.page = None;
        state._browser = None;
        if let Some(h) = state._handle.take() {
            h.abort();
        }
    }

    // Keep Chromium sandbox ENABLED on Windows for security
    let launch = chromiumoxide::BrowserConfig::builder()
        .with_head()
        .window_size(1440, 900)
        .arg("--disable-blink-features=AutomationControlled")
        .build()
        .map_err(|e| format!("browser config: {e}"))?;

    // Strict 10s timeout so CDP launch can never freeze the assistant
    let (browser, mut handler) = tokio::time::timeout(
        Duration::from_secs(10),
        chromiumoxide::Browser::launch(launch),
    )
    .await
    .map_err(|_| "browser launch timed out after 10s".to_string())?
    .map_err(|e| format!("browser launch: {e}"))?;

    let handle = tokio::spawn(async move { while let Some(_event) = handler.next().await {} });

    let page = tokio::time::timeout(Duration::from_secs(6), browser.new_page("about:blank"))
        .await
        .map_err(|_| "new_page timed out after 6s".to_string())?
        .map_err(|e| format!("new page: {e}"))?;

    state.page = Some(page);
    state._browser = Some(browser);
    state._handle = Some(handle);

    Ok(())
}

async fn browser_control(args: Value) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();

    if action == "close" {
        let mut state = BROWSER.lock().await;
        state.page = None;
        state._browser = None;
        if let Some(h) = state._handle.take() {
            h.abort();
        }
        return ToolResult::ok("Browser closed.");
    }

    // Fast-path: if no automated CDP browser is open yet and the model only asked to go_to/search/new_tab,
    // open instantly in the user's real default browser (Brave/Chrome/Edge) instead of spinning up blank Chromium!
    if matches!(action.as_str(), "go_to" | "search" | "new_tab") {
        let has_active_cdp = BROWSER.lock().await.page.is_some();
        let target_url = if action == "search" {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if query.is_empty() {
                return ToolResult::err("No search query provided.");
            }
            format!(
                "https://www.google.com/search?q={}",
                urlencoding::encode(query)
            )
        } else {
            let raw = args
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if raw.is_empty() {
                return ToolResult::err("No URL provided.");
            }
            let normalized = if !raw.starts_with("http://") && !raw.starts_with("https://") {
                format!("https://{raw}")
            } else {
                raw
            };
            match validate_public_http_url(&normalized) {
                Ok(u) => u.to_string(),
                Err(e) => return ToolResult::err(e),
            }
        };

        if !has_active_cdp {
            return match open::that(&target_url) {
                Ok(_) => ToolResult::ok(format!("Opened {target_url} in your default browser.")),
                Err(e) => ToolResult::err(format!("Failed to open {target_url}: {e}")),
            };
        }
    }

    if let Err(e) = ensure_browser().await {
        return ToolResult::err(e);
    }

    let mut state = BROWSER.lock().await;

    match action.as_str() {
        "go_to" | "search" | "new_tab" => {
            let url = if action == "search" {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if query.is_empty() {
                    return ToolResult::err("No search query provided.");
                }
                format!(
                    "https://www.google.com/search?q={}",
                    urlencoding::encode(query)
                )
            } else {
                let raw = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let normalized = if !raw.starts_with("http://") && !raw.starts_with("https://") {
                    format!("https://{raw}")
                } else {
                    raw
                };
                match validate_public_http_url(&normalized) {
                    Ok(u) => u.to_string(),
                    Err(e) => return ToolResult::err(e),
                }
            };

            if action == "new_tab"
                && let Some(browser) = &state._browser
            {
                match tokio::time::timeout(Duration::from_secs(8), browser.new_page(&url)).await {
                    Ok(Ok(new_page)) => {
                        state.page = Some(new_page);
                        return ToolResult::ok(format!("New tab opened: {url}"));
                    }
                    Ok(Err(e)) => return ToolResult::err(format!("New tab: {e}")),
                    Err(_) => return ToolResult::err("New tab timed out."),
                }
            }

            let page = state.page.as_ref().unwrap();
            match tokio::time::timeout(Duration::from_secs(10), page.goto(&url)).await {
                Ok(Ok(_)) => {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    let current = page
                        .url()
                        .await
                        .ok()
                        .flatten()
                        .map(|u| u.to_string())
                        .unwrap_or(url);
                    ToolResult::ok(format!("Navigated to {current}"))
                }
                Ok(Err(e)) => ToolResult::err(format!("Navigate: {e}")),
                Err(_) => ToolResult::err("Navigation timed out."),
            }
        }

        "click" => {
            let page = state.page.as_ref().unwrap();
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            if !selector.is_empty() {
                match page.find_element(selector).await {
                    Ok(el) => {
                        let _ = el.click().await;
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        ToolResult::ok(format!("Clicked element: {selector}"))
                    }
                    Err(e) => ToolResult::err(format!("Element not found '{selector}': {e}")),
                }
            } else if !text.is_empty() {
                let needle_json =
                    serde_json::to_string(&text.to_lowercase()).unwrap_or_else(|_| "\"\"".into());
                let js = format!(
                    r#"(() => {{
                        const needle = {needle_json};
                        const els = [...document.querySelectorAll('a, button, [role="button"], input[type="submit"]')];
                        const el = els.find(e => (e.textContent || '').trim().toLowerCase().includes(needle));
                        if (el) {{ el.click(); return 'clicked'; }}
                        return 'not found';
                    }})()"#
                );
                match page.evaluate(js).await {
                    Ok(val) => {
                        let result = val.into_value::<String>().unwrap_or_default();
                        if result == "clicked" {
                            tokio::time::sleep(Duration::from_millis(400)).await;
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
            let page = state.page.as_ref().unwrap();
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if selector.is_empty() {
                return ToolResult::err("No selector provided for type.");
            }
            match page.find_element(selector).await {
                Ok(el) => {
                    let _ = el.click().await;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    match el.type_str(text).await {
                        Ok(_) => ToolResult::ok(format!("Typed into {selector}")),
                        Err(e) => ToolResult::err(format!("Type: {e}")),
                    }
                }
                Err(e) => ToolResult::err(format!("Element not found '{selector}': {e}")),
            }
        }

        "scroll" => {
            let page = state.page.as_ref().unwrap();
            let direction = args
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("down");
            let amount: i32 = args
                .get("amount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(500);
            let delta = if direction == "up" { -amount } else { amount };
            let js = format!("window.scrollBy(0, {delta})");
            let _ = page.evaluate(js).await;
            ToolResult::ok(format!("Scrolled {direction} {amount}px"))
        }

        "get_text" => {
            let page = state.page.as_ref().unwrap();
            match page
                .evaluate("document.body ? document.body.innerText : ''")
                .await
            {
                Ok(val) => {
                    let text = val.into_value::<String>().unwrap_or_default();
                    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    let body = if clean.len() > 6000 {
                        format!("{}...(truncated)", truncate_chars(&clean, 6000))
                    } else {
                        clean
                    };
                    ToolResult::ok_untrusted("web", body)
                }
                Err(e) => ToolResult::err(format!("Get text: {e}")),
            }
        }

        "get_url" => {
            let page = state.page.as_ref().unwrap();
            match page.url().await {
                Ok(Some(url)) => ToolResult::ok(url.to_string()),
                Ok(None) => ToolResult::ok("about:blank".to_string()),
                Err(e) => ToolResult::err(format!("Get URL: {e}")),
            }
        }

        "press" => {
            let page = state.page.as_ref().unwrap();
            let key = args.get("text").and_then(|v| v.as_str()).unwrap_or("Enter");
            let key_json = serde_json::to_string(key).unwrap_or_else(|_| "\"Enter\"".into());
            let js = format!(
                r#"(() => {{
                    const k = {key_json};
                    const target = document.activeElement || document;
                    target.dispatchEvent(new KeyboardEvent('keydown', {{key: k, bubbles: true}}));
                    target.dispatchEvent(new KeyboardEvent('keyup', {{key: k, bubbles: true}}));
                }})()"#
            );
            match page.evaluate(js).await {
                Ok(_) => ToolResult::ok(format!("Pressed {key}")),
                Err(e) => ToolResult::err(format!("Press: {e}")),
            }
        }

        "screenshot" => {
            let page = state.page.as_ref().unwrap();
            match page
                .screenshot(chromiumoxide::page::ScreenshotParams::default())
                .await
            {
                Ok(bytes) => match reencode_bytes_to_jpeg(&bytes, 1280, 80) {
                    Ok((jpeg, w, h)) => {
                        let kb = jpeg.len() / 1024;
                        ToolResult::ok_with_image(
                            format!(
                                "Browser screenshot captured ({w}x{h}, {kb} KB JPEG) and sent to vision input."
                            ),
                            jpeg,
                        )
                    }
                    Err(e) => ToolResult::err(format!("Screenshot encode: {e}")),
                },
                Err(e) => ToolResult::err(format!("Screenshot: {e}")),
            }
        }

        "close_tab" => {
            let page = state.page.as_ref().unwrap();
            let _ = page
                .execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
                .await;
            state.page = None;
            ToolResult::ok("Tab closed.")
        }

        "back" => {
            let page = state.page.as_ref().unwrap();
            let _ = page.evaluate("window.history.back()").await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            ToolResult::ok("Navigated back.")
        }

        "forward" => {
            let page = state.page.as_ref().unwrap();
            let _ = page.evaluate("window.history.forward()").await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            ToolResult::ok("Navigated forward.")
        }

        "reload" => {
            let page = state.page.as_ref().unwrap();
            let _ = page.reload().await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            ToolResult::ok("Page reloaded.")
        }

        other => ToolResult::err(format!("Unknown browser action: '{other}'")),
    }
}
