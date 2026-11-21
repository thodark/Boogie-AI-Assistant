use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "game_updater",
            "THE ONLY tool for any Steam game request. Use for: installing, downloading, \
             updating games, listing installed games, checking download status. \
             ALWAYS call directly for any Steam/game request. NEVER use browser_control or web_search for Steam.",
            vec![
                ("action", param("action", "string", "update | install | list | download_status (default: update)")),
                ("game_name", param("game_name", "string", "Game name (partial match supported)")),
                ("app_id", param("app_id", "string", "Steam AppID for install (optional)")),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(game_updater(args))),
    );
}

fn known_appids() -> HashMap<&'static str, (&'static str, &'static str)> {
    let mut m = HashMap::new();
    m.insert("pubg", ("578080", "PUBG: Battlegrounds"));
    m.insert("pubg battlegrounds", ("578080", "PUBG: Battlegrounds"));
    m.insert("gta5", ("271590", "Grand Theft Auto V"));
    m.insert("gta v", ("271590", "Grand Theft Auto V"));
    m.insert("grand theft auto v", ("271590", "Grand Theft Auto V"));
    m.insert("cs2", ("730", "Counter-Strike 2"));
    m.insert("csgo", ("730", "Counter-Strike 2"));
    m.insert("counter-strike 2", ("730", "Counter-Strike 2"));
    m.insert("dota2", ("570", "Dota 2"));
    m.insert("dota 2", ("570", "Dota 2"));
    m.insert("rust", ("252490", "Rust"));
    m.insert("valheim", ("892970", "Valheim"));
    m.insert("cyberpunk", ("1091500", "Cyberpunk 2077"));
    m.insert("cyberpunk 2077", ("1091500", "Cyberpunk 2077"));
    m.insert("elden ring", ("1245620", "ELDEN RING"));
    m.insert("minecraft", ("1672970", "Minecraft Launcher"));
    m.insert("apex legends", ("1172470", "Apex Legends"));
    m.insert("apex", ("1172470", "Apex Legends"));
    m.insert("fortnite", ("1517990", "Fortnite"));
    m.insert("goose goose duck", ("1568590", "Goose Goose Duck"));
    m.insert("among us", ("945360", "Among Us"));
    m.insert("fall guys", ("1097150", "Fall Guys"));
    m.insert("rocket league", ("252950", "Rocket League"));
    m.insert("warframe", ("230410", "Warframe"));
    m.insert("destiny 2", ("1085660", "Destiny 2"));
    m.insert("team fortress 2", ("440", "Team Fortress 2"));
    m.insert("tf2", ("440", "Team Fortress 2"));
    m.insert("left 4 dead 2", ("550", "Left 4 Dead 2"));
    m.insert("l4d2", ("550", "Left 4 Dead 2"));
    m.insert("path of exile", ("238960", "Path of Exile"));
    m.insert("poe", ("238960", "Path of Exile"));
    m.insert("lost ark", ("1599340", "Lost Ark"));
    m.insert("new world", ("1063730", "New World: Aeternum"));
    m.insert("war thunder", ("236390", "War Thunder"));
    m
}

#[cfg(windows)]
fn find_steam_path() -> Option<PathBuf> {
    use windows::Win32::System::Registry::*;
    use windows::core::*;

    let keys = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Valve\Steam"),
    ];
    for (hive, subkey) in keys {
        let hstr: HSTRING = subkey.into();
        let mut hkey = HKEY::default();
        unsafe {
            if RegOpenKeyExW(hive, &hstr, Some(0), KEY_READ, &mut hkey).is_ok() {
                let val_name: HSTRING = "InstallPath".into();
                let mut buf = [0u16; 512];
                let mut size = (buf.len() * 2) as u32;
                if RegQueryValueExW(hkey, &val_name, None, None, Some(buf.as_mut_ptr() as *mut u8), Some(&mut size)).is_ok() {
                    let len = (size as usize / 2).saturating_sub(1);
                    let path_str = String::from_utf16_lossy(&buf[..len]);
                    let p = PathBuf::from(&path_str);
                    let _ = RegCloseKey(hkey);
                    if p.join("steam.exe").exists() {
                        return Some(p);
                    }
                }
                let _ = RegCloseKey(hkey);
            }
        }
    }

    for p in [
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
        PathBuf::from(r"C:\Program Files\Steam"),
        PathBuf::from(r"C:\Steam"),
        PathBuf::from(r"D:\Steam"),
    ] {
        if p.join("steam.exe").exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(not(windows))]
