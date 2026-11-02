#![windows_subsystem = "windows"]
//! Tunewright – a small remote control for Music Assistant,
//! either directly or through Home Assistant.

mod config;
mod ha;
mod icons;
mod ma;
mod model;
mod worker;
mod ws;

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use config::Config;
use icons::{icon_button, Icon};
use model::*;
use worker::{Cmd, Event};

#[derive(PartialEq)]
enum Conn {
    Disconnected,
    Connecting,
    Connected(String),
    Reconnecting(String),
}

struct App {
    cfg: Config,
    password: String,
    conn: Conn,
    show_settings: bool,
    message: Option<(String, bool, Instant)>, // text, is_error, when
    players: Vec<PlayerInfo>,
    selected: Option<String>,
    vol_edit: Option<u8>,
    hold_refresh_until: Instant,
    query: String,
    results: Vec<SearchItem>,
    results_for: String,
    searching: bool,
    enqueue: Enqueue,
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || worker::run(cmd_rx, ev_tx, ctx));

        let cfg = Config::load();
        let mut app = Self {
            selected: cfg.last_player.clone(),
            password: String::new(),
            conn: Conn::Disconnected,
            show_settings: true,
            message: None,
            players: Vec::new(),
            vol_edit: None,
            hold_refresh_until: Instant::now(),
            query: String::new(),
            results: Vec::new(),
            results_for: String::new(),
            searching: false,
            enqueue: Enqueue::Play,
            tx: cmd_tx,
            rx: ev_rx,
            cfg,
        };
        // Sign straight back in if we have something to sign in with.
        let can_auto = !app.cfg.url.is_empty()
            && match app.cfg.auth {
                AuthMethod::Token => !app.cfg.token.is_empty(),
                AuthMethod::Password => {
                    app.cfg.saved_token.is_some() && app.cfg.saved_token_for == app.cfg.identity()
                }
            };
        if can_auto {
            app.connect();
        }
        app
    }

    fn connect(&mut self) {
        let saved = if self.cfg.saved_token_for == self.cfg.identity() { self.cfg.saved_token.clone() } else { None };
        let req = ConnectRequest {
            mode: self.cfg.mode,
            url: self.cfg.url.clone(),
            auth: self.cfg.auth,
            username: self.cfg.username.clone(),
            password: self.password.clone(),
            token: self.cfg.token.clone(),
            saved_token: saved,
        };
        self.conn = Conn::Connecting;
        self.message = None;
        self.cfg.save();
        let _ = self.tx.send(Cmd::Connect(req));
    }

    fn flash(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some((text.into(), error, Instant::now()));
    }

    fn pump_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Connected { label, new_saved_token } => {
                    if let Some(t) = new_saved_token {
                        self.cfg.saved_token = Some(t);
                        self.cfg.saved_token_for = self.cfg.identity();
                        self.cfg.save();
                    }
                    self.password.clear();
                    if self.conn == Conn::Connecting {
                        self.show_settings = false;
                    }
                    self.conn = Conn::Connected(label);
                }
                Event::Disconnected(err) => {
                    self.conn = Conn::Disconnected;
                    self.show_settings = true;
                    self.players.clear();
                    self.searching = false;
                    if let Some(e) = err {
                        self.flash(e, true);
                    }
                }
                Event::Reconnecting(e) => {
                    self.conn = Conn::Reconnecting(e);
                    self.searching = false;
                }
                Event::Players(p) => {
                    if Instant::now() >= self.hold_refresh_until {
                        self.players = p;
                    }
                    let still_there = self.selected.as_ref().is_some_and(|s| self.players.iter().any(|p| &p.id == s));
                    if !still_there {
                        // Prefer whatever is playing.
                        self.selected = self
                            .players
                            .iter()
                            .find(|p| p.state == State::Playing)
                            .or_else(|| self.players.first())
                            .map(|p| p.id.clone());
                    }
                }
                Event::SearchResults(q, r) => {
                    self.searching = false;
                    self.results_for = q;
                    self.results = r;
                }
                Event::Error(e) => {
                    self.searching = false;
                    self.flash(e, true);
                }
            }
        }
    }

    /// Apply a change to the local copy immediately so the UI feels instant,
    /// and ignore stale refreshes for a moment.
    fn optimistic(&mut self, id: &str, f: impl FnOnce(&mut PlayerInfo)) {
        if let Some(p) = self.players.iter_mut().find(|p| p.id == id) {
            f(p);
        }
        self.hold_refresh_until = Instant::now() + Duration::from_millis(1200);
    }

    fn select(&mut self, id: String) {
        self.cfg.last_player = Some(id.clone());
        self.selected = Some(id);
        self.vol_edit = None;
        self.cfg.save();
    }

    fn connection_form(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connect");
        ui.add_space(6.0);
        let before = self.cfg.identity();
        egui::Grid::new("conn").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Connect via");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.cfg.mode, Mode::MusicAssistant, "Music Assistant");
                ui.radio_value(&mut self.cfg.mode, Mode::HomeAssistant, "Home Assistant");
            });
            ui.end_row();

            ui.label("Server URL");
            let hint = match self.cfg.mode {
                Mode::MusicAssistant => "http://homeassistant.local:8095",
                Mode::HomeAssistant => "http://homeassistant.local:8123",
            };
            ui.add(egui::TextEdit::singleline(&mut self.cfg.url).hint_text(hint).desired_width(300.0));
            ui.end_row();

            ui.label("Sign in with");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.cfg.auth, AuthMethod::Password, "Username and password");
                ui.radio_value(&mut self.cfg.auth, AuthMethod::Token, "Token");
            });
            ui.end_row();

            match self.cfg.auth {
                AuthMethod::Password => {
                    ui.label("Username");
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.username).desired_width(300.0));
                    ui.end_row();
                    ui.label("Password");
                    let has_saved = self.cfg.saved_token.is_some() && self.cfg.saved_token_for == self.cfg.identity();
                    let hint = if has_saved { "(signed in before – leave blank)" } else { "" };
                    let r = ui.add(egui::TextEdit::singleline(&mut self.password).password(true).hint_text(hint).desired_width(300.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.connect();
                    }
                    ui.end_row();
                }
                AuthMethod::Token => {
                    ui.label("Token");
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.token).password(true).desired_width(300.0));
                    ui.end_row();
                }
            }
        });
        if self.cfg.identity() != before {
            self.results.clear();
        }

        ui.add_space(4.0);
        let tip = match (self.cfg.mode, self.cfg.auth) {
            (Mode::MusicAssistant, AuthMethod::Token) => "Create a long-lived token in Music Assistant under your profile settings.",
            (Mode::HomeAssistant, AuthMethod::Token) => "Create a long-lived access token in Home Assistant under your profile, Security tab.",
            (Mode::HomeAssistant, AuthMethod::Password) => "Uses your Home Assistant login. Accounts with two-factor sign-in need a token instead.",
            (Mode::MusicAssistant, AuthMethod::Password) => "Uses your Music Assistant login. Your password isn't stored – a server token is kept instead.",
        };
        ui.label(RichText::new(tip).small().weak());
        ui.checkbox(&mut self.cfg.remember, "Remember me on this computer");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let busy = self.conn == Conn::Connecting;
            if ui.add_enabled(!busy && !self.cfg.url.trim().is_empty(), egui::Button::new("Connect")).clicked() {
                self.connect();
            }
            if busy {
                ui.spinner();
                ui.label("Connecting…");
            }
            if matches!(self.conn, Conn::Connected(_) | Conn::Reconnecting(_)) {
                if ui.button("Disconnect").clicked() {
                    let _ = self.tx.send(Cmd::Disconnect);
                }
                if ui.button("Back to player").clicked() {
                    self.show_settings = false;
                }
            }
        });
    }

    fn player_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected.clone() else {
            ui.centered_and_justified(|ui| ui.label("No players found"));
            return;
        };
        let Some(p) = self.players.iter().find(|p| p.id == id).cloned() else { return };

        // Now playing
        ui.add_space(4.0);
        ui.label(RichText::new(&p.name).weak());
        let title = p.title.clone().unwrap_or_else(|| "Nothing playing".into());
        ui.label(RichText::new(title).size(22.0).strong());
        let sub: Vec<String> = [p.artist.clone(), p.album.clone()].into_iter().flatten().filter(|s| !s.is_empty()).collect();
        if !sub.is_empty() {
            ui.label(sub.join(" – "));
        }
        ui.add_space(10.0);

        // Transport
        ui.horizontal(|ui| {
            if icon_button(ui, Icon::Previous, "", 26.0, false).on_hover_text("Previous").clicked() {
                let _ = self.tx.send(Cmd::Previous(id.clone()));
            }
            let playing = p.state == State::Playing;
            let pp = if playing { Icon::Pause } else { Icon::Play };
            if icon_button(ui, pp, "", 26.0, false).on_hover_text("Play/pause").clicked() {
                let _ = self.tx.send(Cmd::PlayPause(id.clone()));
                self.optimistic(&id, |x| x.state = if playing { State::Paused } else { State::Playing });
            }
            if icon_button(ui, Icon::Next, "", 26.0, false).on_hover_text("Skip").clicked() {
                let _ = self.tx.send(Cmd::Next(id.clone()));
            }
        });
        ui.add_space(10.0);

        // Volume and mute
        ui.horizontal(|ui| {
            let muted = p.muted.unwrap_or(false);
            let icon = if muted { Icon::Muted } else { Icon::Speaker };
            if icon_button(ui, icon, "", 18.0, muted)
                .on_hover_text(if muted { "Unmute" } else { "Mute" })
                .clicked()
            {
                let _ = self.tx.send(Cmd::Mute(id.clone(), !muted));
                self.optimistic(&id, |x| x.muted = Some(!muted));
            }
            let current = p.volume.unwrap_or(0);
            if ui.button("−").on_hover_text("Volume down").clicked() {
                let v = current.saturating_sub(5);
                let _ = self.tx.send(Cmd::Volume(id.clone(), v));
                self.optimistic(&id, |x| x.volume = Some(v));
            }
            let mut v = self.vol_edit.unwrap_or(current);
            let r = ui.add_enabled(
                p.volume.is_some(),
                egui::Slider::new(&mut v, 0..=100).suffix("%").show_value(true),
            );
            if r.dragged() {
                self.vol_edit = Some(v);
            }
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.vol_edit = None;
                let _ = self.tx.send(Cmd::Volume(id.clone(), v));
                self.optimistic(&id, |x| x.volume = Some(v));
            }
            if ui.button("+").on_hover_text("Volume up").clicked() {
                let v = (current + 5).min(100);
                let _ = self.tx.send(Cmd::Volume(id.clone(), v));
                self.optimistic(&id, |x| x.volume = Some(v));
            }
        });
        ui.add_space(6.0);

        // Repeat and shuffle
        ui.horizontal(|ui| {
            ui.label("Repeat");
            for (mode, icon, label) in [(None, None, "Off"), (Some(Repeat::One), Some(Icon::RepeatOne), "One"), (Some(Repeat::All), Some(Icon::RepeatAll), "All")] {
                let mode = mode.unwrap_or(Repeat::Off);
                let clicked = match icon {
                    Some(i) => icon_button(ui, i, label, 16.0, p.repeat == mode).clicked(),
                    None => ui
                        .add(egui::Button::new(label).selected(p.repeat == mode).min_size(egui::vec2(40.0, 28.0)))
                        .clicked(),
                };
                if clicked && p.repeat != mode {
                    let _ = self.tx.send(Cmd::Repeat(id.clone(), mode));
                    self.optimistic(&id, |x| x.repeat = mode);
                }
            }
            ui.separator();
            if icon_button(ui, Icon::Shuffle, "Shuffle", 16.0, p.shuffle).clicked() {
                let s = !p.shuffle;
                let _ = self.tx.send(Cmd::Shuffle(id.clone(), s));
                self.optimistic(&id, |x| x.shuffle = s);
            }
        });

        ui.add_space(12.0);
        ui.separator();
        self.search_panel(ui, &id);
    }

    fn search_panel(&mut self, ui: &mut egui::Ui, player_id: &str) {
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search tracks, albums, artists, playlists, and radio")
                    .desired_width(ui.available_width() - 190.0),
            );
            let go = ui.add_enabled(!self.searching && !self.query.trim().is_empty(), egui::Button::new("Search")).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.query.trim().is_empty());
            if go {
                self.searching = true;
                let _ = self.tx.send(Cmd::Search(self.query.trim().to_string()));
            }
            egui::ComboBox::from_id_salt("enqueue")
                .selected_text(self.enqueue.label())
                .width(110.0)
                .show_ui(ui, |ui| {
                    for e in [Enqueue::Play, Enqueue::Replace, Enqueue::Next, Enqueue::Add] {
                        ui.selectable_value(&mut self.enqueue, e, e.label());
                    }
                })
                .response
                .on_hover_text("What the play button does with the queue");
        });
        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching…");
            });
        }
        if !self.results_for.is_empty() && self.results.is_empty() && !self.searching {
            ui.label(RichText::new(format!("Nothing found for “{}”", self.results_for)).weak());
        }

        let mut play: Option<SearchItem> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut last_type = String::new();
            for item in &self.results {
                if item.media_type != last_type {
                    last_type = item.media_type.clone();
                    ui.add_space(6.0);
                    ui.label(RichText::new(group_title(&last_type)).strong());
                }
                ui.horizontal(|ui| {
                    if icon_button(ui, Icon::Play, "", 10.0, false).on_hover_text(self.enqueue.label()).clicked() {
                        play = Some(item.clone());
                    }
                    ui.label(&item.name);
                    if !item.subtitle.is_empty() {
                        ui.label(RichText::new(&item.subtitle).weak());
                    }
                });
            }
        });
        if let Some(item) = play {
            let what = format!("{}: {}", self.enqueue.label(), item.name);
            let _ = self.tx.send(Cmd::Play(player_id.to_string(), item, self.enqueue));
            self.flash(what, false);
        }
    }
}

