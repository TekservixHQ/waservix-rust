//! Rust client for the [WAservix WhatsApp API Gateway](https://tekservix.com/waservix/).
//!
//! WAservix links your own WhatsApp number by QR code (the same way WhatsApp Web does) and
//! gives you a REST API to send text, images, documents, locations, polls and more.
//!
//! ```no_run
//! # async fn run() -> Result<(), waservix::Error> {
//! let wa = waservix::Client::new(std::env::var("WASERVIX_KEY").unwrap());
//! let res = wa.send_text("+15551234567", "Hello Aline, your order is ready").await?;
//! println!("{} {}", res["msgId"], res["status"]); // 58213 "in_progress"
//! # Ok(()) }
//! ```
//!
//! Keep the API key on your server. API reference: <https://tekservix.com/waservix/docs/>

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::time::Duration;

pub use reqwest::Method;
pub use serde_json::{json, Value};

/// Crate version, sent in the User-Agent header.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Production API base URL.
pub const DEFAULT_BASE_URL: &str = "https://tekservix.com/waservix/api";

/// Errors returned by the client. A 503 means the device is reconnecting: retry shortly.
#[derive(Debug)]
pub enum Error {
    /// The API answered with an error.
    Api {
        /// HTTP status.
        status: u16,
        /// Message from the API, or `HTTP <status>`.
        message: String,
        /// Field errors, when the API sends them.
        errors: Option<Value>,
        /// Parsed JSON body, or the raw text as a JSON string.
        body: Value,
        /// Seconds to wait before retrying (sent with 429).
        retry_after: Option<f64>,
    },
    /// Network or HTTP failure before an API answer arrived.
    Http(reqwest::Error),
    /// Bad argument.
    Invalid(String),
}

