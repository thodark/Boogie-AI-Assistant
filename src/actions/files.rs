use crate::tools::{ToolRegistry, ToolResult};
use crate::util::truncate_chars;
use serde_json::Value;
use std::path::Path;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "read_file",
            "Read the contents of a text file.",
            vec![("path", param("path", "string", "File path to read"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(read_file(args))),
    );

    registry.register(
        tool(
            "write_file",
            "Write text content to a file. ALWAYS opens the native Windows Save As dialog so the user \
             chooses the exact folder and filename before writing (cancels cleanly if the user clicks Cancel).",
            vec![
                (
                    "path",
                    param(
                        "path",
                        "string",
                        "Suggested filename or starting path (user chooses the final save location in Save As dialog)",
                    ),
                ),
                ("content", param("content", "string", "Content to write")),
            ],
            vec!["path", "content"],
        ),
        Box::new(|args| Box::pin(write_file(args))),
    );

    registry.register(
        tool(
            "list_directory",
            "List files and folders in a directory.",
            vec![("path", param("path", "string", "Directory path"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(list_directory(args))),
    );

    registry.register(
        tool(
            "delete_file",
            "Move a file or folder to the recycle bin (requires user confirmation).",
            vec![("path", param("path", "string", "Path to delete"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(delete_file(args))),
    );

    registry.register(
        tool(
            "copy_file",
            "Copy a file or folder.",
            vec![
                ("source", param("source", "string", "Source path")),
                (
                    "destination",
                    param("destination", "string", "Destination path"),
                ),
            ],
            vec!["source", "destination"],
        ),
        Box::new(|args| Box::pin(copy_file(args))),
    );

    registry.register(
        tool(
            "move_file",
            "Move or rename a file or folder (requires user confirmation).",
            vec![
                ("source", param("source", "string", "Source path")),
                (
                    "destination",
                    param("destination", "string", "Destination path"),
                ),
            ],
            vec!["source", "destination"],
        ),
        Box::new(|args| Box::pin(move_file(args))),
    );

    registry.register(
        tool(
            "search_files",
            "Search for files matching a pattern.",
            vec![
                (
                    "directory",
                    param("directory", "string", "Directory to search in"),
                ),
                (
                    "pattern",
                    param("pattern", "string", "Glob pattern like *.txt"),
                ),
            ],
            vec!["directory", "pattern"],
        ),
        Box::new(|args| Box::pin(search_files(args))),
    );
}

async fn read_file(args: Value) -> ToolResult {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || match std::fs::read_to_string(&path) {
        Ok(content) => {
            let body = if content.len() > 50_000 {
                format!(
                    "{}...(truncated at 50000 bytes)",
                    truncate_chars(&content, 50_000)
                )
            } else {
                content
            };
            ToolResult::ok_untrusted("file", body)
        }
        Err(e) => ToolResult::err(format!("cannot read {path}: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("read_file task failed: {e}")))
}

async fn write_file(args: Value) -> ToolResult {
    let suggested = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("note.txt")
        .trim()
        .to_string();
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    tokio::task::spawn_blocking(move || {
        let suggested_path = Path::new(&suggested);
        let initial_dir = suggested_path.parent().filter(|p| p.is_dir());
        let default_name = suggested_path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("note.txt");
        let ext = suggested_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("txt");
        let exts: Vec<&str> = if ext.is_empty() {
            vec!["txt"]
        } else {
            vec![ext]
        };

        let chosen = match crate::actions::documents::pick_save_path(
            "Boogie — Save File As",
            default_name,
            "Text / Code File",
            &exts,
            initial_dir,
            true,
        ) {
            Ok(Some(p)) => p,
            Ok(None) => {
                crate::confirm::write_audit_log("write_file", "CANCELLED_USER", default_name);
                return ToolResult::ok(
                    "User cancelled the Save As dialog. No file was written.".to_string(),
                );
            }
            Err(e) => return ToolResult::err(e),
        };

        if let Some(parent) = chosen.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).ok();
        }
        match std::fs::write(&chosen, &content) {
            Ok(_) => {
                let chosen_str = chosen.display().to_string();
                crate::confirm::write_audit_log("write_file", "SAVED", &chosen_str);
                ToolResult::ok(format!(
                    "Saved file to {} ({} bytes)",
                    chosen_str,
                    content.len()
                ))
            }
            Err(e) => ToolResult::err(format!("cannot write {}: {e}", chosen.display())),
        }
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("write_file task failed: {e}")))
}

async fn list_directory(args: Value) -> ToolResult {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();
    tokio::task::spawn_blocking(move || match std::fs::read_dir(&path) {
        Ok(entries) => {
            let mut lines = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                lines.push(if is_dir { format!("{name}/") } else { name });
            }
            lines.sort();
            ToolResult::ok(lines.join("\n"))
        }
        Err(e) => ToolResult::err(format!("cannot list {path}: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("list_directory task failed: {e}")))
}

async fn delete_file(args: Value) -> ToolResult {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || match trash::delete(&path) {
        Ok(_) => ToolResult::ok(format!("moved to recycle bin: {path}")),
        Err(e) => ToolResult::err(format!("delete failed: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("delete_file task failed: {e}")))
}

async fn copy_file(args: Value) -> ToolResult {
    let src = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let dst = args
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || {
        let p = Path::new(&src);
        if p.is_dir() {
            fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
                std::fs::create_dir_all(dst)?;
                for entry in std::fs::read_dir(src)? {
                    let entry = entry?;
                    let target = dst.join(entry.file_name());
                    if entry.file_type()?.is_dir() {
                        copy_dir(&entry.path(), &target)?;
                    } else {
                        std::fs::copy(entry.path(), target)?;
                    }
                }
                Ok(())
            }
            match copy_dir(p, Path::new(&dst)) {
                Ok(_) => ToolResult::ok(format!("copied dir {src} -> {dst}")),
                Err(e) => ToolResult::err(format!("copy dir failed: {e}")),
            }
        } else {
            match std::fs::copy(&src, &dst) {
                Ok(_) => ToolResult::ok(format!("copied {src} -> {dst}")),
                Err(e) => ToolResult::err(format!("copy failed: {e}")),
            }
        }
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("copy_file task failed: {e}")))
}

async fn move_file(args: Value) -> ToolResult {
    let src = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let dst = args
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || match std::fs::rename(&src, &dst) {
        Ok(_) => ToolResult::ok(format!("moved {src} -> {dst}")),
        Err(e) => ToolResult::err(format!("move failed: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("move_file task failed: {e}")))
}

async fn search_files(args: Value) -> ToolResult {
    let dir = args
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();

    tokio::task::spawn_blocking(move || {
        fn walk(dir: &Path, pattern: &str, results: &mut Vec<String>, depth: usize) {
            if depth > 10 || results.len() >= 200 {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if results.len() >= 200 {
                        return;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    let path = entry.path();
                    if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        walk(&path, pattern, results, depth + 1);
                    } else if matches_glob(&name, pattern) {
                        results.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }

        fn matches_glob(name: &str, pattern: &str) -> bool {
            let nl = name.to_lowercase();
            let pl = pattern.to_lowercase();
            if pl == "*" {
                return true;
            }
            if let Some(ext) = pl.strip_prefix("*.") {
                return nl.ends_with(&format!(".{ext}"));
            }
            if let Some(prefix) = pl.strip_suffix('*') {
                return nl.starts_with(prefix);
            }
            nl == pl
        }

        let mut results = Vec::new();
        walk(Path::new(&dir), &pattern, &mut results, 0);
        ToolResult::ok(results.join("\n"))
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("search_files task failed: {e}")))
}
