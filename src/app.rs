use crate::{
    clipboard::Clipboard,
    input::{self, Input},
    vault::{EntryData, Session},
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use keepass::db::EntryId;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

const BLACK: Color = Color::Rgb(0, 0, 0);
const WHITE: Color = Color::Rgb(255, 255, 255);
const LABELS: [&str; 5] = ["Title", "Username", "Password", "URL", "Notes"];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Browse,
    Lock,
    Quit,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Browse,
    Search,
    Edit,
    Delete,
    Discard(Action),
}

struct Form {
    id: Option<EntryId>,
    fields: [Input; 5],
    field: usize,
    dirty: bool,
}
impl Form {
    fn new(id: Option<EntryId>, data: EntryData) -> Self {
        Self {
            id,
            fields: [
                Input::from(&data.title),
                Input::from(&data.username),
                Input::from(&data.password),
                Input::from(&data.url),
                Input::from(&data.notes),
            ],
            field: 0,
            dirty: false,
        }
    }
    fn data(&self) -> EntryData {
        EntryData {
            title: Zeroizing::new(self.fields[0].text().into()),
            username: Zeroizing::new(self.fields[1].text().into()),
            password: Zeroizing::new(self.fields[2].text().into()),
            url: Zeroizing::new(self.fields[3].text().into()),
            notes: Zeroizing::new(self.fields[4].text().into()),
        }
    }
}

struct App {
    path: PathBuf,
    creating: bool,
    master: Input,
    confirmation: Input,
    auth_field: usize,
    session: Option<Session>,
    entries: Vec<(EntryId, String)>,
    visible: Vec<usize>,
    selected: usize,
    query: Input,
    form: Option<Form>,
    mode: Mode,
    clipboard: Clipboard,
    reveal_until: Option<Instant>,
    last_activity: Instant,
    lock_after: Duration,
    status: String,
    exit: bool,
}

