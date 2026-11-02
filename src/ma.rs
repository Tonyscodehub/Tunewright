//! Direct connection to a Music Assistant server (WebSocket API on port 8095 by default).

use serde_json::{json, Value};

use crate::model::*;
use crate::ws::JsonSocket;

pub struct MusicAssistant {
    ws: JsonSocket,
    next_id: u64,
    pub server_label: String,
}

impl MusicAssistant {
    /// Connect and authenticate. Returns the backend and, when a password login was used,
    /// a long-lived token to remember instead of the password.
    pub fn connect(req: &ConnectRequest) -> Result<(Self, Option<String>)> {
        let base = normalise_url(&req.url);
        let mut ws = JsonSocket::connect(&ws_url(&base, "/ws"))?;
        // The server greets every new connection with its server info.
        let info = ws.recv()?;
        let schema = info.get("schema_version").and_then(|v| v.as_i64()).unwrap_or(0);
        let version = info.get("server_version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        let mut me = Self { ws, next_id: 1, server_label: format!("Music Assistant {version}") };

        if schema < 28 {
            // Older servers have no authentication.
            return Ok((me, None));
        }

        match req.auth {
            AuthMethod::Token => {
                me.authenticate(req.token.trim())?;
                Ok((me, None))
            }
            AuthMethod::Password => {
                if let Some(saved) = &req.saved_token {
                    if me.authenticate(saved).is_ok() {
                        return Ok((me, None));
                    }
                    // Saved token revoked or expired: start again with the password.
                    if req.password.is_empty() {
                        return Err("Your saved sign-in no longer works – enter your password".into());
                    }
                    return Self::connect(&ConnectRequest { saved_token: None, ..req.clone() });
                }
                if req.password.is_empty() {
                    return Err("Enter your Music Assistant password".into());
                }
                let access = login(&base, req.username.trim(), &req.password)?;
                me.authenticate(&access)?;
                let long_lived = me.create_long_lived_token();
                Ok((me, long_lived))
            }
        }
    }

    fn authenticate(&mut self, token: &str) -> Result<()> {
        if token.is_empty() {
            return Err("Enter a Music Assistant token".into());
        }
        let ok = self.cmd("auth", json!({ "token": token }))?;
        if ok.is_null() || ok == json!(false) {
            return Err("Music Assistant rejected the token".into());
        }
        Ok(())
    }

    fn create_long_lived_token(&mut self) -> Option<String> {
        self.cmd("auth/token/create", json!({ "name": "Tunewright" }))
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
    }

    /// Send a command and wait for its result, collecting chunked (partial) results.
    fn cmd(&mut self, command: &str, args: Value) -> Result<Value> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        self.ws.send(&json!({ "message_id": id, "command": command, "args": args }))?;
        let mut partial: Option<Vec<Value>> = None;
        loop {
            let msg = self.ws.recv()?;
            if msg.get("message_id").and_then(|m| m.as_str()) != Some(id.as_str()) {
                continue; // events and other replies
            }
            if let Some(code) = msg.get("error_code") {
                let details = msg.get("details").and_then(|d| d.as_str()).unwrap_or("unknown error");
                return Err(format!("{command} failed ({code}): {details}"));
            }
            let result = msg.get("result").cloned().unwrap_or(Value::Null);
            let is_partial = msg.get("partial").and_then(|p| p.as_bool()).unwrap_or(false);
            if is_partial || partial.is_some() {
                let acc = partial.get_or_insert_with(Vec::new);
                if let Value::Array(items) = result {
                    acc.extend(items);
                }
                if !is_partial {
                    return Ok(Value::Array(partial.take().unwrap_or_default()));
                }
                continue;
            }
            return Ok(result);
        }
    }

    fn player_cmd(&mut self, command: &str, p: &PlayerInfo, extra: Value) -> Result<()> {
        let mut args = json!({ "player_id": p.id });
        if let (Value::Object(a), Value::Object(e)) = (&mut args, extra) {
            a.extend(e);
        }
        self.cmd(command, args).map(|_| ())
    }
}

