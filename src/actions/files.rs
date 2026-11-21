use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;
use std::path::Path;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "read_file",
            "Read the contents of a file",
            vec![("path", param("path", "string", "File path to read"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(read_file(args))),
    );

    registry.register(
        tool(
            "write_file",
            "Write content to a file",
            vec![
                ("path", param("path", "string", "File path to write")),
                ("content", param("content", "string", "Content to write")),
            ],
            vec!["path", "content"],
        ),
        Box::new(|args| Box::pin(write_file(args))),
    );

    registry.register(
        tool(
            "list_directory",
            "List files and folders in a directory",
            vec![("path", param("path", "string", "Directory path"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(list_directory(args))),
    );

    registry.register(
        tool(
            "delete_file",
            "Move a file or folder to the recycle bin",
            vec![("path", param("path", "string", "Path to delete"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(delete_file(args))),
    );

    registry.register(
        tool(
            "copy_file",
            "Copy a file or folder",
            vec![
                ("source", param("source", "string", "Source path")),
                ("destination", param("destination", "string", "Destination path")),
            ],
            vec!["source", "destination"],
        ),
        Box::new(|args| Box::pin(copy_file(args))),
    );

    registry.register(
        tool(
            "move_file",
            "Move/rename a file or folder",
            vec![
                ("source", param("source", "string", "Source path")),
                ("destination", param("destination", "string", "Destination path")),
            ],
            vec!["source", "destination"],
        ),
        Box::new(|args| Box::pin(move_file(args))),
    );

    registry.register(
        tool(
            "search_files",
            "Search for files matching a pattern",
            vec![
                ("directory", param("directory", "string", "Directory to search in")),
                ("pattern", param("pattern", "string", "Glob pattern like *.txt")),
            ],
            vec!["directory", "pattern"],
        ),
        Box::new(|args| Box::pin(search_files(args))),
    );
}

async fn read_file(args: Value) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    match std::fs::read_to_string(path) {
        Ok(content) => {
            if content.len() > 50_000 {
                ToolResult::ok(format!("{}...(truncated at 50000 chars)", &content[..50_000]))
            } else {
                ToolResult::ok(content)
            }
        }
        Err(e) => ToolResult::err(format!("cannot read {path}: {e}")),
    }
}

async fn write_file(args: Value) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or_default();
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match std::fs::write(path, content) {
        Ok(_) => ToolResult::ok(format!("wrote {path} ({} bytes)", content.len())),
        Err(e) => ToolResult::err(format!("cannot write {path}: {e}")),
    }
}

async fn list_directory(args: Value) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut lines = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                lines.push(if is_dir {
                    format!("{name}/")
                } else {
                    name
                });
            }
            lines.sort();
            ToolResult::ok(lines.join("\n"))
        }
        Err(e) => ToolResult::err(format!("cannot list {path}: {e}")),
    }
}

async fn delete_file(args: Value) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    match trash::delete(path) {
        Ok(_) => ToolResult::ok(format!("moved to recycle bin: {path}")),
        Err(e) => ToolResult::err(format!("delete failed: {e}")),
    }
}

async fn copy_file(args: Value) -> ToolResult {
    let src = args.get("source").and_then(|v| v.as_str()).unwrap_or_default();
    let dst = args.get("destination").and_then(|v| v.as_str()).unwrap_or_default();
    let p = Path::new(src);
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
        match copy_dir(p, Path::new(dst)) {
            Ok(_) => ToolResult::ok(format!("copied dir {src} -> {dst}")),
            Err(e) => ToolResult::err(format!("copy dir failed: {e}")),
        }
    } else {
        match std::fs::copy(src, dst) {
            Ok(_) => ToolResult::ok(format!("copied {src} -> {dst}")),
            Err(e) => ToolResult::err(format!("copy failed: {e}")),
        }
    }
}

async fn move_file(args: Value) -> ToolResult {
    let src = args.get("source").and_then(|v| v.as_str()).unwrap_or_default();
    let dst = args.get("destination").and_then(|v| v.as_str()).unwrap_or_default();
    match std::fs::rename(src, dst) {
        Ok(_) => ToolResult::ok(format!("moved {src} -> {dst}")),
        Err(e) => ToolResult::err(format!("move failed: {e}")),
    }
}

async fn search_files(args: Value) -> ToolResult {
    let dir = args.get("directory").and_then(|v| v.as_str()).unwrap_or(".");
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");

    fn walk(dir: &Path, pattern: &str, results: &mut Vec<String>, depth: usize) {
        if depth > 10 {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
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
        if pattern == "*" {
            return true;
        }
        if let Some(ext) = pattern.strip_prefix("*.") {
            return name.ends_with(&format!(".{ext}"));
        }
        if let Some(prefix) = pattern.strip_suffix("*") {
            return name.starts_with(prefix);
        }
        name == pattern
    }

    let mut results = Vec::new();
    walk(Path::new(dir), pattern, &mut results, 0);
    results.truncate(200);
    ToolResult::ok(results.join("\n"))
}
