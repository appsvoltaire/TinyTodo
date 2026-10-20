//! Notification policy: which Slack messages pop up, and never twice.
//! Pure logic; persistence and I/O live elsewhere.

use crate::slack::Message;
use std::cmp::Ordering;

/// Compares Slack timestamps ("1792534645.644179") numerically.
pub fn ts_cmp(a: &str, b: &str) -> Ordering {
    fn key(ts: &str) -> (u64, u64) {
        let (secs, frac) = ts.split_once('.').unwrap_or((ts, "0"));
        (secs.parse().unwrap_or(0), frac.parse().unwrap_or(0))
    }
    key(a).cmp(&key(b))
}

/// Who "we" are in the channel: TinyTodo's bot user plus any ignored users.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub bot_id: String,
    pub ignore_user_ids: Vec<String>,
}

impl Identity {
    /// True for replies sent from TinyTodo (or by ignored users, e.g. you in Slack).
    pub fn is_mine(&self, m: &Message) -> bool {
        let user = m.user.as_deref();
        (user.is_some() && user == Some(self.user_id.as_str()))
            || (m.bot_id.is_some() && m.bot_id.as_deref() == Some(self.bot_id.as_str()))
            || user.is_some_and(|u| self.ignore_user_ids.iter().any(|i| i == u))
    }
}

/// Ordinary conversation messages, as opposed to joins, edits, deletions, etc.
pub fn is_conversational(m: &Message) -> bool {
    !m.hidden
        && matches!(m.subtype.as_deref(), None | Some("bot_message" | "thread_broadcast" | "me_message" | "file_share"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Already handled before (reconnect overlap, Slack retry, restart).
    Duplicate,
    /// New, but nothing to show (our own reply, a join, an edit...).
    Seen,
    /// New assistant message: show it.
    Notify,
}

/// Delivery gate with a monotonic cursor: the newest message timestamp handled.
#[derive(Debug, Clone)]
pub struct Gate {
    cursor: Option<String>,
    identity: Identity,
}

impl Gate {
    pub fn new(cursor: Option<String>, identity: Identity) -> Self {
        Self { cursor, identity }
    }

    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Starts from `ts` without notifying anything at or before it (first run).
    pub fn start_at(&mut self, ts: &str) {
        if self.cursor.is_none() {
            self.cursor = Some(ts.to_string());
        }
    }

    pub fn admit(&mut self, m: &Message) -> Verdict {
        if self.cursor.as_deref().is_some_and(|c| ts_cmp(&m.ts, c) != Ordering::Greater) {
            return Verdict::Duplicate;
        }
        self.cursor = Some(m.ts.clone());
        if is_conversational(m) && !self.identity.is_mine(m) && !m.text.trim().is_empty() {
            Verdict::Notify
        } else {
            Verdict::Seen
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(ts: &str, user: &str, text: &str) -> Message {
        Message {
            ts: ts.into(),
            user: Some(user.into()),
            bot_id: None,
            text: text.into(),
            subtype: None,
            hidden: false,
        }
    }

    fn gate(cursor: Option<&str>) -> Gate {
        let identity = Identity { user_id: "UBOT".into(), bot_id: "BBOT".into(), ignore_user_ids: vec!["UME".into()] };
        Gate::new(cursor.map(Into::into), identity)
    }

    #[test]
    fn timestamps_compare_numerically() {
        assert_eq!(ts_cmp("9.000001", "10.000000"), Ordering::Less);
        assert_eq!(ts_cmp("10.000002", "10.000001"), Ordering::Greater);
        assert_eq!(ts_cmp("10.5", "10.5"), Ordering::Equal);
    }

    #[test]
    fn assistant_messages_notify_once() {
        let mut g = gate(None);
        let m = msg("100.000001", "UASSIST", "Call the dentist");
        assert_eq!(g.admit(&m), Verdict::Notify);
        assert_eq!(g.admit(&m), Verdict::Duplicate);
        assert_eq!(g.cursor(), Some("100.000001"));
    }

    #[test]
    fn older_messages_are_duplicates_after_restart() {
        let mut g = gate(Some("200.000000"));
        assert_eq!(g.admit(&msg("150.000000", "UASSIST", "old")), Verdict::Duplicate);
        assert_eq!(g.admit(&msg("200.000000", "UASSIST", "same")), Verdict::Duplicate);
        assert_eq!(g.admit(&msg("200.000001", "UASSIST", "new")), Verdict::Notify);
    }

    #[test]
    fn own_and_ignored_messages_are_seen_not_notified() {
        let mut g = gate(None);
        assert_eq!(g.admit(&msg("1.0", "UBOT", "my reply")), Verdict::Seen);
        assert_eq!(g.admit(&msg("2.0", "UME", "typed in slack")), Verdict::Seen);
        let mut by_bot = msg("3.0", "UOTHER", "x");
        by_bot.user = None;
        by_bot.bot_id = Some("BBOT".into());
        assert_eq!(g.admit(&by_bot), Verdict::Seen);
        // ...but they still advance the cursor.
        assert_eq!(g.cursor(), Some("3.0"));
    }

    #[test]
    fn messages_without_user_or_bot_are_not_mine() {
        let identity = Identity::default();
        let mut m = msg("1.0", "", "hi");
        m.user = None;
        assert!(!identity.is_mine(&m));
    }

    #[test]
    fn system_messages_and_edits_do_not_notify() {
        let mut g = gate(None);
        let mut join = msg("1.0", "UASSIST", "joined");
        join.subtype = Some("channel_join".into());
        assert_eq!(g.admit(&join), Verdict::Seen);
        let mut edit = msg("2.0", "UASSIST", "");
        edit.subtype = Some("message_changed".into());
        edit.hidden = true;
        assert_eq!(g.admit(&edit), Verdict::Seen);
        assert_eq!(g.admit(&msg("3.0", "UASSIST", "   ")), Verdict::Seen);
        let mut bot = msg("4.0", "UASSIST", "from a bot");
        bot.subtype = Some("bot_message".into());
        assert_eq!(g.admit(&bot), Verdict::Notify);
    }

    #[test]
    fn start_at_only_applies_without_cursor() {
        let mut g = gate(None);
        g.start_at("50.0");
        g.start_at("10.0");
        assert_eq!(g.cursor(), Some("50.0"));
        assert_eq!(g.admit(&msg("50.0", "UASSIST", "old")), Verdict::Duplicate);
    }
}
