use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use waservix::{json, verify_webhook, Client, Error, Value};

#[derive(Clone, Debug, Default)]
struct Seen {
    method: String,
    path: String,
    auth: String,
    body: Value,
}

/// Tiny HTTP server so the tests need no mock crates.
fn server() -> (String, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let shared = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let (mut len, mut auth) = (0usize, String::new());
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" || h.is_empty() {
                    break;
                }
                let (name, value) = h.split_once(':').unwrap();
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => len = value.trim().parse().unwrap(),
                    "authorization" => auth = value.trim().to_string(),
                    _ => {}
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let body: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap() };
            *shared.lock().unwrap() = Seen { method, path: path.clone(), auth, body };

            let (status, reply) = if path.starts_with("/down") {
                ("503 Service Unavailable", r#"{"success":false,"message":"Device reconnecting"}"#)
            } else if path.starts_with("/soft") {
                ("200 OK", r#"{"success":false,"message":"Invalid number"}"#)
            } else {
                ("200 OK", r#"{"success":true,"data":{"msgId":58213,"status":"in_progress"}}"#)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (base, seen)
}

#[tokio::test]
async fn send_and_paths() {
    let (base, seen) = server();
    let wa = Client::builder("wpk_test").base_url(format!("{base}/api")).build();

    let res = wa.send_text("+1555", "hi").await.unwrap();
    assert_eq!(res["msgId"], 58213);
    {
        let s = seen.lock().unwrap();
        assert_eq!((s.method.as_str(), s.path.as_str(), s.auth.as_str()), ("POST", "/api/send-message", "Bearer wpk_test"));
        assert_eq!(s.body, json!({"to": "+1555", "text": "hi"}));
    }

    wa.send_image("+1", "https://x/y.jpg", None).await.unwrap();
    assert_eq!(seen.lock().unwrap().body, json!({"to": "+1", "imageUrl": "https://x/y.jpg"}));
    wa.send_document("+1", "https://x/a.pdf", Some("a.pdf"), Some("c")).await.unwrap();
    assert_eq!(seen.lock().unwrap().body["fileName"], "a.pdf");
    wa.send_location("+1", -1.9441, 30.0619, Some("Shop")).await.unwrap();
    assert_eq!(seen.lock().unwrap().body["location"]["name"], "Shop");
    wa.send_message("1203@g.us", json!({"poll": {"name": "Lunch?", "options": ["Yes", "No"]}})).await.unwrap();
    assert_eq!(seen.lock().unwrap().body["to"], "1203@g.us");

    wa.get_message_info(5).await.unwrap();
    assert_eq!(seen.lock().unwrap().path, "/api/messages/5/info");
    wa.edit_message(5, "t").await.unwrap();
    assert_eq!(seen.lock().unwrap().method, "PUT");
    wa.on_whatsapp("+1555").await.unwrap();
    assert_eq!(seen.lock().unwrap().path, "/api/on-whatsapp/%2B1555");
    wa.get_group_metadata("1203@g.us").await.unwrap();
    assert_eq!(seen.lock().unwrap().path, "/api/groups/1203%40g.us/metadata");
    wa.connect_session(3, None).await.unwrap();
    assert_eq!(seen.lock().unwrap().body, json!({"method": "qr"}));
    wa.connect_session(3, Some("+1555")).await.unwrap();
    assert_eq!(seen.lock().unwrap().body, json!({"method": "code", "phone": "+1555"}));

    assert!(matches!(wa.send_text("", "x").await, Err(Error::Invalid(_))));
    assert!(matches!(wa.send_message("+1", json!("text")).await, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn errors() {
    let (base, _) = server();
    let err = Client::builder("k").base_url(format!("{base}/down")).build().get_status().await.unwrap_err();
    assert_eq!(err.status(), Some(503));
    assert_eq!(err.to_string(), "WAservix: Device reconnecting (HTTP 503)");

    let err = Client::builder("k").base_url(format!("{base}/soft")).build().send_text("+1", "x").await.unwrap_err();
    match err {
        Error::Api { status, message, .. } => assert_eq!((status, message.as_str()), (200, "Invalid number")),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn webhook_secret() {
    assert!(verify_webhook("abc", "abc"));
    assert!(!verify_webhook("abd", "abc"));
    assert!(!verify_webhook("abcd", "abc"));
    assert!(!verify_webhook("", "abc"));
}
