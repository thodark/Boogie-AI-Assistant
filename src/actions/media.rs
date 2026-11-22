use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "set_wallpaper",
            "Set the desktop wallpaper to an image file path.",
            vec![("path", param("path", "string", "Path to image file"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(set_wallpaper(args))),
    );

    registry.register(
        tool(
            "clipboard_read",
            "Read text from the system clipboard.",
            vec![],
            vec![],
        ),
        Box::new(|args| Box::pin(clipboard_read(args))),
    );

    registry.register(
        tool(
            "clipboard_write",
            "Write text to the system clipboard.",
            vec![("text", param("text", "string", "Text to copy"))],
            vec!["text"],
        ),
        Box::new(|args| Box::pin(clipboard_write(args))),
    );
}

async fn set_wallpaper(args: Value) -> ToolResult {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || {
            use std::ffi::OsStr;
            use std::os::windows::ffi::OsStrExt;
            use windows::Win32::UI::WindowsAndMessaging::*;
            let wide: Vec<u16> = OsStr::new(&path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            unsafe {
                match SystemParametersInfoW(
                    SPI_SETDESKWALLPAPER,
                    0,
                    Some(wide.as_ptr() as *mut _),
                    SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
                ) {
                    Ok(_) => ToolResult::ok(format!("wallpaper set to {path}")),
                    Err(e) => ToolResult::err(format!("set wallpaper failed: {e}")),
                }
            }
        })
        .await
        .unwrap_or_else(|e| ToolResult::err(format!("set_wallpaper task failed: {e}")))
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        ToolResult::err("set_wallpaper is Windows-only")
    }
}

async fn clipboard_read(_args: Value) -> ToolResult {
    tokio::task::spawn_blocking(|| match arboard::Clipboard::new() {
        Ok(mut cb) => match cb.get_text() {
            Ok(text) => ToolResult::ok_untrusted("clipboard", text),
            Err(e) => ToolResult::err(format!("clipboard read: {e}")),
        },
        Err(e) => ToolResult::err(format!("clipboard init: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("clipboard_read task failed: {e}")))
}

async fn clipboard_write(args: Value) -> ToolResult {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || match arboard::Clipboard::new() {
        Ok(mut cb) => match cb.set_text(text) {
            Ok(_) => ToolResult::ok("copied to clipboard"),
            Err(e) => ToolResult::err(format!("clipboard write: {e}")),
        },
        Err(e) => ToolResult::err(format!("clipboard init: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("clipboard_write task failed: {e}")))
}
