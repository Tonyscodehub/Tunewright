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
        let (sock, _) = tungstenite::connect(url).map_err(|e| format!("Could not connect to {url}: {e}"))?;
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
