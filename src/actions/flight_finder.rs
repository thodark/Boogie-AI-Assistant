use crate::tools::{ToolRegistry, ToolResult};
use chrono::Datelike;
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "flight_finder",
            "Search for flights. Opens Google Flights with the specified route and date. \
             Supports natural date formats and IATA airport codes or city names.",
            vec![
                ("origin", param("origin", "string", "Departure city or IATA code (e.g. SGN, Ho Chi Minh)")),
                ("destination", param("destination", "string", "Arrival city or IATA code (e.g. NRT, Tokyo)")),
                ("date", param("date", "string", "Departure date (YYYY-MM-DD, or natural like 'tomorrow', 'March 15')")),
                ("return_date", param("return_date", "string", "Return date (optional, for round trip)")),
                ("passengers", param("passengers", "string", "Number of passengers (default: 1)")),
                ("cabin", param("cabin", "string", "economy | premium | business | first (default: economy)")),
            ],
            vec!["origin", "destination", "date"],
        ),
        Box::new(|args| Box::pin(flight_finder(args))),
    );
}

fn parse_date(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_lowercase();

    if regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(trimmed) {
        return trimmed.to_string();
    }

    let today = chrono::Local::now().date_naive();

    if lower.contains("today") {
        return today.format("%Y-%m-%d").to_string();
    }
    if lower.contains("tomorrow") {
        return (today + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
    }

    let months = [
        ("january", 1), ("february", 2), ("march", 3), ("april", 4),
        ("may", 5), ("june", 6), ("july", 7), ("august", 8),
        ("september", 9), ("october", 10), ("november", 11), ("december", 12),
    ];
    for (name, num) in months {
        if lower.contains(name) {
            if let Some(day) = regex::Regex::new(r"\d{1,2}").unwrap().find(trimmed) {
                let d: u32 = day.as_str().parse().unwrap_or(1);
                let year = if num >= today.month() { today.year() } else { today.year() + 1 };
                return format!("{year}-{num:02}-{d:02}");
            }
        }
    }

    today.format("%Y-%m-%d").to_string()
}

fn cabin_code(cabin: &str) -> &str {
    match cabin.to_lowercase().as_str() {
        "premium" => "2",
        "business" => "3",
        "first" => "4",
        _ => "1",
    }
}

async fn flight_finder(args: Value) -> ToolResult {
    let origin = args.get("origin").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let destination = args.get("destination").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let date_raw = args.get("date").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let return_raw = args.get("return_date").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    let passengers: u32 = args.get("passengers").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cabin = args.get("cabin").and_then(|v| v.as_str()).unwrap_or("economy").trim().to_string();

    if origin.is_empty() || destination.is_empty() {
        return ToolResult::err("Please specify both origin and destination.");
    }

    let date = parse_date(&date_raw);
    let return_date = if return_raw.is_empty() { None } else { Some(parse_date(&return_raw)) };

    let trip = if let Some(ref ret) = return_date {
        format!("Flights+from+{origin}+to+{destination}+on+{date}+returning+{ret}")
    } else {
        format!("Flights+from+{origin}+to+{destination}+on+{date}")
    };

    let cc = cabin_code(&cabin);
    let url = format!(
        "https://www.google.com/travel/flights?q={trip}&curr=USD&cabin={cc}&adults={passengers}"
    );

    match open::that(&url) {
        Ok(_) => {
            let mut msg = format!(
                "Opened Google Flights for {origin} → {destination} on {date}"
            );
            if let Some(ref ret) = return_date {
                msg.push_str(&format!(", returning {ret}"));
            }
            if passengers > 1 {
                msg.push_str(&format!(", {passengers} passengers"));
            }
            if cabin != "economy" {
                msg.push_str(&format!(", {} class", cabin));
            }
            msg.push('.');
            ToolResult::ok(msg)
        }
        Err(e) => ToolResult::err(format!("Failed to open Google Flights: {e}")),
    }
}
