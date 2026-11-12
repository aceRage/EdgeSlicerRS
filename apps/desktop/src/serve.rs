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
const PRINTERS_SCRIPT: &str = include_str!("../ui/printers.js");
const PRINTERS_STYLE: &str = include_str!("../ui/printers.css");
const SETTINGS_SCRIPT: &str = include_str!("../ui/settings.js");
const SETTINGS_STYLE: &str = include_str!("../ui/settings.css");
/// The 3D views' scripts, under `ui/gl`.
const GL: [(&str, &str); 10] = [
    ("math.js", include_str!("../ui/gl/math.js")),
    ("camera.js", include_str!("../ui/gl/camera.js")),
    ("shaders.js", include_str!("../ui/gl/shaders.js")),
    ("renderer.js", include_str!("../ui/gl/renderer.js")),
    ("scene.js", include_str!("../ui/gl/scene.js")),
    ("toolpath.js", include_str!("../ui/gl/toolpath.js")),
    ("controls.js", include_str!("../ui/gl/controls.js")),
    ("gcode.js", include_str!("../ui/gl/gcode.js")),
    ("stage.js", include_str!("../ui/gl/stage.js")),
    ("gl.css", include_str!("../ui/gl/gl.css")),
];
/// The files of the tools that change what stands on the plates (the
/// painting tools among them), and the list of filaments.
const EDIT: [(&str, &str); 7] = [
    ("filaments.js", include_str!("../ui/filaments.js")),
    ("paint.js", include_str!("../ui/paint.js")),
    ("paint.css", include_str!("../ui/paint.css")),
    ("scene-edit.js", include_str!("../ui/scene-edit.js")),
    ("plates.js", include_str!("../ui/plates.js")),
    ("objects.js", include_str!("../ui/objects.js")),
    ("edit.css", include_str!("../ui/edit.css")),
];
/// Saving a project, and what is asked before a project or the window
/// closes.
const SAVE_SCRIPT: &str = include_str!("../ui/save.js");
/// Sending to printers, and the printer's page in Device.
const HOSTS: [(&str, &str); 2] = [
    ("hosts.js", include_str!("../ui/hosts.js")),
    ("hosts.css", include_str!("../ui/hosts.css")),
];
/// The page's words: the table of the language chosen, and then the
/// page's other scripts (`i18n.js` loads them).
const WORDS: [(&str, &str); 1] = [("i18n.js", include_str!("../ui/i18n.js"))];
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
    respond_bytes(request, status, kind, body.into_bytes());
}

fn respond_bytes(request: Request, status: u16, kind: &str, body: Vec<u8>) {
    let header = Header::from_bytes("Content-Type", kind).expect("a plain header");
    let _ = request.respond(
        Response::from_data(body)
            .with_status_code(status)
            .with_header(header),
    );
}

/// The arguments of a call, from the body of its request; else what to
/// answer instead.
fn arguments(request: &mut Request, token: &str) -> Result<Value, (u16, &'static str)> {
    if header(request, "X-Edge-Token") != Some(token) {
        return Err((403, "this call is not from the page"));
    }
    let mut body = String::new();
    let read = request
        .as_reader()
        .take(LONGEST_CALL + 1)
        .read_to_string(&mut body);
    if read.is_err() || body.len() as u64 > LONGEST_CALL {
        return Err((400, "the call could not be read"));
    }
    serde_json::from_str(&body).map_err(|_| (400, "the call is not JSON"))
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
            // The page's own policy: everything from here, nothing from the
            // network, but the printers' own pages in a frame, each named
            // (the connections in force when the page is loaded; the page
            // loads itself again for a new one, hosts.js `framable`).
            let frames = session.frame_origins();
            let page = asset("index.html", PAGE)
                .replace("__EDGE_TOKEN__", token)
                .replace("__EDGE_FRAMES__", &frames.join(" "));
            let frame_src = if frames.is_empty() {
                "'none'".to_owned()
            } else {
                frames.join(" ")
            };
            let policy = format!(
                "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-src {frame_src}; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
            );
            let html = Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                .expect("a plain header");
            let csp = Header::from_bytes("Content-Security-Policy", policy.as_bytes())
                .expect("a plain header");
            let _ = request.respond(
                Response::from_data(page.into_bytes())
                    .with_header(html)
                    .with_header(csp),
            );
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
        (Method::Get, "/printers.js") => {
            respond(
                request,
                200,
                "text/javascript; charset=utf-8",
                asset("printers.js", PRINTERS_SCRIPT),
            );
        }
        (Method::Get, "/settings.js") => {
            respond(
                request,
                200,
                "text/javascript; charset=utf-8",
                asset("settings.js", SETTINGS_SCRIPT),
            );
        }
        (Method::Get, "/settings.css") => {
            respond(
                request,
                200,
                "text/css; charset=utf-8",
                asset("settings.css", SETTINGS_STYLE),
            );
        }
        (Method::Get, "/printers.css") => {
            respond(
                request,
                200,
                "text/css; charset=utf-8",
                asset("printers.css", PRINTERS_STYLE),
            );
        }
        (Method::Get, "/save.js") => {
            respond(
                request,
                200,
                "text/javascript; charset=utf-8",
                asset("save.js", SAVE_SCRIPT),
            );
        }
        (Method::Get, path)
            if EDIT
                .iter()
                .chain(&HOSTS)
                .chain(&WORDS)
                .any(|(known, _)| path[1..] == **known) =>
        {
            let (name, built_in) = EDIT
                .iter()
                .chain(&HOSTS)
                .chain(&WORDS)
                .find(|(known, _)| path[1..] == **known)
                .expect("one of them is it");
            let kind = if name.ends_with(".css") {
                "text/css; charset=utf-8"
            } else {
                "text/javascript; charset=utf-8"
            };
            respond(request, 200, kind, asset(name, built_in));
        }
        (Method::Get, path) if path.starts_with("/gl/") => {
            let name = &path["/gl/".len()..];
            match GL.iter().find(|(known, _)| *known == name) {
                Some((name, built_in)) => {
                    let kind = if name.ends_with(".css") {
                        "text/css; charset=utf-8"
                    } else {
                        "text/javascript; charset=utf-8"
                    };
                    respond(request, 200, kind, asset(&format!("gl/{name}"), built_in));
                }
                None => respond(request, 404, "text/plain", "nothing here".into()),
            }
        }
        // A call answered in bytes: the bytes themselves, or 422 with the
        // sentence that says why not.
        (Method::Post, path) if path.starts_with("/bytes/") => {
            let args = match arguments(&mut request, token) {
                Ok(args) => args,
                Err((status, why)) => return respond(request, status, "text/plain", why.into()),
            };
            match session.call_bytes(&path["/bytes/".len()..], &args) {
                Ok(bytes) => respond_bytes(request, 200, "application/octet-stream", bytes),
                Err(error) => respond(request, 422, "text/plain; charset=utf-8", error),
            }
        }
        (Method::Post, path) if path.starts_with("/api/") => {
            let args = match arguments(&mut request, token) {
                Ok(args) => args,
                Err((status, why)) => return respond(request, status, "text/plain", why.into()),
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
