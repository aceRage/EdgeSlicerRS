//! Printers that are not there: an HTTP server and a websocket on
//! 127.0.0.1, each on a port the system picks, answering as a script
//! says and keeping what they were sent. For the tests only; nothing here
//! reaches the network.

use std::{
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use tungstenite::Message;

/// One request as the mock received it.
#[derive(Clone, Debug)]
pub struct Got {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Got {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The parts of a multipart form: name, file name, bytes.
    pub fn form(&self) -> Vec<(String, Option<String>, Vec<u8>)> {
        let kind = self.header("Content-Type").unwrap_or_default();
        let boundary = kind.split("boundary=").nth(1).expect("a multipart form");
        let marker = format!("--{boundary}");
        let mut parts = Vec::new();
        let body = &self.body;
        let mut at = find(body, marker.as_bytes(), 0).expect("a first boundary");
        loop {
            at += marker.len();
            if body[at..].starts_with(b"--") {
                break;
            }
            at += 2; // \r\n
            let head_end = find(body, b"\r\n\r\n", at).expect("a part's head");
            let head = String::from_utf8_lossy(&body[at..head_end]).into_owned();
            let start = head_end + 4;
            let next = find(body, format!("\r\n{marker}").as_bytes(), start).expect("a part's end");
            let quoted = |key: &str| {
                head.split(&format!("{key}=\""))
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .map(str::to_owned)
            };
            parts.push((
                quoted(" name")
                    .or_else(|| quoted("name"))
                    .unwrap_or_default(),
                quoted("filename"),
                body[start..next].to_vec(),
            ));
            at = next + 2;
        }
        parts
    }

    /// A form's field as text.
    pub fn field(&self, name: &str) -> Option<String> {
        self.form()
            .into_iter()
            .find(|(key, _, _)| key == name)
            .map(|(_, _, bytes)| String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// What the mock answers: status and body.
pub type Script = dyn Fn(&Got, &[Got]) -> (u16, String) + Send + Sync;

/// An HTTP printer on 127.0.0.1.
pub struct Printer {
    pub port: u16,
    pub got: Arc<Mutex<Vec<Got>>>,
    stop: Arc<AtomicBool>,
}

impl Printer {
    /// `script` is given each request and those before it. `slow` waits
    /// that long before reading a request's body (to see a cancel).
    pub fn start(script: Box<Script>, slow: Option<Duration>) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = server.server_addr().to_ip().expect("an IP address").port();
        let got = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (kept, stopped) = (got.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                if let Some(wait) = slow {
                    std::thread::sleep(wait);
                }
                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);
                let one = Got {
                    method: request.method().to_string(),
                    url: request.url().to_owned(),
                    headers: request
                        .headers()
                        .iter()
                        .map(|header| (header.field.to_string(), header.value.to_string()))
                        .collect(),
                    body,
                };
                let before = kept.lock().unwrap().clone();
                let (status, text) = script(&one, &before);
                kept.lock().unwrap().push(one);
                let _ = request
                    .respond(tiny_http::Response::from_string(text).with_status_code(status));
            }
        });
        Self { port, got, stop }
    }

    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }

    /// The paths asked for, in order, with their methods.
    pub fn asked(&self) -> Vec<String> {
        self.got()
            .iter()
            .map(|got| format!("{} {}", got.method, got.url))
            .collect()
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A port that takes a connection and never answers: a printer that
/// hangs.
pub struct Silent {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Silent {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                if let Ok((stream, _)) = listener.accept() {
                    held.push(stream);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Self { port, stop }
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What the websocket answers each message with: none for silence.
pub type Says = dyn Fn(&Value, usize) -> Option<Value> + Send + Sync;

/// The Centauri Carbon's websocket on 127.0.0.1: one connection at a
/// time, each message read as JSON and answered as `says` says (given the
/// message and how many came before it, on every connection).
pub struct Socket {
    pub port: u16,
    pub got: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
}

impl Socket {
    pub fn start(says: Box<Says>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (kept, stopped) = (got.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let Ok(mut socket) = tungstenite::accept(stream) else {
                    continue;
                };
                socket
                    .get_ref()
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                while !stopped.load(Ordering::Relaxed) {
                    match socket.read() {
                        Ok(Message::Text(text)) => {
                            let message: Value =
                                serde_json::from_str(text.as_str()).unwrap_or_default();
                            let count = kept.lock().unwrap().len();
                            kept.lock().unwrap().push(message.clone());
                            if let Some(answer) = says(&message, count) {
                                let _ = socket.send(Message::text(answer.to_string()));
                            }
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(io))
                            if matches!(
                                io.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                }
            }
        });
        Self { port, got, stop }
    }

    pub fn got(&self) -> Vec<Value> {
        self.got.lock().unwrap().clone()
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A G-code file of `size` bytes, every byte telling where it is.
pub fn gcode(size: usize) -> Arc<Vec<u8>> {
    let mut text = Vec::with_capacity(size);
    let mut line = 0u64;
    while text.len() < size {
        text.extend_from_slice(format!("G1 X{} Y{} ; {line}\n", line % 200, line % 190).as_bytes());
        line += 1;
    }
    text.truncate(size);
    Arc::new(text)
}