impl App {
    fn new(path: PathBuf, creating: bool, lock_after: Duration) -> Self {
        Self {
            path,
            creating,
            master: Input::default(),
            confirmation: Input::default(),
            auth_field: 0,
            session: None,
            entries: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            query: Input::default(),
            form: None,
            mode: Mode::Browse,
            clipboard: Clipboard::new(),
            reveal_until: None,
            last_activity: Instant::now(),
            lock_after,
            status: "Experimental MVP — use test data and keep encrypted backups.".into(),
            exit: false,
        }
    }
    fn unlock(&mut self) {
        if self.creating && self.master.text() != self.confirmation.text() {
            self.status = "Master passwords do not match".into();
            self.master.clear();
            self.confirmation.clear();
            self.auth_field = 0;
            return;
        }
        let result = if self.creating {
            Session::create(&self.path, self.master.text())
        } else {
            Session::unlock(&self.path, self.master.text())
        };
        self.master.clear();
        self.confirmation.clear();
        self.auth_field = 0;
        match result {
            Ok(session) => {
                self.path = session.path().to_owned();
                let readonly = session.readonly;
                self.session = Some(session);
                self.creating = false;
                self.mode = Mode::Browse;
                self.last_activity = Instant::now();
                self.refresh(None);
                self.status=if readonly { "Foreign vault opened read-only. Editing is restricted to TermVault-created vaults." } else { "Unlocked — n new · Enter edit · g new generated password · Ctrl+L lock" }.into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }
    fn clear_entries(&mut self) {
        for (_, title) in &mut self.entries {
            title.zeroize();
        }
        self.entries.clear();
        self.visible.clear();
        self.selected = 0;
    }
    fn refresh(&mut self, preferred: Option<EntryId>) {
        self.clear_entries();
        if let Some(session) = &self.session {
            self.entries = session.entries();
        }
        self.filter(preferred);
    }
    fn filter(&mut self, preferred: Option<EntryId>) {
        let query = self.query.text().to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (_, title))| title.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        self.selected = preferred
            .and_then(|id| self.visible.iter().position(|i| self.entries[*i].0 == id))
            .unwrap_or(0);
        self.reveal_until = None;
    }
    fn selected_id(&self) -> Option<EntryId> {
        self.visible.get(self.selected).map(|i| self.entries[*i].0)
    }
    fn lock(&mut self) {
        self.form = None;
        self.session = None;
        self.clear_entries();
        self.query.clear();
        self.master.clear();
        self.confirmation.clear();
        self.reveal_until = None;
        self.clipboard.clear();
        self.mode = Mode::Browse;
        self.status = "Locked. Entry drafts were discarded; saved records remain encrypted.".into();
    }
    fn action(&mut self, action: Action) {
        match action {
            Action::Browse => {
                self.form = None;
                self.mode = Mode::Browse;
                self.reveal_until = None;
            }
            Action::Lock => self.lock(),
            Action::Quit => {
                self.lock();
                self.exit = true;
            }
        }
    }
    fn leave(&mut self, action: Action) {
        self.reveal_until = None;
        if self.form.as_ref().is_some_and(|form| form.dirty) {
            self.mode = Mode::Discard(action);
        } else {
            self.action(action);
        }
    }
    fn edit(&mut self, new: bool) {
        let Some(session) = &self.session else {
            return;
        };
        if session.readonly {
            self.status =
                "Read-only vault; create a separate TermVault vault to edit records.".into();
            return;
        }
        let id = if new { None } else { self.selected_id() };
        let data = if new {
            EntryData::default()
        } else {
            let Some(data) = id.and_then(|id| session.entry(id)) else {
                return;
            };
            data
        };
        self.form = Some(Form::new(id, data));
        self.mode = Mode::Edit;
        self.reveal_until = None;
        self.status =
            "Tab selects fields · Ctrl+S saves · Ctrl+G generates · Ctrl+U clears a field".into();
    }
    fn generate(&mut self) {
        match input::generate(24) {
            Ok(password) => {
                if let Some(form) = &mut self.form {
                    form.fields[2] = Input::from(&password);
                    form.dirty = true;
                    self.reveal_until = None;
                    self.status =
                        "Generated 24-character password (hidden). Ctrl+S saves the entry.".into();
                }
            }
            Err(error) => self.status = error.to_string(),
        }
    }
    fn save(&mut self) -> bool {
        let Some(form) = &self.form else {
            return false;
        };
        if !form.dirty && form.id.is_some() {
            self.action(Action::Browse);
            return true;
        }
        let data = form.data();
        let id = form.id;
        let Some(session) = &mut self.session else {
            return false;
        };
        match session.update(id, &data) {
            Ok(id) => {
                self.form = None;
                self.mode = Mode::Browse;
                self.query.clear();
                self.refresh(Some(id));
                self.status =
                    "Saved. An encrypted backup of the previous vault was retained.".into();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        }
    }
    fn copy(&mut self, username: bool) {
        let data = self
            .selected_id()
            .and_then(|id| self.session.as_ref()?.entry(id));
        if let Some(data) = data {
            let text = if username {
                &*data.username
            } else {
                &*data.password
            };
            self.status=match self.clipboard.copy(text,Instant::now()){Ok(())=>"Copied. Best-effort clipboard clearing in 20 seconds; clipboard history is not a secure store.".into(),Err(error)=>error.to_string()};
        }
    }
    fn tick(&mut self, now: Instant) -> bool {
        self.clipboard.tick(now);
        if self.session.is_some()
            && now.saturating_duration_since(self.last_activity) >= self.lock_after
        {
            self.lock();
            self.status =
                "Auto-locked after inactivity. Unsaved entry drafts were discarded.".into();
            return true;
        }
        if self.reveal_until.is_some_and(|deadline| now >= deadline) {
            self.reveal_until = None;
            return true;
        }
        false
    }
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        self.last_activity = Instant::now();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL)
            && !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SUPER);
        let letter = match key.code {
            KeyCode::Char(c) => Some(match c.to_lowercase().next().unwrap_or(c) {
                'ы' => 's',
                'д' => 'l',
                'й' => 'q',
                'п' => 'g',
                c => c,
            }),
            _ => None,
        };
        if ctrl && letter == Some('q') {
            self.leave(Action::Quit);
            return;
        }
        if self.session.is_none() {
            match key.code {
                KeyCode::Esc => self.exit = true,
                KeyCode::Tab | KeyCode::BackTab if self.creating => {
                    self.auth_field = 1 - self.auth_field
                }
                KeyCode::Enter if self.creating && self.auth_field == 0 => self.auth_field = 1,
                KeyCode::Enter => self.unlock(),
                _ => {
                    let field = if self.auth_field == 0 {
                        &mut self.master
                    } else {
                        &mut self.confirmation
                    };
                    field.key(key, false);
                }
            }
            return;
        }
        if ctrl && letter == Some('l') {
            self.leave(Action::Lock);
            return;
        }
        match self.mode {
            Mode::Edit => {
                if ctrl && letter == Some('s') {
                    self.save();
                } else if ctrl && letter == Some('g') {
                    self.generate();
                } else if ctrl && letter == Some('p') {
                    self.reveal_until = Some(Instant::now() + Duration::from_secs(10));
                } else if key.code == KeyCode::Esc {
                    self.leave(Action::Browse);
                } else if let Some(form) = &mut self.form {
                    match key.code {
                        KeyCode::Tab => form.field = (form.field + 1) % 5,
                        KeyCode::BackTab => form.field = (form.field + 4) % 5,
                        KeyCode::Enter if form.field == 4 => {
                            form.dirty |= form.fields[4].insert("\n", true);
                        }
                        KeyCode::Enter => form.field = (form.field + 1) % 5,
                        _ => form.dirty |= form.fields[form.field].key(key, form.field == 4),
                    }
                }
            }
            Mode::Discard(action) => match key.code {
                KeyCode::Char('d') => self.action(action),
                KeyCode::Char('s') => {
                    if self.save() {
                        self.action(action);
                    }
                }
                KeyCode::Esc | KeyCode::Char('c') => self.mode = Mode::Edit,
                _ => {}
            },
            Mode::Delete => match key.code {
                KeyCode::Esc => self.mode = Mode::Browse,
                KeyCode::Char('d') => {
                    if let Some(id) = self.selected_id() {
                        let result = self.session.as_mut().unwrap().delete(id);
                        match result {
                            Ok(()) => {
                                self.refresh(None);
                                self.status="Entry removed. Encrypted backups still contain its previous versions.".into();
                                self.mode = Mode::Browse;
                            }
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                }
                _ => {}
            },
            Mode::Search => match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.filter(None);
                    self.mode = Mode::Browse;
                }
                KeyCode::Enter => self.mode = Mode::Browse,
                _ => {
                    if self.query.key(key, false) {
                        self.filter(None);
                    }
                }
            },
            Mode::Browse => match key.code {
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.reveal_until = None;
                }
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.visible.len().saturating_sub(1));
                    self.reveal_until = None;
                }
                KeyCode::Char('q') => self.action(Action::Quit),
                KeyCode::Char('/') => self.mode = Mode::Search,
                KeyCode::Esc => {
                    self.query.clear();
                    self.filter(None);
                    self.reveal_until = None;
                }
                KeyCode::Char('n') => self.edit(true),
                KeyCode::Char('g') => {
                    self.edit(true);
                    if self.mode == Mode::Edit {
                        self.generate();
                    }
                }
                KeyCode::Enter => self.edit(false),
                KeyCode::Char('d') if self.selected_id().is_some() => self.mode = Mode::Delete,
                KeyCode::Char('p') => {
                    self.reveal_until = Some(Instant::now() + Duration::from_secs(10))
                }
                KeyCode::Char('c') => self.copy(false),
                KeyCode::Char('u') => self.copy(true),
                _ => {}
            },
        }
    }
    fn paste(&mut self, text: &str) {
        self.last_activity = Instant::now();
        if self.session.is_none() {
            let field = if self.auth_field == 0 {
                &mut self.master
            } else {
                &mut self.confirmation
            };
            if !field.insert(text, false) {
                self.status = "Paste rejected: unsupported characters or input too large".into();
            }
            return;
        }
        match self.mode {
            Mode::Edit => {
                if let Some(form) = &mut self.form {
                    let normalized = Zeroizing::new(text.replace("\r\n", "\n"));
                    if form.fields[form.field].insert(&normalized, form.field == 4) {
                        form.dirty = true;
                    } else {
                        self.status =
                            "Paste rejected: unsupported characters or input too large".into();
                    }
                }
            }
            Mode::Search if self.query.insert(text, false) => self.filter(None),
            _ => {}
        }
    }
    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(base()), area);
        if area.width < 60 || area.height < 20 {
            let text = if matches!(self.mode, Mode::Discard(_)) {
                "Unsaved entry\n[s] Save  [d] Discard\n[c/Esc] Cancel"
            } else {
                "TermVault — resize to 60 × 20\nCtrl+L locks · Ctrl+Q exits"
            };
            frame.render_widget(
                Paragraph::new(format!("{text}\n{}", input::display(&self.status)))
                    .wrap(Wrap { trim: false })
                    .style(base()),
                area,
            );
            return;
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(5),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);
        let state = if self.session.is_some() {
            "UNLOCKED"
        } else {
            "LOCKED"
        };
        frame.render_widget(
            Paragraph::new(format!(" ◆ TermVault  ·  {state}  ·  EXPERIMENTAL MVP"))
                .style(base().add_modifier(Modifier::BOLD)),
            rows[0],
        );
        if self.session.is_none() {
            let title = if self.creating {
                " Create NEW vault · keep a backup of your master passphrase "
            } else {
                " Unlock vault "
            };
            let mut text = vec![
                Line::from(input::display(&self.path.display().to_string())),
                Line::from(""),
                Line::from("Master password (never shown):"),
                Line::from(self.master.rendered(true, self.auth_field == 0)),
            ];
            if self.creating {
                text.extend([
                    Line::from(""),
                    Line::from("Confirm master password:"),
                    Line::from(self.confirmation.rendered(true, self.auth_field == 1)),
                    Line::from(""),
                    Line::from("Use a strong passphrase of at least 12 characters."),
                ]);
            }
            text.extend([
                Line::from(""),
                Line::from("Enter unlocks / creates · Tab changes field · Esc exits"),
                Line::from(
                    "No recovery backdoor. Never pass a master password in shell arguments.",
                ),
            ]);
            frame.render_widget(
                Paragraph::new(text)
                    .block(panel(title))
                    .wrap(Wrap { trim: false }),
                rows[1],
            );
        } else if let Some(form) = &self.form {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(3),
                ])
                .split(rows[1]);
            for i in 0..5 {
                let active = form.field == i;
                let masked = i == 2 && self.reveal_until.is_none();
                frame.render_widget(
                    Paragraph::new(form.fields[i].rendered(masked, active))
                        .block(panel(&format!(
                            " {}{}{} ",
                            if active { "› " } else { "" },
                            LABELS[i],
                            if form.dirty && i == 0 {
                                " · unsaved"
                            } else {
                                ""
                            }
                        )))
                        .wrap(Wrap { trim: false }),
                    parts[i],
                );
            }
        } else {
            let parts = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
                .split(rows[1]);
            let items: Vec<ListItem> = self
                .visible
                .iter()
                .map(|i| ListItem::new(self.entries[*i].1.as_str()))
                .collect();
            frame.render_stateful_widget(
                List::new(items)
                    .block(panel(&format!(" Entries · {} ", self.visible.len())))
                    .highlight_style(invert())
                    .highlight_symbol("› "),
                parts[0],
                &mut ListState::default()
                    .with_selected((!self.visible.is_empty()).then_some(self.selected)),
            );
            let data = self
                .selected_id()
                .and_then(|id| self.session.as_ref()?.entry(id));
            let text = if let Some(data) = data {
                let password = if self.reveal_until.is_some() {
                    input::display(&data.password)
                } else {
                    "••••••••  [p reveals for 10 seconds]".into()
                };
                format!(
                    "{}\n\nUsername: {}\n\nPassword: {}\n\nURL: {}\n\nNotes:\n{}",
                    input::display(&data.title),
                    input::display(&data.username),
                    password,
                    input::display(&data.url),
                    input::display(&data.notes)
                )
            } else {
                "No entries.\n\nPress n to add a record, or g to start with a generated password.\nYour data is saved only when you press Ctrl+S.\n\nThis MVP is not audited. Use test credentials first.".into()
            };
            frame.render_widget(
                Paragraph::new(text)
                    .block(panel(" Entry · passwords hidden by default "))
                    .wrap(Wrap { trim: false }),
                parts[1],
            );
        }
        frame.render_widget(
            Paragraph::new(input::display(&self.status))
                .wrap(Wrap { trim: false })
                .style(base()),
            rows[2],
        );
        frame.render_widget(
            Paragraph::new(if self.form.is_some() {
                " Tab field · Ctrl+S save · Ctrl+G generate · Ctrl+P reveal · Esc back "
            } else {
                " ↑↓ select · Enter edit · n new · / search · c copy · p reveal · Ctrl+L lock "
            })
            .style(base()),
            rows[3],
        );
        match self.mode {
            Mode::Search=>popup(frame,&format!("{}\nEnter applies · Esc clears",self.query.rendered(false,true))," Filter entry titles ",5),
            Mode::Discard(_)=>popup(frame,"Unsaved changes\n\n[s] Save and continue   [d] Discard   [c/Esc] Cancel\nInactivity auto-lock discards unsaved drafts."," Unsaved entry ",7),
            Mode::Delete=>popup(frame,"Delete the selected entry?\n\n[d] Delete   [Esc] Cancel\nEncrypted backups retain previous versions; this is not secure erasure."," Confirm deletion ",7),
            _=>{}
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.lock();
    }
}
fn base() -> Style {
    Style::default().fg(WHITE).bg(BLACK)
}
fn invert() -> Style {
    Style::default()
        .fg(BLACK)
        .bg(WHITE)
        .add_modifier(Modifier::BOLD)
}
fn panel(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_owned())
        .style(base())
}
fn popup(frame: &mut Frame, text: &str, title: &str, height: u16) {
    let area = frame.area();
    let width = 82.min(area.width);
    let height = height.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(title))
            .wrap(Wrap { trim: false }),
        rect,
    );
}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

