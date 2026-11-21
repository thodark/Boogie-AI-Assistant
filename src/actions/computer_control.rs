use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::sync::Mutex;

use super::{param, tool};

enum UndoAction {
    Volume(f32),
    DarkMode,
}

static UNDO_STACK: Mutex<Vec<UndoAction>> = Mutex::new(Vec::new());

pub fn pop_undo() -> Option<String> {
    let action = UNDO_STACK.lock().ok()?.pop()?;
    match action {
        UndoAction::Volume(prev) => {
            match set_volume_com(prev) {
                Ok(()) => Some(format!("Volume restored to {}%", (prev * 100.0) as u32)),
                Err(e) => Some(format!("Failed to restore volume: {e}")),
            }
        }
        UndoAction::DarkMode => {
            let r = dark_mode_toggle();
            Some(r.output)
        }
    }
}

#[cfg(windows)]
fn get_volume_com() -> Option<f32> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::System::Com::*;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let volume: IAudioEndpointVolume = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
        volume.GetMasterVolumeLevelScalar().ok()
    }
}

#[cfg(not(windows))]
fn get_volume_com() -> Option<f32> { None }

fn push_undo(action: UndoAction) {
    if let Ok(mut stack) = UNDO_STACK.lock() {
        if stack.len() > 20 {
            stack.drain(..10);
        }
        stack.push(action);
    }
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "computer_settings",
            "Controls the computer: volume, brightness, window management, keyboard shortcuts, \
             typing text on screen, dark mode, screenshots, lock screen, tab management, zoom, scrolling. \
             restart, shutdown and toggle_wifi require user confirmation. \
             Pick the exact action name from: \
             volume_up | volume_down | volume_set | mute | \
             brightness_up | brightness_down | sleep_display | \
             pause_video | close_window | full_screen | \
             minimize | maximize | snap_left | snap_right | \
             switch_window | show_desktop | task_manager | focus_search | \
             refresh_page | close_tab | new_tab | next_tab | prev_tab | \
             go_back | go_forward | zoom_in | zoom_out | zoom_reset | \
             find_on_page | scroll_up | scroll_down | scroll_top | \
             scroll_bottom | page_up | page_down | copy | paste | cut | \
             undo | redo | select_all | save | enter | escape | press_key | \
             type_text | screenshot | lock_screen | open_settings | \
             file_explorer | open_run | dark_mode | toggle_wifi | \
             restart | shutdown",
            vec![
                ("action", param("action", "string", "The exact action name from the list above")),
                ("value", param("value", "string", "Optional value: volume level 0-100, text to type, key name")),
            ],
            vec!["action"],
        ),
        Box::new(|args| Box::pin(computer_settings(args))),
    );
}

