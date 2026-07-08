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
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::client::BuzzClient;
use crate::commands::messages::{send_message_literal, SendMessageParams};
use crate::error::CliError;

const DEFAULT_TYPES: &[InboxCategory] = &[
    InboxCategory::NeedsAction,
    InboxCategory::Mention,
    InboxCategory::Activity,
];
const ACTIVITY_CONTEXT_CONCURRENCY: usize = 8;
const INPUT_POLL_MS: u64 = 50;
const THREAD_CONTEXT_LIMIT: u32 = 100;
const GUI_MENTION_KINDS: &[u32] = &[
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_TEXT_NOTE,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];
const GUI_NEEDS_ACTION_KINDS: &[u32] = &[
    KIND_WORKFLOW_APPROVAL_REQUESTED,
    KIND_WORKFLOW_APPROVAL_GRANTED,
    KIND_WORKFLOW_APPROVAL_DENIED,
];
const THREAD_KINDS: &[u32] = &[
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_EDIT,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_FORUM_COMMENT,
];

pub struct InboxOptions {
    pub limit: Option<u32>,
    pub poll_seconds: u64,
    pub types: Option<String>,
}

pub async fn dispatch(cmd: crate::InboxCmd, client: &BuzzClient) -> Result<(), CliError> {
    let options = InboxOptions {
        limit: cmd.limit,
        poll_seconds: cmd.poll_seconds,
        types: cmd.types,
    };

    if cmd.debug_json {
        print_inbox_debug_json(client, options).await
    } else {
        run_inbox_tui(client, options).await
    }
}

async fn print_inbox_debug_json(
    client: &BuzzClient,
    options: InboxOptions,
) -> Result<(), CliError> {
    let categories = parse_categories(options.types.as_deref())?;
    let limit = options.limit.unwrap_or(50).clamp(1, 100);
    let snapshot = fetch_inbox_snapshot(client, limit, &categories).await?;
    let rows: Vec<serde_json::Value> = snapshot
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "id": item.event.id,
                "thread_id": item.id,
                "kind": item.event.kind,
                "category": item.primary_category().feed_type(),
                "categories": item.categories.iter().map(|category| category.feed_type()).collect::<Vec<_>>(),
                "created_at": item.event.created_at,
                "latest_activity_at": item.latest_activity_at,
                "channel_id": item.event.channel_id,
                "author": item.event.pubkey,
                "preview": truncate_for_list(&single_line(&item.event.content), 160),
            })
        })
        .collect();
    let output = serde_json::to_string_pretty(&rows)
        .map_err(|e| CliError::Other(format!("failed to serialize inbox rows: {e}")))?;
    println!("{output}");
    Ok(())
}

