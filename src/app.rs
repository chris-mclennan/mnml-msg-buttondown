//! App state — per-tab item lists + a selection cursor. Items are a
//! 2-variant enum (Email / Subscriber) since drafts/sent/scheduled
//! all share the Email shape.

use crate::buttondown::{self, Auth, Email, Subscriber};
use crate::config::{Config, Tab};
use anyhow::Result;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct TabSpec {
    pub kind: String,
}

impl TabSpec {
    pub fn resolve(t: &Tab) -> Result<Self> {
        match t.kind.as_str() {
            "drafts" | "sent" | "scheduled" | "subscribers" => Ok(Self {
                kind: t.kind.clone(),
            }),
            other => anyhow::bail!("tab `{}`: unknown kind {other:?}", t.name),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Email(Email),
    Subscriber(Subscriber),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Item::Email(e) => &e.id,
            Item::Subscriber(s) => &s.id,
        }
    }

    pub fn primary_label(&self) -> String {
        match self {
            Item::Email(e) => {
                if e.subject.is_empty() {
                    "(no subject)".to_string()
                } else {
                    e.subject.clone()
                }
            }
            Item::Subscriber(s) => {
                if s.email_address.is_empty() {
                    "(no email)".to_string()
                } else {
                    s.email_address.clone()
                }
            }
        }
    }

    /// `kind` controls which Email columns are shown (drafts → word
    /// count, sent → stats, scheduled → publish_date highlighted).
    pub fn secondary_label(&self, kind: &str) -> String {
        match (self, kind) {
            (Item::Email(e), "drafts") => {
                let ts = e
                    .creation_date
                    .as_deref()
                    .map(short_date)
                    .unwrap_or_else(|| "—".into());
                let wc = e
                    .word_count
                    .map(|n| format!("{n}w"))
                    .unwrap_or_else(|| "—w".into());
                format!("{ts} · {wc}")
            }
            (Item::Email(e), "sent") => {
                let ts = e
                    .publish_date
                    .as_deref()
                    .map(short_date)
                    .unwrap_or_else(|| "—".into());
                let typ = e.email_type.as_deref().unwrap_or("—");
                let stats = match &e.analytics {
                    Some(a) => {
                        let o = a.opens.map(|n| n.to_string()).unwrap_or_else(|| "—".into());
                        let c = a
                            .clicks
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "—".into());
                        format!(" · {o}o/{c}c")
                    }
                    None => String::new(),
                };
                format!("{ts} · {typ}{stats}")
            }
            (Item::Email(e), "scheduled") => {
                let ts = e
                    .publish_date
                    .as_deref()
                    .map(short_date)
                    .unwrap_or_else(|| "—".into());
                format!("⏰ {ts}")
            }
            (Item::Email(_), _) => String::new(),
            (Item::Subscriber(s), _) => {
                let typ = s.sub_type.as_deref().unwrap_or("—");
                let ts = s
                    .creation_date
                    .as_deref()
                    .map(short_date)
                    .unwrap_or_else(|| "—".into());
                let notes = s
                    .notes
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .map(|n| {
                        let n = n.lines().next().unwrap_or(n);
                        if n.chars().count() > 40 {
                            let mut s: String = n.chars().take(39).collect();
                            s.push('…');
                            format!(" · {s}")
                        } else {
                            format!(" · {n}")
                        }
                    })
                    .unwrap_or_default();
                format!("{typ} · {ts}{notes}")
            }
        }
    }
}

/// `2026-05-01T00:00:00Z` → `2026-05-01`. Best-effort.
fn short_date(ts: &str) -> String {
    if let Some((d, _)) = ts.split_once('T') {
        return d.to_string();
    }
    ts.to_string()
}

pub struct ItemsTab {
    pub items: Vec<Item>,
    pub selected: usize,
    pub last_loaded: Option<Instant>,
    pub last_error: Option<String>,
    pub loading: bool,
    /// Set when the API reports more results than we received (i.e.
    /// the user needs to know there's a page 2+ we haven't fetched).
    pub truncated: bool,
    /// Total count reported by the API (when known).
    pub total_count: Option<u32>,
}

