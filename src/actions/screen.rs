use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "take_screenshot",
            "Capture the screen and return as base64 PNG",
            vec![("monitor", param("monitor", "integer", "Monitor index (0-based)"))],
            vec![],
        ),
        Box::new(|args| Box::pin(take_screenshot(args))),
    );

    registry.register(
        tool(
            "click_screen",
            "Click at screen coordinates",
            vec![
                ("x", param("x", "integer", "X coordinate")),
                ("y", param("y", "integer", "Y coordinate")),
                ("button", param("button", "string", "left, right, or middle")),
            ],
            vec!["x", "y"],
        ),
        Box::new(|args| Box::pin(click_screen(args))),
    );

    registry.register(
        tool(
            "type_text",
            "Type text using keyboard simulation",
            vec![("text", param("text", "string", "Text to type"))],
            vec!["text"],
        ),
        Box::new(|args| Box::pin(type_text(args))),
    );

    registry.register(
        tool(
            "press_key",
            "Press a keyboard key or combination",
            vec![("keys", param("keys", "string", "Key combination like ctrl+c"))],
            vec!["keys"],
        ),
        Box::new(|args| Box::pin(press_key(args))),
    );
}

async fn take_screenshot(args: Value) -> ToolResult {
    let monitor = args.get("monitor").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    match screenshots::Screen::all() {
        Ok(screens) => {
            if let Some(screen) = screens.get(monitor) {
                match screen.capture() {
                    Ok(img) => {
                        use image::ImageEncoder;
                        let mut png_buf = Vec::new();
                        let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
                        let (w, h) = (img.width(), img.height());
                        match encoder.write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8) {
                            Ok(_) => {
                                use base64::Engine as _;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
                                ToolResult::ok(format!("data:image/png;base64,{b64}"))
                            }
                            Err(e) => ToolResult::err(format!("png encode: {e}")),
                        }
                    }
                    Err(e) => ToolResult::err(format!("capture failed: {e}")),
                }
            } else {
                ToolResult::err(format!("monitor {monitor} not found"))
            }
        }
        Err(e) => ToolResult::err(format!("screen list failed: {e}")),
    }
}

async fn click_screen(args: Value) -> ToolResult {
    let _x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
    let _y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
    let _button = args
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left");

    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
        unsafe {
            let _ = SetCursorPos(_x as i32, _y as i32);
            let _flags = match _button {
                "right" => MOUSEEVENTF_RIGHTDOWN | MOUSEEVENTF_RIGHTUP,
                "middle" => MOUSEEVENTF_MIDDLEDOWN | MOUSEEVENTF_MIDDLEUP,
                _ => MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_LEFTUP,
            };
            let down = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: match _button {
                            "right" => MOUSEEVENTF_RIGHTDOWN,
                            "middle" => MOUSEEVENTF_MIDDLEDOWN,
                            _ => MOUSEEVENTF_LEFTDOWN,
                        },
                        ..Default::default()
                    },
                },
            };
            let up = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: match _button {
                            "right" => MOUSEEVENTF_RIGHTUP,
                            "middle" => MOUSEEVENTF_MIDDLEUP,
                            _ => MOUSEEVENTF_LEFTUP,
                        },
                        ..Default::default()
                    },
                },
            };
            SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
        }
        ToolResult::ok(format!("clicked ({_x}, {_y}) {_button}"))
    }

    #[cfg(not(windows))]
    ToolResult::err("click_screen is Windows-only")
}

async fn type_text(args: Value) -> ToolResult {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    #[cfg(windows)]
    {
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
        ToolResult::ok(format!("typed {} chars", text.len()))
    }

    #[cfg(not(windows))]
    ToolResult::err("type_text is Windows-only")
}

async fn press_key(args: Value) -> ToolResult {
    let _keys = args
        .get("keys")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    #[cfg(windows)]
    {
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

        let parts: Vec<&str> = _keys.split('+').collect();
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
        ToolResult::ok(format!("pressed {_keys}"))
    }

    #[cfg(not(windows))]
    ToolResult::err("press_key is Windows-only")
}
