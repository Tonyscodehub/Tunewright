//! Types shared between the UI, the worker thread and the two backends.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    MusicAssistant,
    HomeAssistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthMethod {
    #[default]
    Password,
    Token,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

impl Repeat {
    pub fn as_str(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::One => "one",
            Repeat::All => "all",
        }
    }
    pub fn parse(s: &str) -> Repeat {
        match s {
            "one" => Repeat::One,
            "all" => Repeat::All,
            _ => Repeat::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    Playing,
    Paused,
    #[default]
    Idle,
    Off,
}

/// What happens to the queue when a search result is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Enqueue {
    #[default]
    Play,
    Replace,
    Next,
    Add,
}

impl Enqueue {
    pub fn as_str(self) -> &'static str {
        match self {
            Enqueue::Play => "play",
            Enqueue::Replace => "replace",
            Enqueue::Next => "next",
            Enqueue::Add => "add",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Enqueue::Play => "Play now",
            Enqueue::Replace => "Replace queue",
            Enqueue::Next => "Play next",
            Enqueue::Add => "Add to queue",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PlayerInfo {
    /// MA player id, or HA entity id.
    pub id: String,
    /// MA queue id (unused for HA).
    pub queue_id: String,
    pub name: String,
    pub state: State,
    /// 0 to 100.
    pub volume: Option<u8>,
    pub muted: Option<bool>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub repeat: Repeat,
    pub shuffle: bool,
}

#[derive(Clone, Debug)]
pub struct SearchItem {
    pub media_type: String,
    pub name: String,
    pub subtitle: String,
    pub uri: String,
}

#[derive(Clone, Debug)]
pub struct ConnectRequest {
    pub mode: Mode,
    pub url: String,
    pub auth: AuthMethod,
    pub username: String,
    pub password: String,
    pub token: String,
    /// Stored MA long-lived token, or HA refresh token, from an earlier login.
    pub saved_token: Option<String>,
}

pub type Result<T> = std::result::Result<T, String>;

/// Everything the UI needs from a server, whichever kind it is.
pub trait Backend: Send {
    fn players(&mut self) -> Result<Vec<PlayerInfo>>;
    fn play_pause(&mut self, p: &PlayerInfo) -> Result<()>;
    fn next(&mut self, p: &PlayerInfo) -> Result<()>;
    fn previous(&mut self, p: &PlayerInfo) -> Result<()>;
    fn set_volume(&mut self, p: &PlayerInfo, volume: u8) -> Result<()>;
    fn set_mute(&mut self, p: &PlayerInfo, muted: bool) -> Result<()>;
    fn set_repeat(&mut self, p: &PlayerInfo, repeat: Repeat) -> Result<()>;
    fn set_shuffle(&mut self, p: &PlayerInfo, shuffle: bool) -> Result<()>;
    fn search(&mut self, query: &str) -> Result<Vec<SearchItem>>;
    fn play_media(&mut self, p: &PlayerInfo, item: &SearchItem, enqueue: Enqueue) -> Result<()>;
}

/// Pull the display parts out of a Music Assistant media item (shared by both backends,
/// because Home Assistant's music_assistant.search returns the same shape).
pub fn search_item_from_json(v: &serde_json::Value, fallback_type: &str) -> Option<SearchItem> {
    let uri = v.get("uri")?.as_str()?.to_string();
    let mut name = v.get("name")?.as_str()?.to_string();
    if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
        if !ver.is_empty() {
            name = format!("{name} ({ver})");
        }
    }
    let media_type = v
        .get("media_type")
        .and_then(|x| x.as_str())
        .unwrap_or(fallback_type)
        .to_string();
    let artists: Vec<&str> = v
        .get("artists")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("name")?.as_str()).collect())
        .unwrap_or_default();
    let album = v.get("album").and_then(|a| a.get("name")).and_then(|x| x.as_str());
    let mut subtitle = artists.join(", ");
    if let Some(album) = album {
        if media_type == "track" {
            if subtitle.is_empty() {
                subtitle = album.to_string();
            } else {
                subtitle = format!("{subtitle} – {album}");
            }
        }
    }
    if subtitle.is_empty() {
        if let Some(owner) = v.get("owner").and_then(|x| x.as_str()) {
            subtitle = owner.to_string();
        }
    }
    Some(SearchItem { media_type, name, subtitle, uri })
}

pub const SEARCH_GROUPS: [(&str, &str); 5] = [
    ("tracks", "track"),
    ("albums", "album"),
    ("artists", "artist"),
    ("playlists", "playlist"),
    ("radio", "radio"),
];

/// Normalise a base URL to scheme://host[:port]. Anything after the host (a page path
/// copied from the browser, "#/home", an ingress link) is dropped, because the APIs live
/// at fixed paths from the server root.
pub fn normalise_url(url: &str) -> String {
    let u = url.trim();
    let (scheme, rest) = match u.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("http".to_string(), u),
    };
    let scheme = if scheme == "https" { "https" } else { "http" };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}")
}

pub fn ws_url(base: &str, path: &str) -> String {
    let b = base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    format!("{b}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls() {
        assert_eq!(normalise_url("homeassistant.local:8123/lovelace/0"), "http://homeassistant.local:8123");
        assert_eq!(normalise_url(" http://192.168.1.5:8095/#/home "), "http://192.168.1.5:8095");
        assert_eq!(normalise_url("HTTPS://ha.example.com/hassio/ingress/x?y=1"), "https://ha.example.com");
        assert_eq!(normalise_url("http://10.0.0.2:8095/"), "http://10.0.0.2:8095");
        assert_eq!(ws_url("https://a.b", "/ws"), "wss://a.b/ws");
    }
}
