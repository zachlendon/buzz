use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::time::{Duration, Instant};

use buzz_core::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_MANAGED_AGENT, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_DIFF, KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_MESSAGE_V2, KIND_TEXT_NOTE,
    KIND_WORKFLOW_APPROVAL_DENIED, KIND_WORKFLOW_APPROVAL_GRANTED,
    KIND_WORKFLOW_APPROVAL_REQUESTED,
};
use chrono::{Local, TimeZone};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::stream::{self, StreamExt};
use nostr::PublicKey;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::client::BuzzClient;
use crate::commands::messages::{send_message_literal, SendMessageParams};
use crate::error::CliError;

const CHANNEL_LIMIT: u32 = 500;
const DEFAULT_MESSAGE_LIMIT: u32 = 80;
const INPUT_POLL_MS: u64 = 50;
const ACTIVITY_CONTEXT_CONCURRENCY: usize = 8;
const MESSAGE_KINDS: &[u32] = &[
    KIND_TEXT_NOTE,
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];
const THREAD_KINDS: &[u32] = &[
    KIND_TEXT_NOTE,
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_EDIT,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_FORUM_COMMENT,
];
const INBOX_MENTION_KINDS: &[u32] = &[
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_TEXT_NOTE,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];
const INBOX_NEEDS_ACTION_KINDS: &[u32] = &[
    KIND_WORKFLOW_APPROVAL_REQUESTED,
    KIND_WORKFLOW_APPROVAL_GRANTED,
    KIND_WORKFLOW_APPROVAL_DENIED,
];
const INBOX_CATEGORIES: &[InboxCategory] = &[
    InboxCategory::NeedsAction,
    InboxCategory::Mention,
    InboxCategory::Activity,
];

