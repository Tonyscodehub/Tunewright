//! Control Music Assistant through Home Assistant (WebSocket API at /api/websocket).
//! Players are the Music Assistant media_player entities; search and play use the
//! music_assistant.search and music_assistant.play_media actions.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::model::*;
use crate::ws::JsonSocket;

pub struct HomeAssistant {
    ws: JsonSocket,
    next_id: u64,
    /// Music Assistant player entities.
    ma_entities: HashSet<String>,
    /// False when the Music Assistant integration wasn't found: show every media player.
    integration_found: bool,
    config_entry_id: Option<String>,
    pub server_label: String,
}

impl HomeAssistant {
    /// Connect and authenticate. Returns the backend and, for a password login,
    /// a refresh token to remember instead of the password.
    pub fn connect(req: &ConnectRequest) -> Result<(Self, Option<String>)> {
        let base = normalise_url(&req.url);
        let (access, new_refresh) = match req.auth {
            AuthMethod::Token => {
                let t = req.token.trim().to_string();
                if t.is_empty() {
                    return Err("Paste a Home Assistant long-lived access token".into());
                }
                (t, None)
            }
            AuthMethod::Password => {
                let refreshed = req.saved_token.as_deref().and_then(|r| refresh(&base, r).ok());
                match refreshed {
                    Some(a) => (a, None),
                    None => {
                        if req.password.is_empty() {
                            return Err("Enter your Home Assistant password".into());
                        }
                        let (a, r) = login_flow(&base, req.username.trim(), &req.password)?;
                        (a, Some(r))
                    }
                }
            }
        };

        let mut ws = JsonSocket::connect(&ws_url(&base, "/api/websocket")).map_err(|e| {
            if crate::ws::probe_kind(&base) == Some("music_assistant") {
                format!("{base} is Music Assistant – choose \"Music Assistant\" above, or use Home Assistant's address (usually port 8123)")
            } else {
                e
            }
        })?;
        let hello = ws.recv()?;
        let version = hello.get("ha_version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        ws.send(&json!({ "type": "auth", "access_token": access }))?;
        let reply = ws.recv()?;
        if reply.get("type").and_then(|t| t.as_str()) != Some("auth_ok") {
            let msg = reply.get("message").and_then(|m| m.as_str()).unwrap_or("rejected");
            return Err(format!("Home Assistant sign-in failed: {msg}"));
        }

        let mut me = Self {
            ws,
            next_id: 1,
            ma_entities: HashSet::new(),
            integration_found: false,
            config_entry_id: None,
            server_label: format!("Home Assistant {version}"),
        };
        me.discover();
        Ok((me, new_refresh))
    }

    /// Find the Music Assistant integration's entities and config entry.
    fn discover(&mut self) {
        if let Ok(Value::Array(entries)) = self.cmd(json!({ "type": "config/entity_registry/list" })) {
            for e in entries {
                if e.get("platform").and_then(|p| p.as_str()) != Some("music_assistant") {
                    continue;
                }
                let Some(id) = e.get("entity_id").and_then(|i| i.as_str()) else { continue };
                if !id.starts_with("media_player.") {
                    continue;
                }
                self.integration_found = true;
                if self.config_entry_id.is_none() {
                    self.config_entry_id =
                        e.get("config_entry_id").and_then(|c| c.as_str()).map(str::to_string);
                }
                // Dashboard displays (screens that can show Music Assistant's party or
                // now-playing dashboard) are named after the screen but aren't players.
                let unique = e.get("unique_id").and_then(|u| u.as_str()).unwrap_or("");
                if unique.starts_with("dashboard:") {
                    continue;
                }
                // Respect entities disabled or hidden in Home Assistant.
                let set = |k: &str| e.get(k).is_some_and(|v| !v.is_null());
                if set("disabled_by") || set("hidden_by") {
                    continue;
                }
                self.ma_entities.insert(id.to_string());
            }
        }
        if self.config_entry_id.is_none() {
            if let Ok(Value::Array(entries)) =
                self.cmd(json!({ "type": "config_entries/get", "domain": "music_assistant" }))
            {
                self.config_entry_id = entries
                    .iter()
                    .find(|e| e.get("state").and_then(|s| s.as_str()) == Some("loaded"))
                    .or_else(|| entries.first())
                    .and_then(|e| e.get("entry_id").and_then(|i| i.as_str()))
                    .map(str::to_string);
            }
        }
    }

    fn cmd(&mut self, mut msg: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        msg["id"] = json!(id);
        self.ws.send(&msg)?;
        loop {
            let reply = self.ws.recv()?;
            if reply.get("id").and_then(|i| i.as_u64()) != Some(id) {
                continue;
            }
            if reply.get("success").and_then(|s| s.as_bool()) == Some(true) {
                return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
            }
            let err = reply
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("request failed");
            return Err(err.to_string());
        }
    }

    fn service(&mut self, domain: &str, service: &str, entity: Option<&str>, data: Value, want_response: bool) -> Result<Value> {
        let mut msg = json!({
            "type": "call_service",
            "domain": domain,
            "service": service,
            "service_data": data,
        });
        if let Some(e) = entity {
            msg["target"] = json!({ "entity_id": e });
        }
        if want_response {
            msg["return_response"] = json!(true);
        }
        self.cmd(msg).map_err(|e| format!("{domain}.{service}: {e}"))
    }

    fn mp(&mut self, service: &str, p: &PlayerInfo, data: Value) -> Result<()> {
        self.service("media_player", service, Some(&p.id), data, false).map(|_| ())
    }
}

fn refresh(base: &str, refresh_token: &str) -> Result<String> {
    let client_id = format!("{base}/");
    let mut resp = ureq::post(&format!("{base}/auth/token"))
        .send_form([
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id.as_str()),
        ])
        .map_err(|e| e.to_string())?;
    let v: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
    v.get("access_token").and_then(|t| t.as_str()).map(str::to_string).ok_or_else(|| "no token".into())
}

