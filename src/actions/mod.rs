mod screen;
mod browser;
mod computer_control;
mod files;
mod inline;
mod reminder;
mod system;
mod web;
mod media;
mod utils;
mod youtube;
mod game_updater;
mod send_message;
pub mod background_monitor;
mod flight_finder;
mod video_player;

use crate::tools::{ToolDeclaration, ToolRegistry};
use serde_json::{json, Value};

pub fn register_all(registry: &mut ToolRegistry) {
    screen::register(registry);
    browser::register(registry);
    computer_control::register(registry);
    files::register(registry);
    inline::register(registry);
    reminder::register(registry);
    system::register(registry);
    web::register(registry);
    media::register(registry);
    utils::register(registry);
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
