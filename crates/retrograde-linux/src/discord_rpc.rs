use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout};
use tracing::{debug, warn};

/// Default Discord Application ID registered for Retrograde.
pub const DEFAULT_DISCORD_CLIENT_ID: &str = "1573878488337879223";

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

const MAX_FRAME_LEN: usize = 64 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);
const PRESENCE_RETRY_THROTTLE: Duration = Duration::from_secs(3);
const SEND_COALESCE: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(800);
const DRIFT_THRESHOLD_S: f64 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscordRpcStatus {
    #[default]
    Disabled,
    WaitingForDiscord,
    Connected,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordPlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscordPresence {
    pub status: DiscordPlaybackStatus,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub position_s: f64,
    pub duration_s: f64,
}

enum RpcCommand {
    Update(DiscordPresence),
    SetClientId(String),
    Shutdown,
}

#[derive(Debug)]
enum FrameReadError {
    Closed,
    Io,
}

enum HandshakeOutcome {
    Connected(UnixStream),
    Rejected,
    Unavailable,
}

enum ConnectStep {
    Done,
    RestartWithNewId,
    Shutdown,
}

/// Resolves the Discord Application ID from `RETRO_DISCORD_CLIENT_ID`, configured setting, or default.
pub fn resolve_client_id(configured: &str) -> String {
    let env_val = std::env::var("RETRO_DISCORD_CLIENT_ID").ok();
    resolve_client_id_with_env(configured, env_val.as_deref())
}

fn resolve_client_id_with_env(configured: &str, env_val: Option<&str>) -> String {
    if let Some(env_id) = env_val {
        let trimmed = env_id.trim();
        if is_valid_client_id(trimmed) {
            return trimmed.to_owned();
        }
    }
    let trimmed = configured.trim();
    if is_valid_client_id(trimmed) {
        return trimmed.to_owned();
    }
    DEFAULT_DISCORD_CLIENT_ID.to_owned()
}

pub fn is_valid_client_id(id: &str) -> bool {
    (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Lists candidate Discord IPC Unix socket paths across standard, Flatpak, Vesktop, and Snap dirs.
pub fn candidate_socket_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    for var in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Some(val) = std::env::var_os(var)
            && !val.is_empty()
        {
            let p = PathBuf::from(val);
            if seen.insert(p.clone()) {
                dirs.push(p);
            }
        }
    }
    let tmp = PathBuf::from("/tmp");
    if seen.insert(tmp.clone()) {
        dirs.push(tmp);
    }

    let prefixes = [
        "",
        "app/com.discordapp.Discord",
        "app/com.discordapp.DiscordCanary",
        "app/com.discordapp.DiscordPTB",
        "app/dev.vencord.Vesktop",
        "snap.discord",
    ];
    let mut out = Vec::new();
    for dir in &dirs {
        for prefix in &prefixes {
            let base = if prefix.is_empty() {
                dir.clone()
            } else {
                dir.join(prefix)
            };
            for idx in 0..10 {
                out.push(base.join(format!("discord-ipc-{idx}")));
            }
        }
    }
    out
}

/// Trims, applies a fallback when empty, truncates safely by UTF-16 code units, and ensures >= 2 chars.
pub fn clamp_discord_str(input: &str, fallback: &str, max_utf16_units: usize) -> String {
    let trimmed = input.trim();
    let src = if trimmed.is_empty() { fallback } else { trimmed };
    let total_units: usize = src.chars().map(char::len_utf16).sum();
    let mut s = if total_units <= max_utf16_units {
        src.to_owned()
    } else if max_utf16_units == 0 {
        String::new()
    } else {
        let budget = max_utf16_units - 1;
        let mut out = String::new();
        let mut used = 0usize;
        for ch in src.chars() {
            let u = ch.len_utf16();
            if used + u > budget {
                break;
            }
            out.push(ch);
            used += u;
        }
        out.push('…');
        out
    };
    if s.chars().count() < 2 {
        // Discord rejects activity strings shorter than 2 characters.
        s.push('\u{200B}');
    }
    s
}

/// Advances a cached `Playing` presence's position by elapsed wall time so reconnects stay in sync.
pub fn adjusted_presence(
    presence: &DiscordPresence,
    received_at: Instant,
    now: Instant,
) -> DiscordPresence {
    let mut adjusted = presence.clone();
    if adjusted.status == DiscordPlaybackStatus::Playing && adjusted.position_s.is_finite() {
        let elapsed = now.saturating_duration_since(received_at).as_secs_f64();
        let pos = adjusted.position_s.max(0.0) + elapsed;
        adjusted.position_s = if adjusted.duration_s.is_finite() && adjusted.duration_s > 0.0 {
            pos.min(adjusted.duration_s)
        } else {
            pos
        };
    }
    adjusted
}

/// Builds a Discord `SET_ACTIVITY` payload for the given playback snapshot.
pub fn build_set_activity_payload(
    pid: u32,
    nonce: u64,
    presence: &DiscordPresence,
    now_unix_s: i64,
) -> Value {
    let activity = match presence.status {
        DiscordPlaybackStatus::Stopped => Value::Null,
        DiscordPlaybackStatus::Playing | DiscordPlaybackStatus::Paused => {
            let title = clamp_discord_str(&presence.title, "Unknown Track", 128);
            let album = clamp_discord_str(&presence.album, "Retrograde", 128);
            let state = match presence.status {
                DiscordPlaybackStatus::Playing => {
                    clamp_discord_str(&presence.artist, "Unknown Artist", 128)
                }
                DiscordPlaybackStatus::Paused => {
                    let artist = clamp_discord_str(&presence.artist, "Unknown Artist", 116);
                    clamp_discord_str(&format!("{artist} • Paused"), "Paused", 128)
                }
                DiscordPlaybackStatus::Stopped => unreachable!(),
            };
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_owned(), json!(2));
            obj.insert("details".to_owned(), json!(title));
            obj.insert("state".to_owned(), json!(state));
            obj.insert(
                "assets".to_owned(),
                json!({
                    "large_image": "retrograde",
                    "large_text": album,
                }),
            );
            if presence.status == DiscordPlaybackStatus::Playing
                && presence.duration_s.is_finite()
                && presence.duration_s > 0.0
            {
                let pos = if presence.position_s.is_finite() {
                    presence.position_s.clamp(0.0, presence.duration_s)
                } else {
                    0.0
                };
                let start = now_unix_s - pos.round() as i64;
                let end = (start + presence.duration_s.round() as i64).max(start + 1);
                obj.insert("timestamps".to_owned(), json!({ "start": start, "end": end }));
            }
            Value::Object(obj)
        }
    };

    json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": activity,
        },
        "nonce": nonce.to_string(),
    })
}

