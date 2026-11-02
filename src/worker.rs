//! Background thread that owns the server connection, so the window never blocks.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::ha::HomeAssistant;
use crate::ma::MusicAssistant;
use crate::model::*;

pub enum Cmd {
    Connect(ConnectRequest),
    Disconnect,
    PlayPause(String),
    Next(String),
    Previous(String),
    Volume(String, u8),
    Mute(String, bool),
    Repeat(String, Repeat),
    Shuffle(String, bool),
    Search(String),
    Play(String, SearchItem, Enqueue),
}

pub enum Event {
    Connected { label: String, new_saved_token: Option<String> },
    Disconnected(Option<String>),
    Reconnecting(String),
    Players(Vec<PlayerInfo>),
    SearchResults(String, Vec<SearchItem>),
    Error(String),
}

const POLL: Duration = Duration::from_millis(1500);

pub fn run(rx: Receiver<Cmd>, tx: Sender<Event>, ctx: eframe::egui::Context) {
    let send = |e: Event| {
        let _ = tx.send(e);
        ctx.request_repaint();
    };
    let mut backend: Option<Box<dyn Backend>> = None;
    let mut last_req: Option<ConnectRequest> = None;
    let mut players: Vec<PlayerInfo> = Vec::new();
    let mut next_retry = Instant::now();

    let connect = |req: &ConnectRequest| -> Result<(Box<dyn Backend>, String, Option<String>)> {
        match req.mode {
            Mode::MusicAssistant => {
                let (b, t) = MusicAssistant::connect(req)?;
                let label = b.server_label.clone();
                Ok((Box::new(b), label, t))
            }
            Mode::HomeAssistant => {
                let (b, t) = HomeAssistant::connect(req)?;
                let label = b.server_label.clone();
                Ok((Box::new(b), label, t))
            }
        }
    };

    loop {
        let cmd = match rx.recv_timeout(POLL) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        match cmd {
            Some(Cmd::Connect(req)) => {
                backend = None;
                match connect(&req) {
                    Ok((b, label, tok)) => {
                        backend = Some(b);
                        send(Event::Connected { label, new_saved_token: tok.clone() });
                        let mut r = req.clone();
                        if tok.is_some() {
                            r.saved_token = tok;
                        }
                        last_req = Some(r);
                    }
                    Err(e) => {
                        last_req = None;
                        send(Event::Disconnected(Some(e)));
                        continue;
                    }
                }
            }
            Some(Cmd::Disconnect) => {
                backend = None;
                last_req = None;
                players.clear();
                send(Event::Disconnected(None));
                continue;
            }
            Some(other) => {
                if let Some(b) = backend.as_mut() {
                    if let Err(e) = handle(b.as_mut(), &players, other, &send) {
                        if e.starts_with("Connection lost") || e.contains("closed the connection") {
                            backend = None;
                            send(Event::Reconnecting(e));
                        } else {
                            send(Event::Error(e));
                        }
                    }
                }
            }
            None => {}
        }

        // Lost connection: retry every few seconds with the last good details.
        if backend.is_none() {
            if let Some(req) = &last_req {
                if Instant::now() >= next_retry {
                    match connect(req) {
                        Ok((b, label, _)) => {
                            backend = Some(b);
                            send(Event::Connected { label, new_saved_token: None });
                        }
                        Err(e) => {
                            next_retry = Instant::now() + Duration::from_secs(5);
                            send(Event::Reconnecting(e));
                        }
                    }
                }
            }
        }

        if let Some(b) = backend.as_mut() {
            match b.players() {
                Ok(p) => {
                    players = p.clone();
                    send(Event::Players(p));
                }
                Err(e) => {
                    backend = None;
                    send(Event::Reconnecting(e));
                }
            }
        }
    }
}

fn handle(b: &mut dyn Backend, players: &[PlayerInfo], cmd: Cmd, send: &dyn Fn(Event)) -> Result<()> {
    let find = |id: &str| -> Result<PlayerInfo> {
        players
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| "That player is no longer available".to_string())
    };
    match cmd {
        Cmd::PlayPause(id) => b.play_pause(&find(&id)?),
        Cmd::Next(id) => b.next(&find(&id)?),
        Cmd::Previous(id) => b.previous(&find(&id)?),
        Cmd::Volume(id, v) => b.set_volume(&find(&id)?, v),
        Cmd::Mute(id, m) => b.set_mute(&find(&id)?, m),
        Cmd::Repeat(id, r) => b.set_repeat(&find(&id)?, r),
        Cmd::Shuffle(id, s) => b.set_shuffle(&find(&id)?, s),
        Cmd::Search(q) => {
            let res = b.search(&q)?;
            send(Event::SearchResults(q, res));
            Ok(())
        }
        Cmd::Play(id, item, enq) => b.play_media(&find(&id)?, &item, enq),
        Cmd::Connect(_) | Cmd::Disconnect => Ok(()),
    }
}