fn group_title(t: &str) -> &str {
    match t {
        "track" => "Tracks",
        "album" => "Albums",
        "artist" => "Artists",
        "playlist" => "Playlists",
        "radio" => "Radio",
        other => other,
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump_events();
        if let Some((_, _, when)) = &self.message {
            if when.elapsed() > Duration::from_secs(8) {
                self.message = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tunewright").strong());
                ui.separator();
                match &self.conn {
                    Conn::Connected(label) => {
                        icons::dot(ui, Color32::from_rgb(80, 180, 100));
                        ui.label(label);
                    }
                    Conn::Reconnecting(e) => {
                        icons::dot(ui, Color32::from_rgb(220, 160, 40));
                        ui.label("Reconnecting…").on_hover_text(e);
                    }
                    Conn::Connecting => {
                        ui.spinner();
                        ui.label("Connecting…");
                    }
                    Conn::Disconnected => {
                        icons::dot(ui, Color32::GRAY);
                        ui.label("Not connected");
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.show_settings && ui.button("Connection").clicked() {
                        self.show_settings = true;
                    }
                });
            });
        });

        if let Some((text, err, _)) = &self.message {
            let (text, err) = (text.clone(), *err);
            egui::Panel::bottom("status").show(ui, |ui| {
                let colour = if err { Color32::from_rgb(220, 80, 70) } else { ui.visuals().text_color() };
                ui.colored_label(colour, text);
            });
        }

        let connected = matches!(self.conn, Conn::Connected(_) | Conn::Reconnecting(_));
        if self.show_settings || !connected {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.add_space(12.0);
                self.connection_form(ui);
            });
            return;
        }

        egui::Panel::left("players").resizable(true).default_size(190.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Players").strong());
            ui.add_space(4.0);
            let mut pick = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for p in &self.players {
                    let sel = self.selected.as_deref() == Some(p.id.as_str());
                    ui.horizontal(|ui| {
                        let c = ui.visuals().weak_text_color();
                        match p.state {
                            State::Playing => icons::small(ui, Icon::Play, Color32::from_rgb(80, 180, 100)),
                            State::Paused => icons::small(ui, Icon::Pause, c),
                            State::Off => icons::small(ui, Icon::Power, c),
                            State::Idle => {
                                ui.add_space(12.0);
                            }
                        }
                        if ui.selectable_label(sel, &p.name).clicked() {
                            pick = Some(p.id.clone());
                        }
                    });
                }
            });
            if let Some(id) = pick {
                self.select(id);
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            self.player_panel(ui);
        });
    }
}