/// Returns `true` when `next` differs meaningfully from `last` (status, track, or >2.5 s seek).
pub fn should_send_presence(
    last: Option<&(DiscordPresence, Instant)>,
    next: &DiscordPresence,
    now: Instant,
) -> bool {
    let Some((prev, prev_at)) = last else {
        return true;
    };
    if prev.status != next.status
        || prev.title != next.title
        || prev.artist != next.artist
        || prev.album != next.album
        || (prev.duration_s.round() as i64) != (next.duration_s.round() as i64)
    {
        return true;
    }
    if next.status == DiscordPlaybackStatus::Playing {
        let elapsed = now.saturating_duration_since(*prev_at).as_secs_f64();
        let expected_pos = prev.position_s + elapsed;
        return (next.position_s - expected_pos).abs() > DRIFT_THRESHOLD_S;
    }
    false
}

pub struct DiscordRpcService {
    cmd_tx: mpsc::UnboundedSender<RpcCommand>,
    handle: Option<JoinHandle<()>>,
}

impl DiscordRpcService {
    pub fn start(client_id: String, status_tx: mpsc::UnboundedSender<DiscordRpcStatus>) -> Self {
        Self::start_with_sockets(client_id, status_tx, None)
    }

    fn start_with_sockets(
        client_id: String,
        status_tx: mpsc::UnboundedSender<DiscordRpcStatus>,
        override_sockets: Option<Vec<PathBuf>>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(run_rpc_worker(
            client_id,
            cmd_rx,
            status_tx,
            override_sockets,
        ));
        Self {
            cmd_tx,
            handle: Some(handle),
        }
    }

    pub fn update_presence(&self, presence: DiscordPresence) {
        let _ = self.cmd_tx.send(RpcCommand::Update(presence));
    }

    pub fn set_client_id(&self, client_id: String) {
        let _ = self.cmd_tx.send(RpcCommand::SetClientId(client_id));
    }

    pub async fn shutdown(mut self) {
        let _ = self.cmd_tx.send(RpcCommand::Shutdown);
        if let Some(mut handle) = self.handle.take()
            && timeout(SHUTDOWN_TIMEOUT, &mut handle).await.is_err()
        {
            handle.abort();
        }
    }
}

struct WorkerState<'a> {
    client_id: String,
    pid: u32,
    nonce: u64,
    latest_presence: Option<(DiscordPresence, Instant)>,
    last_sent: Option<(DiscordPresence, Instant)>,
    pending_flush: Option<Instant>,
    stream: Option<UnixStream>,
    read_buf: Vec<u8>,
    last_attempt: Option<Instant>,
    rejected: bool,
    current_status: Option<DiscordRpcStatus>,
    status_tx: mpsc::UnboundedSender<DiscordRpcStatus>,
    override_sockets: Option<Vec<PathBuf>>,
    cmd_rx: &'a mut mpsc::UnboundedReceiver<RpcCommand>,
}

impl<'a> WorkerState<'a> {
    fn emit_status(&mut self, next: DiscordRpcStatus) {
        if self.current_status != Some(next) {
            self.current_status = Some(next);
            let _ = self.status_tx.send(next);
        }
    }

