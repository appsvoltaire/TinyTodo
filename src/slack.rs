//! Slack: Web API calls, Socket Mode, and the long-running feed that turns
//! socket traffic into notifications (with catch-up and reconnects).

use crate::config::{State, Tokens};
use crate::policy::{Gate, Identity, Verdict};
use futures::channel::mpsc;
use futures::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as Ws;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Message {
    pub ts: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

/// Thin Slack Web API client. Cheap to clone.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    tokens: Arc<Tokens>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("slack::Client")
    }
}

impl Client {
    pub fn new(tokens: Tokens) -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("http client");
        Self { http, tokens: Arc::new(tokens) }
    }

    async fn call(&self, method: &str, token: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let resp = self
            .http
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(token)
            .form(params)
            .send()
            .await
            .map_err(|e| format!("{method}: {}", e.without_url()))?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(format!("{method}: rate limited"));
        }
        let body: Value = resp.json().await.map_err(|e| format!("{method}: {}", e.without_url()))?;
        if body["ok"].as_bool() == Some(true) {
            Ok(body)
        } else {
            Err(format!("{method}: {}", body["error"].as_str().unwrap_or("unknown error")))
        }
    }

    async fn bot(&self, method: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        self.call(method, &self.tokens.bot, params).await
    }

    /// Who we are, so our own posts never notify.
    pub async fn identity(&self, ignore_user_ids: Vec<String>) -> Result<Identity, String> {
        let r = self.bot("auth.test", &[]).await?;
        let field = |k: &str| r[k].as_str().unwrap_or_default().to_string();
        Ok(Identity { user_id: field("user_id"), bot_id: field("bot_id"), ignore_user_ids })
    }

    /// Accepts a channel ID or a channel name (with or without `#`).
    pub async fn find_channel(&self, name_or_id: &str) -> Result<String, String> {
        let name = name_or_id.trim_start_matches('#');
        if looks_like_channel_id(name) {
            return Ok(name.to_string());
        }
        let mut cursor = String::new();
        loop {
            let r = self
                .bot(
                    "conversations.list",
                    &[
                        // Private only: needs just `groups:read`. Public channels work by ID.
                        ("types", "private_channel"),
                        ("exclude_archived", "true"),
                        ("limit", "1000"),
                        ("cursor", &cursor),
                    ],
                )
                .await?;
            let found = r["channels"].as_array().into_iter().flatten().find(|c| c["name"] == name);
            if let Some(id) = found.and_then(|c| c["id"].as_str()) {
                return Ok(id.to_string());
            }
            cursor = r["response_metadata"]["next_cursor"].as_str().unwrap_or_default().to_string();
            if cursor.is_empty() {
                return Err(format!("channel #{name} not found (is the TinyTodo app invited to it?)"));
            }
        }
    }

    /// Top-level channel messages, oldest first. `oldest` is exclusive.
    pub async fn history(&self, channel: &str, oldest: Option<&str>, limit: u32) -> Result<Vec<Message>, String> {
        let limit = limit.to_string();
        let mut params = vec![("channel", channel), ("limit", limit.as_str())];
        if let Some(oldest) = oldest {
            params.push(("oldest", oldest));
        }
        let r = self.bot("conversations.history", &params).await?;
        let mut messages: Vec<Message> =
            serde_json::from_value(r["messages"].clone()).map_err(|e| format!("conversations.history: {e}"))?;
        messages.reverse();
        Ok(messages)
    }

    pub async fn post(&self, channel: &str, text: &str) -> Result<Message, String> {
        let r = self.bot("chat.postMessage", &[("channel", channel), ("text", text)]).await?;
        serde_json::from_value(r["message"].clone()).map_err(|e| format!("chat.postMessage: {e}"))
    }

    async fn socket_url(&self) -> Result<String, String> {
        let r = self.call("apps.connections.open", &self.tokens.app, &[]).await?;
        let url = r["url"].as_str().ok_or("apps.connections.open: no url")?.to_string();
        // Makes Slack rotate the connection every few seconds; handy for testing reconnects.
        Ok(if std::env::var_os("TINYTODO_DEBUG_RECONNECTS").is_some() { url + "&debug_reconnects=true" } else { url })
    }
}