pub fn run(path: PathBuf, creating: bool, lock_after: Duration) -> Result<()> {
    let mut app = App::new(path, creating, lock_after);
    enable_raw_mode()?;
    let _guard = Guard;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    // Deliberately omit panic payloads: upstream formatting could contain secrets.
    std::panic::set_hook(Box::new(|_| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        eprintln!("TermVault stopped unexpectedly. Unsaved drafts may be lost; encrypted backups are retained.");
    }));
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    terminal.draw(|frame| app.draw(frame))?;
    while !app.exit {
        let mut changed = app.tick(Instant::now());
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => app.key(key),
                Event::Paste(text) => {
                    let text = Zeroizing::new(text);
                    app.paste(&text);
                }
                _ => {}
            }
            changed = true;
        }
        if changed && !app.exit {
            terminal.draw(|frame| app.draw(frame))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut app = App::new(path, true, Duration::from_secs(120));
        app.master.insert("test master passphrase", false);
        app.confirmation.insert("test master passphrase", false);
        app.unlock();
        assert!(app.session.is_some(), "{}", app.status);
        (root, app)
    }
    fn screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    #[test]
    fn passwords_are_masked_in_unlock_browse_and_form() {
        let (_root, mut app) = app();
        app.edit(true);
        let form = app.form.as_mut().unwrap();
        form.fields[0] = Input::from("Example");
        form.fields[2] = Input::from("hidden-secret-XYZ");
        form.dirty = true;
        assert!(!screen(&app).contains("hidden-secret-XYZ"));
        assert!(app.save());
        assert!(!screen(&app).contains("hidden-secret-XYZ"));
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(screen(&app).contains("hidden-secret-XYZ"));
        app.tick(Instant::now() + Duration::from_secs(11));
        assert!(!screen(&app).contains("hidden-secret-XYZ"));
        app.lock();
        app.master.insert("MASTER_MUST_BE_HIDDEN", false);
        let output = screen(&app);
        assert!(!output.contains("MASTER_MUST_BE_HIDDEN"));
        assert!(!output.contains("Example"));
    }
    #[test]
    fn auto_lock_discards_draft_and_manual_exit_requires_confirmation() {
        let (_root, mut app) = app();
        app.edit(true);
        app.form.as_mut().unwrap().dirty = true;
        app.leave(Action::Quit);
        assert_eq!(app.mode, Mode::Discard(Action::Quit));
        assert!(!app.exit);
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Edit);
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.session.is_none());
        assert!(app.form.is_none());
        assert!(app.entries.is_empty());
    }
    #[test]
    fn failed_save_keeps_draft_and_never_shows_field_values_in_error() {
        let (_root, mut app) = app();
        app.edit(true);
        let form = app.form.as_mut().unwrap();
        form.fields[2] = Input::from("DO_NOT_PRINT_PASSWORD");
        form.dirty = true;
        assert!(!app.save());
        assert!(app.form.is_some());
        assert!(!app.status.contains("DO_NOT_PRINT_PASSWORD"));
    }

    #[test]
    fn cancelled_creation_does_not_write_and_confirmation_is_masked() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing/vault.kdbx");
        let mut app = App::new(path.clone(), true, Duration::from_secs(120));
        app.master.insert("first passphrase", false);
        app.confirmation.insert("different passphrase", false);
        let view = screen(&app);
        assert!(!view.contains("first passphrase"));
        assert!(!view.contains("different passphrase"));
        app.unlock();
        assert!(app.session.is_none());
        assert!(app.master.text().is_empty());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn small_terminal_still_shows_unsaved_confirmation() {
        let (_root, mut app) = app();
        app.edit(true);
        app.form.as_mut().unwrap().dirty = true;
        app.leave(Action::Lock);
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Save"));
        assert!(text.contains("Discard"));
        assert!(text.contains("Cancel"));
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(app.session.is_none());
    }
}
