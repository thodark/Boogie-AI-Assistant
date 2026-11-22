use crate::AppEvent;
use crate::config::{MemoryEntry, search_memory, update_memory};
use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::broadcast;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    let tx_shutdown = event_tx;
    registry.register(
        tool(
            "shutdown_boogie",
            "Shut down Boogie (the assistant itself). This does NOT shut down the computer. \
             Use this when the user asks to turn off, close, or shut down Boogie/the assistant. \
             NEVER use run_command for this.",
            vec![],
            vec![],
        ),
        Box::new(move |args| {
            let tx = tx_shutdown.clone();
            Box::pin(shutdown_boogie(args, tx))
        }),
    );

    registry.register(
        tool(
            "save_memory",
            "Silently save a fact about the user to long-term memory. Use whenever you learn \
             something important about the user (preferences, name, language, habits). \
             Do NOT announce that you are saving — just do it silently.",
            vec![
                (
                    "category",
                    param(
                        "category",
                        "string",
                        "Memory category (e.g. identity, preferences, habits)",
                    ),
                ),
                ("key", param("key", "string", "Key name for this memory")),
                ("value", param("value", "string", "The value to remember")),
            ],
            vec!["key", "value"],
        ),
        Box::new(|args| Box::pin(save_memory_handler(args))),
    );

    registry.register(
        tool(
            "recall_memory",
            "Search stored memories about the user. Use when you need to remember something about the user.",
            vec![
                (
                    "query",
                    param("query", "string", "Search query (empty to list all)"),
                ),
                ("limit", param("limit", "integer", "Max results to return")),
            ],
            vec![],
        ),
        Box::new(|args| Box::pin(recall_memory_handler(args))),
    );

    registry.register(
        tool(
            "system_status",
            "Get current system status: CPU usage, RAM, disk space, uptime, OS.",
            vec![],
            vec![],
        ),
        Box::new(|args| Box::pin(system_status(args))),
    );

    registry.register(
        tool(
            "undo",
            "Undo the last reversible action (volume change, dark mode toggle, etc.).",
            vec![],
            vec![],
        ),
        Box::new(|args| Box::pin(undo_handler(args))),
    );
}

async fn shutdown_boogie(_args: Value, event_tx: broadcast::Sender<AppEvent>) -> ToolResult {
    tracing::info!("shutdown_boogie called — requesting graceful shutdown");
    let _ = event_tx.send(AppEvent::Shutdown);
    ToolResult::ok("Shutting down Boogie gracefully.")
}

async fn save_memory_handler(args: Value) -> ToolResult {
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("general");
    let key = args.get("key").and_then(|v| v.as_str()).unwrap_or_default();
    let value = args
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if key.is_empty() || value.is_empty() {
        return ToolResult::err("key and value are required");
    }
    let mut cat_map = HashMap::new();
    cat_map.insert(
        key.to_string(),
        MemoryEntry {
            value: value.to_string(),
        },
    );
    let mut mem = HashMap::new();
    mem.insert(category.to_string(), cat_map);
    update_memory(&mem);
    ToolResult::ok(format!("saved {category}/{key}"))
}

async fn recall_memory_handler(args: Value) -> ToolResult {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let result = search_memory(query, limit);
    ToolResult::ok(result)
}

async fn system_status(_args: Value) -> ToolResult {
    tokio::task::spawn_blocking(|| {
        use sysinfo::{Disks, System};
        let mut sys = System::new_all();
        sys.refresh_cpu_usage();
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let disk_info = {
            let disks = Disks::new_with_refreshed_list();
            let mut lines = Vec::new();
            for disk in disks.list() {
                let total = disk.total_space() as f64 / 1_073_741_824.0;
                let free = disk.available_space() as f64 / 1_073_741_824.0;
                let used = total - free;
                let pct = if total > 0.0 {
                    used / total * 100.0
                } else {
                    0.0
                };
                lines.push(format!(
                    "  {}: {:.1}/{:.1} GB ({:.0}% used)",
                    disk.mount_point().display(),
                    used,
                    total,
                    pct,
                ));
            }
            lines.join("\n")
        };

        let total_mem = sys.total_memory().max(1) as f64;
        let used_mem = sys.used_memory() as f64;

        let info = format!(
            "CPU: {} cores, {:.1}% usage\n\
             RAM: {:.1} GB / {:.1} GB ({:.0}% used)\n\
             Disks:\n{}\n\
             Uptime: {} min\n\
             OS: {} {}",
            sys.cpus().len(),
            sys.global_cpu_usage(),
            used_mem / 1_073_741_824.0,
            total_mem / 1_073_741_824.0,
            used_mem / total_mem * 100.0,
            disk_info,
            System::uptime() / 60,
            System::name().unwrap_or_default(),
            System::os_version().unwrap_or_default(),
        );
        ToolResult::ok(info)
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("system_status failed: {e}")))
}

async fn undo_handler(_args: Value) -> ToolResult {
    tokio::task::spawn_blocking(super::computer_control::pop_undo)
        .await
        .ok()
        .flatten()
        .map(ToolResult::ok)
        .unwrap_or_else(|| ToolResult::err("No undoable actions in history."))
}
