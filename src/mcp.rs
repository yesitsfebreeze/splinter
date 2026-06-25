use crate::tools;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Req {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
pub struct Resp {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrObj>,
}

#[derive(Serialize)]
struct ErrObj {
    code: i32,
    message: String,
}

fn ok(id: Option<Value>, result: Value) -> Resp {
    Resp {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn err(id: Option<Value>, code: i32, msg: String) -> Resp {
    Resp {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(ErrObj { code, message: msg }),
    }
}

pub async fn handle(input: &str) -> Option<Resp> {
    let req: Req = match serde_json::from_str(input) {
        Ok(r) => r,
        Err(e) => return Some(err(None, -32700, e.to_string())),
    };

    req.id.as_ref()?;

    let id = req.id.clone();
    let params = req.params.unwrap_or(json!({}));

    Some(match req.method.as_str() {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "splinter", "version": "0.1.0" }
            }),
        ),
        "tools/list" => ok(id, json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("").to_string();
            let args = params["arguments"].clone();
            match tools::call(&name, args).await {
                Ok(text) => ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
                Err(e) => ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true }),
                ),
            }
        }
        "ping" => ok(id, json!({})),
        m => err(id, -32601, format!("unknown method: {m}")),
    })
}
