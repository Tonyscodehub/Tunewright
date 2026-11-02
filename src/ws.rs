//! A thin blocking JSON-over-WebSocket client.

use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::model::Result;

pub struct JsonSocket {
    sock: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl JsonSocket {
    pub fn connect(url: &str) -> Result<Self> {
        let (sock, _) = tungstenite::connect(url).map_err(|e| match &e {
            tungstenite::Error::Http(r) => format!("{url} answered {} – is this the right kind of server and port?", r.status()),
            _ => format!("Could not connect to {url}: {e}"),
        })?;
        let timeout = Some(Duration::from_secs(20));
        match sock.get_ref() {
            MaybeTlsStream::Plain(s) => {
                let _ = s.set_read_timeout(timeout);
                let _ = s.set_write_timeout(timeout);
            }
            MaybeTlsStream::Rustls(s) => {
                let _ = s.sock.set_read_timeout(timeout);
                let _ = s.sock.set_write_timeout(timeout);
            }
            _ => {}
        }
        Ok(Self { sock })
    }

    pub fn send(&mut self, v: &Value) -> Result<()> {
        self.sock
            .send(Message::text(v.to_string()))
            .map_err(|e| format!("Connection lost: {e}"))
    }

    /// Read the next JSON message, answering pings and skipping non-text frames.
    pub fn recv(&mut self) -> Result<Value> {
        loop {
            let msg = self.sock.read().map_err(|e| format!("Connection lost: {e}"))?;
            match msg {
                Message::Text(t) => {
                    return serde_json::from_str(t.as_str()).map_err(|e| format!("Bad message from server: {e}"))
                }
                Message::Binary(b) => {
                    if let Ok(v) = serde_json::from_slice(&b) {
                        return Ok(v);
                    }
                }
                Message::Close(_) => return Err("The server closed the connection".into()),
                _ => {}
            }
        }
    }
}

impl Drop for JsonSocket {
    fn drop(&mut self) {
        let _ = self.sock.close(None);
    }
}

/// Work out what's actually at an address, to turn a bare 404 into useful advice.
pub fn probe_kind(base: &str) -> Option<&'static str> {
    // Music Assistant answers /info with its server details, no sign-in needed.
    if let Ok(mut r) = ureq::get(&format!("{base}/info")).call() {
        if let Ok(v) = r.body_mut().read_json::<serde_json::Value>() {
            if v.get("server_id").is_some() && v.get("schema_version").is_some() {
                return Some("music_assistant");
            }
        }
    }
    // Home Assistant answers /api/ with 401 when you're not signed in.
    match ureq::get(&format!("{base}/api/")).call() {
        Err(ureq::Error::StatusCode(401)) => Some("home_assistant"),
        Ok(r) if r.status() == 200 => Some("home_assistant"),
        _ => None,
    }
}