async fn computer_settings(args: Value) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .replace('-', "_")
        .replace(' ', "_");

    let value = args.get("value").and_then(|v| v.as_str()).unwrap_or_default();

    if action.is_empty() {
        return ToolResult::err("No action specified");
    }

    match action.as_str() {
        "volume_up" => {
            if let Some(v) = get_volume_com() { push_undo(UndoAction::Volume(v)); }
            keys(&["volumeup"; 5]);
            ToolResult::ok("Volume increased")
        }
        "volume_down" => {
            if let Some(v) = get_volume_com() { push_undo(UndoAction::Volume(v)); }
            keys(&["volumedown"; 5]);
            ToolResult::ok("Volume decreased")
        }
        "mute" | "unmute" | "toggle_mute" => { key("volumemute"); ToolResult::ok("Mute toggled") }
        "volume_set" => {
            if let Some(v) = get_volume_com() { push_undo(UndoAction::Volume(v)); }
            volume_set(value)
        }
        "brightness_up" => brightness_change(10),
        "brightness_down" => brightness_change(-10),
        "sleep_display" | "screen_off" => sleep_display(),
        "pause_video" | "play_pause" => { key("space"); ToolResult::ok("Play/pause toggled") }
        "close_window" => { hotkey(&["ctrl", "w"]); ToolResult::ok("Window closed") }
        "full_screen" | "fullscreen" => { key("f11"); ToolResult::ok("Fullscreen toggled") }
        "minimize" => { hotkey(&["win", "down"]); ToolResult::ok("Window minimized") }
        "maximize" => { hotkey(&["win", "up"]); ToolResult::ok("Window maximized") }
        "snap_left" => { hotkey(&["win", "left"]); ToolResult::ok("Window snapped left") }
        "snap_right" => { hotkey(&["win", "right"]); ToolResult::ok("Window snapped right") }
        "switch_window" => { hotkey(&["alt", "tab"]); ToolResult::ok("Switched window") }
        "show_desktop" => { hotkey(&["win", "d"]); ToolResult::ok("Desktop shown") }
        "task_manager" => { hotkey(&["ctrl", "shift", "esc"]); ToolResult::ok("Task Manager opened") }
        "focus_search" => { hotkey(&["ctrl", "l"]); ToolResult::ok("Search focused") }
        "refresh_page" | "reload" => { key("f5"); ToolResult::ok("Page refreshed") }
        "close_tab" => { hotkey(&["ctrl", "w"]); ToolResult::ok("Tab closed") }
        "new_tab" => { hotkey(&["ctrl", "t"]); ToolResult::ok("New tab opened") }
        "next_tab" => { hotkey(&["ctrl", "tab"]); ToolResult::ok("Next tab") }
        "prev_tab" => { hotkey(&["ctrl", "shift", "tab"]); ToolResult::ok("Previous tab") }
        "go_back" => { hotkey(&["alt", "left"]); ToolResult::ok("Navigated back") }
        "go_forward" => { hotkey(&["alt", "right"]); ToolResult::ok("Navigated forward") }
        "zoom_in" => { hotkey(&["ctrl", "="]); ToolResult::ok("Zoomed in") }
        "zoom_out" => { hotkey(&["ctrl", "-"]); ToolResult::ok("Zoomed out") }
        "zoom_reset" => { hotkey(&["ctrl", "0"]); ToolResult::ok("Zoom reset") }
        "find_on_page" => { hotkey(&["ctrl", "f"]); ToolResult::ok("Find on page opened") }
        "scroll_up" => { scroll(value.parse().unwrap_or(500)); ToolResult::ok("Scrolled up") }
        "scroll_down" => { scroll(-(value.parse().unwrap_or(500i32))); ToolResult::ok("Scrolled down") }
        "scroll_top" => { hotkey(&["ctrl", "home"]); ToolResult::ok("Scrolled to top") }
        "scroll_bottom" => { hotkey(&["ctrl", "end"]); ToolResult::ok("Scrolled to bottom") }
        "page_up" => { key("pageup"); ToolResult::ok("Page up") }
        "page_down" => { key("pagedown"); ToolResult::ok("Page down") }
        "copy" => { hotkey(&["ctrl", "c"]); ToolResult::ok("Copied") }
        "paste" => { hotkey(&["ctrl", "v"]); ToolResult::ok("Pasted") }
        "cut" => { hotkey(&["ctrl", "x"]); ToolResult::ok("Cut") }
        "undo" => { hotkey(&["ctrl", "z"]); ToolResult::ok("Undone") }
        "redo" => { hotkey(&["ctrl", "y"]); ToolResult::ok("Redone") }
        "select_all" => { hotkey(&["ctrl", "a"]); ToolResult::ok("All selected") }
        "save" => { hotkey(&["ctrl", "s"]); ToolResult::ok("Saved") }
        "enter" => { key("enter"); ToolResult::ok("Enter pressed") }
        "escape" => { key("escape"); ToolResult::ok("Escape pressed") }
        "press_key" => {
            if value.is_empty() {
                ToolResult::err("No key specified")
            } else {
                key(value);
                ToolResult::ok(format!("Pressed {value}"))
            }
        }
        "type_text" | "write_on_screen" | "type" | "write" => {
            if value.is_empty() {
                ToolResult::err("No text to type")
            } else {
                type_text_win(value);
                ToolResult::ok(format!("Typed: {}", &value[..value.len().min(60)]))
            }
        }
        "screenshot" => { hotkey(&["win", "shift", "s"]); ToolResult::ok("Screenshot tool opened") }
        "lock_screen" => { hotkey(&["win", "l"]); ToolResult::ok("Screen locked") }
        "open_settings" => { hotkey(&["win", "i"]); ToolResult::ok("Settings opened") }
        "file_explorer" => { hotkey(&["win", "e"]); ToolResult::ok("File Explorer opened") }
        "open_run" => { hotkey(&["win", "r"]); ToolResult::ok("Run dialog opened") }
        "dark_mode" => {
            push_undo(UndoAction::DarkMode);
            dark_mode_toggle()
        }
        "toggle_wifi" => toggle_wifi(),
        "restart" => run_system_cmd("shutdown /r /t 5"),
        "shutdown" => run_system_cmd("shutdown /s /t 5"),
        _ => ToolResult::err(format!(
            "Unknown action '{action}'. Available: volume_up, volume_down, volume_set, mute, \
             brightness_up, brightness_down, minimize, maximize, snap_left, snap_right, \
             show_desktop, dark_mode, screenshot, lock_screen, etc."
        )),
    }
}