fn looks_like_channel_id(s: &str) -> bool {
    s.len() >= 9
        && matches!(s.as_bytes()[0], b'C' | b'G')
        && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Converts Slack's message markup into plain display text.
pub fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let Some(len) = rest[start..].find('>') else { break };
        out.push_str(&rest[..start]);
        let inner = &rest[start + 1..start + len];
        let (target, label) = match inner.split_once('|') {
            Some((t, l)) => (t, Some(l)),
            None => (inner, None),
        };
        match (target.chars().next(), label) {
            (Some('@'), Some(l)) | (Some('#'), Some(l)) => {
                out.push(target.as_bytes()[0] as char);
                out.push_str(l.trim_start_matches('@'));
            }
            (Some('@' | '#'), None) => out.push_str(target),
            (Some('!'), None) => {
                out.push('@');
                out.push_str(&target[1..]);
            }
            (_, Some(l)) => out.push_str(l),
            (_, None) => out.push_str(target.strip_prefix("mailto:").unwrap_or(target)),
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// A Socket Mode frame, reduced to what we act on.
#[derive(Debug, PartialEq)]
enum Frame {
    Hello,
    Disconnect(String),
    /// Must be acknowledged; carries a message if it was one for our channel.
    Envelope {
        id: String,
        message: Option<Message>,
    },
    Other,
}

fn parse_frame(text: &str, channel: &str) -> Frame {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Frame::Other };
    match v["type"].as_str() {
        Some("hello") => Frame::Hello,
        Some("disconnect") => Frame::Disconnect(v["reason"].as_str().unwrap_or("unknown").to_string()),
        _ => match v["envelope_id"].as_str() {
            Some(id) => {
                let event = &v["payload"]["event"];
                let message = (event["type"] == "message" && event["channel"] == channel)
                    .then(|| serde_json::from_value(event.clone()).ok())
                    .flatten();
                Frame::Envelope { id: id.to_string(), message }
            }
            None => Frame::Other,
        },
    }
}

/// What the feed reports to the UI.
#[derive(Debug, Clone)]
pub enum Event {
    /// Identity and channel resolved; the session can be used to post and read history.
    Ready(Session),
    Connected,
    /// Connection lost or failed; the feed retries on its own.
    Disconnected(String),
    Notify(Message),
}

#[derive(Debug, Clone)]
pub struct Session {
    pub client: Client,
    pub channel: String,
    pub identity: Identity,
}

/// Everything the feed needs to run.
#[derive(Debug, Clone)]
pub struct Feed {
    pub tokens: Tokens,
    pub channel: String,
    pub ignore_user_ids: Vec<String>,
}

impl std::hash::Hash for Feed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.channel.hash(state);
        self.ignore_user_ids.hash(state);
    }
}

impl Feed {
    /// The feed as a stream of events. Runs until the stream is dropped.
    pub fn stream(&self) -> impl Stream<Item = Event> + use<> {
        let (tx, rx) = mpsc::channel(32);
        let runner = futures::stream::once(watch(self.clone(), tx)).filter_map(|()| async { None });
        futures::stream::select(rx, runner)
    }
}

const MAX_BACKOFF: Duration = Duration::from_secs(60);
const PING_EVERY: Duration = Duration::from_secs(30);
const DEAD_AFTER: Duration = Duration::from_secs(90);

async fn watch(feed: Feed, mut out: mpsc::Sender<Event>) {
    let client = Client::new(feed.tokens.clone());
    let mut feed_state: Option<(Session, Gate)> = None;
    let mut backoff = Duration::from_secs(1);
    loop {
        let result = match &mut feed_state {
            Some((session, gate)) => listen(session, gate, &mut out, &mut backoff).await,
            None => match resolve(&client, &feed).await {
                Ok(session) => {
                    let gate = Gate::new(State::load().cursor, session.identity.clone());
                    if out.send(Event::Ready(session.clone())).await.is_err() {
                        return;
                    }
                    feed_state = Some((session, gate));
                    continue;
                }
                Err(e) => Err(e),
            },
        };
        if out.is_closed() {
            return;
        }
        if let Err(reason) = result {
            let _ = out.send(Event::Disconnected(reason)).await;
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }
}

async fn resolve(client: &Client, feed: &Feed) -> Result<Session, String> {
    #[allow(unused_mut)]
    let mut identity = client.identity(feed.ignore_user_ids.clone()).await?;
    // Debug builds only: treat TinyTodo's own posts as the assistant's, so the
    // whole loop can be exercised with just the app's bot token.
    #[cfg(debug_assertions)]
    if std::env::var_os("TINYTODO_DEBUG_NOTIFY_OWN").is_some() {
        identity = Identity { ignore_user_ids: identity.ignore_user_ids, ..Default::default() };
    }
    let channel = client.find_channel(&feed.channel).await?;
    Ok(Session { client: client.clone(), channel, identity })
}

/// One Socket Mode connection. `Ok` means Slack asked us to reconnect.
async fn listen(
    session: &Session,
    gate: &mut Gate,
    out: &mut mpsc::Sender<Event>,
    backoff: &mut Duration,
) -> Result<(), String> {
    let url = session.client.socket_url().await?;
    let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.map_err(|e| format!("websocket: {e}"))?;
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            frame = ws.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(e)) => return Err(format!("websocket: {e}")),
                    None => return Err("connection closed".into()),
                };
                last_seen = Instant::now();
                let text = match frame {
                    Ws::Text(text) => text,
                    Ws::Close(_) => return Err("connection closed by Slack".into()),
                    _ => continue,
                };
                match parse_frame(&text, &session.channel) {
                    Frame::Hello => {
                        *backoff = Duration::from_secs(1);
                        out.send(Event::Connected).await.map_err(|e| e.to_string())?;
                        catch_up(session, gate, out).await?;
                    }
                    Frame::Disconnect(reason) => {
                        eprintln!("tinytodo: Slack asked to reconnect ({reason})");
                        return Ok(());
                    }
                    Frame::Envelope { id, message } => {
                        let ack = serde_json::json!({ "envelope_id": id }).to_string();
                        ws.send(Ws::text(ack)).await.map_err(|e| format!("websocket: {e}"))?;
                        if let Some(message) = message {
                            deliver(gate, out, message).await?;
                        }
                    }
                    Frame::Other => {}
                }
            }
            _ = ping.tick() => {
                if last_seen.elapsed() > DEAD_AFTER {
                    return Err("connection timed out".into());
                }
                ws.send(Ws::Ping(Default::default())).await.map_err(|e| format!("websocket: {e}"))?;
            }
        }
    }
}

