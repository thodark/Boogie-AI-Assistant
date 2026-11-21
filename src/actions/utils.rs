use crate::tools::{ToolRegistry, ToolResult};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "set_timer",
            "Set a timer or reminder",
            vec![
                ("seconds", param("seconds", "integer", "Seconds from now")),
                ("message", param("message", "string", "Reminder message")),
            ],
            vec!["seconds"],
        ),
        Box::new(|args| Box::pin(set_timer(args))),
    );

    registry.register(
        tool(
            "get_datetime",
            "Get the current date and time",
            vec![],
            vec![],
        ),
        Box::new(|args| Box::pin(get_datetime(args))),
    );

    registry.register(
        tool(
            "generate_qrcode",
            "Generate a QR code from text",
            vec![
                ("data", param("data", "string", "Data to encode")),
                ("path", param("path", "string", "Output file path")),
            ],
            vec!["data"],
        ),
        Box::new(|args| Box::pin(generate_qrcode(args))),
    );

    registry.register(
        tool(
            "read_pdf",
            "Extract text from a PDF file",
            vec![("path", param("path", "string", "PDF file path"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(read_pdf(args))),
    );

    registry.register(
        tool(
            "read_excel",
            "Read data from an Excel file",
            vec![
                ("path", param("path", "string", "Excel file path")),
                ("sheet", param("sheet", "string", "Sheet name (default: first)")),
            ],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(read_excel(args))),
    );

    registry.register(
        tool(
            "update_memory",
            "Store information in long-term memory",
            vec![
                ("category", param("category", "string", "Memory category")),
                ("key", param("key", "string", "Memory key")),
                ("value", param("value", "string", "Value to remember")),
            ],
            vec!["category", "key", "value"],
        ),
        Box::new(|args| Box::pin(update_memory_action(args))),
    );

    registry.register(
        tool(
            "search_memory",
            "Search stored memories",
            vec![("query", param("query", "string", "Search query"))],
            vec![],
        ),
        Box::new(|args| Box::pin(search_memory_action(args))),
    );
}

async fn set_timer(args: Value) -> ToolResult {
    let seconds = args.get("seconds").and_then(|v| v.as_u64()).unwrap_or(60);
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Timer done!")
        .to_string();

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
        tracing::info!("TIMER: {message}");
    });

    ToolResult::ok(format!("timer set for {seconds} seconds"))
}

async fn get_datetime(_args: Value) -> ToolResult {
    let now = chrono::Local::now();
    ToolResult::ok(now.format("%Y-%m-%d %H:%M:%S %A").to_string())
}

async fn generate_qrcode(args: Value) -> ToolResult {
    let data = args.get("data").and_then(|v| v.as_str()).unwrap_or_default();
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("qrcode.png");

    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => {
            let img = code.render::<image::Luma<u8>>().build();
            match img.save(path) {
                Ok(_) => ToolResult::ok(format!("QR code saved to {path}")),
                Err(e) => ToolResult::err(format!("save QR: {e}")),
            }
        }
        Err(e) => ToolResult::err(format!("generate QR: {e}")),
    }
}

async fn read_pdf(args: Value) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    match pdf_extract::extract_text(path) {
        Ok(text) => {
            if text.len() > 50_000 {
                ToolResult::ok(format!("{}...(truncated)", &text[..50_000]))
            } else {
                ToolResult::ok(text)
            }
        }
        Err(e) => ToolResult::err(format!("pdf extract: {e}")),
    }
}

async fn read_excel(args: Value) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    let sheet = args.get("sheet").and_then(|v| v.as_str());

    use calamine::Reader;
    match calamine::open_workbook_auto(path) {
        Ok(mut wb) => {
            let names: Vec<String> = wb.sheet_names().to_vec();
            let sheet_name = if let Some(s) = sheet {
                s.to_string()
            } else {
                match names.first() {
                    Some(name) => name.clone(),
                    None => return ToolResult::err("no sheets found"),
                }
            };

            match wb.worksheet_range(&sheet_name) {
                Ok(range) => {
                    let mut lines = Vec::new();
                    for row in range.rows().take(100) {
                        let cells: Vec<String> = row
                            .iter()
                            .map(|c| format!("{c}"))
                            .collect();
                        lines.push(cells.join("\t"));
                    }
                    ToolResult::ok(lines.join("\n"))
                }
                Err(e) => ToolResult::err(format!("read sheet: {e}")),
            }
        }
        Err(e) => ToolResult::err(format!("open workbook: {e}")),
    }
}

async fn update_memory_action(args: Value) -> ToolResult {
    let cat = args.get("category").and_then(|v| v.as_str()).unwrap_or("general");
    let key = args.get("key").and_then(|v| v.as_str()).unwrap_or_default();
    let value = args.get("value").and_then(|v| v.as_str()).unwrap_or_default();

    let mut updates = crate::config::Memory::new();
    let mut cat_map = crate::config::MemoryCategory::new();
    cat_map.insert(
        key.to_string(),
        crate::config::MemoryEntry {
            value: value.to_string(),
        },
    );
    updates.insert(cat.to_string(), cat_map);
    crate::config::update_memory(&updates);

    ToolResult::ok(format!("remembered {cat}/{key}"))
}

async fn search_memory_action(args: Value) -> ToolResult {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
    ToolResult::ok(crate::config::search_memory(query, 20))
}