impl ItemsTab {
    fn empty() -> Self {
        ItemsTab {
            items: Vec::new(),
            selected: 0,
            last_loaded: None,
            last_error: None,
            loading: false,
            truncated: false,
            total_count: None,
        }
    }
}

pub struct TabState {
    pub name: String,
    pub spec: TabSpec,
    pub data: ItemsTab,
}

/// A pending confirm prompt — `[y/n]` overlay on the status line.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Publish (schedule) a draft email — id + publish_date ISO.
    PublishDraft { id: String, publish_date: String },
    /// Delete a subscriber.
    Unsubscribe { id: String, email: String },
}

impl ConfirmAction {
    pub fn prompt(&self) -> String {
        match self {
            ConfirmAction::PublishDraft { publish_date, .. } => {
                format!("publish draft @ {publish_date}? [y/n]")
            }
            ConfirmAction::Unsubscribe { email, .. } => {
                format!("unsubscribe {email}? [y/n]")
            }
        }
    }
}

pub struct App {
    pub cfg: Config,
    pub auth: Auth,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub status: String,
    pub confirm: Option<ConfirmAction>,
}

impl App {
    pub fn new(cfg: Config, auth: Auth) -> Result<Self> {
        let mut tabs = Vec::with_capacity(cfg.tabs.len());
        for t in &cfg.tabs {
            let spec = TabSpec::resolve(t)?;
            tabs.push(TabState {
                name: t.name.clone(),
                data: ItemsTab::empty(),
                spec,
            });
        }
        let mut app = App {
            cfg,
            auth,
            tabs,
            active_tab: 0,
            status: String::new(),
            confirm: None,
        };
        app.refresh_active();
        Ok(app)
    }