pub async fn dispatch(cmd: crate::TuiCmd, client: &BuzzClient) -> Result<(), CliError> {
    let mut app = TuiApp::new(
        cmd.limit.unwrap_or(DEFAULT_MESSAGE_LIMIT).clamp(1, 200),
        cmd.poll_seconds,
        cmd.channel,
        cmd.inbox,
    );
    app.load_initial(client).await?;

    let mut session = TerminalSession::enter()?;
    let mut needs_draw = true;

    loop {
        if app.should_poll() {
            app.status = "Refreshing...".to_string();
            app.mark_refreshed();
            session.draw(&app)?;
            if let Err(err) = app.refresh_current(client).await {
                app.status = format!("Refresh failed: {err}");
            }
            needs_draw = true;
        }

        if needs_draw {
            session.draw(&app)?;
            needs_draw = false;
        }

        if !event::poll(Duration::from_millis(INPUT_POLL_MS)).map_err(terminal_err)? {
            continue;
        }

        let Event::Key(key) = event::read().map_err(terminal_err)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let action = app.handle_key(key);
        needs_draw = true;
        match action {
            TuiAction::None => {}
            TuiAction::Quit => break,
            TuiAction::Refresh => {
                app.status = "Refreshing...".to_string();
                app.mark_refreshed();
                session.draw(&app)?;
                if let Err(err) = app.refresh_current(client).await {
                    app.status = format!("Refresh failed: {err}");
                }
                needs_draw = true;
            }
            TuiAction::OpenSelection => {
                app.status = "Loading selection...".to_string();
                session.draw(&app)?;
                if let Err(err) = app.open_sidebar_selection(client).await {
                    app.status = format!("Load failed: {err}");
                }
                needs_draw = true;
            }
            TuiAction::OpenThread => {
                app.status = "Loading thread...".to_string();
                session.draw(&app)?;
                if let Err(err) = app.open_selected_thread(client).await {
                    app.status = format!("Thread load failed: {err}");
                }
                needs_draw = true;
            }
            TuiAction::BackToChannel => {
                app.status = "Loading channel...".to_string();
                session.draw(&app)?;
                if let Err(err) = app.back_to_channel(client).await {
                    app.status = format!("Channel load failed: {err}");
                }
                needs_draw = true;
            }
            TuiAction::SwitchSidebar(mode) => {
                app.status = "Loading sidebar...".to_string();
                session.draw(&app)?;
                if let Err(err) = app.switch_sidebar(client, mode).await {
                    app.status = format!("Sidebar load failed: {err}");
                }
                needs_draw = true;
            }
            TuiAction::SendDraft => {
                app.status = "Sending...".to_string();
                session.draw(&app)?;
                app.send_draft(client).await;
                needs_draw = true;
            }
        }
    }

    Ok(())
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    fn enter() -> Result<Self, CliError> {
        enable_raw_mode().map_err(terminal_err)?;
        let mut stdout = io::stdout();
        if let Err(err) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(terminal_err(err));
        }

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(terminal_err)?;
        terminal.clear().map_err(terminal_err)?;
        terminal.hide_cursor().map_err(terminal_err)?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, app: &TuiApp) -> Result<(), CliError> {
        self.terminal
            .draw(|frame| draw_tui(frame, app))
            .map(|_| ())
            .map_err(terminal_err)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarMode {
    Channels,
    Inbox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Sidebar,
    Messages,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MessageTarget {
    None,
    Channel {
        channel_id: String,
        channel_name: String,
    },
    Thread {
        channel_id: String,
        channel_name: String,
        root_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ComposeTarget {
    New,
    Reply { event_id: String, event_kind: u32 },
}

enum TuiAction {
    None,
    Quit,
    Refresh,
    OpenSelection,
    OpenThread,
    BackToChannel,
    SwitchSidebar(SidebarMode),
    SendDraft,
}

struct TuiApp {
    channels: Vec<ChannelView>,
    inbox: Vec<InboxRow>,
    messages: Vec<EventView>,
    profiles: HashMap<String, String>,
    channel_names: HashMap<String, String>,
    agent_pubkeys: BTreeSet<String>,
    expanded_agent_events: BTreeSet<String>,
    sidebar_mode: SidebarMode,
    focus: Focus,
    target: MessageTarget,
    selected_channel: usize,
    selected_inbox: usize,
    selected_message: usize,
    draft: String,
    compose_target: Option<ComposeTarget>,
    status: String,
    last_refresh: Option<Instant>,
    last_refresh_label: String,
    limit: u32,
    poll_seconds: u64,
    initial_channel: Option<String>,
    start_inbox: bool,
}

impl TuiApp {
    fn new(
        limit: u32,
        poll_seconds: u64,
        initial_channel: Option<String>,
        start_inbox: bool,
    ) -> Self {
        Self {
            channels: Vec::new(),
            inbox: Vec::new(),
            messages: Vec::new(),
            profiles: HashMap::new(),
            channel_names: HashMap::new(),
            agent_pubkeys: BTreeSet::new(),
            expanded_agent_events: BTreeSet::new(),
            sidebar_mode: if start_inbox {
                SidebarMode::Inbox
            } else {
                SidebarMode::Channels
            },
            focus: Focus::Messages,
            target: MessageTarget::None,
            selected_channel: 0,
            selected_inbox: 0,
            selected_message: 0,
            draft: String::new(),
            compose_target: None,
            status: "Loading...".to_string(),
            last_refresh: None,
            last_refresh_label: String::new(),
            limit,
            poll_seconds,
            initial_channel,
            start_inbox,
        }
    }

    async fn load_initial(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        let snapshot = fetch_channels(client).await?;
        self.apply_channels(snapshot);

        if self.start_inbox {
            self.switch_sidebar(client, SidebarMode::Inbox).await?;
        } else {
            self.select_initial_channel();
            self.open_sidebar_selection(client).await?;
        }

        self.mark_refreshed();
        Ok(())
    }

    fn should_poll(&self) -> bool {
        self.poll_seconds > 0
            && self
                .last_refresh
                .map(|last| last.elapsed() >= Duration::from_secs(self.poll_seconds))
                .unwrap_or(false)
    }

    fn mark_refreshed(&mut self) {
        self.last_refresh = Some(Instant::now());
        self.last_refresh_label = Local::now().format("%H:%M:%S").to_string();
    }

    async fn refresh_current(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        match self.sidebar_mode {
            SidebarMode::Channels => {
                let selected_id = self
                    .selected_channel()
                    .map(|channel| channel.id.clone())
                    .or_else(|| self.current_channel_id().map(str::to_string));
                self.apply_channels(fetch_channels(client).await?);
                if let Some(id) = selected_id {
                    if let Some(index) = self.channels.iter().position(|channel| channel.id == id) {
                        self.selected_channel = index;
                    }
                }
            }
            SidebarMode::Inbox => {
                self.apply_inbox(fetch_inbox(client, self.limit).await?);
            }
        }

        match self.target.clone() {
            MessageTarget::None => self.open_sidebar_selection(client).await?,
            MessageTarget::Channel {
                channel_id,
                channel_name,
            } => self.load_channel(client, channel_id, channel_name).await?,
            MessageTarget::Thread {
                channel_id,
                channel_name,
                root_id,
            } => {
                self.load_thread(client, channel_id, channel_name, root_id)
                    .await?;
            }
        }

        self.mark_refreshed();
        Ok(())
    }

    async fn switch_sidebar(
        &mut self,
        client: &BuzzClient,
        mode: SidebarMode,
    ) -> Result<(), CliError> {
        self.sidebar_mode = mode;
        self.focus = Focus::Sidebar;
        match mode {
            SidebarMode::Channels => {
                if self.channels.is_empty() {
                    self.apply_channels(fetch_channels(client).await?);
                }
            }
            SidebarMode::Inbox => {
                if self.inbox.is_empty() {
                    self.apply_inbox(fetch_inbox(client, self.limit).await?);
                }
            }
        }
        self.open_sidebar_selection(client).await
    }

    async fn open_sidebar_selection(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        match self.sidebar_mode {
            SidebarMode::Channels => {
                let Some(channel) = self.selected_channel().cloned() else {
                    self.messages.clear();
                    self.target = MessageTarget::None;
                    self.status = "No channels found.".to_string();
                    return Ok(());
                };
                self.load_channel(client, channel.id, channel.name).await
            }
            SidebarMode::Inbox => {
                let Some(row) = self.selected_inbox_row().cloned() else {
                    self.messages.clear();
                    self.target = MessageTarget::None;
                    self.status = "No inbox items found.".to_string();
                    return Ok(());
                };
                let Some(channel_id) = row.event.channel_id.clone() else {
                    self.messages = vec![row.event.clone()];
                    self.target = MessageTarget::None;
                    self.status = "Inbox item has no channel context.".to_string();
                    return Ok(());
                };
                let channel_name = self.channel_label(&channel_id);
                let root_id = thread_key(&row.event);
                self.load_thread(client, channel_id, channel_name, root_id)
                    .await
            }
        }
    }

    async fn load_channel(
        &mut self,
        client: &BuzzClient,
        channel_id: String,
        channel_name: String,
    ) -> Result<(), CliError> {
        let snapshot = fetch_channel_messages(client, &channel_id, self.limit).await?;
        self.apply_message_snapshot(snapshot);
        self.target = MessageTarget::Channel {
            channel_id,
            channel_name,
        };
        self.selected_message = self.messages.len().saturating_sub(1);
        self.focus = Focus::Messages;
        self.status = format!(
            "Loaded {} message{}. n new, r reply, Enter thread.",
            self.messages.len(),
            if self.messages.len() == 1 { "" } else { "s" }
        );
        Ok(())
    }

    async fn load_thread(
        &mut self,
        client: &BuzzClient,
        channel_id: String,
        channel_name: String,
        root_id: String,
    ) -> Result<(), CliError> {
        let snapshot = fetch_thread_messages(client, &channel_id, &root_id, self.limit).await?;
        self.apply_message_snapshot(snapshot);
        self.target = MessageTarget::Thread {
            channel_id,
            channel_name,
            root_id,
        };
        self.selected_message = self.messages.len().saturating_sub(1);
        self.focus = Focus::Messages;
        self.status = format!(
            "Loaded thread with {} event{}. Esc returns to channel.",
            self.messages.len(),
            if self.messages.len() == 1 { "" } else { "s" }
        );
        Ok(())
    }

    async fn open_selected_thread(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        let Some(event) = self.selected_event().cloned() else {
            self.status = "No message selected.".to_string();
            return Ok(());
        };
        let Some(channel_id) = event
            .channel_id
            .clone()
            .or_else(|| self.current_channel_id().map(str::to_string))
        else {
            self.status = "Selected message has no channel context.".to_string();
            return Ok(());
        };
        let channel_name = self.channel_label(&channel_id);
        let root_id = thread_key(&event);
        self.load_thread(client, channel_id, channel_name, root_id)
            .await
    }

    async fn back_to_channel(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        let MessageTarget::Thread {
            channel_id,
            channel_name,
            ..
        } = self.target.clone()
        else {
            self.focus = Focus::Sidebar;
            self.status = "Sidebar focused.".to_string();
            return Ok(());
        };
        self.load_channel(client, channel_id, channel_name).await
    }

    fn apply_channels(&mut self, channels: ChannelSnapshot) {
        self.channel_names
            .extend(channels.channels.iter().map(|channel| {
                (
                    channel.id.clone(),
                    if channel.name.is_empty() {
                        short_hex(&channel.id)
                    } else {
                        channel.name.clone()
                    },
                )
            }));
        self.channels = channels.channels;
        self.selected_channel = self
            .selected_channel
            .min(self.channels.len().saturating_sub(1));
    }

    fn apply_inbox(&mut self, snapshot: InboxSnapshot) {
        self.profiles.extend(snapshot.profiles);
        self.channel_names.extend(snapshot.channel_names);
        self.agent_pubkeys.extend(snapshot.agent_pubkeys);
        self.inbox = snapshot.rows;
        self.selected_inbox = self.selected_inbox.min(self.inbox.len().saturating_sub(1));
    }

    fn apply_message_snapshot(&mut self, snapshot: MessageSnapshot) {
        self.profiles.extend(snapshot.profiles);
        self.channel_names.extend(snapshot.channel_names);
        self.agent_pubkeys.extend(snapshot.agent_pubkeys);
        self.messages = snapshot.events;
        self.selected_message = self
            .selected_message
            .min(self.messages.len().saturating_sub(1));
    }

    fn select_initial_channel(&mut self) {
        let Some(raw) = self.initial_channel.as_deref() else {
            return;
        };
        let needle = raw.to_ascii_lowercase();
        if let Some(index) = self.channels.iter().position(|channel| {
            channel.id.eq_ignore_ascii_case(raw) || channel.name.to_ascii_lowercase() == needle
        }) {
            self.selected_channel = index;
        }
    }

    fn selected_channel(&self) -> Option<&ChannelView> {
        self.channels.get(self.selected_channel)
    }

    fn selected_inbox_row(&self) -> Option<&InboxRow> {
        self.inbox.get(self.selected_inbox)
    }

    fn selected_event(&self) -> Option<&EventView> {
        self.messages.get(self.selected_message)
    }

    fn current_channel_id(&self) -> Option<&str> {
        match &self.target {
            MessageTarget::Channel { channel_id, .. }
            | MessageTarget::Thread { channel_id, .. } => Some(channel_id),
            MessageTarget::None => None,
        }
    }

    fn current_channel_name(&self) -> Option<&str> {
        match &self.target {
            MessageTarget::Channel { channel_name, .. }
            | MessageTarget::Thread { channel_name, .. } => Some(channel_name),
            MessageTarget::None => None,
        }
    }

    fn channel_label(&self, channel_id: &str) -> String {
        self.channel_names
            .get(channel_id)
            .cloned()
            .unwrap_or_else(|| short_hex(channel_id))
    }

    fn author_label(&self, pubkey: &str) -> String {
        self.profiles
            .get(&pubkey.to_ascii_lowercase())
            .filter(|name| !name.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| short_hex(pubkey))
    }

    fn is_agent_pubkey(&self, pubkey: &str) -> bool {
        normalize_hex64(pubkey)
            .as_ref()
            .is_some_and(|pubkey| self.agent_pubkeys.contains(pubkey))
    }

    fn is_collapsed_agent_reply(&self, event: &EventView) -> bool {
        self.is_agent_pubkey(&event.pubkey)
            && is_thread_reply(&event.tags)
            && !self.expanded_agent_events.contains(&event.id)
    }

    fn toggle_agent_replies(&mut self) {
        let agent_reply_ids: Vec<String> = self
            .messages
            .iter()
            .filter(|event| self.is_agent_pubkey(&event.pubkey) && is_thread_reply(&event.tags))
            .map(|event| event.id.clone())
            .collect();

        if agent_reply_ids.is_empty() {
            self.status = "No agent replies in this view.".to_string();
            return;
        }

        let has_collapsed = agent_reply_ids
            .iter()
            .any(|id| !self.expanded_agent_events.contains(id));
        if has_collapsed {
            for id in &agent_reply_ids {
                self.expanded_agent_events.insert(id.clone());
            }
            self.status = format!("Expanded {} agent replies.", agent_reply_ids.len());
        } else {
            for id in &agent_reply_ids {
                self.expanded_agent_events.remove(id);
            }
            self.status = format!("Collapsed {} agent replies.", agent_reply_ids.len());
        }
    }

    fn start_new_message(&mut self) {
        if self.current_channel_id().is_none() {
            self.status = "Open a channel before composing.".to_string();
            return;
        }
        self.compose_target = Some(ComposeTarget::New);
        self.draft.clear();
        self.status = "Composing new message. Enter sends, Esc cancels.".to_string();
    }

    fn start_reply(&mut self) {
        let Some(event) = self.selected_event() else {
            self.status = "Select a message before replying.".to_string();
            return;
        };
        if self.current_channel_id().is_none() {
            self.status = "Selected message has no channel context.".to_string();
            return;
        }
        let event_id = event.id.clone();
        let event_kind = event.kind;
        self.compose_target = Some(ComposeTarget::Reply {
            event_id: event_id.clone(),
            event_kind,
        });
        self.draft.clear();
        self.status = format!(
            "Replying to {}. Enter sends, Esc cancels.",
            short_hex(&event_id)
        );
    }

    async fn send_draft(&mut self, client: &BuzzClient) {
        let content = self.draft.trim().to_string();
        if content.is_empty() {
            self.status = "Message is empty.".to_string();
            return;
        }

        let Some(channel_id) = self.current_channel_id().map(str::to_string) else {
            self.status = "No channel target for message.".to_string();
            return;
        };

        let Some(compose_target) = self.compose_target.clone() else {
            self.status = "No active composer.".to_string();
            return;
        };

        let (reply_to, kind) = match compose_target {
            ComposeTarget::New => (None, None),
            ComposeTarget::Reply {
                event_id,
                event_kind,
            } => {
                let kind = matches!(event_kind, KIND_FORUM_POST | KIND_FORUM_COMMENT)
                    .then_some(KIND_FORUM_COMMENT as u16);
                (Some(event_id), kind)
            }
        };

        let params = SendMessageParams {
            channel_id,
            content,
            kind,
            reply_to,
            broadcast: false,
            files: Vec::new(),
        };

        match send_message_literal(client, params).await {
            Ok(raw) => {
                let sent_id = parse_write_event_id(&raw)
                    .map(|id| short_hex(&id))
                    .unwrap_or_else(|| "accepted".to_string());
                self.compose_target = None;
                self.draft.clear();
                let refresh_result = match self.target.clone() {
                    MessageTarget::Channel {
                        channel_id,
                        channel_name,
                    } => self.load_channel(client, channel_id, channel_name).await,
                    MessageTarget::Thread {
                        channel_id,
                        channel_name,
                        root_id,
                    } => {
                        self.load_thread(client, channel_id, channel_name, root_id)
                            .await
                    }
                    MessageTarget::None => Ok(()),
                };
                self.status = match refresh_result {
                    Ok(()) => format!("Sent ({sent_id})."),
                    Err(err) => format!("Sent ({sent_id}), but refresh failed: {err}"),
                };
            }
            Err(err) => {
                self.status = format!("Send failed: {err}");
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> TuiAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return TuiAction::Quit;
        }

        if self.compose_target.is_some() {
            return self.handle_composing_key(key);
        }

        match key.code {
            KeyCode::Char('q') => TuiAction::Quit,
            KeyCode::Char('R') => TuiAction::Refresh,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Messages,
                    Focus::Messages => Focus::Sidebar,
                };
                TuiAction::None
            }
            KeyCode::Char('c') => TuiAction::SwitchSidebar(SidebarMode::Channels),
            KeyCode::Char('i') => TuiAction::SwitchSidebar(SidebarMode::Inbox),
            KeyCode::Char('n') => {
                self.start_new_message();
                TuiAction::None
            }
            KeyCode::Char('r') => {
                self.start_reply();
                TuiAction::None
            }
            KeyCode::Char('x') => {
                self.toggle_agent_replies();
                TuiAction::None
            }
            KeyCode::Esc => {
                if matches!(self.target, MessageTarget::Thread { .. }) {
                    TuiAction::BackToChannel
                } else {
                    self.focus = Focus::Sidebar;
                    TuiAction::None
                }
            }
            _ => match self.focus {
                Focus::Sidebar => self.handle_sidebar_key(key),
                Focus::Messages => self.handle_messages_key(key),
            },
        }
    }

    fn handle_sidebar_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Enter => TuiAction::OpenSelection,
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_sidebar(1);
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_sidebar(-1);
                TuiAction::None
            }
            KeyCode::Home => {
                *self.selected_sidebar_mut() = 0;
                TuiAction::None
            }
            KeyCode::End => {
                *self.selected_sidebar_mut() = self.sidebar_len().saturating_sub(1);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_messages_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Enter => TuiAction::OpenThread,
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_message(1);
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_message(-1);
                TuiAction::None
            }
            KeyCode::Home => {
                self.selected_message = 0;
                TuiAction::None
            }
            KeyCode::End => {
                self.selected_message = self.messages.len().saturating_sub(1);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_composing_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Esc => {
                self.compose_target = None;
                self.draft.clear();
                self.status = "Composer cancelled.".to_string();
                TuiAction::None
            }
            KeyCode::Enter => TuiAction::SendDraft,
            KeyCode::Backspace => {
                self.draft.pop();
                TuiAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.draft.clear();
                TuiAction::None
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.draft.push(c);
                TuiAction::None
            }
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
                self.draft.push(c);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn move_sidebar(&mut self, delta: isize) {
        let len = self.sidebar_len();
        if len == 0 {
            return;
        }
        let current = self.selected_sidebar_index();
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            (current + delta as usize).min(len - 1)
        };
        *self.selected_sidebar_mut() = next;
    }

    fn move_message(&mut self, delta: isize) {
        if self.messages.is_empty() {
            return;
        }
        self.selected_message = if delta.is_negative() {
            self.selected_message.saturating_sub(delta.unsigned_abs())
        } else {
            (self.selected_message + delta as usize).min(self.messages.len() - 1)
        };
    }

    fn sidebar_len(&self) -> usize {
        match self.sidebar_mode {
            SidebarMode::Channels => self.channels.len(),
            SidebarMode::Inbox => self.inbox.len(),
        }
    }

    fn selected_sidebar_index(&self) -> usize {
        match self.sidebar_mode {
            SidebarMode::Channels => self.selected_channel,
            SidebarMode::Inbox => self.selected_inbox,
        }
    }

    fn selected_sidebar_mut(&mut self) -> &mut usize {
        match self.sidebar_mode {
            SidebarMode::Channels => &mut self.selected_channel,
            SidebarMode::Inbox => &mut self.selected_inbox,
        }
    }
}

#[derive(Clone, Debug)]
struct ChannelView {
    id: String,
    name: String,
    about: Option<String>,
    archived: bool,
}

#[derive(Clone, Debug)]
struct EventView {
    id: String,
    pubkey: String,
    kind: u32,
    content: String,
    created_at: i64,
    tags: Vec<Vec<String>>,
    channel_id: Option<String>,
}

#[derive(Clone, Debug)]
struct InboxRow {
    event: EventView,
    categories: Vec<InboxCategory>,
    latest_activity_at: i64,
    group_count: usize,
}

impl InboxRow {
    fn primary_category(&self) -> InboxCategory {
        self.categories
            .iter()
            .copied()
            .min_by_key(|category| category.priority())
            .unwrap_or(InboxCategory::Mention)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum InboxCategory {
    NeedsAction,
    Mention,
    Activity,
}

impl InboxCategory {
    fn feed_type(self) -> &'static str {
        match self {
            Self::NeedsAction => "needs_action",
            Self::Mention => "mentions",
            Self::Activity => "activity",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::NeedsAction => "Needs action",
            Self::Mention => "Mention",
            Self::Activity => "Activity",
        }
    }

    fn priority(self) -> usize {
        match self {
            Self::NeedsAction => 0,
            Self::Mention => 1,
            Self::Activity => 2,
        }
    }
}

struct ChannelSnapshot {
    channels: Vec<ChannelView>,
}

struct InboxSnapshot {
    rows: Vec<InboxRow>,
    profiles: HashMap<String, String>,
    channel_names: HashMap<String, String>,
    agent_pubkeys: BTreeSet<String>,
}

struct MessageSnapshot {
    events: Vec<EventView>,
    profiles: HashMap<String, String>,
    channel_names: HashMap<String, String>,
    agent_pubkeys: BTreeSet<String>,
}

fn draw_tui(frame: &mut Frame<'_>, app: &TuiApp) {
    let area = frame.area();
    let bottom_height = if app.compose_target.is_some() { 5 } else { 4 };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(bottom_height)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(vertical[0]);

    draw_sidebar(frame, app, panes[0]);
    draw_messages(frame, app, panes[1]);
    draw_footer(frame, app, vertical[1]);
}

fn draw_sidebar(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let focused = app.focus == Focus::Sidebar && app.compose_target.is_none();
    let title = match app.sidebar_mode {
        SidebarMode::Channels => {
            if focused {
                "Channels *"
            } else {
                "Channels"
            }
        }
        SidebarMode::Inbox => {
            if focused {
                "Inbox *"
            } else {
                "Inbox"
            }
        }
    };

    let items = match app.sidebar_mode {
        SidebarMode::Channels => app
            .channels
            .iter()
            .map(|channel| {
                let muted = channel.archived;
                let style = if muted {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };
                let about = channel
                    .about
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| truncate_for_list(&single_line(value), 72))
                    .unwrap_or_else(|| short_hex(&channel.id));
                ListItem::new(Text::from(vec![
                    Line::from(Span::styled(format!("#{}", channel.name), style)),
                    Line::from(Span::styled(about, Style::default().fg(Color::DarkGray))),
                ]))
            })
            .collect::<Vec<_>>(),
        SidebarMode::Inbox => app
            .inbox
            .iter()
            .map(|row| {
                let count = if row.group_count > 1 {
                    format!(" +{}", row.group_count - 1)
                } else {
                    String::new()
                };
                let author = app.author_label(&row.event.pubkey);
                let channel = row
                    .event
                    .channel_id
                    .as_deref()
                    .map(|id| format!("#{}", app.channel_label(id)))
                    .unwrap_or_else(|| "global".to_string());
                let preview = truncate_for_list(&single_line(&row.event.content), 84);
                ListItem::new(Text::from(vec![
                    Line::from(vec![
                        Span::styled(
                            row.primary_category().label(),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(count),
                        Span::raw("  "),
                        Span::styled(author, Style::default().add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled(channel, Style::default().fg(Color::Cyan)),
                        Span::raw("  "),
                        Span::styled(
                            format_timestamp(row.latest_activity_at),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]),
                    Line::from(Span::raw(preview)),
                ]))
            })
            .collect::<Vec<_>>(),
    };

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if app.sidebar_len() > 0 {
        state.select(Some(app.selected_sidebar_index()));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_messages(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let focused = app.focus == Focus::Messages && app.compose_target.is_none();
    let title = match &app.target {
        MessageTarget::None => {
            if focused {
                "Messages *".to_string()
            } else {
                "Messages".to_string()
            }
        }
        MessageTarget::Channel { channel_name, .. } => {
            if focused {
                format!("#{channel_name} *")
            } else {
                format!("#{channel_name}")
            }
        }
        MessageTarget::Thread {
            channel_name,
            root_id,
            ..
        } => {
            if focused {
                format!("Thread {}  #{channel_name} *", short_hex(root_id))
            } else {
                format!("Thread {}  #{channel_name}", short_hex(root_id))
            }
        }
    };

    let items: Vec<ListItem<'_>> = if app.messages.is_empty() {
        vec![ListItem::new(Text::from(Line::from(Span::styled(
            "No messages loaded.",
            Style::default().fg(Color::DarkGray),
        ))))]
    } else {
        app.messages
            .iter()
            .map(|message| ListItem::new(Text::from(message_lines(app, message))))
            .collect()
    };
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if !app.messages.is_empty() {
        state.select(Some(app.selected_message.min(app.messages.len() - 1)));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn message_lines<'a>(app: &'a TuiApp, message: &'a EventView) -> Vec<Line<'a>> {
    let is_agent = app.is_agent_pubkey(&message.pubkey);
    let is_reply = is_thread_reply(&message.tags);
    let is_collapsed = app.is_collapsed_agent_reply(message);
    let mut lines = Vec::new();
    let mut header = vec![
        Span::styled(
            app.author_label(&message.pubkey),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format_timestamp(message.created_at),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(short_hex(&message.id), Style::default().fg(Color::DarkGray)),
    ];
    if is_reply {
        header.push(Span::raw("  "));
        header.push(Span::styled("reply", Style::default().fg(Color::DarkGray)));
    }
    if is_agent {
        header.push(Span::raw("  "));
        header.push(Span::styled(
            if is_collapsed {
                "agent hidden"
            } else {
                "agent"
            },
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(header));

    if is_collapsed {
        lines.push(Line::from(Span::styled(
            "  [agent reply hidden] Press x to expand agent replies in this view.",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    if message.content.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no content)",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for content_line in message.content.lines().take(12) {
        lines.push(Line::from(Span::raw(format!(
            "  {}",
            truncate_for_list(content_line, 160)
        ))));
    }
    if message.content.lines().count() > 12 {
        lines.push(Line::from(Span::styled(
            "  ...",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn draw_footer(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let (title, body, style) = if app.compose_target.is_some() {
        let target = match app.compose_target.as_ref() {
            Some(ComposeTarget::New) => app
                .current_channel_name()
                .map(|name| format!("New message in #{name}"))
                .unwrap_or_else(|| "New message".to_string()),
            Some(ComposeTarget::Reply { event_id, .. }) => {
                format!("Reply to {}", short_hex(event_id))
            }
            None => "Compose".to_string(),
        };
        (
            target,
            if app.draft.is_empty() {
                "Type a message. Enter sends, Esc cancels, Ctrl-U clears.".to_string()
            } else {
                app.draft.clone()
            },
            Style::default().fg(Color::White),
        )
    } else {
        (
            "Keys".to_string(),
            format!(
                "Tab focus  c/i tabs  j/k move  Enter open  n new  r reply  x agents  R/q\n{}{}",
                app.status,
                if app.last_refresh_label.is_empty() {
                    String::new()
                } else {
                    format!("  {}", app.last_refresh_label)
                }
            ),
            Style::default().fg(Color::Gray),
        )
    };

    let paragraph = Paragraph::new(body)
        .block(Block::default().title(title).borders(Borders::ALL))
        .style(style)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

async fn fetch_channels(client: &BuzzClient) -> Result<ChannelSnapshot, CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let member_filter = serde_json::json!({
        "kinds": [39002],
        "#p": [my_pk],
        "limit": CHANNEL_LIMIT,
    });
    let member_raw = client.query(&member_filter).await?;
    let member_events: Vec<serde_json::Value> = serde_json::from_str(&member_raw)
        .map_err(|e| CliError::Other(format!("failed to parse channel memberships: {e}")))?;
    let channel_ids: Vec<String> = unique_owned(
        member_events
            .iter()
            .filter_map(|event| first_tag_value(&parse_tags(event.get("tags")), "d"))
            .collect(),
    );

    let metadata_filter = if channel_ids.is_empty() {
        serde_json::json!({
            "kinds": [39000],
            "limit": CHANNEL_LIMIT,
        })
    } else {
        serde_json::json!({
            "kinds": [39000],
            "#d": channel_ids,
            "limit": CHANNEL_LIMIT,
        })
    };
    let metadata_raw = client.query(&metadata_filter).await?;
    let metadata_events: Vec<serde_json::Value> = serde_json::from_str(&metadata_raw)
        .map_err(|e| CliError::Other(format!("failed to parse channels: {e}")))?;
    let mut channels: Vec<ChannelView> = metadata_events
        .iter()
        .filter_map(channel_from_value)
        .collect();
    channels.sort_by(|a, b| {
        a.archived
            .cmp(&b.archived)
            .then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(ChannelSnapshot { channels })
}

async fn fetch_channel_messages(
    client: &BuzzClient,
    channel_id: &str,
    limit: u32,
) -> Result<MessageSnapshot, CliError> {
    let filter = serde_json::json!({
        "kinds": MESSAGE_KINDS,
        "#h": [channel_id],
        "limit": limit,
    });
    let raw = client.query(&filter).await?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse messages: {e}")))?;
    let mut events: Vec<EventView> = values.iter().filter_map(event_from_value).collect();
    events.sort_by_key(|event| event.created_at);
    message_snapshot(client, events).await
}

async fn fetch_thread_messages(
    client: &BuzzClient,
    channel_id: &str,
    root_id: &str,
    limit: u32,
) -> Result<MessageSnapshot, CliError> {
    let reply_filter = serde_json::json!({
        "kinds": THREAD_KINDS,
        "#h": [channel_id],
        "#e": [root_id],
        "limit": limit,
    });
    let root_filter = serde_json::json!({
        "ids": [root_id],
        "limit": 1,
    });
    let raw = client.query_multi(&[reply_filter, root_filter]).await?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse thread: {e}")))?;
    let mut events = dedupe_events(values.iter().filter_map(event_from_value).collect());
    events.sort_by_key(|event| event.created_at);
    message_snapshot(client, events).await
}

async fn message_snapshot(
    client: &BuzzClient,
    events: Vec<EventView>,
) -> Result<MessageSnapshot, CliError> {
    let pubkeys: Vec<String> = events.iter().map(|event| event.pubkey.clone()).collect();
    let profiles = fetch_profiles(client, &pubkeys).await.unwrap_or_default();
    let agent_pubkeys = fetch_agent_pubkeys(client, &pubkeys)
        .await
        .unwrap_or_default();
    let channel_ids: Vec<String> = events
        .iter()
        .filter_map(|event| event.channel_id.clone())
        .collect();
    let channel_names = fetch_channel_names(client, &channel_ids)
        .await
        .unwrap_or_default();

    Ok(MessageSnapshot {
        events,
        profiles,
        channel_names,
        agent_pubkeys,
    })
}

async fn fetch_inbox(client: &BuzzClient, limit: u32) -> Result<InboxSnapshot, CliError> {
    let mut entries = Vec::new();
    let my_pk = client.keys().public_key().to_hex().to_ascii_lowercase();

    for category in INBOX_CATEGORIES {
        let query_limit = if *category == InboxCategory::Activity {
            limit.saturating_mul(5).clamp(limit, 100)
        } else {
            limit
        };
        let filter = inbox_filter(*category, &my_pk, query_limit);
        let raw = client.query(&filter).await?;
        let values: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse inbox: {e}")))?;
        let mut events: Vec<EventView> = values.iter().filter_map(event_from_value).collect();
        if *category == InboxCategory::Activity {
            events = filter_relevant_activity(client, events, &my_pk).await;
        }
        entries.extend(events.into_iter().map(|event| (*category, event)));
    }

    let rows = group_inbox(entries);
    let pubkeys: Vec<String> = rows.iter().map(|row| row.event.pubkey.clone()).collect();
    let profiles = fetch_profiles(client, &pubkeys).await.unwrap_or_default();
    let agent_pubkeys = fetch_agent_pubkeys(client, &pubkeys)
        .await
        .unwrap_or_default();
    let channel_ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row.event.channel_id.clone())
        .collect();
    let channel_names = fetch_channel_names(client, &channel_ids)
        .await
        .unwrap_or_default();

    Ok(InboxSnapshot {
        rows,
        profiles,
        channel_names,
        agent_pubkeys,
    })
}

fn inbox_filter(category: InboxCategory, my_pubkey: &str, limit: u32) -> serde_json::Value {
    match category {
        InboxCategory::Mention => serde_json::json!({
            "kinds": INBOX_MENTION_KINDS,
            "#p": [my_pubkey],
            "limit": limit,
        }),
        InboxCategory::NeedsAction => serde_json::json!({
            "kinds": INBOX_NEEDS_ACTION_KINDS,
            "#p": [my_pubkey],
            "limit": limit.min(20),
        }),
        InboxCategory::Activity => serde_json::json!({
            "#p": [my_pubkey],
            "feed_types": [category.feed_type()],
            "limit": limit,
        }),
    }
}

async fn filter_relevant_activity(
    client: &BuzzClient,
    events: Vec<EventView>,
    my_pubkey: &str,
) -> Vec<EventView> {
    let mut candidates = Vec::new();
    let mut context_requests = Vec::new();
    let mut requested_roots = BTreeSet::new();

    for event in events {
        if !is_relevant_activity_candidate(&event, my_pubkey) {
            continue;
        }
        let Some(channel_id) = event.channel_id.as_deref() else {
            continue;
        };
        let root_id = thread_key(&event);
        if requested_roots.insert(root_id.clone()) {
            context_requests.push((root_id.clone(), channel_id.to_string()));
        }
        candidates.push((root_id, event));
    }

    let context_results =
        stream::iter(context_requests.into_iter().map(|(root_id, channel_id)| {
            let client = client.clone();
            async move {
                let context = fetch_thread_events(&client, &channel_id, &root_id)
                    .await
                    .ok();
                (root_id, context)
            }
        }))
        .buffer_unordered(ACTIVITY_CONTEXT_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let context_by_root: HashMap<String, Vec<EventView>> = context_results
        .into_iter()
        .filter_map(|(root_id, context)| context.map(|context| (root_id, context)))
        .collect();

    candidates
        .into_iter()
        .filter_map(|(root_id, event)| {
            let context = context_by_root.get(&root_id)?;
            thread_context_relevant_to_user(&event, context, my_pubkey).then_some(event)
        })
        .collect()
}

async fn fetch_thread_events(
    client: &BuzzClient,
    channel_id: &str,
    root_id: &str,
) -> Result<Vec<EventView>, CliError> {
    let reply_filter = serde_json::json!({
        "kinds": THREAD_KINDS,
        "#h": [channel_id],
        "#e": [root_id],
        "limit": 100,
    });
    let root_filter = serde_json::json!({
        "ids": [root_id],
        "limit": 1,
    });
    let raw = client.query_multi(&[reply_filter, root_filter]).await?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse thread context: {e}")))?;
    let mut events = dedupe_events(values.iter().filter_map(event_from_value).collect());
    events.sort_by_key(|event| event.created_at);
    Ok(events)
}

fn is_relevant_activity_candidate(event: &EventView, my_pubkey: &str) -> bool {
    event.pubkey.to_ascii_lowercase() != my_pubkey
        && is_thread_reply(&event.tags)
        && !has_mention_for_pubkey(event, my_pubkey)
        && !is_broadcast_reply(&event.tags)
}

fn thread_context_relevant_to_user(
    candidate: &EventView,
    context: &[EventView],
    my_pubkey: &str,
) -> bool {
    let root_id = thread_key(candidate);
    context.iter().any(|event| {
        event.id != candidate.id
            && (event.pubkey.to_ascii_lowercase() == my_pubkey
                || has_mention_for_pubkey(event, my_pubkey)
                || (event.id == root_id && event.pubkey.to_ascii_lowercase() == my_pubkey))
    })
}

async fn fetch_profiles(
    client: &BuzzClient,
    pubkeys: &[String],
) -> Result<HashMap<String, String>, CliError> {
    let authors = unique_non_empty(pubkeys);
    if authors.is_empty() {
        return Ok(HashMap::new());
    }

    let filter = serde_json::json!({
        "kinds": [0],
        "authors": authors,
        "limit": authors.len(),
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse profiles: {e}")))?;

    let mut profiles = HashMap::new();
    for event in events {
        let Some(pubkey) = event.get("pubkey").and_then(|value| value.as_str()) else {
            continue;
        };
        let Some(content) = event.get("content").and_then(|value| value.as_str()) else {
            continue;
        };
        let Ok(profile) = serde_json::from_str::<serde_json::Value>(content) else {
            continue;
        };
        let label = profile
            .get("display_name")
            .or_else(|| profile.get("name"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        if let Some(label) = label {
            profiles.insert(pubkey.to_ascii_lowercase(), label);
        }
    }

    Ok(profiles)
}

async fn fetch_channel_names(
    client: &BuzzClient,
    channel_ids: &[String],
) -> Result<HashMap<String, String>, CliError> {
    let ids = unique_non_empty(channel_ids);
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let filter = serde_json::json!({
        "kinds": [39000],
        "#d": ids,
        "limit": ids.len(),
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse channel names: {e}")))?;

    let mut channels = HashMap::new();
    for event in events {
        let tags = parse_tags(event.get("tags"));
        let Some(id) = first_tag_value(&tags, "d") else {
            continue;
        };
        let Some(name) = first_tag_value(&tags, "name") else {
            continue;
        };
        channels.insert(id, name);
    }

    Ok(channels)
}

async fn fetch_agent_pubkeys(
    client: &BuzzClient,
    pubkeys: &[String],
) -> Result<BTreeSet<String>, CliError> {
    let mut agent_pubkeys = BTreeSet::new();
    let authors = unique_owned(
        pubkeys
            .iter()
            .filter_map(|value| normalize_hex64(value))
            .collect(),
    );

    if !authors.is_empty() {
        let filter = serde_json::json!({
            "kinds": [0],
            "authors": authors,
            "limit": authors.len(),
        });
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse agent profiles: {e}")))?;
        for event in events {
            if !profile_has_valid_oa_owner_value(&event) {
                continue;
            }
            let Some(pubkey) = event
                .get("pubkey")
                .and_then(|value| value.as_str())
                .and_then(normalize_hex64)
            else {
                continue;
            };
            agent_pubkeys.insert(pubkey);
        }
    }

    let owner_pubkey = client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex());
    if let Some(owner_pubkey) = normalize_hex64(&owner_pubkey) {
        let filter = serde_json::json!({
            "kinds": [KIND_MANAGED_AGENT],
            "authors": [owner_pubkey],
            "limit": 200,
        });
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse managed agents: {e}")))?;
        for event in events {
            for tag in parse_tags(event.get("tags")) {
                if tag.first().map(|value| value.as_str()) != Some("d") {
                    continue;
                }
                let Some(pubkey) = tag.get(1).and_then(|value| normalize_hex64(value)) else {
                    continue;
                };
                agent_pubkeys.insert(pubkey);
            }
        }
    }

    Ok(agent_pubkeys)
}

fn profile_has_valid_oa_owner_value(event: &serde_json::Value) -> bool {
    let Some(target_pubkey) = event
        .get("pubkey")
        .and_then(|value| value.as_str())
        .and_then(normalize_hex64)
        .and_then(|value| PublicKey::from_hex(&value).ok())
    else {
        return false;
    };

    for tag in parse_tags(event.get("tags")) {
        if tag.first().map(|value| value.as_str()) != Some("auth") || tag.len() != 4 {
            continue;
        }
        let Ok(json) = serde_json::to_string(&tag) else {
            continue;
        };
        if buzz_sdk::nip_oa::verify_auth_tag(&json, &target_pubkey).is_ok() {
            return true;
        }
    }

    false
}

fn channel_from_value(value: &serde_json::Value) -> Option<ChannelView> {
    let tags = parse_tags(value.get("tags"));
    Some(ChannelView {
        id: first_tag_value(&tags, "d")?,
        name: first_tag_value(&tags, "name")?,
        about: first_tag_value(&tags, "about"),
        archived: first_tag_value(&tags, "archived").as_deref() == Some("true"),
    })
}

fn event_from_value(value: &serde_json::Value) -> Option<EventView> {
    let tags = parse_tags(value.get("tags"));
    Some(EventView {
        id: value.get("id")?.as_str()?.to_string(),
        pubkey: value.get("pubkey")?.as_str()?.to_string(),
        kind: value.get("kind")?.as_u64()? as u32,
        content: value
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
        created_at: value.get("created_at")?.as_i64()?,
        channel_id: first_tag_value(&tags, "h"),
        tags,
    })
}

fn group_inbox(entries: Vec<(InboxCategory, EventView)>) -> Vec<InboxRow> {
    struct Group {
        event: EventView,
        categories: Vec<InboxCategory>,
        event_ids: BTreeSet<String>,
        latest_activity_at: i64,
    }

    let mut groups: HashMap<String, Group> = HashMap::new();
    for (category, event) in entries {
        let key = thread_key(&event);
        let group = groups.entry(key).or_insert_with(|| Group {
            event: event.clone(),
            categories: Vec::new(),
            event_ids: BTreeSet::new(),
            latest_activity_at: event.created_at,
        });

        if !group.categories.contains(&category) {
            group.categories.push(category);
            group.categories.sort_by_key(|value| value.priority());
        }
        group.event_ids.insert(event.id.clone());
        if event.created_at >= group.latest_activity_at {
            group.latest_activity_at = event.created_at;
            group.event = event;
        }
    }

    let mut rows: Vec<InboxRow> = groups
        .into_values()
        .map(|group| InboxRow {
            event: group.event,
            categories: group.categories,
            latest_activity_at: group.latest_activity_at,
            group_count: group.event_ids.len(),
        })
        .collect();
    rows.sort_by_key(|row| Reverse(row.latest_activity_at));
    rows
}

fn parse_tags(value: Option<&serde_json::Value>) -> Vec<Vec<String>> {
    value
        .and_then(|value| value.as_array())
        .map(|tags| {
            tags.iter()
                .filter_map(|tag| {
                    let parts: Vec<String> = tag
                        .as_array()?
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect();
                    (!parts.is_empty()).then_some(parts)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn first_tag_value(tags: &[Vec<String>], key: &str) -> Option<String> {
    tags.iter()
        .find(|tag| tag.first().map(|value| value.as_str()) == Some(key))
        .and_then(|tag| tag.get(1))
        .filter(|value| !value.is_empty())
        .cloned()
}

fn has_mention_for_pubkey(event: &EventView, pubkey: &str) -> bool {
    let normalized = pubkey.to_ascii_lowercase();
    event.tags.iter().any(|tag| {
        tag.first().map(|value| value.as_str()) == Some("p")
            && tag
                .get(1)
                .map(|value| value.to_ascii_lowercase() == normalized)
                .unwrap_or(false)
    })
}

fn is_thread_reply(tags: &[Vec<String>]) -> bool {
    thread_reference(tags).0.is_some()
}

fn is_broadcast_reply(tags: &[Vec<String>]) -> bool {
    tags.iter().any(|tag| {
        tag.first().map(|value| value.as_str()) == Some("broadcast")
            && tag.get(1).map(|value| value.as_str()) == Some("1")
    })
}

fn thread_reference(tags: &[Vec<String>]) -> (Option<String>, Option<String>) {
    let mut root = None;
    let mut reply = None;
    for tag in tags {
        if tag.first().map(|value| value.as_str()) != Some("e") {
            continue;
        }
        let Some(id) = tag.get(1).filter(|id| is_hex64(id)) else {
            continue;
        };
        match tag.get(3).map(|value| value.as_str()) {
            Some("root") => root = Some(id.clone()),
            Some("reply") => reply = Some(id.clone()),
            _ => {}
        }
    }

    let Some(parent_id) = reply else {
        return (None, None);
    };
    let root_id = root.unwrap_or_else(|| parent_id.clone());
    (Some(parent_id), Some(root_id))
}

fn thread_key(event: &EventView) -> String {
    thread_reference(&event.tags)
        .1
        .unwrap_or_else(|| event.id.clone())
}

fn dedupe_events(events: Vec<EventView>) -> Vec<EventView> {
    let mut by_id = HashMap::new();
    for event in events {
        by_id.insert(event.id.clone(), event);
    }
    by_id.into_values().collect()
}

fn unique_non_empty(values: &[String]) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert((*value).to_string()))
        .collect()
}

fn unique_owned(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn parse_write_event_id(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()?
        .get("event_id")?
        .as_str()
        .map(str::to_string)
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_for_list(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let suffix = "...";
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(suffix.len()))
        .collect::<String>();
    out.push_str(suffix);
    out
}

fn short_hex(value: &str) -> String {
    value.chars().take(8).collect()
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn normalize_hex64(value: &str) -> Option<String> {
    let value = value.trim();
    is_hex64(value).then(|| value.to_ascii_lowercase())
}

fn format_timestamp(unix_seconds: i64) -> String {
    Local
        .timestamp_opt(unix_seconds, 0)
        .single()
        .map(|dt| dt.format("%b %-d %H:%M").to_string())
        .unwrap_or_else(|| unix_seconds.to_string())
}

fn terminal_err(err: io::Error) -> CliError {
    CliError::Other(format!("terminal error: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SELF_PK: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const AGENT_PK: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const CHANNEL_ID: &str = "80a927fd-a695-4895-971e-e49c974b0fff";

    fn event_with_pubkey(
        id: &str,
        pubkey: &str,
        created_at: i64,
        tags: Vec<Vec<&str>>,
    ) -> EventView {
        let value = json!({
            "id": id,
            "pubkey": pubkey,
            "kind": 9,
            "content": "body",
            "created_at": created_at,
            "tags": tags,
        });
        event_from_value(&value).expect("valid event")
    }

    fn rendered_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn agent_thread_replies_are_hidden_until_expanded() {
        let root = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let reply = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut event = event_with_pubkey(
            reply,
            AGENT_PK,
            2,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", root, "", "reply"],
            ],
        );
        event.content = "agent body should be hidden".to_string();

        let mut app = TuiApp::new(50, 0, None, false);
        app.messages = vec![event];
        app.agent_pubkeys.insert(AGENT_PK.to_string());

        let collapsed = rendered_text(&message_lines(&app, &app.messages[0]));
        assert!(collapsed.contains("[agent reply hidden]"));
        assert!(!collapsed.contains("agent body should be hidden"));

        app.expanded_agent_events.insert(reply.to_string());
        let expanded = rendered_text(&message_lines(&app, &app.messages[0]));
        assert!(expanded.contains("agent body should be hidden"));
    }

    #[test]
    fn top_level_agent_messages_are_not_hidden() {
        let mut event = event_with_pubkey(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            AGENT_PK,
            2,
            vec![vec!["h", CHANNEL_ID]],
        );
        event.content = "top-level agent body".to_string();

        let mut app = TuiApp::new(50, 0, None, false);
        app.messages = vec![event];
        app.agent_pubkeys.insert(AGENT_PK.to_string());

        let rendered = rendered_text(&message_lines(&app, &app.messages[0]));
        assert!(!rendered.contains("[agent reply hidden]"));
        assert!(rendered.contains("top-level agent body"));
    }

    #[test]
    fn activity_candidate_requires_external_non_mention_thread_reply() {
        let root = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let parent = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let candidate = event_with_pubkey(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            AGENT_PK,
            2,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
            ],
        );
        assert!(is_relevant_activity_candidate(&candidate, SELF_PK));

        let mention = event_with_pubkey(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            AGENT_PK,
            2,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["p", SELF_PK],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
            ],
        );
        assert!(!is_relevant_activity_candidate(&mention, SELF_PK));

        let broadcast = event_with_pubkey(
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            AGENT_PK,
            2,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
                vec!["broadcast", "1"],
            ],
        );
        assert!(!is_relevant_activity_candidate(&broadcast, SELF_PK));
    }
}
