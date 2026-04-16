use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::bus::{Bus, Event};
use crate::json::Json;
use crate::util::{AppResult, app_error, read_text_if_exists};

/// Run the HTTP + SSE server until `stop` flips to true.
///
/// Routes:
///   GET /healthz   → "ok"
///   GET /inbox     → raw inbox.json (passthrough)
///   GET /activity  → last 200 activity.log lines
///   GET /events    → Server-Sent Events (SSE) stream subscribed to the bus
///
/// Bound to 127.0.0.1 only (localhost dashboard). No auth.
pub fn serve(
    address: SocketAddr,
    inbox_dir: PathBuf,
    bus: Bus,
    stop: Arc<AtomicBool>,
) -> AppResult<()> {
    if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
        return Err(app_error(format!(
            "refusing to bind http server on non-loopback address {address}"
        )));
    }
    let listener = TcpListener::bind(address)
        .map_err(|error| app_error(format!("failed to bind {address}: {error}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| app_error(format!("failed to set non-blocking: {error}")))?;

    eprintln!("breeze: http server listening on http://{address}");

    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                let inbox_dir = inbox_dir.clone();
                let bus = bus.clone();
                let stop = stop.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &inbox_dir, &bus, &stop) {
                        eprintln!("breeze: http connection error: {error}");
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                eprintln!("breeze: http accept failed: {error}");
                thread::sleep(Duration::from_millis(250));
            }
        }
    }
    Ok(())
}

fn handle_connection(
    stream: TcpStream,
    inbox_dir: &PathBuf,
    bus: &Bus,
    stop: &Arc<AtomicBool>,
) -> AppResult<()> {
    stream
        .set_nonblocking(false)
        .map_err(|error| app_error(format!("set_blocking failed: {error}")))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| app_error(format!("set_read_timeout failed: {error}")))?;
    let request = read_request_line(&stream)?;
    let request_line = request.request_line.clone();
    let route = parse_route(&request_line);
    match route {
        Route::Dashboard => write_dashboard(stream),
        Route::Healthz => write_plain(stream, 200, "ok\n"),
        Route::Inbox => write_json_file(stream, &inbox_dir.join("inbox.json")),
        Route::Activity => write_activity_tail(stream, &inbox_dir.join("activity.log"), 200),
        Route::Events => stream_events(stream, bus, stop),
        Route::NotFound => write_plain(stream, 404, "not found\n"),
    }
}

const DASHBOARD_HTML: &str = include_str!("dashboard.html");

fn write_dashboard(mut stream: TcpStream) -> AppResult<()> {
    let body = DASHBOARD_HTML;
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        len = body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| app_error(format!("dashboard header write failed: {error}")))?;
    stream
        .write_all(body.as_bytes())
        .map_err(|error| app_error(format!("dashboard body write failed: {error}")))
}

#[derive(Debug)]
struct RequestHead {
    request_line: String,
}

fn read_request_line(stream: &TcpStream) -> AppResult<RequestHead> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| app_error(format!("read request line failed: {error}")))?;
    let mut header = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            break;
        }
        if line == "\r\n" || line.is_empty() {
            break;
        }
        header.push_str(&line);
    }
    Ok(RequestHead {
        request_line: request_line.trim_end().to_string(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Dashboard,
    Healthz,
    Inbox,
    Activity,
    Events,
    NotFound,
}

fn parse_route(request_line: &str) -> Route {
    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return Route::NotFound;
    };
    if method != "GET" {
        return Route::NotFound;
    }
    let Some(raw_path) = parts.next() else {
        return Route::NotFound;
    };
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    match path {
        "/" | "/dashboard" | "/index.html" => Route::Dashboard,
        "/healthz" => Route::Healthz,
        "/inbox" => Route::Inbox,
        "/activity" => Route::Activity,
        "/events" => Route::Events,
        _ => Route::NotFound,
    }
}

fn write_plain(mut stream: TcpStream, status: u16, body: &str) -> AppResult<()> {
    let reason = reason_phrase(status);
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| app_error(format!("write failed: {error}")))
}

fn write_json_file(mut stream: TcpStream, path: &std::path::Path) -> AppResult<()> {
    let contents = match read_text_if_exists(path)? {
        Some(value) => value,
        None => {
            return write_plain(stream, 404, "inbox.json not found\n");
        }
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n\
         {contents}",
        len = contents.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| app_error(format!("write failed: {error}")))
}

fn write_activity_tail(
    mut stream: TcpStream,
    path: &std::path::Path,
    max_lines: usize,
) -> AppResult<()> {
    let body = tail_as_json_array(path, max_lines);
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| app_error(format!("write failed: {error}")))
}