async fn run_inbox_tui(client: &BuzzClient, options: InboxOptions) -> Result<(), CliError> {
    let categories = parse_categories(options.types.as_deref())?;
    let limit = options.limit.unwrap_or(50).clamp(1, 100);
    let mut app = InboxApp::new(limit, options.poll_seconds, categories);
    app.refresh(client).await?;
    app.load_selected_context(client).await?;

    let mut session = TerminalSession::enter()?;
    let (task_tx, mut task_rx) = mpsc::unbounded_channel();
    let mut tasks = InboxTasks::new(task_tx);

    loop {
        while let Ok(result) = task_rx.try_recv() {
            tasks.apply_result(result, &mut app, client.clone());
        }

        if app.should_poll() && !tasks.is_refreshing() {
            tasks.start_refresh(client.clone(), &mut app);
        }

        session.draw(&app)?;

        if !event::poll(Duration::from_millis(INPUT_POLL_MS)).map_err(terminal_err)? {
            continue;
        }

        let Event::Key(key) = event::read().map_err(terminal_err)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match app.handle_key(key) {
            InboxAction::None => {}
            InboxAction::Quit => break,
            InboxAction::Refresh => {
                tasks.start_refresh(client.clone(), &mut app);
            }
            InboxAction::LoadContext => {
                tasks.start_selected_context_load(client.clone(), &mut app);
            }
            InboxAction::SendReply => {
                app.send_reply(client).await;
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

    fn draw(&mut self, app: &InboxApp) -> Result<(), CliError> {
        self.terminal
            .draw(|frame| draw_inbox(frame, app))
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
enum InboxMode {
    Browsing,
    Composing,
}

enum InboxAction {
    None,
    Quit,
    Refresh,
    LoadContext,
    SendReply,
}

struct InboxTasks {
    tx: mpsc::UnboundedSender<InboxTaskResult>,
    refresh_task: Option<JoinHandle<()>>,
    context_task: Option<JoinHandle<()>>,
    next_context_request_id: u64,
    active_context_request_id: Option<u64>,
}

impl InboxTasks {
    fn new(tx: mpsc::UnboundedSender<InboxTaskResult>) -> Self {
        Self {
            tx,
            refresh_task: None,
            context_task: None,
            next_context_request_id: 1,
            active_context_request_id: None,
        }
    }

    fn is_refreshing(&self) -> bool {
        self.refresh_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    fn start_refresh(&mut self, client: BuzzClient, app: &mut InboxApp) {
        if self.is_refreshing() {
            app.status = "Refresh already running.".to_string();
            return;
        }

        app.mark_refresh_started();
        let tx = self.tx.clone();
        let limit = app.limit;
        let categories = app.categories.clone();
        self.refresh_task = Some(tokio::spawn(async move {
            let result = fetch_inbox_snapshot(&client, limit, &categories).await;
            let _ = tx.send(InboxTaskResult::Refresh(result));
        }));
    }

    fn start_selected_context_load(&mut self, client: BuzzClient, app: &mut InboxApp) {
        if let Some(task) = self.context_task.take() {
            task.abort();
        }
        self.active_context_request_id = None;

        let Some(request) = app.begin_selected_context_load() else {
            return;
        };

        let request_id = self.next_context_request_id;
        self.next_context_request_id = self.next_context_request_id.saturating_add(1);
        self.active_context_request_id = Some(request_id);

        let tx = self.tx.clone();
        self.context_task = Some(tokio::spawn(async move {
            let result = fetch_context_snapshot(&client, &request.event, &request.channel_id).await;
            let _ = tx.send(InboxTaskResult::Context {
                request_id,
                item_id: request.item_id,
                result,
            });
        }));
    }

    fn apply_result(&mut self, result: InboxTaskResult, app: &mut InboxApp, client: BuzzClient) {
        match result {
            InboxTaskResult::Refresh(result) => {
                self.refresh_task = None;
                match result {
                    Ok(snapshot) => {
                        app.apply_refresh(snapshot);
                        self.start_selected_context_load(client, app);
                    }
                    Err(err) => {
                        app.status = format!("Refresh failed: {err}");
                    }
                }
            }
            InboxTaskResult::Context {
                request_id,
                item_id,
                result,
            } => {
                if self.active_context_request_id != Some(request_id) {
                    return;
                }
                self.context_task = None;
                self.active_context_request_id = None;
                app.apply_context_result(item_id, result);
            }
        }
    }
}

impl Drop for InboxTasks {
    fn drop(&mut self) {
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
        if let Some(task) = self.context_task.take() {
            task.abort();
        }
        self.active_context_request_id = None;
    }
}

enum InboxTaskResult {
    Refresh(Result<InboxSnapshot, CliError>),
    Context {
        request_id: u64,
        item_id: String,
        result: Result<ContextSnapshot, CliError>,
    },
}

struct ContextLoadRequest {
    item_id: String,
    event: EventView,
    channel_id: String,
}

struct InboxApp {
    items: Vec<InboxItem>,
    selected: usize,
    context: Vec<EventView>,
    context_cache: HashMap<String, Vec<EventView>>,
    pending_context_id: Option<String>,
    context_scroll: u16,
    draft: String,
    mode: InboxMode,
    status: String,
    profiles: HashMap<String, String>,
    channel_names: HashMap<String, String>,
    agent_pubkeys: BTreeSet<String>,
    expanded_agent_events: BTreeSet<String>,
    last_refresh: Option<Instant>,
    last_refresh_label: String,
    limit: u32,
    poll_seconds: u64,
    categories: Vec<InboxCategory>,
}

impl InboxApp {
    fn new(limit: u32, poll_seconds: u64, categories: Vec<InboxCategory>) -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            context: Vec::new(),
            context_cache: HashMap::new(),
            pending_context_id: None,
            context_scroll: 0,
            draft: String::new(),
            mode: InboxMode::Browsing,
            status: "Loading inbox...".to_string(),
            profiles: HashMap::new(),
            channel_names: HashMap::new(),
            agent_pubkeys: BTreeSet::new(),
            expanded_agent_events: BTreeSet::new(),
            last_refresh: None,
            last_refresh_label: String::new(),
            limit,
            poll_seconds,
            categories,
        }
    }

    fn should_poll(&self) -> bool {
        self.poll_seconds > 0
            && self
                .last_refresh
                .map(|last| last.elapsed() >= Duration::from_secs(self.poll_seconds))
                .unwrap_or(false)
    }

    async fn refresh(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        let snapshot = fetch_inbox_snapshot(client, self.limit, &self.categories).await?;
        self.apply_refresh(snapshot);
        Ok(())
    }

    fn mark_refresh_started(&mut self) {
        self.last_refresh = Some(Instant::now());
        self.status = "Refreshing inbox...".to_string();
    }

    fn apply_refresh(&mut self, snapshot: InboxSnapshot) {
        let previous_id = self.selected_item().map(|item| item.id.clone());

        self.items = snapshot.items;
        self.profiles.extend(snapshot.profiles);
        self.channel_names.extend(snapshot.channel_names);
        self.agent_pubkeys.extend(snapshot.agent_pubkeys);
        self.selected = previous_id
            .and_then(|id| self.items.iter().position(|item| item.id == id))
            .unwrap_or_else(|| self.selected.min(self.items.len().saturating_sub(1)));
        self.context_scroll = 0;
        self.pending_context_id = None;

        let item_ids: BTreeSet<String> = self.items.iter().map(|item| item.id.clone()).collect();
        self.context_cache.retain(|id, _| item_ids.contains(id));

        if self.items.is_empty() {
            self.context.clear();
        }

        self.last_refresh = Some(Instant::now());
        self.last_refresh_label = format_clock_now();
        self.status = if self.items.is_empty() {
            "No inbox items found. Press R to refresh, q to quit.".to_string()
        } else {
            format!(
                "Loaded {} inbox item{}. Press r to reply, R to refresh, q to quit.",
                self.items.len(),
                if self.items.len() == 1 { "" } else { "s" }
            )
        };
    }

    async fn load_selected_context(&mut self, client: &BuzzClient) -> Result<(), CliError> {
        let Some(item) = self.selected_item().cloned() else {
            self.context.clear();
            return Ok(());
        };

        let snapshot = match item.event.channel_id.as_deref() {
            Some(channel_id) => fetch_context_snapshot(client, &item.event, channel_id).await?,
            None => ContextSnapshot {
                events: vec![item.event.clone()],
                profiles: HashMap::new(),
                agent_pubkeys: BTreeSet::new(),
            },
        };
        self.apply_context_snapshot(item.id, snapshot);
        Ok(())
    }

    fn begin_selected_context_load(&mut self) -> Option<ContextLoadRequest> {
        let Some(item) = self.selected_item().cloned() else {
            self.context.clear();
            self.pending_context_id = None;
            return None;
        };

        if let Some(cached) = self.context_cache.get(&item.id) {
            self.context = cached.clone();
            self.context_scroll = 0;
            self.pending_context_id = None;
            return None;
        }

        let Some(channel_id) = item.event.channel_id.clone() else {
            self.context = vec![item.event.clone()];
            self.context_cache
                .insert(item.id.clone(), self.context.clone());
            self.context_scroll = 0;
            self.pending_context_id = None;
            return None;
        };

        self.context = vec![item.event.clone()];
        self.context_scroll = 0;
        self.pending_context_id = Some(item.id.clone());
        self.status = format!("Loading thread {}...", short_hex(&item.event.id));

        Some(ContextLoadRequest {
            item_id: item.id,
            event: item.event,
            channel_id,
        })
    }

    fn apply_context_result(&mut self, item_id: String, result: Result<ContextSnapshot, CliError>) {
        if self.pending_context_id.as_deref() == Some(item_id.as_str()) {
            self.pending_context_id = None;
        }

        match result {
            Ok(snapshot) => self.apply_context_snapshot(item_id, snapshot),
            Err(err) => {
                if self.selected_item().map(|item| item.id.as_str()) == Some(item_id.as_str()) {
                    self.status = format!("Thread load failed: {err}");
                }
            }
        }
    }

    fn apply_context_snapshot(&mut self, item_id: String, snapshot: ContextSnapshot) {
        self.profiles.extend(snapshot.profiles);
        self.agent_pubkeys.extend(snapshot.agent_pubkeys);
        self.context_cache
            .insert(item_id.clone(), snapshot.events.clone());

        if self.selected_item().map(|item| item.id.as_str()) == Some(item_id.as_str()) {
            self.context = snapshot.events;
            self.context_scroll = 0;
            self.status = "Thread loaded. Press r to reply, R to refresh, q to quit.".to_string();
        }
    }

    fn is_agent_pubkey(&self, pubkey: &str) -> bool {
        normalize_pubkey(pubkey)
            .as_ref()
            .is_some_and(|pubkey| self.agent_pubkeys.contains(pubkey))
    }

    async fn send_reply(&mut self, client: &BuzzClient) {
        let content = self.draft.trim().to_string();
        if content.is_empty() {
            self.status = "Reply is empty.".to_string();
            return;
        }

        let Some(item) = self.selected_item().cloned() else {
            self.status = "No inbox item selected.".to_string();
            return;
        };
        let Some(channel_id) = item.event.channel_id.clone() else {
            self.status = "This inbox item has no channel target for replies.".to_string();
            return;
        };

        let kind = match item.event.kind {
            45001 | 45003 => Some(45003),
            _ => None,
        };
        let params = SendMessageParams {
            channel_id,
            content,
            kind,
            reply_to: Some(item.event.id.clone()),
            broadcast: false,
            files: Vec::new(),
        };

        self.status = "Sending reply...".to_string();
        match send_message_literal(client, params).await {
            Ok(raw) => {
                let sent_id = parse_write_event_id(&raw)
                    .map(|id| short_hex(&id))
                    .unwrap_or_else(|| "accepted".to_string());
                self.mode = InboxMode::Browsing;
                self.draft.clear();
                self.status = format!("Reply sent ({sent_id}). Refreshing...");
                if let Err(err) = self.refresh(client).await {
                    self.status = format!("Reply sent, but refresh failed: {err}");
                    return;
                }
                if let Err(err) = self.load_selected_context(client).await {
                    self.status = format!("Reply sent, but thread reload failed: {err}");
                } else {
                    self.status = format!("Reply sent ({sent_id}).");
                }
            }
            Err(err) => {
                self.status = format!("Send failed: {err}");
            }
        }
    }

    fn selected_item(&self) -> Option<&InboxItem> {
        self.items.get(self.selected)
    }

    fn handle_key(&mut self, key: KeyEvent) -> InboxAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return InboxAction::Quit;
        }

        match self.mode {
            InboxMode::Browsing => self.handle_browsing_key(key),
            InboxMode::Composing => self.handle_composing_key(key),
        }
    }

    fn handle_browsing_key(&mut self, key: KeyEvent) -> InboxAction {
        match key.code {
            KeyCode::Char('q') => InboxAction::Quit,
            KeyCode::Char('R') => InboxAction::Refresh,
            KeyCode::Char('r') | KeyCode::Enter => {
                if self
                    .selected_item()
                    .and_then(|item| item.event.channel_id.as_ref())
                    .is_some()
                {
                    self.mode = InboxMode::Composing;
                    self.status = "Composing reply. Enter sends, Esc cancels.".to_string();
                } else {
                    self.status = "This inbox item has no channel target for replies.".to_string();
                }
                InboxAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.items.len() {
                    self.selected += 1;
                    return InboxAction::LoadContext;
                }
                InboxAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                    return InboxAction::LoadContext;
                }
                InboxAction::None
            }
            KeyCode::Home => {
                if self.selected != 0 {
                    self.selected = 0;
                    return InboxAction::LoadContext;
                }
                InboxAction::None
            }
            KeyCode::End => {
                let last = self.items.len().saturating_sub(1);
                if self.selected != last {
                    self.selected = last;
                    return InboxAction::LoadContext;
                }
                InboxAction::None
            }
            KeyCode::PageDown => {
                self.context_scroll = self.context_scroll.saturating_add(8);
                InboxAction::None
            }
            KeyCode::PageUp => {
                self.context_scroll = self.context_scroll.saturating_sub(8);
                InboxAction::None
            }
            KeyCode::Char('x') => {
                self.toggle_visible_agent_replies();
                InboxAction::None
            }
            _ => InboxAction::None,
        }
    }

    fn toggle_visible_agent_replies(&mut self) {
        let agent_event_ids: Vec<String> = self
            .context
            .iter()
            .filter(|event| self.is_agent_pubkey(&event.pubkey))
            .map(|event| event.id.clone())
            .collect();

        if agent_event_ids.is_empty() {
            self.status = "No agent replies in this thread.".to_string();
            return;
        }

        let has_collapsed = agent_event_ids
            .iter()
            .any(|id| !self.expanded_agent_events.contains(id));
        if has_collapsed {
            for id in &agent_event_ids {
                self.expanded_agent_events.insert(id.clone());
            }
            self.status = format!(
                "Expanded {} agent repl{}.",
                agent_event_ids.len(),
                if agent_event_ids.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
        } else {
            for id in &agent_event_ids {
                self.expanded_agent_events.remove(id);
            }
            self.status = format!(
                "Collapsed {} agent repl{}.",
                agent_event_ids.len(),
                if agent_event_ids.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
        }
    }

    fn handle_composing_key(&mut self, key: KeyEvent) -> InboxAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = InboxMode::Browsing;
                self.status = "Reply cancelled.".to_string();
                InboxAction::None
            }
            KeyCode::Enter => InboxAction::SendReply,
            KeyCode::Backspace => {
                self.draft.pop();
                InboxAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.draft.clear();
                InboxAction::None
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.draft.push(c);
                InboxAction::None
            }
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
                self.draft.push(c);
                InboxAction::None
            }
            _ => InboxAction::None,
        }
    }
}

