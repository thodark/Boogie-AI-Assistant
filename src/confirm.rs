use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_request_id() -> u64 {
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    RequireConfirm { title: String, detail: String },
    Deny(String),
}

/// Read-only shell commands allowed to execute without confirmation, provided no shell
/// chaining, redirection, or subshell metacharacters appear in the command string.
const READONLY_SHELL_ALLOWLIST: &[&str] = &[
    "dir",
    "ls",
    "pwd",
    "cd",
    "whoami",
    "hostname",
    "ipconfig",
    "ver",
    "date",
    "time",
    "echo",
    "where",
    "systeminfo",
    "tasklist",
    "get-date",
    "get-location",
    "get-childitem",
    "ping",
    "nslookup",
];

const SHELL_CHAIN_TOKENS: &[&str] = &["&&", "||", ";", "|", ">", "<", "&", "`", "$(", "%("];

/// Protected Windows system processes that Boogie must never terminate.
pub const PROTECTED_PROCESSES: &[&str] = &[
    "system",
    "idle",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "svchost.exe",
    "dwm.exe",
    "explorer.exe",
    "audiodg.exe",
    "boogie.exe",
];

const CONFIRM_CS_ACTIONS: &[&str] = &[
    "shutdown",
    "restart",
    "toggle_wifi",
    "sleep_display",
    "screen_off",
    "lock_screen",
];

/// Returns true if a shell command is strictly a single safe read-only command.
pub fn is_safe_readonly_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }
    if SHELL_CHAIN_TOKENS.iter().any(|tok| trimmed.contains(tok)) {
        return false;
    }
    let first_word = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    READONLY_SHELL_ALLOWLIST.contains(&first_word.as_str())
}

pub fn is_protected_process_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let with_exe = if lower.ends_with(".exe") {
        lower.clone()
    } else {
        format!("{lower}.exe")
    };
    PROTECTED_PROCESSES
        .iter()
        .any(|p| *p == lower || *p == with_exe)
}

fn is_sensitive_write_path(path: &str) -> bool {
    let norm = path.trim().replace('/', "\\").to_lowercase();
    norm.starts_with("c:\\windows")
        || norm.starts_with("c:\\program files")
        || norm.starts_with("c:\\programdata")
        || norm.contains("api_keys.json")
        || norm.contains("\\system32\\")
        || norm.contains("\\startup\\")
}