fn tail_as_json_array(path: &std::path::Path, max_lines: usize) -> String {
    let Ok(Some(text)) = read_text_if_exists(path) else {
        return "[]".to_string();
    };
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    let mut body = String::from("[");
    let mut first = true;
    for line in &lines[start..] {
        if !first {
            body.push(',');
        }
        first = false;
        body.push_str(line);
    }
    body.push(']');
    body
}

fn stream_events(mut stream: TcpStream, bus: &Bus, stop: &Arc<AtomicBool>) -> AppResult<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: text/event-stream\r\n\
              Cache-Control: no-store\r\n\
              Connection: keep-alive\r\n\
              X-Accel-Buffering: no\r\n\
              \r\n",
        )
        .map_err(|error| app_error(format!("sse headers failed: {error}")))?;

    // Send a hello so the client knows the stream is live.
    send_sse(&mut stream, "ready", &Json::str("subscribed").encode())?;

    let receiver = bus.subscribe();
    while !stop.load(Ordering::Relaxed) {
        match receiver.recv_timeout(Duration::from_secs(15)) {
            Some(event) => {
                if let Err(error) = emit_event(&mut stream, &event) {
                    // Client disconnected or broken pipe — exit cleanly.
                    let text = format!("{error}");
                    if text.contains("Broken pipe") || text.contains("closed") {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
            None => {
                // Keep-alive ping so intermediate proxies don't close the stream.
                if stream.write_all(b": ping\n\n").is_err() {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn emit_event(stream: &mut TcpStream, event: &Event) -> AppResult<()> {
    match event {
        Event::InboxUpdated {
            last_poll,
            total,
            new_count,
        } => {
            let payload = Json::Object(vec![
                ("last_poll".to_string(), Json::str(last_poll.clone())),
                ("total".to_string(), Json::Number(*total as i64)),
                ("new_count".to_string(), Json::Number(*new_count as i64)),
            ])
            .encode();
            send_sse(stream, "inbox", &payload)
        }
        Event::Activity(line) => send_sse(stream, "activity", line),
    }
}

fn send_sse(stream: &mut TcpStream, event: &str, data: &str) -> AppResult<()> {
    // Each SSE frame is `event: <name>\ndata: <line>\n` possibly repeated,
    // terminated by a blank line.
    let mut frame = String::new();
    frame.push_str("event: ");
    frame.push_str(event);
    frame.push('\n');
    for line in data.lines() {
        frame.push_str("data: ");
        frame.push_str(line);
        frame.push('\n');
    }
    if data.ends_with('\n') {
        frame.push_str("data: \n");
    }
    frame.push('\n');
    stream
        .write_all(frame.as_bytes())
        .map_err(|error| app_error(format!("sse write failed: {error}")))?;
    stream
        .flush()
        .map_err(|error| app_error(format!("sse flush failed: {error}")))
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{Route, parse_route, tail_as_json_array};
    use std::io::Write;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn parses_known_routes() {
        assert_eq!(parse_route("GET /healthz HTTP/1.1"), Route::Healthz);
        assert_eq!(parse_route("GET /inbox HTTP/1.1"), Route::Inbox);
        assert_eq!(parse_route("GET /activity HTTP/1.1"), Route::Activity);
        assert_eq!(parse_route("GET /events HTTP/1.1"), Route::Events);
        assert_eq!(parse_route("GET / HTTP/1.1"), Route::Dashboard);
        assert_eq!(parse_route("GET /dashboard HTTP/1.1"), Route::Dashboard);
        assert_eq!(parse_route("GET /index.html HTTP/1.1"), Route::Dashboard);
        assert_eq!(parse_route("GET /inbox?all=1 HTTP/1.1"), Route::Inbox);
        assert_eq!(parse_route("POST /inbox HTTP/1.1"), Route::NotFound);
        assert_eq!(parse_route("GET /nope HTTP/1.1"), Route::NotFound);
        assert_eq!(parse_route(""), Route::NotFound);
    }

    #[test]
    fn tail_returns_last_n_lines_as_json_array() {
        let dir = temp_dir();
        let path = dir.join("activity-test.log");
        let _ = std::fs::remove_file(&path);
        {
            let mut file = std::fs::File::create(&path).expect("create");
            writeln!(file, "{{\"n\":1}}").unwrap();
            writeln!(file, "{{\"n\":2}}").unwrap();
            writeln!(file, "{{\"n\":3}}").unwrap();
        }
        let body = tail_as_json_array(&path, 2);
        assert_eq!(body, "[{\"n\":2},{\"n\":3}]");
        let body_all = tail_as_json_array(&path, 10);
        assert_eq!(body_all, "[{\"n\":1},{\"n\":2},{\"n\":3}]");
        let _ = std::fs::remove_file(&path);
    }
}