#[derive(Clone, Debug)]
struct InboxItem {
    id: String,
    event: EventView,
    categories: Vec<InboxCategory>,
    latest_activity_at: i64,
    group_count: usize,
}

impl InboxItem {
    fn primary_category(&self) -> InboxCategory {
        self.categories
            .iter()
            .copied()
            .min_by_key(|category| category.priority())
            .unwrap_or(InboxCategory::Mention)
    }
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

struct InboxSnapshot {
    items: Vec<InboxItem>,
    profiles: HashMap<String, String>,
    channel_names: HashMap<String, String>,
    agent_pubkeys: BTreeSet<String>,
}

struct ContextSnapshot {
    events: Vec<EventView>,
    profiles: HashMap<String, String>,
    agent_pubkeys: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum InboxCategory {
    NeedsAction,
    Mention,
    AgentActivity,
    Activity,
}

impl InboxCategory {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "mentions" | "mention" => Some(Self::Mention),
            "needs_action" | "needs-action" => Some(Self::NeedsAction),
            "activity" => Some(Self::Activity),
            "agent_activity" | "agent-activity" => Some(Self::AgentActivity),
            _ => None,
        }
    }

    fn feed_type(self) -> &'static str {
        match self {
            Self::NeedsAction => "needs_action",
            Self::Mention => "mentions",
            Self::AgentActivity => "agent_activity",
            Self::Activity => "activity",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::NeedsAction => "Needs action",
            Self::Mention => "Mention",
            Self::AgentActivity => "Agent",
            Self::Activity => "Activity",
        }
    }

    fn priority(self) -> usize {
        match self {
            Self::NeedsAction => 0,
            Self::Mention => 1,
            Self::AgentActivity => 2,
            Self::Activity => 3,
        }
    }

    fn is_activity_like(self) -> bool {
        matches!(self, Self::Activity | Self::AgentActivity)
    }
}

