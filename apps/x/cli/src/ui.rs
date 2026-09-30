//! `x ui` — the terminal arm of the delivery-surface comparison.
//!
//! Same registry, same transport, same renderer as the CLI: the only thing
//! that differs is where the output goes. That is deliberate — an arm of a
//! benchmark that reimplements the data path measures the reimplementation,
//! not the surface.
//!
//! Scope is explicit: the TUI *browses* every operation and *executes* the
//! ones that need no input (a GET with no path parameters). For anything that
//! takes arguments it shows the exact `x …` command line to run. A half-built
//! input form would be a worse editor than the shell the user is already in.

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, text::Text};

use crate::Globals;
use crate::exec::Transport;
use crate::model::Registry;
use crate::render;
use crate::spec::Credential;

/// Poll interval. Long enough that an idle TUI is not a busy loop (this is a
/// measured arm: idle CPU is one of the numbers), short enough to feel live.
const TICK: Duration = Duration::from_millis(200);

struct App<'a> {
    registry: &'a Registry,
    /// Indices into `registry.ops`, filtered by the focused API.
    visible: Vec<usize>,
    selected: usize,
    output: String,
    status: String,
    /// What credential this run holds, shown permanently: a TUI that answers
    /// 401 without saying whether it even had a credential is the reason this
    /// line exists.
    credential: String,
    scroll: u16,
}

impl<'a> App<'a> {
    fn new(registry: &'a Registry, focus: Option<String>, globals: &Globals) -> Self {
        let visible = registry
            .ops
            .iter()
            .enumerate()
            .filter(|(_, op)| match &focus {
                Some(key) => registry.apis[op.api].key == *key,
                None => true,
            })
            .map(|(i, _)| i)
            .collect();

        let credential = match (globals.token.is_some(), globals.session.is_some()) {
            (true, _) => "bearer token".to_owned(),
            (false, true) => "session cookie".to_owned(),
            (false, false) => "no credential".to_owned(),
        };

        Self {
            registry,
            visible,
            selected: 0,
            output: String::new(),
            status: "enter run · j/k move · q quit".to_owned(),
            credential,
            scroll: 0,
        }
    }

    fn current(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }

    fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        self.selected = match delta {
            d if d < 0 => self.selected.saturating_sub(d.unsigned_abs()),
            d => (self.selected + d as usize).min(last),
        };
        self.scroll = 0;
    }
}

pub async fn run(
    registry: &Registry,
    focus: Option<String>,
    globals: &Globals,
) -> eyre::Result<()> {
    if let Some(key) = &focus
        && !registry.apis.iter().any(|a| a.key == *key)
    {
        let keys: Vec<&str> = registry.apis.iter().map(|a| a.key.as_str()).collect();
        return Err(eyre::eyre!("no API `{key}`; loaded: {}", keys.join(", ")));
    }

    let transport = Transport::new(
        globals.timeout,
        globals.token.clone(),
        globals.session.clone(),
        globals.headers.clone(),
    )?;
    let mut app = App::new(registry, focus, globals);
    let mut terminal = ratatui::init();

    let outcome = event_loop(&mut terminal, &mut app, &transport).await;
    ratatui::restore();
    outcome
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App<'_>,
    transport: &Transport,
) -> eyre::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;

        if !event::poll(TICK)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('j') | KeyCode::Down => app.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => app.move_by(-1),
            KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
            KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
            KeyCode::Enter => execute(app, transport).await,
            _ => {}
        }
    }
}

async fn execute(app: &mut App<'_>, transport: &Transport) {
    let Some(index) = app.current() else { return };
    let op = &app.registry.ops[index];

    if op.method != "get" || op.arity() > 0 {
        app.status = format!("run: {}", command_line(app.registry, index));
        app.output = String::new();
        return;
    }

    let invocation = crate::command::Invocation {
        op: index,
        url: app.registry.apis[op.api].url(&op.path),
        method: "GET".to_owned(),
        query: Vec::new(),
        body: None,
        credentials: op.credentials.clone(),
    };

    app.scroll = 0;
    match transport.send(&invocation).await {
        Ok(outcome) => {
            app.status = format!("{} · {} bytes", outcome.status, outcome.bytes);
            app.output = outcome
                .body
                .as_ref()
                .map(|b| render::render(b, render::Format::Table, None).unwrap_or_default())
                .unwrap_or_else(|| "(no content)".to_owned());
        }
        Err(error) => {
            app.status = "error".to_owned();
            app.output = error.to_string();
        }
    }
}

/// The shell command equivalent to the highlighted row — the TUI's answer for
/// operations it will not run itself.
fn command_line(registry: &Registry, index: usize) -> String {
    let op = &registry.ops[index];
    let mut parts = vec!["x".to_owned(), op.verb.clone(), op.resource.clone()];
    for param in &op.path_params {
        parts.push(format!("<{}>", param.name.to_uppercase()));
    }
    for field in op.body_fields.iter().filter(|f| f.required) {
        parts.push(format!("--{} <{}>", field.name, field.name.to_uppercase()));
    }
    parts.join(" ")
}

fn draw(frame: &mut Frame, app: &App) {
    let [list_area, output_area] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .areas(frame.area());
    let [output_body, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(output_area);

    // Keep the cursor row visible without a stateful widget: render a window
    // of rows around the selection.
    let height = list_area.height.saturating_sub(2) as usize;
    let start = app.selected.saturating_sub(height.saturating_sub(1) / 2);
    let rows: Vec<Line> = app
        .visible
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(position, index)| {
            let op = &app.registry.ops[*index];
            let label = format!(
                "{:<8} {:<16} {:<6} {}",
                op.verb,
                op.resource,
                op.method.to_uppercase(),
                app.registry.apis[op.api].key
            );
            if position == app.selected {
                Line::from(Span::styled(
                    label,
                    Style::default().add_modifier(Modifier::REVERSED),
                ))
            } else {
                Line::from(label)
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(Text::from(rows)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" operations ({}) ", app.visible.len())),
        ),
        list_area,
    );

    let title = app
        .current()
        .map(|i| format!(" {} ", app.registry.ops[i].operation_id))
        .unwrap_or_else(|| " output ".to_owned());

    frame.render_widget(
        Paragraph::new(app.output.as_str())
            .block(Block::default().borders(Borders::ALL).title(title))
            .scroll((app.scroll, 0)),
        output_body,
    );

    // What the highlighted operation needs, next to what this run holds: the
    // two facts a 401 in the output pane is otherwise missing.
    let requirement = app
        .current()
        .map(|i| match app.registry.ops[i].credentials.as_slice() {
            [] => "operation declares no credential".to_owned(),
            declared => format!(
                "needs {}",
                declared
                    .iter()
                    .map(Credential::label)
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
        })
        .unwrap_or_default();

    frame.render_widget(
        Paragraph::new(app.status.as_str())
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" x holds {} · {requirement} ", app.credential)),
            ),
        status_area,
    );
}
