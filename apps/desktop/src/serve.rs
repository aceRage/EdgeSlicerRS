//! The page over HTTP, for a browser on this machine: for working on the
//! page, and for checking it where no window can be opened.
//!
//! The calls read and write files, so they are kept to this machine and
//! to the page served here: the server listens on the loopback address
//! only, answers only when addressed by it, and every call must carry a
//! token that only the served page is given. Another site open in the
//! same browser can do none of that.

use std::{
    hash::{BuildHasher, Hasher},
    io::Read,
    sync::Arc,
};

use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::api::Session;

const PAGE: &str = include_str!("../ui/index.html");
const SCRIPT: &str = include_str!("../ui/app.js");
const STYLE: &str = include_str!("../ui/style.css");
/// A file of the page. A debug build reads it from the source folder each
/// time, so a change to the page shows on reload; any other build serves
/// what it was built with.
fn asset(name: &str, built_in: &'static str) -> String {
    if cfg!(debug_assertions) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui")
            .join(name);
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
    }
    built_in.to_owned()
}

/// More than any call sends; a longer body is refused unread.
const LONGEST_CALL: u64 = 64 * 1024;

/// Unguessable from outside: the standard library seeds each hasher from
/// the operating system.
fn token() -> String {
    let word = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("{:016x}{:016x}", word(), word())
}

fn respond(request: Request, status: u16, kind: &str, body: String) {
    let header = Header::from_bytes("Content-Type", kind).expect("a plain header");
    let _ = request.respond(
        Response::from_string(body)
            .with_status_code(status)
            .with_header(header),
    );
}

fn header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn handle(mut request: Request, session: &Arc<Session>, token: &str, port: u16) {
    let host = header(&request, "Host").unwrap_or_default();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return respond(
            request,
            403,
            "text/plain",
            "not served under that name".into(),
        );
    }
    let url = request.url().to_owned();
    match (request.method(), url.as_str()) {
        (Method::Get, "/") => {
            let page = asset("index.html", PAGE).replace("__EDGE_TOKEN__", token);
            respond(request, 200, "text/html; charset=utf-8", page);
        }
        (Method::Get, "/app.js") => {
            respond(
                request,
                200,
                "text/javascript; charset=utf-8",
                asset("app.js", SCRIPT),
            );
        }
        (Method::Get, "/style.css") => {
            respond(
                request,
                200,
                "text/css; charset=utf-8",
                asset("style.css", STYLE),
            );
        }
        (Method::Post, path) if path.starts_with("/api/") => {
            if header(&request, "X-Edge-Token") != Some(token) {
                return respond(
                    request,
                    403,
                    "text/plain",
                    "this call is not from the page".into(),
                );
            }
            let mut body = String::new();
            let read = request
                .as_reader()
                .take(LONGEST_CALL + 1)
                .read_to_string(&mut body);
            if read.is_err() || body.len() as u64 > LONGEST_CALL {
                return respond(
                    request,
                    400,
                    "text/plain",
                    "the call could not be read".into(),
                );
            }
            let args: Value = match serde_json::from_str(&body) {
                Ok(args) => args,
                Err(_) => {
                    return respond(request, 400, "text/plain", "the call is not JSON".into());
                }
            };
            let answer = match session.call(&path["/api/".len()..], &args) {
                Ok(value) => json!({ "ok": value }),
                Err(error) => json!({ "error": error }),
            };
            respond(request, 200, "application/json", answer.to_string());
        }
        _ => respond(request, 404, "text/plain", "nothing here".into()),
    }
}

/// Serves until the process is stopped.
pub fn run(session: Arc<Session>, port: u16) -> Result<(), String> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| format!("port {port} could not be opened: {e}"))?;
    let token = Arc::new(token());
    println!("EdgeSlicerRS is at http://127.0.0.1:{port}/ (this machine only)");
    for request in server.incoming_requests() {
        let (session, token) = (session.clone(), token.clone());
        std::thread::spawn(move || handle(request, &session, &token, port));
    }
    Ok(())
}
