use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "open_app",
            "Open or bring to front ANY installed Windows application on this computer \
             (e.g. Brave, Discord, Chrome, Edge, Cốc Cốc, Visual Studio Code, Spotify, Zalo, \
             Telegram, WhatsApp, Steam, Notepad, Calculator, Settings, Word, Excel, etc.). \
             Expand abbreviations ('vs code' -> 'Visual Studio Code', 'wp' -> 'WhatsApp'). \
             If the app is already open, brings its window to the front. \
             If the user wants to open a website in a specific browser (e.g. 'open Brave and go to youtube.com'), \
             you can call open_url with browser='brave' directly in one step.",
            vec![(
                "app_name",
                param(
                    "app_name",
                    "string",
                    "Application name to open (e.g. 'Discord', 'Brave', 'Visual Studio Code', 'Spotify')",
                ),
            )],
            vec!["app_name"],
        ),
        Box::new(|args| Box::pin(open_app(args))),
    );
}

/// Fold a name to lowercase alphanumeric characters (strips spaces, punctuation, diacritics).
pub fn fold_name(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Canonical Windows aliases for common spoken app names (does NOT restrict other apps).
const ALIASES: &[(&str, &[&str])] = &[
    ("brave", &["brave", "brave browser"]),
    ("chrome", &["chrome", "google chrome"]),
    ("edge", &["msedge", "microsoft edge", "edge"]),
    ("coccoc", &["cốc cốc", "coc coc", "coccoc", "browser"]),
    ("firefox", &["firefox", "mozilla firefox"]),
    ("opera", &["opera"]),
    ("discord", &["discord"]),
    ("whatsapp", &["whatsapp"]),
    ("telegram", &["telegram", "telegram desktop"]),
    ("zalo", &["zalo"]),
    ("slack", &["slack"]),
    ("zoom", &["zoom", "zoom workplace"]),
    ("teams", &["ms-teams", "msteams", "microsoft teams"]),
    ("spotify", &["spotify"]),
    ("vlc", &["vlc", "vlc media player"]),
    ("vscode", &["code", "visual studio code", "vscode"]),
    ("visual studio code", &["code", "visual studio code"]),
    ("vs code", &["code", "visual studio code"]),
    ("code", &["code", "visual studio code"]),
    ("terminal", &["wt", "windows terminal"]),
    ("cmd", &["cmd", "command prompt"]),
    ("powershell", &["powershell", "windows powershell"]),
    ("notepad", &["notepad"]),
    ("calculator", &["calc", "calculator"]),
    ("calc", &["calc", "calculator"]),
    ("paint", &["mspaint", "paint"]),
    ("explorer", &["explorer", "file explorer"]),
    ("file explorer", &["explorer", "file explorer"]),
    ("task manager", &["taskmgr", "task manager"]),
    ("settings", &["ms-settings:", "settings"]),
    ("steam", &["steam"]),
    ("epic", &["epicgameslauncher", "epic games launcher"]),
    ("word", &["winword", "word", "microsoft word"]),
    ("excel", &["excel", "microsoft excel"]),
    (
        "powerpoint",
        &["powerpnt", "powerpoint", "microsoft powerpoint"],
    ),
    ("obs", &["obs64", "obs studio"]),
    ("notion", &["notion"]),
    ("obsidian", &["obsidian"]),
    ("capcut", &["capcut"]),
    ("figma", &["figma"]),
    ("postman", &["postman"]),
];

fn expand_candidates(spoken: &str) -> Vec<String> {
    let trimmed = spoken.trim();
    let lower = trimmed.to_lowercase();
    let mut out = vec![trimmed.to_string()];

    for &(key, vals) in ALIASES {
        if lower == key || fold_name(&lower) == fold_name(key) {
            for &v in vals {
                if !out.iter().any(|x| x.eq_ignore_ascii_case(v)) {
                    out.push(v.to_string());
                }
            }
        }
    }
    out
}

/// Resolves a browser executable path on Windows (used by `open_url` when a specific browser is requested).
pub fn find_browser_exe(browser_name: &str) -> Option<PathBuf> {
    let candidates = expand_candidates(browser_name);
    for cand in &candidates {
        if let Some(p) = find_in_app_paths(cand) {
            return Some(p);
        }
        if let Some(p) = find_on_path(cand) {
            return Some(p);
        }
    }
    // Check standard Windows browser install paths
    let lower = fold_name(browser_name);
    let known_paths: &[(&str, &[&str])] = &[
        (
            "brave",
            &[
                r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
                r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
            ],
        ),
        (
            "chrome",
            &[
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            ],
        ),
        (
            "edge",
            &[
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            ],
        ),
        (
            "firefox",
            &[
                r"C:\Program Files\Mozilla Firefox\firefox.exe",
                r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe",
            ],
        ),
    ];

    for &(tag, paths) in known_paths {
        if lower.contains(tag) {
            for &p in paths {
                let pb = PathBuf::from(p);
                if pb.exists() {
                    return Some(pb);
                }
            }
            if let Some(local) = dirs::data_local_dir() {
                let per_user = match tag {
                    "brave" => local.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"),
                    "chrome" => local.join(r"Google\Chrome\Application\chrome.exe"),
                    _ => PathBuf::new(),
                };
                if per_user.exists() {
                    return Some(per_user);
                }
            }
        }
    }
    None
}

async fn open_app(args: Value) -> ToolResult {
    let spoken = args
        .get("app_name")
        .or_else(|| args.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    if spoken.is_empty() {
        return ToolResult::err("No application name was provided.");
    }

    if crate::util::is_blocked_executable(&spoken)
        && (spoken.contains('\\') || spoken.contains('/'))
    {
        return ToolResult::err(format!(
            "Refused to directly execute script/binary path via open_app: '{spoken}'. \
             Pass an installed application name instead."
        ));
    }

    tokio::task::spawn_blocking(move || open_app_sync(&spoken))
        .await
        .unwrap_or_else(|e| ToolResult::err(format!("open_app task failed: {e}")))
}

fn open_app_sync(spoken: &str) -> ToolResult {
    let candidates = expand_candidates(spoken);

    // 1. If already running with a visible window, restore and bring to foreground immediately
    if let Some(focused_name) = try_focus_running_app(&candidates) {
        return ToolResult::ok(format!(
            "{focused_name} is already open — brought to front."
        ));
    }

    // 2. Search Windows Start Menu shortcuts (.lnk) — finds Discord, Brave, VS Code, Zalo, etc.
    if let Some(lnk) = find_start_menu_shortcut(&candidates) {
        let label = lnk
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(spoken)
            .to_string();
        match open::that(&lnk) {
            Ok(_) => return ToolResult::ok(format!("Opened {label}.")),
            Err(e) => tracing::warn!("failed to launch shortcut {}: {e}", lnk.display()),
        }
    }

    // 3. Check Windows Registry App Paths (HKLM & HKCU)
    for cand in &candidates {
        if let Some(exe) = find_in_app_paths(cand) {
            match spawn_detached(&exe) {
                Ok(_) => return ToolResult::ok(format!("Opened {spoken}.")),
                Err(e) => tracing::warn!("failed to spawn {}: {e}", exe.display()),
            }
        }
    }

    // 4. Check known browser / local app paths (e.g. Discord Update.exe, Brave, Chrome)
    if let Some(browser_exe) = find_browser_exe(spoken)
        && spawn_detached(&browser_exe).is_ok()
    {
        return ToolResult::ok(format!("Opened {spoken}."));
    }

    // 5. Check executables on PATH (notepad, calc, wt, mspaint, explorer, taskmgr, etc.)
    for cand in &candidates {
        if let Some(exe) = find_on_path(cand)
            && spawn_detached(&exe).is_ok()
        {
            return ToolResult::ok(format!("Opened {spoken}."));
        }
    }

    // 6. Check URI schemes (e.g. ms-settings:, spotify:, steam:)
    for cand in &candidates {
        if let Some((scheme, _)) = cand.split_once(':')
            && scheme.len() > 1
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && open::that(cand).is_ok()
        {
            return ToolResult::ok(format!("Opened {spoken}."));
        }
    }

    // 7. Fallback: Windows Store / UWP Apps via Get-StartApps (WhatsApp, Calculator, Clock, etc.)
    #[cfg(windows)]
    if let Some(app_name) = try_launch_uwp_start_app(&candidates) {
        return ToolResult::ok(format!("Opened {app_name}."));
    }

    ToolResult::err(format!(
        "Could not find an installed application matching '{spoken}'. \
         Check that it is installed or try its full name."
    ))
}

fn spawn_detached(exe: &Path) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn().map(|_| ())
}

/// Finds an executable by name in the `PATH` environment variable (no shell involved).
fn find_on_path(name: &str) -> Option<PathBuf> {
    if name.contains(':') || name.contains('\\') || name.contains('/') {
        return None;
    }
    let path_var = std::env::var_os("PATH")?;
    let exts: &[&str] = if name.to_lowercase().ends_with(".exe") {
        &[""]
    } else {
        &[".exe", ".cmd"]
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn find_in_app_paths(name: &str) -> Option<PathBuf> {
    use windows::Win32::System::Registry::*;
    use windows::core::HSTRING;

    if name.contains(':') || name.contains('\\') || name.contains('/') {
        return None;
    }
    let exe_name = if name.to_lowercase().ends_with(".exe") {
        name.to_string()
    } else {
        format!("{name}.exe")
    };
    let subkey = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe_name}");

    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let hstr: HSTRING = subkey.as_str().into();
        let mut hkey = HKEY::default();
        unsafe {
            if RegOpenKeyExW(hive, &hstr, Some(0), KEY_READ, &mut hkey).is_ok() {
                let mut buf = [0u16; 1024];
                let mut size = (buf.len() * 2) as u32;
                let ok = RegQueryValueExW(
                    hkey,
                    None,
                    None,
                    None,
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut size),
                )
                .is_ok();
                let _ = RegCloseKey(hkey);
                if ok && size >= 2 {
                    let len = (size as usize / 2).saturating_sub(1);
                    let raw = String::from_utf16_lossy(&buf[..len])
                        .trim_matches(|c| c == '"' || c == '\0' || c == ' ')
                        .to_string();
                    let pb = PathBuf::from(&raw);
                    if pb.is_file() {
                        return Some(pb);
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn find_in_app_paths(_name: &str) -> Option<PathBuf> {
    None
}

fn score_shortcut_match(stem: &str, candidates: &[String]) -> u32 {
    let stem_folded = fold_name(stem);
    if stem_folded.is_empty() {
        return 0;
    }
    let stem_words: Vec<String> = stem
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let mut best = 0u32;
    for cand in candidates {
        let cf = fold_name(cand);
        if cf.is_empty() {
            continue;
        }
        // Skip uninstallers / help shortcuts
        if stem_words.iter().any(|w| w == "uninstall" || w == "readme") {
            continue;
        }
        if stem_folded == cf {
            best = best.max(100);
        } else if stem_words.iter().any(|w| w == &cf) {
            best = best.max(85);
        } else if stem_folded.starts_with(&cf) && cf.len() >= 3 {
            best = best.max(70);
        } else if cf.len() >= 4 && stem_folded.contains(&cf) {
            best = best.max(50);
        }
    }
    best
}

fn find_start_menu_shortcut(candidates: &[String]) -> Option<PathBuf> {
    let mut roots = vec![PathBuf::from(
        r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
    )];
    if let Some(roaming) = dirs::config_dir() {
        roots.push(roaming.join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(desktop) = dirs::desktop_dir() {
        roots.push(desktop);
    }

    let mut best: Option<(u32, usize, PathBuf)> = None;

    fn walk(
        dir: &Path,
        depth: usize,
        candidates: &[String],
        best: &mut Option<(u32, usize, PathBuf)>,
    ) {
        if depth > 4 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, depth + 1, candidates, best);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                let score = score_shortcut_match(stem, candidates);
                if score > 0 {
                    let len = stem.len();
                    let better = match best {
                        None => true,
                        Some((bs, bl, _)) => score > *bs || (score == *bs && len < *bl),
                    };
                    if better {
                        *best = Some((score, len, path));
                    }
                }
            }
        }
    }

    for root in roots {
        if root.exists() {
            walk(&root, 0, candidates, &mut best);
        }
    }

    best.map(|(_, _, p)| p)
}

#[cfg(windows)]
fn try_focus_running_app(candidates: &[String]) -> Option<String> {
    use sysinfo::{ProcessesToUpdate, System};
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    let my_pid = std::process::id();
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let cand_folded: Vec<String> = candidates
        .iter()
        .map(|c| fold_name(c))
        .filter(|s| s.len() >= 2)
        .collect();

    let mut matched_pids: Vec<(u32, String)> = Vec::new();
    for (pid, proc_) in sys.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == my_pid {
            continue;
        }
        let pname = proc_.name().to_string_lossy().to_string();
        let stem = pname
            .strip_suffix(".exe")
            .or_else(|| pname.strip_suffix(".EXE"))
            .unwrap_or(&pname);
        let pfold = fold_name(stem);
        if cand_folded.contains(&pfold) {
            matched_pids.push((pid_u32, stem.to_string()));
        }
    }

    if matched_pids.is_empty() {
        return None;
    }

    struct SearchCtx {
        pids: Vec<(u32, String)>,
        found: Option<(HWND, String)>,
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let ctx = &mut *(lparam.0 as *mut SearchCtx);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            if GetWindowTextLengthW(hwnd) == 0 {
                return BOOL(1);
            }
            let mut win_pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut win_pid));
            if let Some((_, name)) = ctx.pids.iter().find(|(p, _)| *p == win_pid) {
                ctx.found = Some((hwnd, name.clone()));
                return BOOL(0);
            }
            BOOL(1)
        }
    }

    let mut ctx = SearchCtx {
        pids: matched_pids,
        found: None,
    };

    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut SearchCtx as isize));
    }

    if let Some((hwnd, name)) = ctx.found {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        return Some(name);
    }

    None
}

