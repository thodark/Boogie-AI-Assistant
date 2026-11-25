use crate::tools::{ToolRegistry, ToolResult};
use crate::util::encode_rgba_to_jpeg;
use crate::{AppEvent, ChimeKind};
use serde_json::Value;
use tokio::sync::broadcast;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    registry.register(
        tool(
            "screen_process",
            "Quickly look at the user's computer screen for general UI awareness or casual questions \
             (what app is open, where a button is, what's playing). \
             DO NOT use this for math, science, homework, exams, quizzes, code debugging, or reading \
             dense text/formulas — ALWAYS call study_solve instead for high-resolution lossless OCR \
             and step-by-step verified reasoning.",
            vec![("monitor", param("monitor", "integer", "Monitor index (0-based, default 0)"))],
            vec![],
        ),
        Box::new(move |args| {
            let tx = event_tx.clone();
            Box::pin(capture_screen_vision(args, tx))
        }),
    );

    registry.register(
        tool(
            "click_screen",
            "Click at screen coordinates (matching the DPI-aware physical pixels of the screen).",
            vec![
                ("x", param("x", "integer", "X coordinate")),
                ("y", param("y", "integer", "Y coordinate")),
                (
                    "button",
                    param("button", "string", "left, right, or middle (default: left)"),
                ),
                (
                    "monitor",
                    param(
                        "monitor",
                        "integer",
                        "Optional 0-based monitor index if coordinates are relative to a monitor",
                    ),
                ),
            ],
            vec!["x", "y"],
        ),
        Box::new(|args| Box::pin(click_screen(args))),
    );

    registry.register(
        tool(
            "type_text",
            "Type text using keyboard simulation (supports full Unicode / Vietnamese).",
            vec![("text", param("text", "string", "Text to type"))],
            vec!["text"],
        ),
        Box::new(|args| Box::pin(type_text(args))),
    );

    registry.register(
        tool(
            "press_key",
            "Press a keyboard key or combination (e.g. ctrl+c, alt+tab, enter).",
            vec![(
                "keys",
                param("keys", "string", "Key combination like ctrl+c"),
            )],
            vec!["keys"],
        ),
        Box::new(|args| Box::pin(press_key(args))),
    );
}

async fn capture_screen_vision(args: Value, event_tx: broadcast::Sender<AppEvent>) -> ToolResult {
    let monitor = args.get("monitor").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    tokio::task::spawn_blocking(move || match screenshots::Screen::all() {
        Ok(screens) => {
            if let Some(screen) = screens.get(monitor) {
                let info = screen.display_info;
                match screen.capture() {
                    Ok(img) => {
                        // Fire shutter chime the instant raw screen pixels are safely captured!
                        let _ = event_tx.send(AppEvent::PlayChime(ChimeKind::ScreenCaptured));
                        let (orig_w, orig_h) = (img.width(), img.height());
                        let cap_id = crate::actions::study::next_capture_seq();
                        let win_title = crate::actions::study::get_foreground_window_title();
                        match encode_rgba_to_jpeg(img.as_raw(), orig_w, orig_h, 1920, 92) {
                            Ok((jpeg, out_w, out_h)) => {
                                let kb = jpeg.len() / 1024;
                                ToolResult::ok_with_image(
                                    format!(
                                        "Here is the brand-new screenshot (capture #{cap_id}, active window: '{win_title}', monitor {monitor}, {out_w}x{out_h}, {kb} KB JPEG, scale {:.2}). \
                                         Ignore any previous screenshots or older tabs and look only at this image. \
                                         Speak aloud naturally and concisely in the user's language (no markdown or LaTeX) to answer their question.",
                                        info.scale_factor
                                    ),
                                    jpeg,
                                )
                            }
                            Err(e) => ToolResult::err(format!("jpeg encode: {e}")),
                        }
                    }
                    Err(e) => ToolResult::err(format!("capture failed: {e}")),
                }
            } else {
                ToolResult::err(format!("monitor {monitor} not found ({} available)", screens.len()))
            }
        }
        Err(e) => ToolResult::err(format!("screen list failed: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("screen capture task failed: {e}")))
}

async fn click_screen(args: Value) -> ToolResult {
    let raw_x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let raw_y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let button = args
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left")
        .to_lowercase();
    let monitor_idx = args
        .get("monitor")
        .and_then(|v| v.as_u64())
        .map(|m| m as usize);

    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || {
            use windows::Win32::UI::Input::KeyboardAndMouse::*;
            use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

            // Translate monitor-relative coordinates using physical monitor origin when specified
            let (x, y) = if let Some(idx) = monitor_idx {
                if let Ok(screens) = screenshots::Screen::all() {
                    if let Some(scr) = screens.get(idx) {
                        let di = scr.display_info;
                        (di.x + raw_x, di.y + raw_y)
                    } else {
                        (raw_x, raw_y)
                    }
                } else {
                    (raw_x, raw_y)
                }
            } else {
                (raw_x, raw_y)
            };

            let (down_flag, up_flag) = match button.as_str() {
                "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            };

            unsafe {
                let _ = SetCursorPos(x, y);
                let down = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dwFlags: down_flag,
                            ..Default::default()
                        },
                    },
                };
                let up = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dwFlags: up_flag,
                            ..Default::default()
                        },
                    },
                };
                SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
            }
            ToolResult::ok(format!("clicked ({x}, {y}) {button}"))
        })
        .await
        .unwrap_or_else(|e| ToolResult::err(format!("click task failed: {e}")))
    }

    #[cfg(not(windows))]
    {
        let _ = (raw_x, raw_y, button, monitor_idx);
        ToolResult::err("click_screen is Windows-only")
    }
}

