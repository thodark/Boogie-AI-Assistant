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
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
        }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
        }
    }
}

pub type ToolHandler = Box<dyn Fn(Value) -> futures_util::future::BoxFuture<'static, ToolResult> + Send + Sync>;

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
