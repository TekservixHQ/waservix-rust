# waservix

[![crates.io](https://img.shields.io/crates/v/waservix.svg)](https://crates.io/crates/waservix)
[![docs.rs](https://docs.rs/waservix/badge.svg)](https://docs.rs/waservix)
[![tests](https://github.com/TekservixHQ/waservix-rust/actions/workflows/tests.yml/badge.svg)](https://github.com/TekservixHQ/waservix-rust/actions/workflows/tests.yml)

**WAservix WhatsApp API Gateway** - official Rust client: send WhatsApp messages from your own number. By Tekservix.

Rust client for the [WAservix WhatsApp API Gateway](https://tekservix.com/waservix/).

WAservix links your own WhatsApp number by QR code (the same way WhatsApp Web does) and gives you a REST API to send text, images, video, audio, documents, locations, polls and more, with webhooks for incoming messages and status updates.

- async (reqwest + any Tokio app: Axum, Actix Web, Rocket, Loco)
- Results are `serde_json::Value`, so nothing breaks when the API adds fields
- Webhook check built in, `#![forbid(unsafe_code)]`

## Install

```bash
cargo add waservix
```

Rustls is used by default. For the system's TLS instead: `cargo add waservix --no-default-features --features native-tls`.

## Quick start

Copy your device's API key from the WAservix dashboard (Devices -> API key) and keep it on your server.

```rust
#[tokio::main]
async fn main() -> Result<(), waservix::Error> {
    let wa = waservix::Client::new(std::env::var("WASERVIX_KEY").expect("WASERVIX_KEY"));

    let res = wa.send_text("+15551234567", "Hello Aline, your order is ready").await?;
    println!("{} {}", res["msgId"], res["status"]); // 58213 "in_progress"
    Ok(())
}
```

## Sending

Messages are queued and sent one at a time per number, so each call returns right away with `status: "in_progress"` and a `msgId`.

```rust
use waservix::json;

// Image with caption
wa.send_image("+15551234567", "https://example.com/product.jpg", Some("Back in stock")).await?;

// Document
wa.send_document("+15551234567", "https://example.com/inv-1042.pdf", Some("Invoice-1042.pdf"), Some("Your invoice")).await?;

// Location
wa.send_location("+15551234567", -1.9441, 30.0619, Some("Our shop")).await?;

// Anything the API supports, including groups (JID ending in @g.us)
wa.send_message("120363040000000000@g.us", json!({ "text": "Team meeting at 3pm" })).await?;
wa.send_message("+15551234567", json!({ "poll": { "name": "Lunch?", "options": ["Pizza", "Sushi"] } })).await?;
```

## Status, contacts and groups

```rust
let info  = wa.get_message_info(58213).await?;      // queued, sent, delivered, read, played or failed
let check = wa.on_whatsapp("+15551234567").await?;  // {"exists": true, "jid": "..."}
let conn  = wa.get_status().await?;                 // {"status": "connected"}

wa.edit_message(58213, "Corrected text").await?;    // own text messages, about 15 minutes
let groups = wa.list_groups().await?;
```

Want typed results? `serde_json::from_value::<MyStruct>(value)`.

Anything else in the [API reference](https://tekservix.com/waservix/docs/) is one call away:

```rust
wa.request(waservix::Method::POST, "/messages/read", Some(json!({ "msgId": "3EB0X123456789" }))).await?;
```

## Webhooks

Set a webhook URL and events for the number in the dashboard. Each request carries `X-Webhook-Signature`, which equals the device's webhook secret. Axum example:

```rust
use axum::{body::Bytes, http::{HeaderMap, StatusCode}};

async fn wa_webhook(headers: HeaderMap, body: Bytes) -> StatusCode {
    let sig = headers.get("x-webhook-signature").and_then(|v| v.to_str().ok()).unwrap_or("");
    let secret = std::env::var("WASERVIX_WEBHOOK_SECRET").unwrap_or_default();
    if !waservix::verify_webhook(sig, &secret) {
        return StatusCode::UNAUTHORIZED;
    }
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    if payload["event"] == "messages.received" {
        let m = &payload["data"]["messages"];
        // m["key"]["cleanedSenderPn"], m["messageBody"]: queue slow work, answer fast
    }
    StatusCode::OK
}
```

## Errors

API errors come back as `Error::Api { status, message, errors, body, retry_after }`. A 503 means the device is reconnecting: retry shortly.

Never put the key in a browser (WASM) or mobile app: anyone could read it. Call the API from your server.

## Migrating from WaSender

WAservix accepts the same `/send-message` body and webhook fields as WaSender, so most integrations switch by changing the base URL and key. See the [compatibility notes](https://tekservix.com/waservix/docs/).

## Use it responsibly

WAservix is not Meta's official WhatsApp Business Platform. There's no business verification and no per-conversation fee, but WhatsApp's spam rules apply to your number directly. Message people who expect to hear from you, warm up new numbers slowly and keep the built-in account protection on.

## More from Tekservix

- [SMS Gora SMS Gateway](https://tekservix.com/smsgora/): send SMS through your own Android phones (`cargo add smsgora`)
- [AI Sales Agent](https://tekservix.com/ai-sales-agent): answers customers on WhatsApp and SMS using your website's content
- [Website to App](https://tekservix.com/mobile-apps): turn your website into Android and iOS apps
- WAservix is also available for Node.js, Python, PHP, Go, Java/Kotlin, .NET, Ruby and Swift: see [github.com/TekservixHQ](https://github.com/TekservixHQ)

## License

MIT
