pub mod background_monitor;
mod browser;
mod computer_control;
mod files;
mod flight_finder;
mod game_updater;
mod inline;
mod media;
mod reminder;
mod screen;
mod send_message;
mod system;
mod utils;
mod video_player;
mod web;
mod youtube;

use crate::AppEvent;
use crate::tools::{ToolDeclaration, ToolRegistry};
use serde_json::{Value, json};
use tokio::sync::broadcast;

pub fn register_all(registry: &mut ToolRegistry, event_tx: broadcast::Sender<AppEvent>) {
    screen::register(registry);
    browser::register(registry);
    computer_control::register(registry);
    files::register(registry);
    inline::register(registry, event_tx.clone());
    reminder::register(registry);
    system::register(registry);
    web::register(registry);
    media::register(registry);
    utils::register(registry, event_tx);
    youtube::register(registry);
    game_updater::register(registry);
    send_message::register(registry);
    background_monitor::register(registry);
    flight_finder::register(registry);
    video_player::register(registry);
}

fn param(_name: &str, ty: &str, desc: &str) -> Value {
    json!({
        "type": ty,
        "description": desc,
    })
}

fn tool(name: &str, desc: &str, props: Vec<(&str, Value)>, required: Vec<&str>) -> ToolDeclaration {
    let mut properties = serde_json::Map::new();
    for (k, v) in props {
        properties.insert(k.to_string(), v);
    }
    ToolDeclaration {
        name: name.to_string(),
        description: desc.to_string(),
        parameters: json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }),
    }
}
