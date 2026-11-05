//! Settings saved between runs, in %APPDATA%\Tunewright\config.json on Windows.
//! Passwords are never saved: a password login is swapped for a server token.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{AuthMethod, Mode};

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub url: String,
    pub auth: AuthMethod,
    pub username: String,
    /// Token pasted by the user (token sign-in).
    pub token: String,
    /// MA long-lived token or HA refresh token obtained from a password login.
    pub saved_token: Option<String>,
    /// Which mode/url/username the saved token belongs to.
    pub saved_token_for: String,
    pub remember: bool,
    pub last_player: Option<String>,
    /// Keep the window above other windows.
    pub always_on_top: bool,
}

impl Config {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("Tunewright").join("config.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Config { remember: true, ..Default::default() })
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        let mut c = self.clone();
        if !c.remember {
            c.token.clear();
            c.saved_token = None;
            c.saved_token_for.clear();
        }
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(&c) {
            let _ = std::fs::write(p, s);
        }
    }

    pub fn identity(&self) -> String {
        format!("{:?}|{}|{}", self.mode, crate::model::normalise_url(&self.url), self.username.trim())
    }
}
