use crate::tools::{ToolRegistry, ToolResult};
use chrono::{Datelike, NaiveDate};
use serde_json::Value;

use super::{param, tool};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        tool(
            "flight_finder",
            "Search for flights on Google Flights. Pass origin, destination, and departure date in YYYY-MM-DD format \
             (also supports 'today', 'tomorrow', 'hôm nay', 'ngày mai').",
            vec![
                (
                    "origin",
                    param("origin", "string", "Departure city or IATA code (e.g. SGN, Ho Chi Minh)"),
                ),
                (
                    "destination",
                    param("destination", "string", "Arrival city or IATA code (e.g. NRT, Tokyo)"),
                ),
                (
                    "date",
                    param("date", "string", "Departure date in YYYY-MM-DD format"),
                ),
                (
                    "return_date",
                    param("return_date", "string", "Return date in YYYY-MM-DD format (optional)"),
                ),
                (
                    "passengers",
                    param("passengers", "string", "Number of passengers (default: 1)"),
                ),
                (
                    "cabin",
                    param("cabin", "string", "economy | premium | business | first (default: economy)"),
                ),
            ],
            vec!["origin", "destination", "date"],
        ),
        Box::new(|args| Box::pin(flight_finder(args))),
    );
}

pub fn parse_date(raw: &str, today: NaiveDate) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Date is required (use YYYY-MM-DD).".to_string());
    }

    if let Ok(dt) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(dt.format("%Y-%m-%d").to_string());
    }

    let lower = trimmed.to_lowercase();
    if lower == "today" || lower == "hôm nay" || lower == "hom nay" {
        return Ok(today.format("%Y-%m-%d").to_string());
    }
    if lower == "tomorrow" || lower == "ngày mai" || lower == "ngay mai" || lower == "mai" {
        return Ok((today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string());
    }
    if lower == "ngày mốt" || lower == "ngay mot" || lower == "mốt" {
        return Ok((today + chrono::Duration::days(2))
            .format("%Y-%m-%d")
            .to_string());
    }

    let months = [
        ("january", 1),
        ("february", 2),
        ("march", 3),
        ("april", 4),
        ("may", 5),
        ("june", 6),
        ("july", 7),
        ("august", 8),
        ("september", 9),
        ("october", 10),
        ("november", 11),
        ("december", 12),
    ];
    let day_re = regex::Regex::new(r"\d{1,2}").unwrap();
    for (name, num) in months {
        if lower.contains(name)
            && let Some(day) = day_re.find(trimmed)
            && let Ok(d) = day.as_str().parse::<u32>()
        {
            let year = if num >= today.month() {
                today.year()
            } else {
                today.year() + 1
            };
            if let Some(dt) = NaiveDate::from_ymd_opt(year, num, d) {
                return Ok(dt.format("%Y-%m-%d").to_string());
            }
        }
    }

    Err(format!(
        "Could not parse date '{trimmed}'. Please provide date in YYYY-MM-DD format."
    ))
}

pub fn build_flights_url(
    origin: &str,
    destination: &str,
    date: &str,
    return_date: Option<&str>,
    passengers: u32,
    cabin: &str,
) -> String {
    let mut query = if let Some(ret) = return_date {
        format!("Flights from {origin} to {destination} on {date} returning {ret}")
    } else {
        format!("One way flights from {origin} to {destination} on {date}")
    };
    if passengers > 1 {
        query.push_str(&format!(" for {passengers} adults"));
    }
    let cabin_lower = cabin.trim().to_lowercase();
    if !cabin_lower.is_empty() && cabin_lower != "economy" {
        query.push_str(&format!(" in {cabin_lower} class"));
    }
    format!(
        "https://www.google.com/travel/flights?q={}",
        urlencoding::encode(&query)
    )
}

async fn flight_finder(args: Value) -> ToolResult {
    let origin = args
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let destination = args
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let date_raw = args
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let return_raw = args
        .get("return_date")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let passengers: u32 = args
        .get("passengers")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(1)
        .clamp(1, 9) as u32;
    let cabin = args
        .get("cabin")
        .and_then(|v| v.as_str())
        .unwrap_or("economy")
        .trim()
        .to_string();

    if origin.is_empty() || destination.is_empty() {
        return ToolResult::err("Please specify both origin and destination.");
    }

    let today = chrono::Local::now().date_naive();
    let date = match parse_date(&date_raw, today) {
        Ok(d) => d,
        Err(e) => return ToolResult::err(e),
    };
    let return_date = if return_raw.is_empty() {
        None
    } else {
        match parse_date(&return_raw, today) {
            Ok(d) => Some(d),
            Err(e) => return ToolResult::err(format!("Return date error: {e}")),
        }
    };

    let url = build_flights_url(
        &origin,
        &destination,
        &date,
        return_date.as_deref(),
        passengers,
        &cabin,
    );

    match open::that(&url) {
        Ok(_) => {
            let mut msg = format!("Opened Google Flights for {origin} -> {destination} on {date}");
            if let Some(ref ret) = return_date {
                msg.push_str(&format!(", returning {ret}"));
            }
            if passengers > 1 {
                msg.push_str(&format!(", {passengers} passengers"));
            }
            if !cabin.eq_ignore_ascii_case("economy") {
                msg.push_str(&format!(", {cabin} class"));
            }
            msg.push('.');
            ToolResult::ok(msg)
        }
        Err(e) => ToolResult::err(format!("Failed to open Google Flights: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_and_vietnamese_dates_and_rejects_garbage() {
        let base = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        assert_eq!(parse_date("2026-12-05", base).unwrap(), "2026-12-05");
        assert_eq!(parse_date("ngày mai", base).unwrap(), "2026-11-23");
        assert_eq!(parse_date("hôm nay", base).unwrap(), "2026-11-22");
        assert!(parse_date("not-a-date", base).is_err());
    }

    #[test]
    fn flight_url_properly_encodes_spaces_and_vietnamese() {
        let url = build_flights_url("Hồ Chí Minh", "Tokyo", "2026-12-05", None, 2, "business");
        assert!(url.starts_with("https://www.google.com/travel/flights?q="));
        assert!(!url.contains(' '));
        assert!(!url.contains("&cabin="));
        assert!(!url.contains("&adults="));
    }
}