#[cfg(windows)]
fn key_to_vk(name: &str) -> u16 {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    match name.to_lowercase().as_str() {
        "enter" | "return" => VK_RETURN.0,
        "tab" => VK_TAB.0,
        "escape" | "esc" => VK_ESCAPE.0,
        "space" => VK_SPACE.0,
        "backspace" => VK_BACK.0,
        "delete" | "del" => VK_DELETE.0,
        "home" => VK_HOME.0,
        "end" => VK_END.0,
        "pageup" => VK_PRIOR.0,
        "pagedown" => VK_NEXT.0,
        "up" => VK_UP.0,
        "down" => VK_DOWN.0,
        "left" => VK_LEFT.0,
        "right" => VK_RIGHT.0,
        "f1" => VK_F1.0, "f2" => VK_F2.0, "f3" => VK_F3.0, "f4" => VK_F4.0,
        "f5" => VK_F5.0, "f6" => VK_F6.0, "f7" => VK_F7.0, "f8" => VK_F8.0,
        "f9" => VK_F9.0, "f10" => VK_F10.0, "f11" => VK_F11.0, "f12" => VK_F12.0,
        "ctrl" | "control" => VK_CONTROL.0,
        "alt" => VK_MENU.0,
        "shift" => VK_SHIFT.0,
        "win" | "windows" | "super" => VK_LWIN.0,
        "volumeup" => VK_VOLUME_UP.0,
        "volumedown" => VK_VOLUME_DOWN.0,
        "volumemute" => VK_VOLUME_MUTE.0,
        "=" | "equal" => VK_OEM_PLUS.0,
        "-" | "minus" => VK_OEM_MINUS.0,
        "0" => 0x30, "1" => 0x31, "2" => 0x32, "3" => 0x33, "4" => 0x34,
        "5" => 0x35, "6" => 0x36, "7" => 0x37, "8" => 0x38, "9" => 0x39,
        s if s.len() == 1 => {
            let c = s.chars().next().unwrap().to_ascii_uppercase();
            if c.is_ascii_alphabetic() { c as u16 } else { 0 }
        }
        _ => 0,
    }
}

