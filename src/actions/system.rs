use crate::confirm::is_protected_process_name;
use crate::tools::{ToolRegistry, ToolResult};
use crate::util::{is_blocked_executable, truncate_chars};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "run_command",
            "Execute a shell command and return UTF-8 output. Read-only commands run immediately; mutating commands require user confirmation.",
            vec![
                ("command", param("command", "string", "Command to execute")),
                ("timeout", param("timeout", "integer", "Timeout in seconds (default 30)")),
            ],
            vec!["command"],
        ),
        Box::new(|args| Box::pin(run_command(args))),
    );

    registry.register(
        tool(
            "list_processes",
            "List running processes (optional filter by process name).",
            vec![(
                "filter",
                param("filter", "string", "Filter by process name"),
            )],
            vec![],
        ),
        Box::new(|args| Box::pin(list_processes(args))),
    );

    registry.register(
        tool(
            "kill_process",
            "Terminate a process by exact name (e.g. notepad.exe) or integer PID (requires user confirmation; protected system processes cannot be killed).",
            vec![
                ("name", param("name", "string", "Exact process name (case-insensitive)")),
                ("pid", param("pid", "integer", "Process ID")),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(kill_process(args))),
    );

    registry.register(
        tool(
            "open_application",
            "Open a desktop application by name (e.g. notepad, calc, spotify, code).",
            vec![("name", param("name", "string", "Application name"))],
            vec!["name"],
        ),
        Box::new(|args| Box::pin(open_application(args))),
    );
}

async fn run_command(args: Value) -> ToolResult {
    let cmd = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if cmd.is_empty() {
        return ToolResult::err("No command specified.");
    }
    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .clamp(1, 120);

    // Force UTF-8 codepage so Vietnamese text is never garbled, and hide console window.
    let utf8_cmd = format!("chcp 65001 >nul && {cmd}");

    let mut command = tokio::process::Command::new("cmd");
    command.args(["/C", &utf8_cmd]);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        command.output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut out = stdout.to_string();
            if !stderr.is_empty() {
                out.push_str("\n[stderr] ");
                out.push_str(&stderr);
            }
            let final_out = if out.len() > 10_000 {
                format!("{}...(truncated)", truncate_chars(&out, 10_000))
            } else {
                out
            };
            if output.status.success() {
                ToolResult::ok(final_out)
            } else {
                ToolResult::err(format!("exit code {}: {final_out}", output.status))
            }
        }
        Ok(Err(e)) => ToolResult::err(format!("exec failed: {e}")),
        Err(_) => ToolResult::err(format!("command timed out after {timeout_secs}s")),
    }
}

async fn list_processes(args: Value) -> ToolResult {
    let filter = args
        .get("filter")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();

    tokio::task::spawn_blocking(move || {
        use sysinfo::{ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);

        let mut lines = Vec::new();
        for (pid, proc_) in sys.processes() {
            let name = proc_.name().to_string_lossy().to_string();
            if filter.is_empty() || name.to_lowercase().contains(&filter) {
                lines.push(format!(
                    "PID {} | {} | {:.1} MB",
                    pid,
                    name,
                    proc_.memory() as f64 / 1_048_576.0,
                ));
            }
        }
        lines.sort();
        lines.truncate(50);
        ToolResult::ok(lines.join("\n"))
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("list_processes task failed: {e}")))
}

async fn kill_process(args: Value) -> ToolResult {
    let pid_opt = args.get("pid").and_then(|v| v.as_u64());
    let name_opt = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    tokio::task::spawn_blocking(move || {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);

        if let Some(pid_num) = pid_opt {
            let pid = Pid::from(pid_num as usize);
            if let Some(proc_) = sys.process(pid) {
                let pname = proc_.name().to_string_lossy().to_string();
                if is_protected_process_name(&pname) {
                    return ToolResult::err(format!(
                        "Refused to terminate protected system process '{pname}' (PID {pid})."
                    ));
                }
                proc_.kill();
                return ToolResult::ok(format!("Terminated {pname} (PID {pid})"));
            }
            return ToolResult::err(format!("PID {pid} not found"));
        }

        if let Some(name) = name_opt {
            if is_protected_process_name(&name) {
                return ToolResult::err(format!(
                    "Refused to terminate protected system process '{name}'."
                ));
            }
            let lower = name.to_lowercase();
            let with_exe = if lower.ends_with(".exe") {
                lower.clone()
            } else {
                format!("{lower}.exe")
            };
            let mut killed = Vec::new();
            for (pid, proc_) in sys.processes() {
                let pname = proc_.name().to_string_lossy().to_string();
                let plower = pname.to_lowercase();
                if plower == lower || plower == with_exe {
                    if is_protected_process_name(&pname) {
                        continue;
                    }
                    proc_.kill();
                    killed.push(format!("{pname} ({pid})"));
                }
            }
            if !killed.is_empty() {
                return ToolResult::ok(format!("Terminated: {}", killed.join(", ")));
            }
            return ToolResult::err(format!("No process found with exact name '{name}'"));
        }

        ToolResult::err("Provide either 'pid' (integer) or 'name' (exact process name).")
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("kill_process task failed: {e}")))
}

async fn open_application(args: Value) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        return ToolResult::err("No application name specified.");
    }
    if is_blocked_executable(name) {
        return ToolResult::err(format!(
            "Blocked opening script/executable file directly via open_application: '{name}'. \
             Pass an app name or use run_command with confirmation."
        ));
    }
    match open::that(name) {
        Ok(_) => ToolResult::ok(format!("Opened {name}")),
        Err(e) => ToolResult::err(format!("Failed to open {name}: {e}")),
    }
}