fn find_steam_path() -> Option<PathBuf> {
    None
}

struct SteamGame {
    id: String,
    name: String,
    state: u32,
}

fn get_steam_libraries(steam_path: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_path.join("steamapps")];
    let vdf = steam_path.join("steamapps").join("libraryfolders.vdf");
    if let Ok(content) = std::fs::read_to_string(&vdf) {
        let re = regex::Regex::new(r#""path"\s+"([^"]+)""#).unwrap();
        for cap in re.captures_iter(&content) {
            let raw = cap.get(1).unwrap().as_str().replace("\\\\", "/");
            let lib = PathBuf::from(&raw).join("steamapps");
            if lib.exists() && !libs.contains(&lib) {
                libs.push(lib);
            }
        }
    }
    libs
}

fn get_steam_games(steam_path: &Path) -> Vec<SteamGame> {
    let mut games = Vec::new();
    let id_re = regex::Regex::new(r#""appid"\s+"(\d+)""#).unwrap();
    let name_re = regex::Regex::new(r#""name"\s+"([^"]+)""#).unwrap();
    let state_re = regex::Regex::new(r#""StateFlags"\s+"(\d+)""#).unwrap();

    for lib in get_steam_libraries(steam_path) {
        if let Ok(entries) = std::fs::read_dir(&lib) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if !fname.starts_with("appmanifest_") || !fname.ends_with(".acf") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    let id = id_re.captures(&content).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
                    let name = name_re.captures(&content).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
                    let state = state_re.captures(&content).and_then(|c| c.get(1)).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                    if let (Some(id), Some(name)) = (id, name) {
                        games.push(SteamGame { id, name, state });
                    }
                }
            }
        }
    }
    games
}

fn launch_steam_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = open::that(url);
    }
}

fn search_appid(game_name: &str, steam_path: Option<&Path>) -> Option<(String, String)> {
    let lower = game_name.to_lowercase();

    if let Some(sp) = steam_path {
        for g in get_steam_games(sp) {
            if g.name.to_lowercase().contains(&lower) {
                return Some((g.id, g.name));
            }
        }
    }

    let known = known_appids();
    if let Some(&(id, name)) = known.get(lower.as_str()) {
        return Some((id.to_string(), name.to_string()));
    }
    for (key, &(id, name)) in &known {
        if lower.contains(key) || key.contains(&lower.as_str()) {
            return Some((id.to_string(), name.to_string()));
        }
    }

    None
}