/// After (re)connecting, deliver whatever arrived while we were away.
async fn catch_up(session: &Session, gate: &mut Gate, out: &mut mpsc::Sender<Event>) -> Result<(), String> {
    match gate.cursor().map(str::to_string) {
        Some(cursor) => {
            for message in session.client.history(&session.channel, Some(&cursor), 100).await? {
                deliver(gate, out, message).await?;
            }
        }
        // First run: start from now instead of replaying the whole channel.
        None => {
            if let Some(latest) = session.client.history(&session.channel, None, 1).await?.pop() {
                gate.start_at(&latest.ts);
                save_cursor(gate);
            }
        }
    }
    Ok(())
}

async fn deliver(gate: &mut Gate, out: &mut mpsc::Sender<Event>, message: Message) -> Result<(), String> {
    match gate.admit(&message) {
        Verdict::Duplicate => return Ok(()),
        Verdict::Seen => {}
        Verdict::Notify => out.send(Event::Notify(message)).await.map_err(|e| e.to_string())?,
    }
    save_cursor(gate);
    Ok(())
}

fn save_cursor(gate: &Gate) {
    let state = State { cursor: gate.cursor().map(str::to_string) };
    if let Err(e) = state.save() {
        eprintln!("tinytodo: could not save state: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_decodes_markup() {
        assert_eq!(plain_text("a &lt;b&gt; &amp; c"), "a <b> & c");
        assert_eq!(plain_text("see <https://x.io/a|the doc> now"), "see the doc now");
        assert_eq!(plain_text("<https://x.io>"), "https://x.io");
        assert_eq!(plain_text("<mailto:a@b.c|a@b.c>"), "a@b.c");
        assert_eq!(plain_text("hi <@U123|sam> in <#C1|general>"), "hi @sam in #general");
        assert_eq!(plain_text("hi <@U123>"), "hi @U123");
        assert_eq!(plain_text("<!here> ping"), "@here ping");
        assert_eq!(plain_text("unclosed < bracket"), "unclosed < bracket");
    }

    #[test]
    fn channel_ids_are_recognised() {
        assert!(looks_like_channel_id("C0123ABCDEF"));
        assert!(looks_like_channel_id("G01234ABCD"));
        assert!(!looks_like_channel_id("todos-private"));
        assert!(!looks_like_channel_id("Cool"));
    }

    #[test]
    fn parses_socket_frames() {
        assert_eq!(parse_frame(r#"{"type":"hello","num_connections":1}"#, "C1"), Frame::Hello);
        assert_eq!(
            parse_frame(r#"{"type":"disconnect","reason":"refresh_requested"}"#, "C1"),
            Frame::Disconnect("refresh_requested".into())
        );
        let event = r#"{"envelope_id":"e1","type":"events_api","payload":{"event":
            {"type":"message","channel":"C1","user":"U9","text":"hi","ts":"1.000001"}}}"#;
        let Frame::Envelope { id, message } = parse_frame(event, "C1") else { panic!() };
        assert_eq!(id, "e1");
        assert_eq!(message.unwrap().text, "hi");
        // Other channels and event types are acknowledged but carry nothing.
        assert_eq!(parse_frame(event, "C2"), Frame::Envelope { id: "e1".into(), message: None });
        let reaction = r#"{"envelope_id":"e2","payload":{"event":{"type":"reaction_added"}}}"#;
        assert_eq!(parse_frame(reaction, "C1"), Frame::Envelope { id: "e2".into(), message: None });
        assert_eq!(parse_frame("not json", "C1"), Frame::Other);
    }

    #[test]
    fn edited_message_events_parse_with_subtype() {
        let event = r#"{"envelope_id":"e3","payload":{"event":{"type":"message","subtype":"message_changed",
            "channel":"C1","hidden":true,"ts":"2.000000","message":{"text":"new"}}}}"#;
        let Frame::Envelope { message: Some(m), .. } = parse_frame(event, "C1") else { panic!() };
        assert_eq!(m.subtype.as_deref(), Some("message_changed"));
        assert!(m.hidden);
    }
}