impl Error {
    /// HTTP status for API errors.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            Error::Http(e) => e.status().map(|s| s.as_u16()),
            Error::Invalid(_) => None,
        }
    }

    /// Seconds to wait before retrying, when the API sent `Retry-After`.
    pub fn retry_after(&self) -> Option<f64> {
        match self {
            Error::Api { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api { status, message, .. } => write!(f, "WAservix: {message} (HTTP {status})"),
            Error::Http(e) => write!(f, "WAservix: {e}"),
            Error::Invalid(m) => write!(f, "WAservix: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

/// Builder for [`Client`] with a custom base URL, timeout or reqwest client.
#[derive(Debug)]
pub struct ClientBuilder {
    api_key: String,
    base_url: String,
    timeout: Duration,
    http: Option<reqwest::Client>,
}

impl ClientBuilder {
    /// Change only for testing.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Timeout per request (default 30 seconds). Ignored when you pass your own client.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Use your own reqwest client (proxy, connection pool, middleware).
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Build the client.
    pub fn build(self) -> Client {
        let http = self.http.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(self.timeout)
                .user_agent(format!("waservix-rust/{VERSION}"))
                .build()
                .expect("WAservix: failed to build HTTP client")
        });
        Client { http, api_key: self.api_key, base_url: self.base_url.trim_end_matches('/').to_string() }
    }
}

/// WAservix API client. Cheap to clone; reuse one per app.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl Client {
    /// Client with a device API key (Devices -> API key), or a personal access token for sessions.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::builder(api_key).build()
    }

    /// Builder for a custom base URL, timeout or reqwest client.
    pub fn builder(api_key: impl Into<String>) -> ClientBuilder {
        let api_key = api_key.into();
        assert!(!api_key.is_empty(), "WAservix: api_key is required (Devices -> API key in the WAservix dashboard)");
        ClientBuilder { api_key, base_url: DEFAULT_BASE_URL.to_string(), timeout: Duration::from_secs(30), http: None }
    }

    /// Call any documented endpoint, e.g. `request(Method::GET, "/groups", None)`.
    /// Returns the `data` field of the answer.
    pub async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json");
        if let Some(body) = body {
            req = req.header("Content-Type", "application/json").body(body.to_string());
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        let retry_after = res
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok());
        let text = res.text().await?;
        let json: Option<Value> = if text.is_empty() { None } else { serde_json::from_str(&text).ok() };

        let failed = json.as_ref().and_then(|j| j.get("success")).and_then(Value::as_bool) == Some(false);
        if status >= 400 || failed {
            let message = json
                .as_ref()
                .and_then(|j| j.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {status}"));
            let errors = json.as_ref().and_then(|j| j.get("errors")).cloned();
            return Err(Error::Api { status, message, errors, body: json.unwrap_or(Value::String(text)), retry_after });
        }
        Ok(match json {
            Some(Value::Object(mut o)) if o.contains_key("data") => o.remove("data").unwrap_or(Value::Null),
            Some(v) => v,
            None => Value::Null,
        })
    }

    /// Send any message type to a number or group JID (`...@g.us`). `fields` uses the API names:
    /// text, imageUrl, videoUrl, audioUrl, documentUrl, fileName, stickerUrl, contact, location,
    /// poll, replyTo, mentions, viewOnce. Returns `{"msgId": ..., "jid": ..., "status": "in_progress"}`.
    ///
    /// ```no_run
    /// # async fn run(wa: waservix::Client) -> Result<(), waservix::Error> {
    /// wa.send_message("120363040000000000@g.us", waservix::json!({ "text": "Team meeting at 3pm" })).await?;
    /// # Ok(()) }
    /// ```
    pub async fn send_message(&self, to: &str, fields: Value) -> Result<Value, Error> {
        if to.is_empty() {
            return Err(Error::Invalid("send_message: \"to\" is required (international number or group JID)".into()));
        }
        let mut body = match fields {
            Value::Object(o) => Value::Object(o),
            Value::Null => json!({}),
            _ => return Err(Error::Invalid("send_message: fields must be a JSON object".into())),
        };
        body["to"] = json!(to);
        self.request(Method::POST, "/send-message", Some(body)).await
    }

    /// Send a text message.
    pub async fn send_text(&self, to: &str, text: &str) -> Result<Value, Error> {
        self.send_message(to, json!({ "text": text })).await
    }

    /// Send an image by URL, with an optional caption.
    pub async fn send_image(&self, to: &str, image_url: &str, caption: Option<&str>) -> Result<Value, Error> {
        let mut fields = json!({ "imageUrl": image_url });
        if let Some(c) = caption {
            fields["text"] = json!(c);
        }
        self.send_message(to, fields).await
    }

    /// Send a document by URL.
    pub async fn send_document(&self, to: &str, document_url: &str, file_name: Option<&str>, caption: Option<&str>) -> Result<Value, Error> {
        let mut fields = json!({ "documentUrl": document_url });
        if let Some(n) = file_name {
            fields["fileName"] = json!(n);
        }
        if let Some(c) = caption {
            fields["text"] = json!(c);
        }
        self.send_message(to, fields).await
    }

    /// Send a location pin.
    pub async fn send_location(&self, to: &str, latitude: f64, longitude: f64, name: Option<&str>) -> Result<Value, Error> {
        let mut location = json!({ "latitude": latitude, "longitude": longitude });
        if let Some(n) = name {
            location["name"] = json!(n);
        }
        self.send_message(to, json!({ "location": location })).await
    }

    /// Status: queued, sent, delivered, read, played or failed.
    pub async fn get_message_info(&self, msg_id: impl fmt::Display) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/messages/{}/info", encode(&msg_id.to_string())), None).await
    }

    /// Edit one of your own text messages (about 15 minutes after sending).
    pub async fn edit_message(&self, msg_id: impl fmt::Display, text: &str) -> Result<Value, Error> {
        self.request(Method::PUT, &format!("/messages/{}", encode(&msg_id.to_string())), Some(json!({ "text": text })))
            .await
    }

    /// Delete a message.
    pub async fn delete_message(&self, msg_id: impl fmt::Display) -> Result<Value, Error> {
        self.request(Method::DELETE, &format!("/messages/{}", encode(&msg_id.to_string())), None).await
    }

    /// Resend a failed message.
    pub async fn resend_message(&self, msg_id: impl fmt::Display) -> Result<Value, Error> {
        self.request(Method::POST, &format!("/messages/{}/resend", encode(&msg_id.to_string())), None).await
    }

    /// Is this number on WhatsApp? Returns `{"exists": bool, "jid": ...}`.
    pub async fn on_whatsapp(&self, phone: &str) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/on-whatsapp/{}", encode(phone)), None).await
    }

    /// connected, connecting, need_scan, disconnected, logged_out or expired.
    pub async fn get_status(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/status", None).await
    }

    /// The linked WhatsApp account.
    pub async fn get_user(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/user", None).await
    }

    /// Your contacts.
    pub async fn list_contacts(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/contacts", None).await
    }

    /// One contact.
    pub async fn get_contact(&self, phone: &str) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/contacts/{}", encode(phone)), None).await
    }

    /// Save or rename a contact.
    pub async fn save_contact(&self, phone: &str, name: &str) -> Result<Value, Error> {
        self.request(Method::PUT, "/contacts", Some(json!({ "phone": phone, "name": name }))).await
    }

    /// Groups the number belongs to.
    pub async fn list_groups(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/groups", None).await
    }

    /// Group name, participants and settings.
    pub async fn get_group_metadata(&self, group_jid: &str) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/groups/{}/metadata", encode(group_jid)), None).await
    }

    /// Sessions (needs a personal access token, not a device key).
    pub async fn list_sessions(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/whatsapp-sessions", None).await
    }

    /// One session.
    pub async fn get_session(&self, id: impl fmt::Display) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/whatsapp-sessions/{}", encode(&id.to_string())), None).await
    }

    /// Create a session.
    pub async fn create_session(&self, params: Value) -> Result<Value, Error> {
        self.request(Method::POST, "/whatsapp-sessions", Some(params)).await
    }

    /// Connect a session by QR code (`phone: None`) or pairing code (`phone: Some(number)`).
    pub async fn connect_session(&self, id: impl fmt::Display, phone: Option<&str>) -> Result<Value, Error> {
        let body = match phone {
            Some(p) => json!({ "method": "code", "phone": p }),
            None => json!({ "method": "qr" }),
        };
        self.request(Method::POST, &format!("/whatsapp-sessions/{}/connect", encode(&id.to_string())), Some(body)).await
    }

    /// QR code for linking the number.
    pub async fn get_session_qr_code(&self, id: impl fmt::Display) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/whatsapp-sessions/{}/qrcode", encode(&id.to_string())), None).await
    }
}

/// Constant-time check of the `X-Webhook-Signature` header against your device's webhook secret.
pub fn verify_webhook(signature: &str, secret: &str) -> bool {
    if signature.is_empty() || secret.is_empty() || signature.len() != secret.len() {
        return false;
    }
    signature.bytes().zip(secret.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