async fn type_text(args: Value) -> ToolResult {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || {
            use windows::Win32::UI::Input::KeyboardAndMouse::*;
            for ch in text.encode_utf16() {
                unsafe {
                    let down = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wScan: ch,
                                dwFlags: KEYEVENTF_UNICODE,
                                ..Default::default()
                            },
                        },
                    };
                    let up = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wScan: ch,
                                dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                                ..Default::default()
                            },
                        },
                    };
                    SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
                }
            }
            ToolResult::ok(format!("typed {} chars", text.chars().count()))
        })
        .await
        .unwrap_or_else(|e| ToolResult::err(format!("type task failed: {e}")))
    }

    #[cfg(not(windows))]
    {
        let _ = text;
        ToolResult::err("type_text is Windows-only")
    }
}

async fn press_key(args: Value) -> ToolResult {
    let keys = args
        .get("keys")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || {
            use windows::Win32::UI::Input::KeyboardAndMouse::*;

            fn vk_from_name(name: &str) -> u16 {
                match name.to_lowercase().as_str() {
                    "enter" | "return" => 0x0D,
                    "tab" => 0x09,
                    "escape" | "esc" => 0x1B,
                    "space" => 0x20,
                    "backspace" => 0x08,
                    "delete" | "del" => 0x2E,
                    "up" => 0x26,
                    "down" => 0x28,
                    "left" => 0x25,
                    "right" => 0x27,
                    "home" => 0x24,
                    "end" => 0x23,
                    "pageup" => 0x21,
                    "pagedown" => 0x22,
                    "ctrl" | "control" => 0x11,
                    "alt" => 0x12,
                    "shift" => 0x10,
                    "win" | "windows" | "super" => 0x5B,
                    "f1" => 0x70,
                    "f2" => 0x71,
                    "f3" => 0x72,
                    "f4" => 0x73,
                    "f5" => 0x74,
                    "f6" => 0x75,
                    "f7" => 0x76,
                    "f8" => 0x77,
                    "f9" => 0x78,
                    "f10" => 0x79,
                    "f11" => 0x7A,
                    "f12" => 0x7B,
                    s if s.len() == 1 => s.chars().next().unwrap().to_ascii_uppercase() as u16,
                    _ => 0,
                }
            }

            let parts: Vec<&str> = keys.split('+').collect();
            let mut inputs = Vec::new();

            for part in &parts {
                let vk = vk_from_name(part.trim());
                if vk == 0 {
                    return ToolResult::err(format!("unknown key: {part}"));
                }
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk),
                            ..Default::default()
                        },
                    },
                });
            }

            for part in parts.iter().rev() {
                let vk = vk_from_name(part.trim());
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk),
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                });
            }

            unsafe {
                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            }
            ToolResult::ok(format!("pressed {keys}"))
        })
        .await
        .unwrap_or_else(|e| ToolResult::err(format!("press_key task failed: {e}")))
    }

    #[cfg(not(windows))]
    {
        let _ = keys;
        ToolResult::err("press_key is Windows-only")
    }
}