    pub fn active(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }
    pub fn active_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    pub fn switch_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
            if self.tabs[idx].data.items.is_empty() && self.tabs[idx].data.last_error.is_none() {
                self.refresh_active();
            }
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let tab = self.active_mut();
        if tab.data.items.is_empty() {
            return;
        }
        let n = tab.data.items.len() as isize;
        let cur = tab.data.selected as isize;
        let next = (cur + delta).clamp(0, n - 1);
        tab.data.selected = next as usize;
    }

    pub fn refresh_active(&mut self) {
        let idx = self.active_tab;
        let spec = self.tabs[idx].spec.clone();
        let name = self.tabs[idx].name.clone();
        self.status = format!("loading {name}…");
        self.tabs[idx].data.loading = true;

        let result: Result<(Vec<Item>, Option<u32>)> = match spec.kind.as_str() {
            "drafts" => buttondown::list_emails(&self.auth, "draft").map(|(emails, count)| {
                let items = emails.into_iter().map(Item::Email).collect();
                (items, count)
            }),
            "sent" => buttondown::list_emails(&self.auth, "sent").map(|(emails, count)| {
                let items = emails.into_iter().map(Item::Email).collect();
                (items, count)
            }),
            "scheduled" => {
                buttondown::list_emails(&self.auth, "scheduled").map(|(emails, count)| {
                    let items = emails.into_iter().map(Item::Email).collect();
                    (items, count)
                })
            }
            "subscribers" => buttondown::list_subscribers(&self.auth).map(|(subs, count)| {
                let items = subs.into_iter().map(Item::Subscriber).collect();
                (items, count)
            }),
            _ => unreachable!("validated in TabSpec::resolve"),
        };

        let t = &mut self.tabs[idx];
        t.data.loading = false;
        match result {
            Ok((items, count)) => {
                let received = items.len();
                t.data.items = items;
                t.data.selected = t.data.selected.min(received.saturating_sub(1));
                t.data.last_loaded = Some(Instant::now());
                t.data.last_error = None;
                t.data.total_count = count;
                t.data.truncated = match count {
                    Some(c) => (c as usize) > received,
                    None => false,
                };
                let kind_label = match spec.kind.as_str() {
                    "drafts" => "drafts",
                    "sent" => "sent emails",
                    "scheduled" => "scheduled",
                    "subscribers" => "subscribers",
                    _ => "items",
                };
                let extra = match (count, t.data.truncated) {
                    (Some(c), true) => format!(" (of {c})"),
                    _ => String::new(),
                };
                self.status = format!("{name}: {received} {kind_label}{extra}");
            }
            Err(e) => {
                t.data.last_error = Some(e.to_string());
                self.status = format!("error: {e}");
            }
        }
    }

    /// Tick — runs each frame. All tabs honor the global
    /// `refresh_interval_secs` (no per-tab live-tail here; the
    /// newsletter platform is low-velocity by nature).
    pub fn tick(&mut self) -> bool {
        let idx = self.active_tab;
        let interval = self.cfg.refresh_interval_secs;
        if interval == 0 {
            return false;
        }
        let stale = match self.tabs[idx].data.last_loaded {
            Some(t) => t.elapsed().as_secs() >= interval,
            None => true,
        };
        if stale && !self.tabs[idx].data.loading {
            self.refresh_active();
            true
        } else {
            false
        }
    }

    pub fn focused_item(&self) -> Option<&Item> {
        let t = self.active();
        t.data.items.get(t.data.selected)
    }

    /// `o` / `Enter` — open the focused item in the Buttondown web UI.
    pub fn open_web(&mut self) {
        let url = match self.focused_item() {
            Some(Item::Email(e)) => buttondown::email_url(&self.auth, &e.id),
            Some(Item::Subscriber(s)) => buttondown::subscriber_url(&self.auth, &s.id),
            None => {
                self.status = "no item under cursor".into();
                return;
            }
        };
        match webbrowser::open(&url) {
            Ok(()) => self.status = format!("opened {url}"),
            Err(e) => self.status = format!("open failed: {e}"),
        }
    }

    /// `y` — yank the focused item's id.
    pub fn yank_id(&mut self) {
        let payload = match self.focused_item() {
            Some(item) => item.id().to_string(),
            None => {
                self.status = "no item under cursor".into();
                return;
            }
        };
        if payload.is_empty() {
            self.status = "nothing to copy".into();
            return;
        }
        let len = payload.chars().count();
        match crate::clipboard::copy(&payload) {
            Ok(()) => self.status = format!("copied id ({len} chars)"),
            Err(e) => self.status = format!("copy failed: {e}"),
        }
    }

    /// `p` — schedule a draft for publishing 5 minutes from now. Only
    /// valid on the `drafts` tab. Stages a `[y/n]` confirm.
    pub fn request_publish(&mut self) {
        if self.active().spec.kind != "drafts" {
            self.status = "p only works on the drafts tab".into();
            return;
        }
        let Some(Item::Email(e)) = self.focused_item() else {
            self.status = "no draft under cursor".into();
            return;
        };
        let publish_date = publish_date_5min_from_now();
        self.confirm = Some(ConfirmAction::PublishDraft {
            id: e.id.clone(),
            publish_date,
        });
        self.status = self
            .confirm
            .as_ref()
            .map(|c| c.prompt())
            .unwrap_or_default();
    }

    /// `X` — unsubscribe focused. Only valid on the `subscribers` tab.
    /// Stages a `[y/n]` confirm.
    pub fn request_unsubscribe(&mut self) {
        if self.active().spec.kind != "subscribers" {
            self.status = "X only works on the subscribers tab".into();
            return;
        }
        let Some(Item::Subscriber(s)) = self.focused_item() else {
            self.status = "no subscriber under cursor".into();
            return;
        };
        self.confirm = Some(ConfirmAction::Unsubscribe {
            id: s.id.clone(),
            email: s.email_address.clone(),
        });
        self.status = self
            .confirm
            .as_ref()
            .map(|c| c.prompt())
            .unwrap_or_default();
    }

    /// `y` (when a confirm is pending) — execute the staged action.
    pub fn confirm_yes(&mut self) {
        let Some(action) = self.confirm.take() else {
            return;
        };
        match action {
            ConfirmAction::PublishDraft { id, publish_date } => {
                match buttondown::schedule_draft(&self.auth, &id, &publish_date) {
                    Ok(()) => {
                        self.status = format!("scheduled {id} @ {publish_date}");
                        self.refresh_active();
                    }
                    Err(e) => self.status = format!("schedule failed: {e}"),
                }
            }
            ConfirmAction::Unsubscribe { id, email } => {
                match buttondown::unsubscribe(&self.auth, &id) {
                    Ok(()) => {
                        self.status = format!("unsubscribed {email}");
                        self.refresh_active();
                    }
                    Err(e) => self.status = format!("unsubscribe failed: {e}"),
                }
            }
        }
    }

    /// `n` / `Esc` (when a confirm is pending) — cancel.
    pub fn confirm_no(&mut self) {
        if self.confirm.take().is_some() {
            self.status = "cancelled".into();
        }
    }
}