#[cfg(windows)]
fn send_key(vk: u16, down: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let flags = if down {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn key(name: &str) {
    let vk = key_to_vk(name);
    if vk == 0 { return; }
    send_key(vk, true);
    std::thread::sleep(std::time::Duration::from_millis(30));
    send_key(vk, false);
    std::thread::sleep(std::time::Duration::from_millis(30));
}

#[cfg(windows)]
fn keys(names: &[&str]) {
    for name in names {
        key(name);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn hotkey(names: &[&str]) {
    let vks: Vec<u16> = names.iter().map(|n| key_to_vk(n)).collect();
    for &vk in &vks {
        if vk != 0 { send_key(vk, true); }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    for &vk in vks.iter().rev() {
        if vk != 0 { send_key(vk, false); }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn scroll(amount: i32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: amount as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn type_text_win(text: &str) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    for ch in text.chars() {
        let code = ch as u32;
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: code as u16,
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
                    wScan: code as u16,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn volume_set(value: &str) -> ToolResult {
    let level: u32 = match value.parse::<u32>() {
        Ok(v) => v.min(100),
        Err(_) => return ToolResult::err("Invalid volume level"),
    };
    let scalar = level as f32 / 100.0;
    match set_volume_com(scalar) {
        Ok(()) => ToolResult::ok(format!("Volume set to {level}%")),
        Err(e) => ToolResult::err(format!("Failed to set volume: {e}")),
    }
}

#[cfg(windows)]
fn set_volume_com(scalar: f32) -> Result<(), String> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::System::Com::*;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let enumerator: IMMDeviceEnumerator = CoCreateInstance(
            &MMDeviceEnumerator,
            None,
            CLSCTX_ALL,
        ).map_err(|e| format!("CoCreateInstance: {e}"))?;

        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {e}"))?;

        let volume: IAudioEndpointVolume = device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate: {e}"))?;

        volume
            .SetMasterVolumeLevelScalar(scalar, std::ptr::null())
            .map_err(|e| format!("SetMasterVolumeLevelScalar: {e}"))?;

        Ok(())
    }
}

fn brightness_change(delta: i32) -> ToolResult {
    let cmd = if delta > 0 {
        format!(
            "(Get-WmiObject -Namespace root/wmi -Class WmiMonitorBrightnessMethods)\
             .WmiSetBrightness(1, [math]::Min(100, \
             (Get-WmiObject -Namespace root/wmi -Class WmiMonitorBrightness).CurrentBrightness + {delta}))"
        )
    } else {
        format!(
            "(Get-WmiObject -Namespace root/wmi -Class WmiMonitorBrightnessMethods)\
             .WmiSetBrightness(1, [math]::Max(0, \
             (Get-WmiObject -Namespace root/wmi -Class WmiMonitorBrightness).CurrentBrightness + {delta}))"
        )
    };
    match powershell_no_window(&cmd) {
        Ok(_) => ToolResult::ok(if delta > 0 { "Brightness increased" } else { "Brightness decreased" }),
        Err(e) => ToolResult::err(format!("Brightness change failed: {e}")),
    }
}

#[cfg(windows)]
fn sleep_display() -> ToolResult {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            windows::Win32::Foundation::HWND(0xFFFF as *mut _),
            0x0112, // WM_SYSCOMMAND
            Some(windows::Win32::Foundation::WPARAM(0xF170)), // SC_MONITORPOWER
            Some(windows::Win32::Foundation::LPARAM(2)), // 2 = off
        );
    }
    ToolResult::ok("Display sleeping")
}

fn dark_mode_toggle() -> ToolResult {
    let cmd = r#"
        $key = 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize'
        $cur = (Get-ItemProperty -Path $key).AppsUseLightTheme
        $new = 1 - $cur
        Set-ItemProperty -Path $key -Name AppsUseLightTheme -Value $new
        Set-ItemProperty -Path $key -Name SystemUsesLightTheme -Value $new
        if ($new -eq 0) { 'Dark mode enabled' } else { 'Light mode enabled' }
    "#;
    match powershell_no_window(cmd) {
        Ok(out) => {
            let msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
            ToolResult::ok(if msg.is_empty() { "Dark mode toggled".to_string() } else { msg })
        }
        Err(e) => ToolResult::err(format!("Dark mode toggle failed: {e}")),
    }
}

#[cfg(windows)]
fn powershell_no_window(cmd: &str) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell")
        .args(["-Command", cmd])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
}

#[cfg(not(windows))]
fn powershell_no_window(cmd: &str) -> std::io::Result<std::process::Output> {
    std::process::Command::new("powershell")
        .args(["-Command", cmd])
        .output()
}

fn toggle_wifi() -> ToolResult {
    let cmd = r#"
        $adapter = Get-NetAdapter | Where-Object { $_.Name -like '*Wi*' -or $_.InterfaceDescription -like '*Wireless*' -or $_.InterfaceDescription -like '*Wi-Fi*' } | Select-Object -First 1
        if (-not $adapter) { Write-Output 'No Wi-Fi adapter found'; exit 1 }
        if ($adapter.Status -eq 'Up') {
            Disable-NetAdapter -Name $adapter.Name -Confirm:$false
            Write-Output "Wi-Fi disabled ($($adapter.Name))"
        } else {
            Enable-NetAdapter -Name $adapter.Name -Confirm:$false
            Write-Output "Wi-Fi enabled ($($adapter.Name))"
        }
    "#;
    match powershell_no_window(cmd) {
        Ok(out) => {
            let msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !err.is_empty() && msg.is_empty() {
                ToolResult::err(format!("WiFi toggle failed: {err}"))
            } else {
                ToolResult::ok(if msg.is_empty() { "WiFi toggled".to_string() } else { msg })
            }
        }
        Err(e) => ToolResult::err(format!("WiFi toggle failed: {e}")),
    }
}

fn run_system_cmd(cmd: &str) -> ToolResult {
    match powershell_no_window(cmd) {
        Ok(out) => {
            let msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
            ToolResult::ok(if msg.is_empty() { format!("Executed: {cmd}") } else { msg })
        }
        Err(e) => ToolResult::err(format!("Command failed: {e}")),
    }
}

#[cfg(not(windows))]
fn key(_: &str) {}
#[cfg(not(windows))]
fn keys(_: &[&str]) {}
#[cfg(not(windows))]
fn hotkey(_: &[&str]) {}
#[cfg(not(windows))]
fn scroll(_: i32) {}
#[cfg(not(windows))]
fn type_text_win(_: &str) {}
#[cfg(not(windows))]
fn set_volume_com(_: f32) -> Result<(), String> { Err("Windows only".into()) }
#[cfg(not(windows))]
fn sleep_display() -> ToolResult { ToolResult::err("Windows only") }
