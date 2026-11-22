use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{http_client, reencode_bytes_to_jpeg, truncate_chars, validate_public_http_url};
use futures_util::StreamExt;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "open_url",
            "Open a public http/https URL in the user's default browser (with their profile and logins).",
            vec![("url", param("url", "string", "URL to open"))],
            vec!["url"],
        ),
        Box::new(|args| Box::pin(open_url(args))),
    );

    registry.register(
        tool(
            "web_scrape",
            "Scrape readable text content from a public http/https URL.",
            vec![("url", param("url", "string", "URL to scrape"))],
            vec!["url"],
        ),
        Box::new(|args| Box::pin(web_scrape(args))),
    );

    registry.register(
        tool(
            "browser_control",
            "Controls an automated Chromium browser session: open an address, search, click, type, \
             scroll, read the page, take a vision screenshot, manage tabs, go back/forward. \
             Use open_url instead if you simply want to open a link in the user's normal browser. \
             Actions: go_to, search, click, type, scroll, get_text, get_url, press, screenshot, \
             new_tab, close_tab, back, forward, reload, close.",
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
    let raw = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    let normalized = if !raw.starts_with("http://") && !raw.starts_with("https://") {
        format!("https://{raw}")
    } else {
        raw.to_string()
    };
    let parsed = match validate_public_http_url(&normalized) {
        Ok(u) => u,
        Err(e) => return ToolResult::err(e),
    };
    match open::that(parsed.as_str()) {
        Ok(_) => ToolResult::ok(format!("Opened {} in default browser.", parsed.as_str())),
        Err(e) => ToolResult::err(format!("Failed to open {}: {e}", parsed.as_str())),
    }
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
        let alive = tokio::time::timeout(std::time::Duration::from_secs(3), existing_page.url())
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

    let (browser, mut handler) = chromiumoxide::Browser::launch(launch)
        .await
        .map_err(|e| format!("browser launch: {e}"))?;

    let handle = tokio::spawn(async move { while let Some(_event) = handler.next().await {} });

    let page = browser
        .new_page("about:blank")
        .await
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

            if action == "new_tab"
                && let Some(browser) = &state._browser
            {
                match browser.new_page(&url).await {
                    Ok(new_page) => {
                        state.page = Some(new_page);
                        return ToolResult::ok(format!("New tab opened: {url}"));
                    }
                    Err(e) => return ToolResult::err(format!("New tab: {e}")),
                }
            }

            let page = state.page.as_ref().unwrap();
            match page.goto(&url).await {
                Ok(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                    let current = page
                        .url()
                        .await
                        .ok()
                        .flatten()
                        .map(|u| u.to_string())
                        .unwrap_or(url);
                    ToolResult::ok(format!("Navigated to {current}"))
                }
                Err(e) => ToolResult::err(format!("Navigate: {e}")),
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
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            ToolResult::ok("Navigated back.")
        }

        "forward" => {
            let page = state.page.as_ref().unwrap();
            let _ = page.evaluate("window.history.forward()").await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            ToolResult::ok("Navigated forward.")
        }

        "reload" => {
            let page = state.page.as_ref().unwrap();
            let _ = page.reload().await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            ToolResult::ok("Page reloaded.")
        }

        other => ToolResult::err(format!("Unknown browser action: '{other}'")),
    }
}