#[cfg(not(windows))]
fn try_focus_running_app(_candidates: &[String]) -> Option<String> {
    None
}

#[cfg(windows)]
fn try_launch_uwp_start_app(candidates: &[String]) -> Option<String> {
    use std::os::windows::process::CommandExt;

    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-StartApps | ForEach-Object { \"$($_.Name)`t$($_.AppID)\" }",
        ])
        .creation_flags(0x08000000)
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut best: Option<(u32, usize, String, String)> = None;

    for line in stdout.lines() {
        let Some((name, app_id)) = line.split_once('\t') else {
            continue;
        };
        let score = score_shortcut_match(name, candidates);
        if score >= 70 {
            let len = name.len();
            let better = match &best {
                None => true,
                Some((bs, bl, _, _)) => score > *bs || (score == *bs && len < *bl),
            };
            if better {
                best = Some((score, len, name.to_string(), app_id.trim().to_string()));
            }
        }
    }

    let (_, _, name, app_id) = best?;
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .spawn()
        .ok()?;
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_common_app_aliases_and_scores_shortcuts() {
        let cands = expand_candidates("vs code");
        assert!(
            cands
                .iter()
                .any(|c| c.eq_ignore_ascii_case("Visual Studio Code"))
        );

        assert_eq!(
            score_shortcut_match("Discord", &expand_candidates("discord")),
            100
        );
        assert_eq!(
            score_shortcut_match("Brave", &expand_candidates("brave")),
            100
        );
        assert_eq!(
            score_shortcut_match("Uninstall Discord", &expand_candidates("discord")),
            0
        );
    }
}