/// Evaluates permission tier for any tool call before execution.
/// When `saw_untrusted_in_turn` is true (external web/file/clipboard data was recently read),
/// medium-impact desktop actions automatically escalate to require explicit user confirmation.
pub fn evaluate_tool_call(
    tool_name: &str,
    args: &Value,
    saw_untrusted_in_turn: bool,
) -> PermissionDecision {
    match tool_name {
        "run_command" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim();
            if cmd.is_empty() {
                return PermissionDecision::Deny("No command specified.".to_string());
            }
            if !saw_untrusted_in_turn && is_safe_readonly_command(cmd) {
                PermissionDecision::Allow
            } else {
                let preview = crate::util::truncate_chars(cmd, 180);
                PermissionDecision::RequireConfirm {
                    title: "Run shell command".to_string(),
                    detail: format!("Command: {preview}"),
                }
            }
        }

        "computer_settings" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_lowercase()
                .replace(['-', ' '], "_");
            if CONFIRM_CS_ACTIONS.contains(&action.as_str()) {
                return PermissionDecision::RequireConfirm {
                    title: format!("System action: {action}"),
                    detail: "Please confirm you want to run this system action.".to_string(),
                };
            }
            if saw_untrusted_in_turn
                && matches!(
                    action.as_str(),
                    "type_text" | "write_on_screen" | "type" | "write" | "press_key" | "open_run"
                )
            {
                return PermissionDecision::RequireConfirm {
                    title: format!("Confirm input action ({action})"),
                    detail:
                        "External content was read in this turn; confirm before simulating input."
                            .to_string(),
                };
            }
            PermissionDecision::Allow
        }

        "kill_process" => {
            if let Some(pid) = args.get("pid").and_then(|v| v.as_u64()) {
                let proc_desc = describe_pid_for_confirm(pid);
                if let Some(name) = &proc_desc
                    && is_protected_process_name(name)
                {
                    return PermissionDecision::Deny(format!(
                        "Refused to kill protected system process '{name}' (PID {pid})."
                    ));
                }
                let detail = match proc_desc {
                    Some(name) => format!("Process: {name} (PID {pid})"),
                    None => format!("PID: {pid}"),
                };
                return PermissionDecision::RequireConfirm {
                    title: "Terminate process".to_string(),
                    detail,
                };
            }

            if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    return PermissionDecision::Deny("Empty process name.".to_string());
                }
                if is_protected_process_name(trimmed) {
                    return PermissionDecision::Deny(format!(
                        "Refused to kill protected system process '{trimmed}'."
                    ));
                }
                let matches = find_exact_processes_for_confirm(trimmed);
                let detail = if matches.is_empty() {
                    format!("Target: {trimmed} (no running exact match found)")
                } else {
                    let preview: Vec<String> = matches.iter().take(6).cloned().collect();
                    format!("Will terminate: {}", preview.join(", "))
                };
                return PermissionDecision::RequireConfirm {
                    title: "Terminate process".to_string(),
                    detail,
                };
            }

            PermissionDecision::Deny(
                "Provide either 'pid' (integer) or 'name' (string).".to_string(),
            )
        }

        "send_message" => {
            let platform = args
                .get("platform")
                .and_then(|v| v.as_str())
                .unwrap_or("whatsapp");
            let receiver = args.get("receiver").and_then(|v| v.as_str()).unwrap_or("?");
            let text = args
                .get("message_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = crate::util::truncate_chars(text, 140);
            PermissionDecision::RequireConfirm {
                title: format!("Send {platform} message to {receiver}"),
                detail: format!("Message: \"{preview}\""),
            }
        }

        "write_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if is_sensitive_write_path(path) {
                return PermissionDecision::Deny(format!(
                    "Blocked writing to protected path: {path}"
                ));
            }
            let bytes = args
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            PermissionDecision::RequireConfirm {
                title: "Write / overwrite file".to_string(),
                detail: format!("Path: {path} ({bytes} bytes)"),
            }
        }

        "delete_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            PermissionDecision::RequireConfirm {
                title: "Move file to Recycle Bin".to_string(),
                detail: format!("Path: {path}"),
            }
        }

        "move_file" => {
            let src = args
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let dst = args
                .get("destination")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if is_sensitive_write_path(dst) {
                return PermissionDecision::Deny(format!(
                    "Blocked moving file into protected path: {dst}"
                ));
            }
            PermissionDecision::RequireConfirm {
                title: "Move / rename file".to_string(),
                detail: format!("{src} -> {dst}"),
            }
        }

        // Prompt-injection elevation for medium-risk desktop interaction tools
        "click_screen" | "type_text" | "press_key" | "copy_file" | "open_app"
            if saw_untrusted_in_turn =>
        {
            let args_str = args.to_string();
            let summary = crate::util::truncate_chars(&args_str, 140);
            PermissionDecision::RequireConfirm {
                title: format!("Confirm {tool_name}"),
                detail: format!("External data was read in this turn. Args: {summary}"),
            }
        }

        _ => PermissionDecision::Allow,
    }
}

fn describe_pid_for_confirm(pid: u64) -> Option<String> {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut sys = System::new();
    let target = Pid::from(pid as usize);
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    sys.process(target)
        .map(|p| p.name().to_string_lossy().to_string())
}

