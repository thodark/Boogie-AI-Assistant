use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::time::Duration;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "send_message",
            "Send a message to someone on a messaging platform. \
             Supports: WhatsApp, Telegram, Discord, Signal, Instagram, Messenger. \
             Opens the app, searches for the contact, types and sends the message via desktop automation.",
            vec![
                ("receiver", param("receiver", "string", "Contact name or phone number")),
                ("message_text", param("message_text", "string", "Message to send")),
                ("platform", param("platform", "string", "whatsapp | telegram | discord | signal | instagram | messenger (default: whatsapp)")),
            ],
            vec!["receiver", "message_text"],
        ),
        Box::new(|args| Box::pin(send_message(args))),
    );
}

fn resolve_platform(raw: &str) -> (&str, PlatformKind) {
    let lower = raw.to_lowercase();
    if lower.contains("whatsapp") || lower.contains("wp") || lower.contains("wapp") {
        ("WhatsApp", PlatformKind::DesktopApp)
    } else if lower.contains("telegram") || lower.contains("tg") {
        ("Telegram", PlatformKind::DesktopApp)
    } else if lower.contains("discord") {
        ("Discord", PlatformKind::DesktopApp)
    } else if lower.contains("signal") {
        ("Signal", PlatformKind::DesktopApp)
    } else if lower.contains("instagram") || lower.contains("ig") || lower.contains("insta") {
        ("Instagram", PlatformKind::BrowserUrl("https://www.instagram.com/direct/new/"))
    } else if lower.contains("messenger") || lower.contains("facebook") || lower.contains("fb") {
        ("Messenger", PlatformKind::BrowserUrl("https://www.messenger.com/"))
    } else {
        (raw, PlatformKind::DesktopApp)
    }
}

enum PlatformKind {
    DesktopApp,
    BrowserUrl(&'static str),
}

#[cfg(windows)]
fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[cfg(windows)]
fn press_key(vk: u16) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn hotkey(modifier: u16, key: u16) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let mod_down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(modifier),
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let key_down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(key),
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let key_up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(key),
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mod_up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(modifier),
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[mod_down, key_down, key_up, mod_up], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn type_unicode(text: &str) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    for ch in text.encode_utf16() {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: ch,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: ch,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn paste_text(text: &str) {
    if let Ok(mut clip) = arboard::Clipboard::new() {
        if clip.set_text(text).is_ok() {
            sleep_ms(150);
            hotkey(0x11, 0x56); // Ctrl+V
            sleep_ms(100);
            return;
        }
    }
    type_unicode(text);
}

#[cfg(windows)]
fn open_app_via_start(app_name: &str) -> bool {
    press_key(0x5B); // Win key
    sleep_ms(500);
    paste_text(app_name);
    sleep_ms(600);
    press_key(0x0D); // Enter
    sleep_ms(2500);
    true
}

#[cfg(windows)]
fn open_browser_url(url: &str) -> bool {
    match open::that(url) {
        Ok(()) => {
            sleep_ms(4000);
            true
        }
        Err(_) => false,
    }
}

#[cfg(windows)]
fn search_in_app(query: &str) {
    hotkey(0x11, 0x46); // Ctrl+F
    sleep_ms(500);
    hotkey(0x11, 0x41); // Ctrl+A
    sleep_ms(100);
    press_key(0x2E); // Delete
    sleep_ms(100);
    paste_text(query);
    sleep_ms(1000);
}

#[cfg(windows)]
fn desktop_send(app_name: &str, receiver: &str, message: &str) -> ToolResult {
    if !open_app_via_start(app_name) {
        return ToolResult::err(format!(
            "{app_name} could not be opened, so nothing was typed and no message was sent."
        ));
    }

    sleep_ms(1000);

    search_in_app(receiver);
    press_key(0x0D); // Enter
    sleep_ms(800);

    paste_text(message);
    sleep_ms(200);
    press_key(0x0D); // Enter
    sleep_ms(300);

    ToolResult::ok(format!(
        "Message typed into {app_name} for '{receiver}'. \
         This was done via blind keystrokes — delivery is NOT confirmed."
    ))
}

#[cfg(windows)]
fn browser_send(url: &str, platform: &str, receiver: &str, message: &str) -> ToolResult {
    if !open_browser_url(url) {
        return ToolResult::err(format!(
            "{platform} could not be opened in the browser."
        ));
    }

    paste_text(receiver);
    sleep_ms(1500);

    press_key(0x28); // Down arrow
    sleep_ms(300);
    press_key(0x0D); // Enter
    sleep_ms(400);

    if platform == "Instagram" {
        for _ in 0..4 {
            press_key(0x09); // Tab
            sleep_ms(150);
        }
        press_key(0x0D); // Enter
        sleep_ms(2000);
    }

    paste_text(message);
    sleep_ms(200);
    press_key(0x0D); // Enter
    sleep_ms(300);

    ToolResult::ok(format!(
        "Message typed into {platform} in the browser for '{receiver}'. \
         This was done via blind keystrokes — delivery is NOT confirmed."
    ))
}

async fn send_message(args: Value) -> ToolResult {
    let receiver = args.get("receiver").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let message_text = args.get("message_text").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let platform_raw = args.get("platform").and_then(|v| v.as_str()).unwrap_or("whatsapp").trim().to_string();

    if receiver.is_empty() {
        return ToolResult::err("No recipient was given.");
    }
    if message_text.is_empty() {
        return ToolResult::err("No message text was given.");
    }

    #[cfg(windows)]
    {
        let (name, kind) = resolve_platform(&platform_raw);
        let name = name.to_string();
        let recv = receiver.clone();
        let msg = message_text.clone();

        let result = tokio::task::spawn_blocking(move || {
            match kind {
                PlatformKind::DesktopApp => desktop_send(&name, &recv, &msg),
                PlatformKind::BrowserUrl(url) => browser_send(url, &name, &recv, &msg),
            }
        })
        .await;

        match result {
            Ok(r) => r,
            Err(e) => ToolResult::err(format!("Send failed: {e}")),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (receiver, message_text, platform_raw);
        ToolResult::err("send_message is currently Windows-only.")
    }
}