    fn drop_connection(&mut self) {
        self.stream = None;
        self.read_buf.clear();
        self.last_sent = None;
        self.pending_flush = None;
        let fallback = if self.rejected {
            DiscordRpcStatus::Rejected
        } else {
            DiscordRpcStatus::WaitingForDiscord
        };
        self.emit_status(fallback);
    }

    async fn flush_latest_if_connected(&mut self) {
        let Some((ref presence, received_at)) = self.latest_presence else {
            return;
        };
        let adj = adjusted_presence(presence, received_at, Instant::now());
        if let Some(conn) = self.stream.as_mut()
            && !send_activity(conn, self.pid, &mut self.nonce, &adj, &mut self.last_sent).await
        {
            self.drop_connection();
        }
    }

    async fn connect_until_done(&mut self) -> bool {
        loop {
            match self.try_connect_once().await {
                ConnectStep::Done => {
                    self.flush_latest_if_connected().await;
                    return true;
                }
                ConnectStep::RestartWithNewId => continue,
                ConnectStep::Shutdown => return false,
            }
        }
    }

    async fn try_connect_once(&mut self) -> ConnectStep {
        self.last_attempt = Some(Instant::now());
        self.read_buf.clear();
        self.last_sent = None;
        self.pending_flush = None;

        let paths: Vec<PathBuf> = match &self.override_sockets {
            Some(list) => list.clone(),
            None => candidate_socket_paths(),
        };

        for path in paths {
            let client_id = self.client_id.clone();
            let outcome = {
                let mut fut = std::pin::pin!(connect_and_handshake(&path, &client_id));
                loop {
                    tokio::select! {
                        res = &mut fut => break res,
                        cmd = self.cmd_rx.recv() => match cmd {
                            Some(RpcCommand::Update(p)) => {
                                self.latest_presence = Some((p, Instant::now()));
                            }
                            Some(RpcCommand::SetClientId(next_id)) => {
                                if next_id != self.client_id {
                                    self.client_id = next_id;
                                    self.rejected = false;
                                    return ConnectStep::RestartWithNewId;
                                }
                            }
                            Some(RpcCommand::Shutdown) | None => return ConnectStep::Shutdown,
                        }
                    }
                }
            };
            match outcome {
                HandshakeOutcome::Connected(conn) => {
                    debug!(socket = %path.display(), "Connected to Discord IPC");
                    self.stream = Some(conn);
                    self.rejected = false;
                    self.emit_status(DiscordRpcStatus::Connected);
                    return ConnectStep::Done;
                }
                HandshakeOutcome::Rejected => {
                    self.rejected = true;
                    self.emit_status(DiscordRpcStatus::Rejected);
                    return ConnectStep::Done;
                }
                HandshakeOutcome::Unavailable => {}
            }
        }
        self.emit_status(DiscordRpcStatus::WaitingForDiscord);
        ConnectStep::Done
    }

    async fn handle_update_connected(&mut self, presence: DiscordPresence) {
        let now = Instant::now();
        let need_send = should_send_presence(self.last_sent.as_ref(), &presence, now);
        self.latest_presence = Some((presence.clone(), now));
        if !need_send {
            self.pending_flush = None;
            return;
        }
        let immediate = presence.status == DiscordPlaybackStatus::Stopped
            || self
                .last_sent
                .as_ref()
                .is_none_or(|(_, sent_at)| now.saturating_duration_since(*sent_at) >= SEND_COALESCE);
        if immediate {
            self.pending_flush = None;
            if let Some(conn) = self.stream.as_mut()
                && !send_activity(
                    conn,
                    self.pid,
                    &mut self.nonce,
                    &presence,
                    &mut self.last_sent,
                )
                .await
            {
                self.drop_connection();
            }
        } else if let Some((_, sent_at)) = &self.last_sent {
            self.pending_flush.get_or_insert(*sent_at + SEND_COALESCE);
        }
    }
}

