use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    /// Optional JPEG image bytes to push to Gemini Live `realtimeInput` vision stream.
    pub image_jpeg: Option<Vec<u8>>,
    /// If set, wraps `output` in `<<<UNTRUSTED source=...>>>` markers before returning to the model.
    pub untrusted_source: Option<&'static str>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
            image_jpeg: None,
            untrusted_source: None,
        }
    }

    pub fn ok_untrusted(source: &'static str, output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
            image_jpeg: None,
            untrusted_source: Some(source),
        }
    }

    pub fn ok_with_image(output: impl Into<String>, jpeg: Vec<u8>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
            image_jpeg: Some(jpeg),
            untrusted_source: None,
        }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
            image_jpeg: None,
            untrusted_source: None,
        }
    }

    pub fn is_untrusted(&self) -> bool {
        self.untrusted_source.is_some()
    }

    pub fn formatted_for_model(&self) -> String {
        if let Some(src) = self.untrusted_source {
            crate::util::wrap_untrusted(src, &self.output)
        } else {
            self.output.clone()
        }
    }
}

pub type ToolHandler =
    Box<dyn Fn(Value) -> futures_util::future::BoxFuture<'static, ToolResult> + Send + Sync>;

pub struct ToolRegistry {
    declarations: Vec<ToolDeclaration>,
    handlers: HashMap<String, ToolHandler>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            declarations: Vec::new(),
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, decl: ToolDeclaration, handler: ToolHandler) {
        self.handlers.insert(decl.name.clone(), handler);
        self.declarations.push(decl);
    }

    pub fn declarations(&self) -> &[ToolDeclaration] {
        &self.declarations
    }

    pub fn gemini_tool_json(&self) -> Value {
        let fns: Vec<Value> = self
            .declarations
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "description": d.description,
                    "parameters": d.parameters,
                })
            })
            .collect();
        serde_json::json!([{"functionDeclarations": fns}])
    }

    pub async fn execute(&self, name: &str, args: Value) -> ToolResult {
        match self.handlers.get(name) {
            Some(handler) => handler(args).await,
            None => ToolResult::err(format!("unknown tool: {name}")),
        }
    }
}
