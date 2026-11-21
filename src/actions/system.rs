use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "run_command",
            "Execute a shell command and return output",
            vec![
                ("command", param("command", "string", "Command to execute")),
                ("timeout", param("timeout", "integer", "Timeout in seconds")),
            ],
            vec!["command"],
        ),
        Box::new(|args| Box::pin(run_command(args))),
    );

    registry.register(
        tool(
            "get_system_info",
            "Get system information (CPU, RAM, disk, etc.)",
            vec![],
            vec![],
        ),
        Box::new(|args| Box::pin(get_system_info(args))),
    );

    registry.register(
        tool(
            "list_processes",
            "List running processes",
            vec![("filter", param("filter", "string", "Filter by process name"))],
            vec![],
        ),
        Box::new(|args| Box::pin(list_processes(args))),
    );

    registry.register(
        tool(
            "kill_process",
            "Kill a process by name or PID",
            vec![
                ("name", param("name", "string", "Process name")),
                ("pid", param("pid", "integer", "Process ID")),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(kill_process(args))),
    );

    registry.register(
        tool(
            "open_application",
            "Open an application by name or path",
            vec![("name", param("name", "string", "Application name or path"))],
            vec!["name"],
        ),
        Box::new(|args| Box::pin(open_application(args))),
    );
}

async fn run_command(args: Value) -> ToolResult {
    let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or_default();
    let timeout_secs = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);

    let cmd_lower = cmd.to_lowercase();
    let dangerous = ["shutdown", "restart", "reboot", "poweroff", "format ", "del /f", "rm -rf", "rmdir /s"];
    if dangerous.iter().any(|kw| cmd_lower.contains(kw)) {
        return ToolResult::err(
            "BLOCKED: This command could shut down, restart, or destructively modify the computer. \
             If the user wants to shut down Boogie (the assistant), use the shutdown_boogie tool instead. \
             If the user truly wants to run a dangerous system command, they must confirm it themselves."
        );
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new("cmd")
            .args(["/C", cmd])
            .output(),
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
            if out.len() > 10_000 {
                out.truncate(10_000);
                out.push_str("...(truncated)");
            }
            if output.status.success() {
                ToolResult::ok(out)
            } else {
                ToolResult::err(format!("exit code {}: {out}", output.status))
            }
        }
        Ok(Err(e)) => ToolResult::err(format!("exec failed: {e}")),
        Err(_) => ToolResult::err(format!("command timed out after {timeout_secs}s")),
    }
}

async fn get_system_info(_args: Value) -> ToolResult {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_all();

    let info = format!(
        "OS: {} {}\nCPU: {} cores, {}%\nRAM: {:.1} GB / {:.1} GB\nUptime: {} min",
        System::name().unwrap_or_default(),
        System::os_version().unwrap_or_default(),
        sys.cpus().len(),
        sys.global_cpu_usage(),
        sys.used_memory() as f64 / 1_073_741_824.0,
        sys.total_memory() as f64 / 1_073_741_824.0,
        System::uptime() / 60,
    );
    ToolResult::ok(info)
}

async fn list_processes(args: Value) -> ToolResult {
    use sysinfo::System;
    let filter = args
        .get("filter")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();

    let mut sys = System::new_all();
    sys.refresh_all();

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
}

async fn kill_process(args: Value) -> ToolResult {
    use sysinfo::{Pid, System};
    let mut sys = System::new_all();
    sys.refresh_all();

    if let Some(pid) = args.get("pid").and_then(|v| v.as_u64()) {
        let pid = Pid::from(pid as usize);
        if let Some(proc_) = sys.process(pid) {
            proc_.kill();
            return ToolResult::ok(format!("killed PID {pid}"));
        }
        return ToolResult::err(format!("PID {pid} not found"));
    }

    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        let lower = name.to_lowercase();
        let mut killed = 0;
        for (_pid, proc_) in sys.processes() {
            if proc_.name().to_string_lossy().to_lowercase().contains(&lower) {
                proc_.kill();
                killed += 1;
            }
        }
        if killed > 0 {
            return ToolResult::ok(format!("killed {killed} processes matching '{name}'"));
        }
        return ToolResult::err(format!("no processes matching '{name}'"));
    }

    ToolResult::err("provide either 'pid' or 'name'")
}

async fn open_application(args: Value) -> ToolResult {
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    match open::that(name) {
        Ok(_) => ToolResult::ok(format!("opened {name}")),
        Err(e) => ToolResult::err(format!("failed to open {name}: {e}")),
    }
}