fn draw_inbox(frame: &mut Frame<'_>, app: &InboxApp) {
    let area = frame.area();
    let bottom_height = if app.mode == InboxMode::Composing {
        5
    } else {
        3
    };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(bottom_height)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(vertical[0]);

    draw_inbox_list(frame, app, panes[0]);
    draw_thread(frame, app, panes[1]);
    draw_footer(frame, app, vertical[1]);
}

fn draw_inbox_list(frame: &mut Frame<'_>, app: &InboxApp, area: Rect) {
    let items: Vec<ListItem<'_>> = app
        .items
        .iter()
        .map(|item| {
            let sender = app.author_label(&item.event.pubkey);
            let channel = item
                .event
                .channel_id
                .as_ref()
                .and_then(|id| app.channel_names.get(id))
                .map(|name| format!("#{name}"))
                .unwrap_or_else(|| {
                    item.event
                        .channel_id
                        .as_ref()
                        .map(|id| format!("#{}", short_hex(id)))
                        .unwrap_or_else(|| "global".to_string())
                });
            let count = if item.group_count > 1 {
                format!(" +{}", item.group_count - 1)
            } else {
                String::new()
            };
            let preview = truncate_for_list(&single_line(&item.event.content), 86);
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(
                        item.primary_category().label(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(count),
                    Span::raw("  "),
                    Span::styled(sender, Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(channel, Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled(
                        format_timestamp(item.latest_activity_at),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(Span::raw(preview)),
            ]))
        })
        .collect();

    let title = if app.last_refresh_label.is_empty() {
        "Inbox".to_string()
    } else {
        format!("Inbox  {}", app.last_refresh_label)
    };
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.items.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_thread(frame: &mut Frame<'_>, app: &InboxApp, area: Rect) {
    let title = app
        .selected_item()
        .map(|item| format!("Thread  {}", short_hex(&item.event.id)))
        .unwrap_or_else(|| "Thread".to_string());
    let lines = if app.context.is_empty() {
        vec![Line::from(Span::styled(
            "Select an inbox item to view its context.",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        render_thread_lines(app)
    };

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false })
        .scroll((app.context_scroll, 0));
    frame.render_widget(paragraph, area);
}

fn draw_footer(frame: &mut Frame<'_>, app: &InboxApp, area: Rect) {
    let (title, body) = match app.mode {
        InboxMode::Browsing => (
            "Keys",
            format!(
                "j/k move  PgUp/PgDn scroll  x agents  r reply  R refresh  q quit    {}",
                app.status
            ),
        ),
        InboxMode::Composing => (
            "Reply",
            if app.draft.is_empty() {
                "Type a reply. Enter sends, Esc cancels, Ctrl-U clears.".to_string()
            } else {
                app.draft.clone()
            },
        ),
    };

    let style = if app.mode == InboxMode::Composing {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    let paragraph = Paragraph::new(body)
        .block(Block::default().title(title).borders(Borders::ALL))
        .style(style)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_thread_lines(app: &InboxApp) -> Vec<Line<'_>> {
    let selected_id = app.selected_item().map(|item| item.event.id.as_str());
    let mut lines = Vec::new();

    for event in &app.context {
        let is_selected = selected_id == Some(event.id.as_str());
        let is_agent = app.is_agent_pubkey(&event.pubkey);
        let is_collapsed_agent = is_agent && !app.expanded_agent_events.contains(&event.id);
        let author_style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        let mut header = vec![
            Span::styled(app.author_label(&event.pubkey), author_style),
            Span::raw("  "),
            Span::styled(
                format_timestamp(event.created_at),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(short_hex(&event.id), Style::default().fg(Color::DarkGray)),
        ];
        if is_agent {
            header.push(Span::raw("  "));
            header.push(Span::styled(
                if is_collapsed_agent {
                    "agent hidden"
                } else {
                    "agent"
                },
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(header));

        if is_collapsed_agent {
            lines.push(Line::from(Span::styled(
                "  [agent reply hidden] Press x to expand agent replies in this thread.",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            continue;
        }

        if event.content.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no content)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for content_line in event.content.lines() {
                lines.push(Line::from(Span::raw(format!("  {content_line}"))));
            }
        }
        lines.push(Line::from(""));
    }

    lines
}

impl InboxApp {
    fn author_label(&self, pubkey: &str) -> String {
        self.profiles
            .get(&pubkey.to_ascii_lowercase())
            .filter(|name| !name.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| short_hex(pubkey))
    }
}

async fn fetch_inbox_snapshot(
    client: &BuzzClient,
    limit: u32,
    categories: &[InboxCategory],
) -> Result<InboxSnapshot, CliError> {
    let items = fetch_inbox_items(client, limit, categories).await?;
    let pubkeys: Vec<String> = items.iter().map(|item| item.event.pubkey.clone()).collect();
    let profiles = fetch_profiles(client, &pubkeys).await.unwrap_or_default();
    let agent_pubkeys = fetch_agent_pubkeys(client, &pubkeys)
        .await
        .unwrap_or_default();

    let channel_ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.event.channel_id.clone())
        .collect();
    let channel_names = fetch_channel_names(client, &channel_ids)
        .await
        .unwrap_or_default();

    Ok(InboxSnapshot {
        items,
        profiles,
        channel_names,
        agent_pubkeys,
    })
}

async fn fetch_context_snapshot(
    client: &BuzzClient,
    selected: &EventView,
    channel_id: &str,
) -> Result<ContextSnapshot, CliError> {
    let events = fetch_thread_context(client, selected, channel_id).await?;
    let pubkeys: Vec<String> = events.iter().map(|event| event.pubkey.clone()).collect();
    let profiles = fetch_profiles(client, &pubkeys).await.unwrap_or_default();
    let agent_pubkeys = fetch_agent_pubkeys(client, &pubkeys)
        .await
        .unwrap_or_default();

    Ok(ContextSnapshot {
        events,
        profiles,
        agent_pubkeys,
    })
}

async fn fetch_inbox_items(
    client: &BuzzClient,
    limit: u32,
    categories: &[InboxCategory],
) -> Result<Vec<InboxItem>, CliError> {
    let mut entries = Vec::new();
    let my_pk = client.keys().public_key().to_hex().to_ascii_lowercase();

    for category in categories {
        let query_limit = if category.is_activity_like() {
            limit.saturating_mul(5).clamp(limit, 100)
        } else {
            limit
        };
        let filter = inbox_category_filter(*category, &my_pk, query_limit);
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse feed response: {e}")))?;
        let mut events: Vec<EventView> = events.iter().filter_map(event_from_value).collect();
        if category.is_activity_like() {
            events = filter_relevant_activity_events(client, events, &my_pk).await;
        }
        entries.extend(events.into_iter().map(|event| (*category, event)));
    }

    Ok(group_inbox_items(entries))
}

fn inbox_category_filter(
    category: InboxCategory,
    my_pubkey: &str,
    limit: u32,
) -> serde_json::Value {
    match category {
        InboxCategory::Mention => serde_json::json!({
            "kinds": GUI_MENTION_KINDS,
            "#p": [my_pubkey],
            "limit": limit,
        }),
        InboxCategory::NeedsAction => serde_json::json!({
            "kinds": GUI_NEEDS_ACTION_KINDS,
            "#p": [my_pubkey],
            "limit": limit.min(20),
        }),
        InboxCategory::Activity | InboxCategory::AgentActivity => serde_json::json!({
            "#p": [my_pubkey],
            "feed_types": [category.feed_type()],
            "limit": limit,
        }),
    }
}

async fn filter_relevant_activity_events(
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
            context_requests.push((root_id.clone(), channel_id.to_string(), event.clone()));
        }
        candidates.push((root_id, event));
    }

    let context_results = stream::iter(context_requests.into_iter().map(
        |(root_id, channel_id, event)| {
            let client = client.clone();
            async move {
                let context = fetch_thread_context(&client, &event, &channel_id)
                    .await
                    .ok();
                (root_id, context)
            }
        },
    ))
    .buffer_unordered(ACTIVITY_CONTEXT_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let context_by_root: HashMap<String, Vec<EventView>> = context_results
        .into_iter()
        .filter_map(|(root_id, context)| context.map(|context| (root_id, context)))
        .collect();

    let mut relevant = Vec::new();
    for (root_id, event) in candidates {
        let Some(context) = context_by_root.get(&root_id) else {
            continue;
        };
        if thread_context_relevant_to_user(&event, context, my_pubkey) {
            relevant.push(event);
        }
    }
    relevant
}

fn is_relevant_activity_candidate(event: &EventView, my_pubkey: &str) -> bool {
    event.pubkey.to_ascii_lowercase() != my_pubkey
        && is_thread_reply(&event.tags)
        && !has_mention_for_pubkey(event, my_pubkey)
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

async fn fetch_thread_context(
    client: &BuzzClient,
    selected: &EventView,
    channel_id: &str,
) -> Result<Vec<EventView>, CliError> {
    let root_id = thread_key(selected);
    let reply_filter = serde_json::json!({
        "kinds": THREAD_KINDS,
        "#h": [channel_id],
        "#e": [root_id],
        "limit": THREAD_CONTEXT_LIMIT,
    });
    let root_filter = serde_json::json!({
        "ids": [root_id],
        "limit": 1,
    });
    let selected_filter = serde_json::json!({
        "ids": [selected.id],
        "limit": 1,
    });

    let raw = client
        .query_multi(&[reply_filter, root_filter, selected_filter])
        .await?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse thread response: {e}")))?;
    let mut events = dedupe_events(
        std::iter::once(selected.clone())
            .chain(values.iter().filter_map(event_from_value))
            .collect(),
    );
    events.sort_by_key(|event| event.created_at);
    Ok(events)
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
        .map_err(|e| CliError::Other(format!("failed to parse profiles response: {e}")))?;

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
        .map_err(|e| CliError::Other(format!("failed to parse channels response: {e}")))?;

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
    let authors: Vec<String> = pubkeys
        .iter()
        .filter_map(|value| normalize_pubkey(value))
        .collect();
    let authors = unique_owned(authors);

    if !authors.is_empty() {
        let filter = serde_json::json!({
            "kinds": [0],
            "authors": authors,
            "limit": authors.len(),
        });
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw).map_err(|e| {
            CliError::Other(format!("failed to parse agent profiles response: {e}"))
        })?;
        for event in events {
            if !profile_has_valid_oa_owner_value(&event) {
                continue;
            }
            let Some(pubkey) = event
                .get("pubkey")
                .and_then(|value| value.as_str())
                .and_then(normalize_pubkey)
            else {
                continue;
            };
            agent_pubkeys.insert(pubkey);
        }
    }

    let owner_pubkey = client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex());
    if let Some(owner_pubkey) = normalize_pubkey(&owner_pubkey) {
        let filter = serde_json::json!({
            "kinds": [KIND_MANAGED_AGENT],
            "authors": [owner_pubkey],
            "limit": 200,
        });
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse managed-agent response: {e}")))?;
        for event in events {
            for tag in parse_tags(event.get("tags")) {
                if tag.first().map(|value| value.as_str()) != Some("d") {
                    continue;
                }
                let Some(pubkey) = tag.get(1).and_then(|value| normalize_pubkey(value)) else {
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
        .and_then(normalize_pubkey)
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

fn group_inbox_items(entries: Vec<(InboxCategory, EventView)>) -> Vec<InboxItem> {
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

    let mut items: Vec<InboxItem> = groups
        .into_iter()
        .map(|(id, group)| InboxItem {
            id,
            event: group.event,
            categories: group.categories,
            latest_activity_at: group.latest_activity_at,
            group_count: group.event_ids.len(),
        })
        .collect();
    items.sort_by_key(|item| Reverse(item.latest_activity_at));
    items
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
    thread_reference(tags).0.is_some() && !is_broadcast_reply(tags)
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
        let Some(id) = tag.get(1).filter(|id| is_event_id(id)) else {
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

fn parse_categories(input: Option<&str>) -> Result<Vec<InboxCategory>, CliError> {
    let values: Vec<InboxCategory> = match input {
        Some(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                InboxCategory::parse(value).ok_or_else(|| {
                    CliError::Usage(format!(
                        "invalid inbox type {value:?}; expected mentions, needs_action, activity, or agent_activity"
                    ))
                })
            })
            .collect::<Result<_, _>>()?,
        None => DEFAULT_TYPES.to_vec(),
    };

    if values.is_empty() {
        return Err(CliError::Usage("--types cannot be empty".into()));
    }

    let mut deduped = Vec::new();
    for value in values {
        if !deduped.contains(&value) {
            deduped.push(value);
        }
    }
    deduped.sort_by_key(|value| value.priority());
    Ok(deduped)
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
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_pubkey(value: &str) -> Option<String> {
    let value = value.trim();
    is_event_id(value).then(|| value.to_ascii_lowercase())
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

fn is_event_id(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn format_timestamp(unix_seconds: i64) -> String {
    Local
        .timestamp_opt(unix_seconds, 0)
        .single()
        .map(|dt| dt.format("%b %-d %H:%M").to_string())
        .unwrap_or_else(|| unix_seconds.to_string())
}

fn format_clock_now() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

fn terminal_err(err: io::Error) -> CliError {
    CliError::Other(format!("terminal error: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SELF_PK: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER_PK: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const CHANNEL_ID: &str = "80a927fd-a695-4895-971e-e49c974b0fff";

    fn event(id: &str, created_at: i64, tags: Vec<Vec<&str>>) -> EventView {
        let value = json!({
            "id": id,
            "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "kind": 40002,
            "content": "body",
            "created_at": created_at,
            "tags": tags,
        });
        event_from_value(&value).expect("valid event")
    }

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
    fn thread_key_prefers_root_marker() {
        let root = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let parent = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let event = event(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            1,
            vec![vec!["e", root, "", "root"], vec!["e", parent, "", "reply"]],
        );

        assert_eq!(thread_key(&event), root);
    }

    #[test]
    fn thread_key_uses_reply_marker_for_direct_replies() {
        let parent = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let event = event(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            1,
            vec![vec!["e", parent, "", "reply"]],
        );

        assert_eq!(thread_key(&event), parent);
    }

    #[test]
    fn group_inbox_items_keeps_latest_event_per_thread() {
        let root = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let older = event(root, 10, vec![]);
        let newer = event(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            20,
            vec![vec!["e", root, "", "reply"]],
        );

        let grouped = group_inbox_items(vec![
            (InboxCategory::Mention, older),
            (InboxCategory::NeedsAction, newer),
        ]);

        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].event.created_at, 20);
        assert_eq!(grouped[0].group_count, 2);
        assert_eq!(
            grouped[0].categories,
            vec![InboxCategory::NeedsAction, InboxCategory::Mention]
        );
    }

    #[test]
    fn parse_categories_defaults_to_gui_all_inbox() {
        assert_eq!(
            parse_categories(None).expect("defaults"),
            vec![
                InboxCategory::NeedsAction,
                InboxCategory::Mention,
                InboxCategory::Activity
            ]
        );
    }

    #[test]
    fn mention_filter_matches_desktop_feed_kinds() {
        let filter = inbox_category_filter(InboxCategory::Mention, "abc123", 50);
        assert_eq!(filter.get("#p"), Some(&json!(["abc123"])));
        assert_eq!(filter.get("feed_types"), None);

        let kinds = filter
            .get("kinds")
            .and_then(serde_json::Value::as_array)
            .expect("mention kinds");
        assert!(kinds.contains(&json!(KIND_TEXT_NOTE)));
        assert!(kinds.contains(&json!(KIND_STREAM_MESSAGE)));
        assert!(kinds.contains(&json!(KIND_STREAM_MESSAGE_V2)));
        assert!(kinds.contains(&json!(KIND_FORUM_POST)));
        assert!(kinds.contains(&json!(KIND_FORUM_COMMENT)));
    }

    #[test]
    fn needs_action_filter_matches_desktop_feed_kinds() {
        let filter = inbox_category_filter(InboxCategory::NeedsAction, "abc123", 50);
        assert_eq!(filter.get("#p"), Some(&json!(["abc123"])));
        assert_eq!(filter.get("limit"), Some(&json!(20)));

        let kinds = filter
            .get("kinds")
            .and_then(serde_json::Value::as_array)
            .expect("needs-action kinds");
        assert!(kinds.contains(&json!(KIND_WORKFLOW_APPROVAL_REQUESTED)));
        assert!(kinds.contains(&json!(KIND_WORKFLOW_APPROVAL_GRANTED)));
        assert!(kinds.contains(&json!(KIND_WORKFLOW_APPROVAL_DENIED)));
    }

    #[test]
    fn activity_filter_uses_feed_extension() {
        let filter = inbox_category_filter(InboxCategory::Activity, "abc123", 50);
        assert_eq!(filter.get("#p"), Some(&json!(["abc123"])));
        assert_eq!(filter.get("kinds"), None);
        assert_eq!(filter.get("feed_types"), Some(&json!(["activity"])));
    }

    #[test]
    fn activity_candidate_requires_external_non_mention_thread_reply() {
        let root = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let parent = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let threaded = event_with_pubkey(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            OTHER_PK,
            1,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
            ],
        );
        assert!(is_relevant_activity_candidate(&threaded, SELF_PK));

        let top_level = event_with_pubkey(
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            OTHER_PK,
            1,
            vec![vec!["h", CHANNEL_ID]],
        );
        assert!(!is_relevant_activity_candidate(&top_level, SELF_PK));

        let self_authored = event_with_pubkey(
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            SELF_PK,
            1,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
            ],
        );
        assert!(!is_relevant_activity_candidate(&self_authored, SELF_PK));

        let direct_mention = event_with_pubkey(
            "9999999999999999999999999999999999999999999999999999999999999999",
            OTHER_PK,
            1,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["p", SELF_PK],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
            ],
        );
        assert!(!is_relevant_activity_candidate(&direct_mention, SELF_PK));

        let broadcast = event_with_pubkey(
            "8888888888888888888888888888888888888888888888888888888888888888",
            OTHER_PK,
            1,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
                vec!["broadcast", "1"],
            ],
        );
        assert!(!is_relevant_activity_candidate(&broadcast, SELF_PK));
    }

    #[test]
    fn activity_context_requires_self_participation_or_thread_mention() {
        let root = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let parent = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let candidate = event_with_pubkey(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            OTHER_PK,
            3,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["e", root, "", "root"],
                vec!["e", parent, "", "reply"],
            ],
        );
        let root_by_self = event_with_pubkey(root, SELF_PK, 1, vec![vec!["h", CHANNEL_ID]]);
        assert!(thread_context_relevant_to_user(
            &candidate,
            &[root_by_self.clone(), candidate.clone()],
            SELF_PK,
        ));

        let prior_self_reply = event_with_pubkey(
            parent,
            SELF_PK,
            2,
            vec![vec!["h", CHANNEL_ID], vec!["e", root, "", "reply"]],
        );
        assert!(thread_context_relevant_to_user(
            &candidate,
            &[prior_self_reply, candidate.clone()],
            SELF_PK,
        ));

        let prior_mention = event_with_pubkey(
            parent,
            OTHER_PK,
            2,
            vec![
                vec!["h", CHANNEL_ID],
                vec!["p", SELF_PK],
                vec!["e", root, "", "reply"],
            ],
        );
        assert!(thread_context_relevant_to_user(
            &candidate,
            &[prior_mention, candidate.clone()],
            SELF_PK,
        ));

        let unrelated_root = event_with_pubkey(root, OTHER_PK, 1, vec![vec!["h", CHANNEL_ID]]);
        assert!(!thread_context_relevant_to_user(
            &candidate,
            &[unrelated_root, candidate.clone()],
            SELF_PK,
        ));
    }

    #[test]
    fn render_thread_lines_hides_agent_replies_by_default() {
        let root = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let agent_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let root_event = event_with_pubkey(root, SELF_PK, 1, vec![vec!["h", CHANNEL_ID]]);
        let mut agent_reply = event_with_pubkey(
            agent_id,
            OTHER_PK,
            2,
            vec![vec!["h", CHANNEL_ID], vec!["e", root, "", "reply"]],
        );
        agent_reply.content = "agent body should be hidden".to_string();

        let mut app = InboxApp::new(10, 0, vec![InboxCategory::Mention]);
        app.items.push(InboxItem {
            id: root.to_string(),
            event: root_event.clone(),
            categories: vec![InboxCategory::Mention],
            latest_activity_at: 2,
            group_count: 1,
        });
        app.context = vec![root_event, agent_reply];
        app.agent_pubkeys.insert(OTHER_PK.to_string());

        let collapsed = rendered_text(&render_thread_lines(&app));
        assert!(collapsed.contains("[agent reply hidden]"));
        assert!(!collapsed.contains("agent body should be hidden"));

        app.expanded_agent_events.insert(agent_id.to_string());
        let expanded = rendered_text(&render_thread_lines(&app));
        assert!(expanded.contains("agent body should be hidden"));
    }

    #[test]
    fn toggle_visible_agent_replies_expands_and_collapses_thread_agents() {
        let root = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let agent_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let root_event = event_with_pubkey(root, SELF_PK, 1, vec![vec!["h", CHANNEL_ID]]);
        let agent_reply = event_with_pubkey(
            agent_id,
            OTHER_PK,
            2,
            vec![vec!["h", CHANNEL_ID], vec!["e", root, "", "reply"]],
        );

        let mut app = InboxApp::new(10, 0, vec![InboxCategory::Mention]);
        app.context = vec![root_event, agent_reply];
        app.agent_pubkeys.insert(OTHER_PK.to_string());

        app.toggle_visible_agent_replies();
        assert!(app.expanded_agent_events.contains(agent_id));
        assert!(app.status.contains("Expanded 1 agent reply."));

        app.toggle_visible_agent_replies();
        assert!(!app.expanded_agent_events.contains(agent_id));
        assert!(app.status.contains("Collapsed 1 agent reply."));
    }
}