fn find_exact_processes_for_confirm(name: &str) -> Vec<String> {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let lower = name.trim().to_lowercase();
    let with_exe = if lower.ends_with(".exe") {
        lower.clone()
    } else {
        format!("{lower}.exe")
    };
    let mut out = Vec::new();
    for (pid, p) in sys.processes() {
        let pname = p.name().to_string_lossy().to_string();
        let plower = pname.to_lowercase();
        if plower == lower || plower == with_exe {
            out.push(format!("{pname} ({pid})"));
        }
    }
    out.sort();
    out
}

fn audit_log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".boogie")
        .join("audit.log")
}

/// Appends an entry to `~/.boogie/audit.log` for sensitive actions and confirmation decisions.
pub fn write_audit_log(tool_name: &str, decision: &str, detail: &str) {
    let path = audit_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let flat = detail.replace('\n', " ").replace('\r', "");
    let clean_detail = crate::util::truncate_chars(&flat, 200);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(
            f,
            "[{ts}] tool={tool_name} decision={decision} detail={clean_detail}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn readonly_shell_allowlist_blocks_chaining_bypasses() {
        assert!(is_safe_readonly_command("dir C:\\Users"));
        assert!(is_safe_readonly_command("ipconfig /all"));
        assert!(is_safe_readonly_command("whoami"));
        assert!(!is_safe_readonly_command("echo hi && del /f secret.txt"));
        assert!(!is_safe_readonly_command(
            "dir; Remove-Item -Recurse -Force C:\\x"
        ));
        assert!(!is_safe_readonly_command("echo test > C:\\file.txt"));
        assert!(!is_safe_readonly_command("Remove-Item -Recurse -Force foo"));
    }

    #[test]
    fn run_command_requires_confirm_for_mutating_or_chained_commands() {
        assert_eq!(
            evaluate_tool_call("run_command", &json!({"command": "ipconfig"}), false),
            PermissionDecision::Allow
        );
        assert!(matches!(
            evaluate_tool_call(
                "run_command",
                &json!({"command": "Remove-Item -Recurse -Force x"}),
                false
            ),
            PermissionDecision::RequireConfirm { .. }
        ));
        // Even read-only commands require confirm if untrusted external data was read in turn
        assert!(matches!(
            evaluate_tool_call("run_command", &json!({"command": "ipconfig"}), true),
            PermissionDecision::RequireConfirm { .. }
        ));
    }

    #[test]
    fn kill_process_handles_numeric_pid_and_blocks_system_processes() {
        let res = evaluate_tool_call("kill_process", &json!({"pid": 999999}), false);
        match res {
            PermissionDecision::RequireConfirm { detail, .. } => {
                assert!(
                    detail.contains("999999"),
                    "detail should include numeric PID: {detail}"
                );
            }
            other => panic!("expected RequireConfirm, got {other:?}"),
        }
        assert!(matches!(
            evaluate_tool_call("kill_process", &json!({"name": "lsass.exe"}), false),
            PermissionDecision::Deny(_)
        ));
        assert!(matches!(
            evaluate_tool_call("kill_process", &json!({"name": "explorer"}), false),
            PermissionDecision::Deny(_)
        ));
    }

    #[test]
    fn send_message_always_requires_confirm_with_recipient_and_text() {
        let res = evaluate_tool_call(
            "send_message",
            &json!({"platform": "telegram", "receiver": "Minh", "message_text": "Alo 5h gặp nha"}),
            false,
        );
        match res {
            PermissionDecision::RequireConfirm { title, detail } => {
                assert!(title.contains("Minh"));
                assert!(detail.contains("Alo 5h gặp nha"));
            }
            other => panic!("expected RequireConfirm, got {other:?}"),
        }
    }

    #[test]
    fn untrusted_turn_escalates_desktop_input_tools() {
        assert_eq!(
            evaluate_tool_call("type_text", &json!({"text": "hello"}), false),
            PermissionDecision::Allow
        );
        assert!(matches!(
            evaluate_tool_call("type_text", &json!({"text": "hello"}), true),
            PermissionDecision::RequireConfirm { .. }
        ));
    }
}