/// Send a request and return (status, JSON body) without treating 4xx/5xx as a transport
/// error, so Home Assistant's own message can be shown.
fn ha_request(base: &str, req: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, Value)> {
    match req {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let body = r.body_mut().read_json::<Value>().unwrap_or(Value::Null);
            Ok((status, body))
        }
        Err(e) => {
            let msg = e.to_string();
            Err(crate::ws::explain_network(base, &msg, 8123).unwrap_or(format!("Home Assistant login failed: {msg}")))
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

fn ha_message(body: &Value) -> String {
    body.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string()
}

/// Home Assistant's own sign-in. Works through the providers Home Assistant offers
/// (password, trusted networks, ...) and answers whichever form each one asks for.
/// Returns (access, refresh).
fn login_flow(base: &str, username: &str, password: &str) -> Result<(String, String)> {
    let client_id = format!("{base}/");
    let a = agent();

    let (status, providers) = ha_request(base, a.get(&format!("{base}/auth/providers")).call())?;
    if status == 404 {
        if crate::ws::probe_kind(base) == Some("music_assistant") {
            return Err(format!(
                "{base} is Music Assistant – choose \"Music Assistant\" above, or use Home Assistant's address (usually port 8123)"
            ));
        }
        return Err(format!("{base} doesn't look like Home Assistant – check the address and port (usually 8123)"));
    }
    let mut list = providers
        .get("providers")
        .and_then(|p| p.as_array())
        .cloned()
        .or_else(|| providers.as_array().cloned())
        .unwrap_or_default();
    if list.is_empty() {
        list.push(json!({ "type": "homeassistant", "id": null }));
    }
    // Password provider first, then the rest in Home Assistant's order.
    list.sort_by_key(|p| p.get("type").and_then(|t| t.as_str()) != Some("homeassistant"));

    let mut problems: Vec<String> = Vec::new();
    for prov in &list {
        let kind = prov.get("type").and_then(|t| t.as_str()).unwrap_or("?").to_string();
        let handler = json!([prov.get("type").cloned().unwrap_or(Value::Null), prov.get("id").cloned().unwrap_or(Value::Null)]);
        let (status, mut step) = ha_request(
            base,
            a.post(&format!("{base}/auth/login_flow")).send_json(json!({
                "client_id": client_id,
                "handler": handler,
                "redirect_uri": format!("{base}/?auth_callback=1"),
            })),
        )?;
        let Some(flow_id) = step.get("flow_id").and_then(|f| f.as_str()).map(str::to_string) else {
            problems.push(format!("{kind}: couldn't start ({status} {})", ha_message(&step)));
            continue;
        };

        // Answer up to a few form steps (password form, user picker).
        for _ in 0..3 {
            match step.get("type").and_then(|t| t.as_str()) {
                Some("create_entry") => break,
                Some("abort") => break,
                Some("form") => {}
                _ => break,
            }
            let step_id = step.get("step_id").and_then(|s| s.as_str()).unwrap_or("");
            if step_id == "mfa" || step_id == "select_mfa_module" {
                return Err("This account uses two-factor sign-in – choose Token and paste a long-lived access token instead".into());
            }
            if step.get("errors").and_then(|e| e.get("base")).and_then(|b| b.as_str()) == Some("invalid_auth") {
                break;
            }
            let fields: Vec<Value> = step.get("data_schema").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            let names: Vec<&str> = fields.iter().filter_map(|f| f.get("name")?.as_str()).collect();
            let answer = if names.contains(&"username") && names.contains(&"password") {
                json!({ "username": username, "password": password, "client_id": client_id })
            } else if names == ["user"] {
                // Trusted networks: pick the user whose name matches.
                let opts = fields[0].get("options").and_then(|o| o.as_array()).cloned().unwrap_or_default();
                let wanted = username.to_lowercase();
                let found = opts.iter().find_map(|o| {
                    let id = o.get(0)?.as_str()?;
                    let name = o.get(1)?.as_str()?;
                    (name.to_lowercase() == wanted).then(|| id.to_string())
                });
                match found {
                    Some(id) => json!({ "user": id, "client_id": client_id }),
                    None => {
                        problems.push(format!("{kind}: no user named \"{username}\""));
                        break;
                    }
                }
            } else {
                problems.push(format!("{kind}: asks for {}", names.join(", ")));
                break;
            };
            let (st, next) = ha_request(base, a.post(&format!("{base}/auth/login_flow/{flow_id}")).send_json(answer))?;
            if next.get("type").is_none() {
                problems.push(format!("{kind}: {} ({st})", ha_message(&next)));
                step = Value::Null;
                break;
            }
            step = next;
        }

        match step.get("type").and_then(|t| t.as_str()) {
            Some("create_entry") => {
                let code = step.get("result").and_then(|c| c.as_str()).ok_or("Home Assistant gave no sign-in code")?;
                return exchange_code(&a, base, &client_id, code);
            }
            Some("form") if step.get("errors").and_then(|e| e.get("base")).and_then(|b| b.as_str()) == Some("invalid_auth") => {
                problems.push(format!("{kind}: wrong username or password"));
            }
            Some("abort") => {
                problems.push(format!("{kind}: {}", step.get("reason").and_then(|r| r.as_str()).unwrap_or("stopped")));
            }
            _ => {}
        }
        // Tidy up the unfinished flow; ignore failures.
        let _ = a.delete(&format!("{base}/auth/login_flow/{flow_id}")).call();
    }

    if problems.iter().any(|p| p.ends_with("wrong username or password")) {
        return Err("Home Assistant says the username or password is wrong. Use your Home Assistant username (shown under your profile), not your display name.".into());
    }
    Err(format!("Home Assistant sign-in didn't work: {}", problems.join("; ")))
}

fn exchange_code(a: &ureq::Agent, base: &str, client_id: &str, code: &str) -> Result<(String, String)> {
    let (status, tok) = ha_request(
        base,
        a.post(&format!("{base}/auth/token"))
            .send_form([("grant_type", "authorization_code"), ("code", code), ("client_id", client_id)]),
    )?;
    let access = tok.get("access_token").and_then(|t| t.as_str()).ok_or_else(|| {
        let why = tok.get("error_description").or_else(|| tok.get("error")).and_then(|e| e.as_str()).unwrap_or("");
        format!("Home Assistant didn't issue a token ({status}) {why}")
    })?;
    let refresh = tok.get("refresh_token").and_then(|t| t.as_str()).unwrap_or_default();
    Ok((access.to_string(), refresh.to_string()))
}

impl Backend for HomeAssistant {
    fn players(&mut self) -> Result<Vec<PlayerInfo>> {
        let states = self.cmd(json!({ "type": "get_states" }))?;
        let mut out = Vec::new();
        for s in states.as_array().into_iter().flatten() {
            let Some(id) = s.get("entity_id").and_then(|i| i.as_str()) else { continue };
            if !id.starts_with("media_player.") {
                continue;
            }
            if self.integration_found && !self.ma_entities.contains(id) {
                continue;
            }
            let state_str = s.get("state").and_then(|x| x.as_str()).unwrap_or("");
            if state_str == "unavailable" {
                continue;
            }
            let a = s.get("attributes").cloned().unwrap_or(Value::Null);
            let at = |k: &str| a.get(k).and_then(|x| x.as_str()).map(str::to_string);
            out.push(PlayerInfo {
                id: id.to_string(),
                queue_id: String::new(),
                name: at("friendly_name").unwrap_or_else(|| id.to_string()),
                state: match state_str {
                    "playing" => State::Playing,
                    "paused" => State::Paused,
                    "off" => State::Off,
                    _ => State::Idle,
                },
                volume: a.get("volume_level").and_then(|v| v.as_f64()).map(|v| (v * 100.0).round().clamp(0.0, 100.0) as u8),
                muted: a.get("is_volume_muted").and_then(|v| v.as_bool()),
                title: at("media_title"),
                artist: at("media_artist"),
                album: at("media_album_name"),
                repeat: Repeat::parse(at("repeat").as_deref().unwrap_or("off")),
                shuffle: a.get("shuffle").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    fn play_pause(&mut self, p: &PlayerInfo) -> Result<()> {
        self.mp("media_play_pause", p, json!({}))
    }
    fn next(&mut self, p: &PlayerInfo) -> Result<()> {
        self.mp("media_next_track", p, json!({}))
    }
    fn previous(&mut self, p: &PlayerInfo) -> Result<()> {
        self.mp("media_previous_track", p, json!({}))
    }
    fn set_volume(&mut self, p: &PlayerInfo, volume: u8) -> Result<()> {
        self.mp("volume_set", p, json!({ "volume_level": volume as f64 / 100.0 }))
    }
    fn set_mute(&mut self, p: &PlayerInfo, muted: bool) -> Result<()> {
        self.mp("volume_mute", p, json!({ "is_volume_muted": muted }))
    }
    fn set_repeat(&mut self, p: &PlayerInfo, repeat: Repeat) -> Result<()> {
        self.mp("repeat_set", p, json!({ "repeat": repeat.as_str() }))
    }
    fn set_shuffle(&mut self, p: &PlayerInfo, shuffle: bool) -> Result<()> {
        self.mp("shuffle_set", p, json!({ "shuffle": shuffle }))
    }

    fn search(&mut self, query: &str) -> Result<Vec<SearchItem>> {
        let entry = self
            .config_entry_id
            .clone()
            .ok_or("Couldn't find the Music Assistant integration in Home Assistant")?;
        let types: Vec<&str> = SEARCH_GROUPS.iter().map(|(_, t)| *t).collect();
        let res = self.service(
            "music_assistant",
            "search",
            None,
            json!({ "config_entry_id": entry, "name": query, "media_type": types, "limit": 8 }),
            true,
        )?;
        let res = res.get("response").cloned().unwrap_or(res);
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
        self.service(
            "music_assistant",
            "play_media",
            Some(&p.id),
            json!({ "media_id": item.uri, "media_type": item.media_type, "enqueue": enqueue.as_str() }),
            false,
        )
        .map(|_| ())
    }
}