/// POST /auth/login. Sends both the old flat and the newer nested credential shapes.
fn login(base: &str, username: &str, password: &str) -> Result<String> {
    let body = json!({
        "username": username,
        "password": password,
        "credentials": { "username": username, "password": password },
    });
    let mut resp = ureq::post(&format!("{base}/auth/login"))
        .send_json(&body)
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => "Wrong username or password".to_string(),
            other => format!("Login failed: {other}"),
        })?;
    let v: Value = resp.body_mut().read_json().map_err(|e| format!("Login failed: {e}"))?;
    v.get("token")
        .or_else(|| v.get("access_token"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Login response had no token".to_string())
}

impl Backend for MusicAssistant {
    fn players(&mut self) -> Result<Vec<PlayerInfo>> {
        let players = self.cmd("players/all", json!({}))?;
        let queues = self.cmd("player_queues/all", json!({})).unwrap_or(Value::Null);
        let queues = queues.as_array().cloned().unwrap_or_default();
        let find_queue = |id: &str| queues.iter().find(|q| q.get("queue_id").and_then(|x| x.as_str()) == Some(id));

        let mut out = Vec::new();
        for p in players.as_array().into_iter().flatten() {
            let s = |k: &str| p.get(k).and_then(|x| x.as_str()).map(str::to_string);
            let b = |k: &str| p.get(k).and_then(|x| x.as_bool());
            if b("available") == Some(false) || b("enabled") == Some(false) || b("hide_in_ui") == Some(true) {
                continue;
            }
            let id = s("player_id").unwrap_or_default();
            let active_source = s("active_source").unwrap_or_default();
            let queue = find_queue(&active_source).or_else(|| find_queue(&id));
            let queue_id = queue
                .and_then(|q| q.get("queue_id").and_then(|x| x.as_str()))
                .unwrap_or(&id)
                .to_string();
            let state = if b("powered") == Some(false) {
                State::Off
            } else {
                match s("playback_state").or_else(|| s("state")).as_deref() {
                    Some("playing") => State::Playing,
                    Some("paused") => State::Paused,
                    _ => State::Idle,
                }
            };
            let media = p.get("current_media");
            let ms = |k: &str| media.and_then(|m| m.get(k)).and_then(|x| x.as_str()).map(str::to_string);
            out.push(PlayerInfo {
                name: s("display_name").or_else(|| s("name")).unwrap_or_else(|| id.clone()),
                id,
                queue_id,
                state,
                volume: p.get("volume_level").and_then(|v| v.as_f64()).map(|v| v.clamp(0.0, 100.0) as u8),
                muted: b("volume_muted"),
                title: ms("title"),
                artist: ms("artist"),
                album: ms("album"),
                repeat: Repeat::parse(queue.and_then(|q| q.get("repeat_mode")).and_then(|x| x.as_str()).unwrap_or("off")),
                shuffle: queue.and_then(|q| q.get("shuffle_enabled")).and_then(|x| x.as_bool()).unwrap_or(false),
            });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    fn play_pause(&mut self, p: &PlayerInfo) -> Result<()> {
        self.player_cmd("players/cmd/play_pause", p, json!({}))
    }
    fn next(&mut self, p: &PlayerInfo) -> Result<()> {
        self.player_cmd("players/cmd/next", p, json!({}))
    }
    fn previous(&mut self, p: &PlayerInfo) -> Result<()> {
        self.player_cmd("players/cmd/previous", p, json!({}))
    }
    fn set_volume(&mut self, p: &PlayerInfo, volume: u8) -> Result<()> {
        self.player_cmd("players/cmd/volume_set", p, json!({ "volume_level": volume }))
    }
    fn set_mute(&mut self, p: &PlayerInfo, muted: bool) -> Result<()> {
        self.player_cmd("players/cmd/volume_mute", p, json!({ "muted": muted }))
    }
    fn set_repeat(&mut self, p: &PlayerInfo, repeat: Repeat) -> Result<()> {
        self.cmd("player_queues/repeat", json!({ "queue_id": p.queue_id, "repeat_mode": repeat.as_str() }))
            .map(|_| ())
    }
    fn set_shuffle(&mut self, p: &PlayerInfo, shuffle: bool) -> Result<()> {
        self.cmd("player_queues/shuffle", json!({ "queue_id": p.queue_id, "shuffle_enabled": shuffle }))
            .map(|_| ())
    }

    fn search(&mut self, query: &str) -> Result<Vec<SearchItem>> {
        let types: Vec<&str> = SEARCH_GROUPS.iter().map(|(_, t)| *t).collect();
        let res = self.cmd(
            "music/search",
            json!({ "search_query": query, "media_types": types, "limit": 8, "library_only": false }),
        )?;
        let mut out = Vec::new();
        for (key, t) in SEARCH_GROUPS {
            for item in res.get(key).and_then(|x| x.as_array()).into_iter().flatten() {
                if let Some(si) = search_item_from_json(item, t) {
                    out.push(si);
                }
            }
        }
        Ok(out)
    }

    fn play_media(&mut self, p: &PlayerInfo, item: &SearchItem, enqueue: Enqueue) -> Result<()> {
        // A player showing an external source has no queue of ours; play on its own queue.
        let queue_id = if p.queue_id.is_empty() { &p.id } else { &p.queue_id };
        self.cmd(
            "player_queues/play_media",
            json!({ "queue_id": queue_id, "media": item.uri, "option": enqueue.as_str() }),
        )
        .map(|_| ())
    }
}