async fn game_updater(args: Value) -> ToolResult {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("update").to_lowercase();
    let game_name = args.get("game_name").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let app_id = args.get("app_id").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();

    let steam_path = match find_steam_path() {
        Some(p) => p,
        None => return ToolResult::err("Steam is not installed or not found."),
    };

    match action.as_str() {
        "list" => {
            let games = get_steam_games(&steam_path);
            if games.is_empty() {
                return ToolResult::ok("No Steam games found.");
            }
            let names: Vec<String> = games.iter().take(15).map(|g| g.name.clone()).collect();
            let suffix = if games.len() > 15 { format!(" and {} more", games.len() - 15) } else { String::new() };
            ToolResult::ok(format!("Steam ({} games): {}{}", games.len(), names.join(", "), suffix))
        }

        "download_status" => {
            let games = get_steam_games(&steam_path);
            let active: Vec<&SteamGame> = games.iter().filter(|g| g.state == 1026).collect();
            let pending: Vec<&SteamGame> = games.iter().filter(|g| g.state == 6 || g.state == 516).collect();
            let mut lines = Vec::new();
            if !active.is_empty() {
                let names: Vec<&str> = active.iter().map(|g| g.name.as_str()).collect();
                lines.push(format!("Downloading: {}", names.join(", ")));
            }
            if !pending.is_empty() {
                let names: Vec<&str> = pending.iter().take(5).map(|g| g.name.as_str()).collect();
                lines.push(format!("Pending updates: {}", names.join(", ")));
            }
            if lines.is_empty() {
                ToolResult::ok("No active downloads or pending updates.")
            } else {
                ToolResult::ok(lines.join(" "))
            }
        }

        "install" => {
            if game_name.is_empty() && app_id.is_empty() {
                return ToolResult::err("Please specify a game name or app_id to install.");
            }

            let games = get_steam_games(&steam_path);
            let lower = game_name.to_lowercase();
            let already = games.iter().find(|g| g.name.to_lowercase().contains(&lower));

            if let Some(g) = already {
                if g.state == 4 {
                    return ToolResult::ok(format!("'{}' is already installed and up to date.", g.name));
                }
                launch_steam_url(&format!("steam://update/{}", g.id));
                return ToolResult::ok(format!("'{}' has a pending update. Update started.", g.name));
            }

            let final_id = if !app_id.is_empty() {
                app_id.clone()
            } else {
                match search_appid(&game_name, Some(&steam_path)) {
                    Some((id, _)) => id,
                    None => return ToolResult::err(format!("Could not find '{}' on Steam. Try providing the AppID.", game_name)),
                }
            };

            launch_steam_url(&format!("steam://install/{final_id}"));
            ToolResult::ok(format!("Install started for '{}'. Steam will open the download dialog.", if game_name.is_empty() { &app_id } else { &game_name }))
        }

        "update" | _ => {
            let games = get_steam_games(&steam_path);
            if games.is_empty() {
                return ToolResult::ok("No Steam games found.");
            }

            let targets: Vec<&SteamGame> = if game_name.is_empty() {
                games.iter().collect()
            } else {
                let lower = game_name.to_lowercase();
                let matched: Vec<&SteamGame> = games.iter().filter(|g| g.name.to_lowercase().contains(&lower)).collect();
                if matched.is_empty() {
                    let available: Vec<&str> = games.iter().take(5).map(|g| g.name.as_str()).collect();
                    return ToolResult::err(format!("Game '{}' not found. Installed: {}...", game_name, available.join(", ")));
                }
                matched
            };

            let mut updated = Vec::new();
            let mut already_ok = Vec::new();
            let mut downloading = Vec::new();

            for g in &targets {
                match g.state {
                    4 => already_ok.push(g.name.as_str()),
                    1026 => downloading.push(g.name.as_str()),
                    _ => {
                        launch_steam_url(&format!("steam://update/{}", g.id));
                        updated.push(g.name.as_str());
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                }
            }

            let mut parts = Vec::new();
            if !updated.is_empty() {
                let names: Vec<&str> = updated.iter().take(3).copied().collect();
                let suffix = if updated.len() > 3 { format!(" and {} more", updated.len() - 3) } else { String::new() };
                parts.push(format!("Update started for: {}{}", names.join(", "), suffix));
            }
            if !downloading.is_empty() {
                parts.push(format!("Already updating: {}", downloading.join(", ")));
            }
            if !already_ok.is_empty() {
                if !game_name.is_empty() {
                    parts.push(format!("{} is already up to date.", already_ok[0]));
                } else {
                    parts.push(format!("{} game(s) already up to date.", already_ok.len()));
                }
            }

            if parts.is_empty() {
                ToolResult::ok("No games to update.")
            } else {
                ToolResult::ok(parts.join(" "))
            }
        }
    }
}