/// Build a Buttondown-friendly ISO-8601 publish_date 5 minutes in
/// the future, in UTC.
fn publish_date_5min_from_now() -> String {
    let now = chrono::Utc::now() + chrono::Duration::minutes(5);
    // `2026-06-07T13:00:00Z` — second precision is plenty.
    now.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Tab;

    #[test]
    fn tab_spec_resolves_known_kinds() {
        for kind in ["drafts", "sent", "scheduled", "subscribers"] {
            let t = Tab {
                name: kind.into(),
                kind: kind.into(),
            };
            assert!(TabSpec::resolve(&t).is_ok());
        }
    }

    #[test]
    fn tab_spec_rejects_unknown_kind() {
        let t = Tab {
            name: "bad".into(),
            kind: "bogus".into(),
        };
        assert!(TabSpec::resolve(&t).is_err());
    }

    #[test]
    fn short_date_extracts_ymd() {
        assert_eq!(short_date("2026-05-01T00:00:00Z"), "2026-05-01");
        assert_eq!(short_date("bare-string"), "bare-string");
    }

    #[test]
    fn publish_date_helper_shape() {
        let s = publish_date_5min_from_now();
        // 2026-06-07T13:00:00Z — len 20, ends in Z.
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert!(s.contains('T'));
    }

    /// Confirm-state machine: requesting + accepting + cancelling.
    /// Doesn't actually hit the API (we never reach `confirm_yes`).
    #[test]
    fn publish_confirm_state_machine() {
        let auth = Auth {
            api_key: "k".into(),
        };
        // We can't easily construct an `App` without going through
        // refresh_active (which hits the network). Test the
        // ConfirmAction directly instead.
        let action = ConfirmAction::PublishDraft {
            id: "abc-1".into(),
            publish_date: "2026-06-07T13:00:00Z".into(),
        };
        assert!(action.prompt().contains("publish"));
        assert!(action.prompt().contains("2026-06-07T13:00:00Z"));
        let _ = auth; // silence unused
    }

    #[test]
    fn unsubscribe_confirm_state_machine() {
        let action = ConfirmAction::Unsubscribe {
            id: "sub-1".into(),
            email: "alice@example.com".into(),
        };
        assert!(action.prompt().contains("unsubscribe"));
        assert!(action.prompt().contains("alice@example.com"));
    }

    #[test]
    fn email_item_primary_label_falls_back_for_empty_subject() {
        let email = Email {
            id: "x".into(),
            subject: "".into(),
            body: "".into(),
            status: None,
            email_type: None,
            creation_date: None,
            publish_date: None,
            modification_date: None,
            word_count: None,
            analytics: None,
        };
        assert_eq!(Item::Email(email).primary_label(), "(no subject)");
    }

    #[test]
    fn subscriber_item_secondary_includes_type_and_notes() {
        let sub = Subscriber {
            id: "s".into(),
            email_address: "a@b.com".into(),
            sub_type: Some("premium".into()),
            creation_date: Some("2026-01-01T00:00:00Z".into()),
            notes: Some("VIP".into()),
            source: None,
            metadata: None,
        };
        let label = Item::Subscriber(sub).secondary_label("subscribers");
        assert!(label.contains("premium"));
        assert!(label.contains("2026-01-01"));
        assert!(label.contains("VIP"));
    }
}
