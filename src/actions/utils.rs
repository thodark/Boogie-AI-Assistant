use crate::AppEvent;
use crate::tools::{ToolRegistry, ToolResult};
use crate::util::truncate_chars;
use serde_json::Value;
use tokio::sync::broadcast;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    let tx_timer = event_tx;
    registry.register(
        tool(
            "set_timer",
            "Set a short in-app countdown timer (seconds). When it finishes, Boogie announces it aloud and shows a notification.",
            vec![
                ("seconds", param("seconds", "integer", "Seconds from now")),
                ("message", param("message", "string", "Timer message")),
            ],
            vec!["seconds"],
        ),
        Box::new(move |args| {
            let tx = tx_timer.clone();
            Box::pin(set_timer(args, tx))
        }),
    );

    registry.register(
        tool(
            "get_datetime",
            "Get the current local date and time.",
            vec![],
            vec![],
        ),
        Box::new(|args| Box::pin(get_datetime(args))),
    );

    registry.register(
        tool(
            "generate_qrcode",
            "Generate a QR code PNG image from text and open the mandatory Windows Save As dialog so the user chooses where to save it.",
            vec![
                ("data", param("data", "string", "Data to encode")),
                (
                    "path",
                    param(
                        "path",
                        "string",
                        "Suggested filename (e.g. 'wifi_qr.png'); user chooses the final save location in Save As dialog",
                    ),
                ),
            ],
            vec!["data"],
        ),
        Box::new(|args| Box::pin(generate_qrcode(args))),
    );

    registry.register(
        tool(
            "read_pdf",
            "Extract text from a PDF file.",
            vec![("path", param("path", "string", "PDF file path"))],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(read_pdf(args))),
    );

    registry.register(
        tool(
            "read_excel",
            "Read rows from an Excel spreadsheet.",
            vec![
                ("path", param("path", "string", "Excel file path")),
                (
                    "sheet",
                    param("sheet", "string", "Sheet name (default: first)"),
                ),
            ],
            vec!["path"],
        ),
        Box::new(|args| Box::pin(read_excel(args))),
    );
}

async fn set_timer(args: Value, event_tx: broadcast::Sender<AppEvent>) -> ToolResult {
    let seconds = args
        .get("seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .clamp(1, 86_400);
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Timer done!")
        .trim()
        .to_string();

    let msg_clone = message.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
        tracing::info!("Timer finished: {msg_clone}");
        let _ = event_tx.send(AppEvent::Log(format!("Timer finished: {msg_clone}")));
        let _ = event_tx.send(AppEvent::TimerAlert(msg_clone));
    });

    ToolResult::ok(format!("Timer set for {seconds} seconds ({message})."))
}

async fn get_datetime(_args: Value) -> ToolResult {
    let now = chrono::Local::now();
    ToolResult::ok(now.format("%Y-%m-%d %H:%M:%S %A").to_string())
}

async fn generate_qrcode(args: Value) -> ToolResult {
    let data = args
        .get("data")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let suggested = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("qrcode.png")
        .trim()
        .to_string();

    tokio::task::spawn_blocking(move || match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => {
            let img = code.render::<image::Luma<u8>>().build();
            let sp = std::path::Path::new(&suggested);
            let initial_dir = sp.parent().filter(|p| p.is_dir());
            let default_name = sp
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("qrcode.png");

            let chosen = match crate::actions::documents::pick_save_path(
                "Boogie — Save QR Code As",
                default_name,
                "PNG Image",
                &["png"],
                initial_dir,
                false,
            ) {
                Ok(Some(p)) => p,
                Ok(None) => {
                    crate::confirm::write_audit_log(
                        "generate_qrcode",
                        "CANCELLED_USER",
                        default_name,
                    );
                    return ToolResult::ok(
                        "User cancelled the Save As dialog. QR code was not saved.".to_string(),
                    );
                }
                Err(e) => return ToolResult::err(e),
            };

            match img.save(&chosen) {
                Ok(_) => {
                    let chosen_str = chosen.display().to_string();
                    crate::confirm::write_audit_log("generate_qrcode", "SAVED", &chosen_str);
                    ToolResult::ok(format!("QR code saved to {chosen_str}"))
                }
                Err(e) => ToolResult::err(format!("save QR: {e}")),
            }
        }
        Err(e) => ToolResult::err(format!("generate QR: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("generate_qrcode task failed: {e}")))
}

async fn read_pdf(args: Value) -> ToolResult {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || match pdf_extract::extract_text(&path) {
        Ok(text) => {
            let body = if text.len() > 50_000 {
                format!("{}...(truncated)", truncate_chars(&text, 50_000))
            } else {
                text
            };
            ToolResult::ok_untrusted("document", body)
        }
        Err(e) => ToolResult::err(format!("pdf extract: {e}")),
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("read_pdf task failed: {e}")))
}

async fn read_excel(args: Value) -> ToolResult {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let sheet = args
        .get("sheet")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    tokio::task::spawn_blocking(move || {
        use calamine::Reader;
        match calamine::open_workbook_auto(&path) {
            Ok(mut wb) => {
                let names: Vec<String> = wb.sheet_names().to_vec();
                let sheet_name = if let Some(s) = sheet {
                    s
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
                            let cells: Vec<String> = row.iter().map(|c| format!("{c}")).collect();
                            lines.push(cells.join("\t"));
                        }
                        ToolResult::ok_untrusted("document", lines.join("\n"))
                    }
                    Err(e) => ToolResult::err(format!("read sheet: {e}")),
                }
            }
            Err(e) => ToolResult::err(format!("open workbook: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| ToolResult::err(format!("read_excel task failed: {e}")))
}