fn main() -> eframe::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Tunewright")
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([520.0, 380.0]),
        ..Default::default()
    };
    eframe::run_native("Tunewright", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[cfg(test)]
mod mock_tests {
    //! Run with a mock server: TUNEWRIGHT_MOCK=http://127.0.0.1:8899 cargo test -- --ignored
    use super::*;

    fn base() -> String {
        std::env::var("TUNEWRIGHT_MOCK").expect("set TUNEWRIGHT_MOCK")
    }
    fn req(mode: Mode, auth: AuthMethod, password: &str, token: &str, saved: Option<&str>) -> ConnectRequest {
        ConnectRequest {
            mode,
            url: base(),
            auth,
            username: "tony".into(),
            password: password.into(),
            token: token.into(),
            saved_token: saved.map(str::to_string),
        }
    }

    #[test]
    #[ignore]
    fn music_assistant_flow() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        assert!(ma::MusicAssistant::connect(&req(Mode::MusicAssistant, AuthMethod::Password, "bad", "", None)).is_err());
        let (mut b, saved) = ma::MusicAssistant::connect(&req(Mode::MusicAssistant, AuthMethod::Password, "pw", "", None)).unwrap();
        assert_eq!(saved.as_deref(), Some("LLT"));
        let players = b.players().unwrap();
        assert_eq!(players.len(), 1, "hidden player filtered, partial results merged");
        let p = &players[0];
        assert_eq!((p.volume, p.repeat, p.shuffle, p.state), (Some(42), Repeat::All, true, State::Playing));
        assert_eq!(p.title.as_deref(), Some("Song 1"));
        b.set_volume(p, 55).unwrap();
        b.set_mute(p, true).unwrap();
        b.next(p).unwrap();
        b.previous(p).unwrap();
        b.play_pause(p).unwrap();
        b.set_repeat(p, Repeat::One).unwrap();
        b.set_shuffle(p, false).unwrap();
        let res = b.search("song").unwrap();
        assert_eq!(res.len(), 4);
        assert_eq!(res[0].subtitle, "Artist X – Album Y");
        b.play_media(p, &res[3], Enqueue::Replace).unwrap();
        // Saved token reconnects with no password; pasted token works too.
        let (_, s2) = ma::MusicAssistant::connect(&req(Mode::MusicAssistant, AuthMethod::Password, "", "", Some("LLT"))).unwrap();
        assert!(s2.is_none());
        ma::MusicAssistant::connect(&req(Mode::MusicAssistant, AuthMethod::Token, "", "PASTED", None)).unwrap();
        // Revoked saved token with no password gives a clear error.
        let e = ma::MusicAssistant::connect(&req(Mode::MusicAssistant, AuthMethod::Password, "", "", Some("REVOKED"))).err().unwrap();
        assert!(e.contains("enter your password"), "{e}");
    }

    #[test]
    #[ignore]
    fn home_assistant_flow() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        assert!(ha::HomeAssistant::connect(&req(Mode::HomeAssistant, AuthMethod::Password, "bad", "", None)).is_err());
        let (mut b, refresh) = ha::HomeAssistant::connect(&req(Mode::HomeAssistant, AuthMethod::Password, "pw", "", None)).unwrap();
        assert_eq!(refresh.as_deref(), Some("REFR"));
        let players = b.players().unwrap();
        assert_eq!(players.len(), 1, "only Music Assistant entities");
        let p = &players[0];
        assert_eq!((p.name.as_str(), p.volume, p.muted, p.repeat, p.state), ("Lounge", Some(30), Some(true), Repeat::One, State::Paused));
        b.set_volume(p, 75).unwrap();
        b.set_mute(p, false).unwrap();
        b.next(p).unwrap();
        b.set_repeat(p, Repeat::All).unwrap();
        b.set_shuffle(p, true).unwrap();
        let res = b.search("song").unwrap();
        assert_eq!(res.len(), 4);
        b.play_media(p, &res[0], Enqueue::Next).unwrap();
        // Refresh token sign-in, and a bad long-lived token.
        ha::HomeAssistant::connect(&req(Mode::HomeAssistant, AuthMethod::Password, "", "", Some("REFR"))).unwrap();
        assert!(ha::HomeAssistant::connect(&req(Mode::HomeAssistant, AuthMethod::Token, "", "WRONG", None)).is_err());
        ha::HomeAssistant::connect(&req(Mode::HomeAssistant, AuthMethod::Token, "", "LLAT", None)).unwrap();
    }
}