async fn wait_flush(deadline: Option<Instant>) {
    match deadline {
        Some(at) => sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn run_rpc_worker(
    client_id: String,
    mut cmd_rx: mpsc::UnboundedReceiver<RpcCommand>,
    status_tx: mpsc::UnboundedSender<DiscordRpcStatus>,
    override_sockets: Option<Vec<PathBuf>>,
) {
    let mut state = WorkerState {
        client_id,
        pid: std::process::id(),
        nonce: 1,
        latest_presence: None,
        last_sent: None,
        pending_flush: None,
        stream: None,
        read_buf: Vec::with_capacity(4096),
        last_attempt: None,
        rejected: false,
        current_status: None,
        status_tx,
        override_sockets,
        cmd_rx: &mut cmd_rx,
    };

    let mut reconnect_tick = interval_at(Instant::now() + RECONNECT_INTERVAL, RECONNECT_INTERVAL);
    reconnect_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    if !state.connect_until_done().await {
        return;
    }

    loop {
        if state.stream.is_some() {
            let flush_at = state.pending_flush;
            let mut conn = state.stream.take().expect("checked is_some");
            let outcome = tokio::select! {
                cmd = state.cmd_rx.recv() => ConnectedSelect::Cmd(cmd),
                _ = wait_flush(flush_at), if flush_at.is_some() => ConnectedSelect::Flush,
                res = conn.read_buf(&mut state.read_buf) => ConnectedSelect::Read(res),
            };
            state.stream = Some(conn);

            match outcome {
                ConnectedSelect::Cmd(Some(RpcCommand::Update(presence))) => {
                    state.handle_update_connected(presence).await;
                }
                ConnectedSelect::Cmd(Some(RpcCommand::SetClientId(next_id))) => {
                    if next_id != state.client_id {
                        state.client_id = next_id;
                        state.rejected = false;
                        if let Some(mut old) = state.stream.take() {
                            clear_and_close(&mut old, state.pid, &mut state.nonce).await;
                        }
                        if !state.connect_until_done().await {
                            return;
                        }
                    }
                }
                ConnectedSelect::Cmd(Some(RpcCommand::Shutdown) | None) => {
                    if let Some(mut c) = state.stream.take() {
                        clear_and_close(&mut c, state.pid, &mut state.nonce).await;
                    }
                    return;
                }
                ConnectedSelect::Flush => {
                    state.pending_flush = None;
                    let now = Instant::now();
                    if let Some((ref presence, received_at)) = state.latest_presence {
                        let adj = adjusted_presence(presence, received_at, now);
                        if should_send_presence(state.last_sent.as_ref(), &adj, now)
                            && let Some(c) = state.stream.as_mut()
                            && !send_activity(c, state.pid, &mut state.nonce, &adj, &mut state.last_sent).await
                        {
                            state.drop_connection();
                        }
                    }
                }
                ConnectedSelect::Read(Ok(0) | Err(_)) => {
                    state.drop_connection();
                }
                ConnectedSelect::Read(Ok(_)) => {
                    let mut conn = state.stream.take().expect("just put back");
                    let ok = drain_incoming_frames(&mut conn, &mut state.read_buf).await;
                    state.stream = Some(conn);
                    if !ok {
                        state.drop_connection();
                    }
                }
            }
        } else {
            tokio::select! {
                cmd = state.cmd_rx.recv() => match cmd {
                    Some(RpcCommand::Update(presence)) => {
                        let is_active = presence.status != DiscordPlaybackStatus::Stopped;
                        state.latest_presence = Some((presence, Instant::now()));
                        if is_active
                            && !state.rejected
                            && state.last_attempt.is_none_or(|t| t.elapsed() >= PRESENCE_RETRY_THROTTLE)
                            && !state.connect_until_done().await
                        {
                            return;
                        }
                    }
                    Some(RpcCommand::SetClientId(next_id)) => {
                        if next_id != state.client_id || state.rejected {
                            state.client_id = next_id;
                            state.rejected = false;
                            if !state.connect_until_done().await {
                                return;
                            }
                        }
                    }
                    Some(RpcCommand::Shutdown) | None => return,
                },
                _ = reconnect_tick.tick() => {
                    if !state.rejected && !state.connect_until_done().await {
                        return;
                    }
                }
            }
        }
    }
}

enum ConnectedSelect {
    Cmd(Option<RpcCommand>),
    Flush,
    Read(std::io::Result<usize>),
}

fn is_client_id_rejection(code: Option<i64>, msg: &str) -> bool {
    if matches!(code, Some(4000..=4005)) {
        return true;
    }
    let lower = msg.to_ascii_lowercase();
    lower.contains("client") || lower.contains("invalid") || lower.contains("oauth")
}

async fn connect_and_handshake(path: &Path, client_id: &str) -> HandshakeOutcome {
    if !path.exists() {
        return HandshakeOutcome::Unavailable;
    }
    let mut conn = match UnixStream::connect(path).await {
        Ok(s) => s,
        Err(err) => {
            debug!(socket = %path.display(), error = %err, "Failed connecting Discord IPC socket");
            return HandshakeOutcome::Unavailable;
        }
    };
    let handshake = json!({
        "v": 1,
        "client_id": client_id,
    });
    if !write_json_frame(&mut conn, OP_HANDSHAKE, &handshake).await {
        return HandshakeOutcome::Unavailable;
    }
    let mut buf = Vec::with_capacity(1024);
    let reply = match timeout(HANDSHAKE_TIMEOUT, async {
        loop {
            match try_pop_frame(&mut buf) {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {}
                Err(()) => return Err(FrameReadError::Io),
            }
            match conn.read_buf(&mut buf).await {
                Ok(0) => return Err(FrameReadError::Closed),
                Ok(_) => {}
                Err(_) => return Err(FrameReadError::Io),
            }
        }
    })
    .await
    {
        Ok(Ok(frame)) => frame,
        Ok(Err(err)) => {
            debug!(?err, "Discord IPC closed during handshake");
            return HandshakeOutcome::Unavailable;
        }
        Err(_) => {
            debug!("Discord IPC handshake timed out");
            return HandshakeOutcome::Unavailable;
        }
    };

    let (op, payload) = reply;
    if op == OP_CLOSE {
        let val = serde_json::from_slice::<Value>(&payload).ok();
        let code = val
            .as_ref()
            .and_then(|v| v.get("code"))
            .and_then(Value::as_i64);
        let msg = val
            .as_ref()
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("closed without reason");
        warn!(code, message = msg, "Discord IPC closed handshake");
        return if is_client_id_rejection(code, msg) {
            HandshakeOutcome::Rejected
        } else {
            HandshakeOutcome::Unavailable
        };
    }
    if op != OP_FRAME {
        warn!(op, "Unexpected Discord IPC handshake opcode");
        return HandshakeOutcome::Unavailable;
    }
    let Ok(val) = serde_json::from_slice::<Value>(&payload) else {
        return HandshakeOutcome::Unavailable;
    };
    if val.get("evt").and_then(Value::as_str) == Some("ERROR") {
        let code = val
            .get("data")
            .and_then(|d| d.get("code"))
            .and_then(Value::as_i64);
        let msg = val
            .get("data")
            .and_then(|d| d.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("unknown Discord error");
        warn!(code, error = msg, "Discord IPC rejected handshake");
        return HandshakeOutcome::Rejected;
    }
    HandshakeOutcome::Connected(conn)
}

async fn send_activity(
    conn: &mut UnixStream,
    pid: u32,
    nonce: &mut u64,
    presence: &DiscordPresence,
    last_sent: &mut Option<(DiscordPresence, Instant)>,
) -> bool {
    let now_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let payload = build_set_activity_payload(pid, *nonce, presence, now_unix_s);
    *nonce = nonce.wrapping_add(1);
    if write_json_frame(conn, OP_FRAME, &payload).await {
        *last_sent = Some((presence.clone(), Instant::now()));
        true
    } else {
        false
    }
}

async fn clear_and_close(conn: &mut UnixStream, pid: u32, nonce: &mut u64) {
    let payload = json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": Value::Null },
        "nonce": nonce.to_string(),
    });
    *nonce = nonce.wrapping_add(1);
    let _ = write_json_frame(conn, OP_FRAME, &payload).await;
    let _ = write_raw_frame(conn, OP_CLOSE, b"{}").await;
}

async fn drain_incoming_frames(conn: &mut UnixStream, buf: &mut Vec<u8>) -> bool {
    loop {
        match try_pop_frame(buf) {
            Ok(Some((OP_PING, payload))) => {
                if !write_raw_frame(conn, OP_PONG, &payload).await {
                    return false;
                }
            }
            Ok(Some((OP_CLOSE, payload))) => {
                if let Ok(val) = serde_json::from_slice::<Value>(&payload) {
                    let code = val.get("code").and_then(Value::as_i64);
                    let msg = val.get("message").and_then(Value::as_str);
                    warn!(code, message = msg, "Discord IPC closed connection");
                }
                return false;
            }
            Ok(Some((OP_FRAME, payload))) => {
                if let Ok(val) = serde_json::from_slice::<Value>(&payload)
                    && val.get("evt").and_then(Value::as_str) == Some("ERROR")
                {
                    let msg = val
                        .get("data")
                        .and_then(|d| d.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    warn!(error = msg, "Discord IPC error frame");
                }
            }
            Ok(Some((OP_HANDSHAKE | OP_PONG | _, _))) => {}
            Ok(None) => return true,
            Err(()) => return false,
        }
    }
}

fn try_pop_frame(buf: &mut Vec<u8>) -> Result<Option<(u32, Vec<u8>)>, ()> {
    if buf.len() < 8 {
        return Ok(None);
    }
    let op = u32::from_le_bytes(buf[0..4].try_into().expect("4 bytes"));
    let len = u32::from_le_bytes(buf[4..8].try_into().expect("4 bytes")) as usize;
    if len > MAX_FRAME_LEN {
        return Err(());
    }
    let total = 8usize.checked_add(len).ok_or(())?;
    if buf.len() < total {
        return Ok(None);
    }
    let payload = buf[8..total].to_vec();
    buf.drain(..total);
    Ok(Some((op, payload)))
}

async fn write_json_frame(conn: &mut UnixStream, op: u32, payload: &Value) -> bool {
    let Ok(bytes) = serde_json::to_vec(payload) else {
        return false;
    };
    write_raw_frame(conn, op, &bytes).await
}

async fn write_raw_frame(conn: &mut UnixStream, op: u32, payload: &[u8]) -> bool {
    if payload.len() > MAX_FRAME_LEN {
        return false;
    }
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    timeout(WRITE_TIMEOUT, async {
        conn.write_all(&frame).await?;
        conn.flush().await
    })
    .await
    .is_ok_and(|r| r.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::net::UnixListener;
    use tokio::sync::oneshot;

    static TEST_SOCK_SEQ: AtomicU64 = AtomicU64::new(1);

    fn temp_sock_path() -> PathBuf {
        let seq = TEST_SOCK_SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("rg-drpc-{}-{seq}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn sample_presence(status: DiscordPlaybackStatus) -> DiscordPresence {
        DiscordPresence {
            status,
            title: "Control's Wishes".to_owned(),
            artist: "Robert del Naja".to_owned(),
            album: "Lone Trail OST".to_owned(),
            position_s: 42.2,
            duration_s: 215.4,
        }
    }

    #[test]
    fn payload_playing_paused_and_stopped() {
        let playing = build_set_activity_payload(
            99,
            1,
            &sample_presence(DiscordPlaybackStatus::Playing),
            1_700_000_100,
        );
        assert_eq!(playing["cmd"], "SET_ACTIVITY");
        assert_eq!(playing["args"]["pid"], 99);
        let act = &playing["args"]["activity"];
        assert_eq!(act["type"], 2);
        assert_eq!(act["details"], "Control's Wishes");
        assert_eq!(act["state"], "Robert del Naja");
        assert_eq!(act["timestamps"]["start"], 1_700_000_058);
        assert_eq!(act["timestamps"]["end"], 1_700_000_273);
        assert_eq!(act["assets"]["large_image"], "retrograde");
        assert_eq!(act["assets"]["large_text"], "Lone Trail OST");

        let paused = build_set_activity_payload(
            99,
            2,
            &sample_presence(DiscordPlaybackStatus::Paused),
            1_700_000_100,
        );
        let pact = &paused["args"]["activity"];
        assert_eq!(pact["type"], 2);
        assert_eq!(pact["state"], "Robert del Naja • Paused");
        assert!(pact.get("timestamps").is_none());

        let stopped = build_set_activity_payload(
            99,
            3,
            &sample_presence(DiscordPlaybackStatus::Stopped),
            1_700_000_100,
        );
        assert!(stopped["args"]["activity"].is_null());
    }

    #[test]
    fn clamp_handles_single_char_and_utf16_truncation() {
        let short = clamp_discord_str("A", "Fallback", 128);
        assert_eq!(short.chars().count(), 2);
        assert!(short.starts_with('A'));

        let blank = clamp_discord_str("   ", "Unknown Track", 128);
        assert_eq!(blank, "Unknown Track");

        // Each emoji uses 2 UTF-16 code units; 80 emojis = 160 units -> truncated to <= 128 units.
        let emojis = "🎵".repeat(80);
        let truncated = clamp_discord_str(&emojis, "Fallback", 128);
        assert!(truncated.encode_utf16().count() <= 128);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn dedup_skips_normal_drift_and_detects_seeks() {
        let base = Instant::now();
        let p0 = sample_presence(DiscordPlaybackStatus::Playing);
        assert!(should_send_presence(None, &p0, base));

        let last = (p0.clone(), base);
        let p_drift = DiscordPresence {
            position_s: p0.position_s + 2.2,
            ..p0.clone()
        };
        assert!(!should_send_presence(
            Some(&last),
            &p_drift,
            base + Duration::from_secs(2)
        ));

        let p_seek = DiscordPresence {
            position_s: p0.position_s + 10.0,
            ..p0.clone()
        };
        assert!(should_send_presence(
            Some(&last),
            &p_seek,
            base + Duration::from_secs(1)
        ));

        let p_pause = sample_presence(DiscordPlaybackStatus::Paused);
        assert!(should_send_presence(
            Some(&last),
            &p_pause,
            base + Duration::from_secs(1)
        ));
    }

    #[test]
    fn resolve_client_id_validates_snowflakes_without_host_env() {
        assert!(!is_valid_client_id(""));
        assert!(!is_valid_client_id("1234567890123456")); // 16 digits
        assert!(is_valid_client_id("12345678901234567")); // 17 digits
        assert!(is_valid_client_id("12345678901234567890")); // 20 digits
        assert!(!is_valid_client_id("123456789012345678901")); // 21 digits

        assert_eq!(resolve_client_id_with_env("", None), DEFAULT_DISCORD_CLIENT_ID);
        assert_eq!(
            resolve_client_id_with_env("  12345  ", None),
            DEFAULT_DISCORD_CLIENT_ID
        );
        assert_eq!(
            resolve_client_id_with_env("  987654321012345678  ", None),
            "987654321012345678"
        );
        assert_eq!(
            resolve_client_id_with_env("987654321012345678", Some("111111111111111111")),
            "111111111111111111"
        );
        assert_eq!(
            resolve_client_id_with_env("987654321012345678", Some("bad")),
            "987654321012345678"
        );
    }

    #[test]
    fn adjusted_presence_advances_playing_and_preserves_paused() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(60);
        let p = sample_presence(DiscordPlaybackStatus::Playing);
        let adv = adjusted_presence(&p, t0, t1);
        assert!((adv.position_s - 102.2).abs() < 0.01);

        let far = t0 + Duration::from_secs(500);
        let clamped = adjusted_presence(&p, t0, far);
        assert!((clamped.position_s - p.duration_s).abs() < 0.01);

        let paused = sample_presence(DiscordPlaybackStatus::Paused);
        assert_eq!(adjusted_presence(&paused, t0, t1).position_s, paused.position_s);
    }

    #[test]
    fn try_pop_frame_rejects_oversized_payloads() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&OP_FRAME.to_le_bytes());
        buf.extend_from_slice(&((MAX_FRAME_LEN as u32) + 1).to_le_bytes());
        assert!(try_pop_frame(&mut buf).is_err());
    }

    async fn read_mock_frame(sock: &mut UnixStream) -> Option<(u32, Vec<u8>)> {
        let mut hdr = [0u8; 8];
        if sock.read_exact(&mut hdr).await.is_err() {
            return None;
        }
        let op = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let mut body = vec![0u8; len];
        if sock.read_exact(&mut body).await.is_err() {
            return None;
        }
        Some((op, body))
    }

    #[tokio::test]
    async fn roundtrip_ipc_handshake_activity_ping_and_shutdown() {
        let sock_path = temp_sock_path();
        let listener = UnixListener::bind(&sock_path).unwrap();
        let (pong_tx, pong_rx) = oneshot::channel();

        let server = tokio::spawn(async move {
            let (mut srv, _) = listener.accept().await.unwrap();

            let (op, body) = read_mock_frame(&mut srv).await.unwrap();
            assert_eq!(op, OP_HANDSHAKE);
            let hs: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(hs["v"], 1);
            assert_eq!(hs["client_id"], DEFAULT_DISCORD_CLIENT_ID);

            let ready = json!({ "cmd": "DISPATCH", "evt": "READY", "data": {} });
            assert!(write_json_frame(&mut srv, OP_FRAME, &ready).await);

            let (op, body) = read_mock_frame(&mut srv).await.unwrap();
            assert_eq!(op, OP_FRAME);
            let act: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(act["cmd"], "SET_ACTIVITY");
            assert_eq!(act["args"]["activity"]["details"], "Control's Wishes");

            assert!(write_raw_frame(&mut srv, OP_PING, b"ping-1").await);
            let (op, body) = read_mock_frame(&mut srv).await.unwrap();
            assert_eq!(op, OP_PONG);
            assert_eq!(body, b"ping-1");
            let _ = pong_tx.send(());

            let (op, body) = read_mock_frame(&mut srv).await.unwrap();
            assert_eq!(op, OP_FRAME);
            let clear: Value = serde_json::from_slice(&body).unwrap();
            assert!(clear["args"]["activity"].is_null());

            let (op, _) = read_mock_frame(&mut srv).await.unwrap();
            assert_eq!(op, OP_CLOSE);
        });

        let (status_tx, mut status_rx) = mpsc::unbounded_channel();
        let rpc = DiscordRpcService::start_with_sockets(
            DEFAULT_DISCORD_CLIENT_ID.to_owned(),
            status_tx,
            Some(vec![sock_path.clone()]),
        );

        let st = timeout(Duration::from_secs(2), status_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st, DiscordRpcStatus::Connected);

        rpc.update_presence(sample_presence(DiscordPlaybackStatus::Playing));
        timeout(Duration::from_secs(2), pong_rx).await.unwrap().unwrap();
        rpc.shutdown().await;
        timeout(Duration::from_secs(2), server).await.unwrap().unwrap();

        let _ = std::fs::remove_file(&sock_path);
    }

    #[tokio::test]
    async fn handshake_close_4000_emits_rejected_and_recovers_on_new_id() {
        let sock_path = temp_sock_path();
        let listener = UnixListener::bind(&sock_path).unwrap();

        let server = tokio::spawn(async move {
            // First attempt: bad ID -> OP_CLOSE 4000
            let (mut srv1, _) = listener.accept().await.unwrap();
            let (op, _) = read_mock_frame(&mut srv1).await.unwrap();
            assert_eq!(op, OP_HANDSHAKE);
            let err_close = json!({ "code": 4000, "message": "Invalid Client ID" });
            assert!(write_json_frame(&mut srv1, OP_CLOSE, &err_close).await);
            drop(srv1);

            // Second attempt after SetClientId -> READY
            let (mut srv2, _) = listener.accept().await.unwrap();
            let (op, body) = read_mock_frame(&mut srv2).await.unwrap();
            assert_eq!(op, OP_HANDSHAKE);
            let hs: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(hs["client_id"], "12345678901234567");
            let ready = json!({ "cmd": "DISPATCH", "evt": "READY", "data": {} });
            assert!(write_json_frame(&mut srv2, OP_FRAME, &ready).await);
            // Expect shutdown frames cleanly
            let _ = read_mock_frame(&mut srv2).await;
        });

        let (status_tx, mut status_rx) = mpsc::unbounded_channel();
        let rpc = DiscordRpcService::start_with_sockets(
            "99999999999999999".to_owned(),
            status_tx,
            Some(vec![sock_path.clone()]),
        );

        let st = timeout(Duration::from_secs(2), status_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st, DiscordRpcStatus::Rejected);

        rpc.set_client_id("12345678901234567".to_owned());
        let st2 = timeout(Duration::from_secs(2), status_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st2, DiscordRpcStatus::Connected);

        rpc.shutdown().await;
        timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        let _ = std::fs::remove_file(&sock_path);
    }

    #[tokio::test]
    async fn handshake_error_frame_emits_rejected() {
        let sock_path = temp_sock_path();
        let listener = UnixListener::bind(&sock_path).unwrap();

        let server = tokio::spawn(async move {
            let (mut srv, _) = listener.accept().await.unwrap();
            let _ = read_mock_frame(&mut srv).await.unwrap();
            let err = json!({ "cmd": "DISPATCH", "evt": "ERROR", "data": { "code": 4002, "message": "Invalid Client ID" } });
            assert!(write_json_frame(&mut srv, OP_FRAME, &err).await);
        });

        let (status_tx, mut status_rx) = mpsc::unbounded_channel();
        let rpc = DiscordRpcService::start_with_sockets(
            DEFAULT_DISCORD_CLIENT_ID.to_owned(),
            status_tx,
            Some(vec![sock_path.clone()]),
        );

        let st = timeout(Duration::from_secs(2), status_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st, DiscordRpcStatus::Rejected);

        rpc.shutdown().await;
        timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        let _ = std::fs::remove_file(&sock_path);
    }

    #[tokio::test]
    async fn switch_client_id_advances_elapsed_position_for_cached_playing_track() {
        let sock_path = temp_sock_path();
        let listener = UnixListener::bind(&sock_path).unwrap();
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel::<i64>();

        let server = tokio::spawn(async move {
            let (mut srv1, _) = listener.accept().await.unwrap();
            let _ = read_mock_frame(&mut srv1).await.unwrap();
            assert!(
                write_json_frame(
                    &mut srv1,
                    OP_FRAME,
                    &json!({ "cmd": "DISPATCH", "evt": "READY", "data": {} })
                )
                .await
            );
            let (op, body) = read_mock_frame(&mut srv1).await.unwrap();
            assert_eq!(op, OP_FRAME);
            let act1: Value = serde_json::from_slice(&body).unwrap();
            let start1 = act1["args"]["activity"]["timestamps"]["start"].as_i64().unwrap();
            let _ = first_tx.send(start1);
            // Expect clear + close when client ID changes
            let _ = read_mock_frame(&mut srv1).await;
            let _ = read_mock_frame(&mut srv1).await;
            drop(srv1);

            let (mut srv2, _) = listener.accept().await.unwrap();
            let _ = read_mock_frame(&mut srv2).await.unwrap();
            assert!(
                write_json_frame(
                    &mut srv2,
                    OP_FRAME,
                    &json!({ "cmd": "DISPATCH", "evt": "READY", "data": {} })
                )
                .await
            );
            let (op, body) = read_mock_frame(&mut srv2).await.unwrap();
            assert_eq!(op, OP_FRAME);
            let act2: Value = serde_json::from_slice(&body).unwrap();
            let start2 = act2["args"]["activity"]["timestamps"]["start"].as_i64().unwrap();
            let _ = second_tx.send(start2);
            let _ = read_mock_frame(&mut srv2).await;
        });

        let (status_tx, mut status_rx) = mpsc::unbounded_channel();
        let rpc = DiscordRpcService::start_with_sockets(
            DEFAULT_DISCORD_CLIENT_ID.to_owned(),
            status_tx,
            Some(vec![sock_path.clone()]),
        );

        assert_eq!(
            timeout(Duration::from_secs(2), status_rx.recv()).await.unwrap().unwrap(),
            DiscordRpcStatus::Connected
        );

        rpc.update_presence(DiscordPresence {
            position_s: 10.0,
            duration_s: 200.0,
            ..sample_presence(DiscordPlaybackStatus::Playing)
        });
        let start1 = timeout(Duration::from_secs(2), first_rx).await.unwrap().unwrap();

        tokio::time::sleep(Duration::from_millis(1200)).await;
        rpc.set_client_id("12345678901234567".to_owned());

        let start2 = timeout(Duration::from_secs(2), second_rx).await.unwrap().unwrap();
        // Since both wall-clock time AND adjusted position advanced by ~1.2s, `start = now - pos` stays anchored (~within 1s rounding)!
        assert!((start2 - start1).abs() <= 1);

        rpc.shutdown().await;
        timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        let _ = std::fs::remove_file(&sock_path);
    }
}
