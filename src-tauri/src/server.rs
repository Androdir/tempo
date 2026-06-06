//! Loopback-only HTTP ingest endpoint for the Chrome extension.
//!
//! Bound to 127.0.0.1 only. All data routes require the `X-Tempo-Token` header
//! (also forces a CORS preflight for any cross-origin web page, which we refuse).
//! Nothing here ever makes an outbound request.

use tiny_http::{Header, Method, Request, Response, Server};

use crate::db::Db;
use crate::ingest;
use crate::models::IngestPayload;
use crate::settings;

pub fn start(db: Db, token: String, port: u16) {
    std::thread::spawn(move || {
        let addr = format!("127.0.0.1:{port}");
        let server = match Server::http(&addr) {
            Ok(s) => s,
            Err(e) => {
                // Don't crash the app if the port is busy — just log and stop.
                eprintln!("[tempo] ingest server could not bind {addr}: {e}");
                return;
            }
        };
        eprintln!("[tempo] ingest endpoint listening on http://{addr}");

        for mut request in server.incoming_requests() {
            let (status, body) = match handle(&db, &token, &mut request) {
                Ok(b) => (200u16, b),
                Err((code, msg)) => (code, json_err(&msg)),
            };
            let response = Response::from_string(body)
                .with_status_code(status)
                .with_header(json_header());
            if let Err(e) = request.respond(response) {
                eprintln!("[tempo] failed to send ingest response: {e}");
            }
        }
    });
}

fn handle(db: &Db, token: &str, request: &mut Request) -> Result<String, (u16, String)> {
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("").to_string();

    // Unauthenticated liveness check (exposes no data).
    if method == Method::Get && path == "/health" {
        return Ok(r#"{"ok":true,"app":"tempo"}"#.to_string());
    }

    if !auth_ok(request, token) {
        return Err((401, "unauthorized".to_string()));
    }

    match (method, path.as_str()) {
        (Method::Get, "/config") => {
            let conn = db.lock().map_err(|e| (500, e.to_string()))?;
            let cfg = settings::build_config(&conn).map_err(|e| (500, e.to_string()))?;
            serde_json::to_string(&cfg).map_err(|e| (500, e.to_string()))
        }
        (Method::Post, "/ingest") => {
            let mut body = String::new();
            request
                .as_reader()
                .read_to_string(&mut body)
                .map_err(|e| (400, e.to_string()))?;
            let payload: IngestPayload =
                serde_json::from_str(&body).map_err(|e| (400, format!("bad json: {e}")))?;
            let id = ingest::handle_ingest(db, payload).map_err(|e| (500, e))?;
            Ok(format!(r#"{{"ok":true,"id":{id}}}"#))
        }
        _ => Err((404, "not found".to_string())),
    }
}

fn auth_ok(request: &Request, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    request.headers().iter().any(|h| {
        h.field.as_str().as_str().eq_ignore_ascii_case("x-tempo-token")
            && h.value.as_str() == token
    })
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("valid header")
}

fn json_err(msg: &str) -> String {
    let escaped = serde_json::to_string(msg).unwrap_or_else(|_| "\"error\"".to_string());
    format!(r#"{{"ok":false,"error":{escaped}}}"#)
}
