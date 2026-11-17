use crate::{
    clipboard::Clipboard,
    commands::{Command, Palette},
    i18n::{self, localized, Language},
    input::{self, Input},
    sensitive::{FormEvent, SensitiveForm, SensitiveKind},
    settings,
    theme::{self, base},
    updates::{self, UpdateStatus},
    vault::{DeleteAuthorization, EntryData, Session},
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use keepass::db::{EntryId, GroupId};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    io,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

const NOTICE_TIME: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Debug)]
enum NoticeKind {
    Info,
    Warning,
    Error,
}

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
    BrowserConfirm,
    VaultAction,
    VaultDeleteConfirm,
    VaultDeleted,
    UpdateConsent,
    UpdateResult,
    NewFolder,
    DeleteFolder,
    Folders(bool), // true: move the selected entry, false: browse a folder.
}

struct Form {
    id: Option<EntryId>,
    fields: [Input; 5],
    field: usize,
    dirty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExplorerNode {
    Folder(GroupId),
    Entry(EntryId),
}

struct ExplorerRow {
    node: ExplorerNode,
    parent: GroupId,
    depth: usize,
    label: String,
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
    language: Language,
    settings_file: Option<PathBuf>,
    language_choice: Option<usize>,
    language_visible: Cell<bool>,
    language_hits: RefCell<Vec<(Rect, usize)>>,
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
    folder: Option<GroupId>, // None is the cross-folder "all entries" view.
    folder_name: Input,
    folder_error: Option<String>,
    folder_selected: usize,
    folder_dialog_visible: Cell<bool>,
    expanded_folders: HashSet<GroupId>,
    explorer_focus: Option<ExplorerNode>,
    explorer_hits: RefCell<Vec<(Rect, ExplorerNode)>>,
    palette_hits: RefCell<Vec<(Rect, Command)>>,
    pending_folder_delete: Option<GroupId>,
    form: Option<Form>,
    mode: Mode,
    palette: Option<Palette>,
    palette_visible: Cell<bool>,
    sensitive: Option<SensitiveForm>,
    sensitive_visible: Cell<bool>,
    delete_authorization: Option<DeleteAuthorization>,
    vault_delete_visible: Cell<bool>,
    deleted_backup: Option<PathBuf>,
    update_rx: Option<Receiver<updates::CheckResult>>,
    update_result: Option<UpdateStatus>,
    update_return: Option<Mode>,
    update_consent_visible: Cell<bool>,
    clipboard: Clipboard,
    reveal_until: Option<Instant>,
    last_activity: Instant,
    lock_after: Duration,
    status: String,
    status_until: Option<Instant>,
    status_kind: NoticeKind,
    exit: bool,
    browser_origin: Option<String>,
    browser_credentials: Option<Zeroizing<crate::browser::Credentials>>,
    browser_review_visible: Cell<bool>,
}

impl App {
    fn new(path: PathBuf, creating: bool, lock_after: Duration) -> Self {
        Self {
            language: Language::default(),
            settings_file: None,
            language_choice: None,
            language_visible: Cell::new(false),
            language_hits: RefCell::new(Vec::new()),
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
            folder: None,
            folder_name: Input::default(),
            folder_error: None,
            folder_selected: 0,
            folder_dialog_visible: Cell::new(false),
            expanded_folders: HashSet::new(),
            explorer_focus: None,
            explorer_hits: RefCell::new(Vec::new()),
            palette_hits: RefCell::new(Vec::new()),
            pending_folder_delete: None,
            form: None,
            mode: Mode::Browse,
            palette: None,
            palette_visible: Cell::new(false),
            sensitive: None,
            sensitive_visible: Cell::new(false),
            delete_authorization: None,
            vault_delete_visible: Cell::new(false),
            deleted_backup: None,
            update_rx: None,
            update_result: None,
            update_return: None,
            update_consent_visible: Cell::new(false),
            clipboard: Clipboard::new(),
            reveal_until: None,
            last_activity: Instant::now(),
            lock_after,
            status: String::new(),
            status_until: None,
            status_kind: NoticeKind::Info,
            exit: false,
            browser_origin: None,
            browser_credentials: None,
            browser_review_visible: Cell::new(false),
        }
    }
    /// Explicitly injected settings location; `new` is pure with respect to config.
    fn load_settings(&mut self, path: PathBuf) {
        self.language = settings::language_file(&path);
        self.settings_file = Some(path);
    }
    #[cfg(test)]
    fn load_preferences(&mut self, local_data: PathBuf) {
        self.load_settings(local_data.join("TermVault/settings.json"));
    }
    fn tr(&self, en: &'static str, ru: &'static str) -> &'static str {
        self.language.text(en, ru)
    }
    fn choose_language(&mut self) {
        if !self.language_visible.get() {
            return;
        }
        let Some(choice) = self.language_choice.take() else {
            return;
        };
        self.language = if choice == 0 {
            Language::English
        } else {
            Language::Russian
        };
        self.clear_notice();
        if let Some(path) = &self.settings_file {
            if settings::save_language_file(path, self.language).is_err() {
                self.error("Cannot save language preference; existing settings were not changed.");
            }
        }
    }
    fn notify(&mut self, text: impl Into<String>, kind: NoticeKind) {
        self.status = text.into();
        self.status_kind = kind;
        let lifetime = if kind == NoticeKind::Info {
            NOTICE_TIME
        } else {
            NOTICE_TIME * 2
        };
        self.status_until = Some(Instant::now() + lifetime);
    }
    fn info(&mut self, text: impl Into<String>) {
        self.notify(text, NoticeKind::Info);
    }
    fn error(&mut self, text: impl Into<String>) {
        self.notify(i18n::error(self.language, &text.into()), NoticeKind::Error);
    }
    fn warning(&mut self, text: impl Into<String>) {
        self.notify(text, NoticeKind::Warning);
    }
    fn clear_notice(&mut self) {
        self.status.clear();
        self.status_until = None;
    }
    fn unlock(&mut self) {
        if self.creating && self.master.text() != self.confirmation.text() {
            self.error("Master passwords do not match");
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
                if readonly {
                    self.warning(self.tr("Foreign vault opened read-only. Editing is restricted to TermVault-created vaults.", "Стороннее хранилище открыто только для чтения. Изменять можно лишь хранилища TermVault."));
                } else {
                    self.info(self.tr("Vault unlocked", "Хранилище разблокировано"));
                }
            }
            Err(error) => self.error(error.to_string()),
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
    fn entry_label(&self, id: EntryId, title: &str) -> String {
        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.entry_has_title(id))
        {
            self.tr("Untitled", "Без названия").into()
        } else {
            title.into()
        }
    }
    fn explorer_active(&self) -> bool {
        self.browser_origin.is_none()
            && self.query.text().is_empty()
            && self.session.as_ref().is_some_and(Session::has_folders)
    }
    fn explorer_rows(&self) -> Vec<ExplorerRow> {
        fn append(
            app: &App,
            session: &Session,
            parent: GroupId,
            depth: usize,
            rows: &mut Vec<ExplorerRow>,
        ) {
            for (id, name) in session.folder_children(parent) {
                rows.push(ExplorerRow {
                    node: ExplorerNode::Folder(id),
                    parent,
                    depth,
                    label: name,
                });
                if app.expanded_folders.contains(&id) {
                    append(app, session, id, depth + 1, rows);
                }
            }
            for (id, title) in &app.entries {
                if session.entry_folder(*id) == Some(parent) {
                    rows.push(ExplorerRow {
                        node: ExplorerNode::Entry(*id),
                        parent,
                        depth,
                        label: app.entry_label(*id, title),
                    });
                }
            }
        }
        let mut rows = Vec::new();
        if let Some(session) = &self.session {
            let root = session.root_folder();
            rows.push(ExplorerRow {
                node: ExplorerNode::Folder(root),
                parent: root,
                depth: 0,
                label: "TermVault".into(),
            });
            if self.expanded_folders.contains(&root) {
                append(self, session, root, 1, &mut rows);
            }
        }
        rows
    }
    fn expand_ancestors(&mut self, folder: GroupId) {
        let Some(session) = &self.session else {
            return;
        };
        let mut current = Some(folder);
        while let Some(id) = current {
            self.expanded_folders.insert(id);
            current = session.folder_parent(id);
        }
    }
    fn sync_explorer(&mut self, preferred: Option<EntryId>) {
        if !self.explorer_active() {
            return;
        }
        if self.explorer_focus.is_none() {
            self.expanded_folders
                .insert(self.session.as_ref().unwrap().root_folder());
        }
        if let Some(id) = preferred {
            if let Some(parent) = self
                .session
                .as_ref()
                .and_then(|session| session.entry_folder(id))
            {
                self.expand_ancestors(parent);
                self.explorer_focus = Some(ExplorerNode::Entry(id));
            }
        } else if self.explorer_focus.is_none() {
            if let Some(folder) = self.folder {
                self.expand_ancestors(folder);
                self.explorer_focus = Some(ExplorerNode::Folder(folder));
            }
        }
        let rows = self.explorer_rows();
        if !rows.iter().any(|row| Some(row.node) == self.explorer_focus) {
            self.explorer_focus = rows.first().map(|row| row.node);
        }
    }
    fn select_explorer(&mut self, node: ExplorerNode) {
        self.explorer_focus = Some(node);
        self.folder = match node {
            ExplorerNode::Folder(id) => Some(id),
            ExplorerNode::Entry(id) => self
                .session
                .as_ref()
                .and_then(|session| session.entry_folder(id)),
        };
        self.reveal_until = None;
    }
    fn move_explorer(&mut self, delta: isize) {
        let rows = self.explorer_rows();
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|row| Some(row.node) == self.explorer_focus)
            .unwrap_or(0);
        let index = index.saturating_add_signed(delta).min(rows.len() - 1);
        self.select_explorer(rows[index].node);
    }
    fn explorer_right(&mut self, toggle: bool) {
        if let Some(ExplorerNode::Folder(folder)) = self.explorer_focus {
            if !self.expanded_folders.insert(folder) {
                if toggle {
                    self.expanded_folders.remove(&folder);
                } else {
                    let rows = self.explorer_rows();
                    if let Some(index) = rows
                        .iter()
                        .position(|row| row.node == ExplorerNode::Folder(folder))
                    {
                        if let Some(child) = rows
                            .get(index + 1)
                            .filter(|row| row.depth > rows[index].depth)
                        {
                            self.select_explorer(child.node);
                        }
                    }
                }
            }
            self.reveal_until = None;
        } else if toggle {
            self.edit(false);
        }
    }
    fn explorer_left(&mut self) {
        let Some(focus) = self.explorer_focus else {
            return;
        };
        if let ExplorerNode::Folder(folder) = focus {
            if self.expanded_folders.remove(&folder) {
                self.reveal_until = None;
                return;
            }
        }
        if let Some(row) = self.explorer_rows().iter().find(|row| row.node == focus) {
            let parent = row.parent;
            if Some(ExplorerNode::Folder(parent)) != self.explorer_focus {
                self.select_explorer(ExplorerNode::Folder(parent));
            }
        }
    }
    fn creation_folder(&self) -> Option<GroupId> {
        if self.explorer_active() {
            match self.explorer_focus {
                Some(ExplorerNode::Folder(id)) => return Some(id),
                Some(ExplorerNode::Entry(id)) => return self.session.as_ref()?.entry_folder(id),
                None => {}
            }
        }
        self.folder
    }
    fn delete_folder_target(&self) -> Option<GroupId> {
        if !self.query.text().is_empty() {
            return None;
        }
        let folder = if self.explorer_active() {
            match self.explorer_focus {
                Some(ExplorerNode::Folder(id)) => Some(id),
                Some(ExplorerNode::Entry(_)) => None,
                None => self.folder,
            }
        } else {
            self.folder
        }?;
        (folder != self.session.as_ref()?.root_folder()).then_some(folder)
    }
    fn filter(&mut self, preferred: Option<EntryId>) {
        let query = self.query.text().to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (id, title))| {
                self.entry_label(*id, title).to_lowercase().contains(&query)
                    && (self.browser_origin.is_some()
                        || !query.is_empty()
                        || self.folder.is_none_or(|folder| {
                            self.session
                                .as_ref()
                                .and_then(|session| session.entry_folder(*id))
                                == Some(folder)
                        }))
                    && self.browser_origin.as_ref().is_none_or(|target| {
                        self.session
                            .as_ref()
                            .and_then(|session| session.entry(*id))
                            .is_some_and(|entry| crate::browser::matches(&entry.url, target))
                    })
            })
            .map(|(i, _)| i)
            .collect();
        self.selected = preferred
            .and_then(|id| self.visible.iter().position(|i| self.entries[*i].0 == id))
            .unwrap_or(0);
        self.reveal_until = None;
        self.sync_explorer(preferred);
    }
    fn selected_id(&self) -> Option<EntryId> {
        if self.explorer_active() {
            return match self.explorer_focus {
                Some(ExplorerNode::Entry(id)) => Some(id),
                Some(ExplorerNode::Folder(_)) => None,
                None => self.visible.get(self.selected).map(|i| self.entries[*i].0),
            };
        }
        self.visible.get(self.selected).map(|i| self.entries[*i].0)
    }
    fn lock(&mut self) {
        self.language_choice = None;
        self.language_hits.borrow_mut().clear();
        self.expanded_folders.clear();
        self.explorer_focus = None;
        self.explorer_hits.borrow_mut().clear();
        self.palette_hits.borrow_mut().clear();
        self.pending_folder_delete = None;
        self.folder = None;
        self.folder_name.clear();
        self.folder_error = None;
        self.folder_selected = 0;
        self.sensitive = None;
        self.delete_authorization = None;
        self.deleted_backup = None;
        self.update_rx = None;
        self.update_return = None;
        self.palette = None;
        self.form = None;
        self.session = None;
        self.clear_entries();
        self.query.clear();
        self.master.clear();
        self.confirmation.clear();
        self.reveal_until = None;
        self.clipboard.clear();
        self.mode = Mode::Browse;
        self.info(self.tr("Vault locked", "Хранилище заблокировано"));
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
        self.language_choice = None;
        self.pending_folder_delete = None;
        self.folder_name.clear();
        self.folder_error = None;
        self.sensitive = None;
        self.delete_authorization = None;
        self.update_return = None;
        self.palette = None;
        self.reveal_until = None;
        if self.form.as_ref().is_some_and(|form| form.dirty) {
            self.mode = Mode::Discard(action);
        } else {
            self.action(action);
        }
    }
    fn edit(&mut self, new: bool) {
        if new {
            self.folder = self.creation_folder();
        }
        let Some(session) = &self.session else {
            return;
        };
        if session.readonly {
            self.warning(self.tr("Read-only vault; create a separate TermVault vault to edit records.", "Хранилище только для чтения; создайте отдельное хранилище TermVault для редактирования."));
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
        self.clear_notice();
    }
    fn generate(&mut self) {
        match input::generate(24) {
            Ok(password) => {
                if let Some(form) = &mut self.form {
                    form.fields[2] = Input::from(&password);
                    form.dirty = true;
                    self.reveal_until = None;
                    self.info(self.tr(
                        "Password generated (24 characters)",
                        "Пароль сгенерирован (24 символа)",
                    ));
                }
            }
            Err(error) => self.error(error.to_string()),
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
        let result = match self.folder {
            Some(folder) => session.update_in_folder(id, &data, Some(folder)),
            None => session.update(id, &data),
        };
        match result {
            Ok(id) => {
                self.form = None;
                self.mode = Mode::Browse;
                self.query.clear();
                self.refresh(Some(id));
                self.info(self.tr("Entry saved", "Запись сохранена"));
                true
            }
            Err(error) => {
                self.error(error.to_string());
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
            match self.clipboard.copy(text, Instant::now()) {
                Ok(()) => self.info(if username {
                    self.tr(
                        "Username copied\nClipboard clearing attempted in 20 seconds.",
                        "Логин скопирован\nПопытка очистки буфера — через 20 с.",
                    )
                } else {
                    self.tr(
                        "Password copied\nClipboard clearing attempted in 20 seconds.",
                        "Пароль скопирован\nПопытка очистки буфера — через 20 с.",
                    )
                }),
                Err(error) => self.error(error.to_string()),
            }
        }
    }
    fn available_commands(&self) -> Vec<Command> {
        if self.language_choice.is_some()
            || self.browser_origin.is_some()
            || matches!(
                self.mode,
                Mode::Delete
                    | Mode::Discard(_)
                    | Mode::BrowserConfirm
                    | Mode::VaultAction
                    | Mode::VaultDeleteConfirm
                    | Mode::UpdateConsent
                    | Mode::UpdateResult
                    | Mode::NewFolder
                    | Mode::DeleteFolder
                    | Mode::Folders(_)
            )
        {
            return Vec::new();
        }
        if self.mode == Mode::VaultDeleted {
            return vec![Command::Quit];
        }
        let Some(session) = &self.session else {
            let mut commands = vec![Command::Unlock, Command::Language, Command::CheckUpdates];
            if self.update_result.is_some() {
                commands.push(Command::UpdateResult);
            }
            commands.push(Command::Quit);
            return commands;
        };
        let mut commands = Vec::new();
        match self.mode {
            Mode::Browse => {
                if !session.readonly {
                    commands.extend([Command::NewEntry, Command::NewGeneratedEntry]);
                }
                commands.push(Command::SearchEntries);
                if self.selected_id().is_some() {
                    if !session.readonly {
                        commands.push(Command::EditEntry);
                    }
                    commands.extend([
                        Command::CopyPassword,
                        Command::CopyUsername,
                        Command::RevealPassword,
                    ]);
                    if !session.readonly {
                        commands.push(Command::DeleteEntry);
                    }
                }
                commands.push(Command::OpenFolder);
                if !session.readonly {
                    commands.push(Command::NewFolder);
                    if self.delete_folder_target().is_some() {
                        commands.push(Command::DeleteFolder);
                    }
                    if self.selected_id().is_some() {
                        commands.push(Command::MoveToFolder);
                    }
                }
                if !session.readonly {
                    commands.extend([Command::ChangeMaster, Command::DeleteVault]);
                }
            }
            Mode::Edit if self.form.is_some() => {
                if !session.readonly {
                    commands.extend([Command::SaveEntry, Command::GeneratePassword]);
                }
                commands.extend([Command::RevealPassword, Command::Back]);
            }
            Mode::Search => commands.push(Command::Back),
            _ => {}
        }
        commands.extend([Command::Language, Command::CheckUpdates]);
        if self.update_result.is_some() {
            commands.push(Command::UpdateResult);
        }
        commands.extend([Command::Lock, Command::Quit]);
        commands
    }
    fn open_palette(&mut self) {
        if self.available_commands().is_empty() {
            self.warning(if self.browser_origin.is_some() {
                self.tr(
                    "Commands are unavailable during browser fill.",
                    "Палитра недоступна в режиме автозаполнения.",
                )
            } else {
                self.tr(
                    "Confirm or cancel the current action first.",
                    "Сначала подтвердите или отмените текущее действие.",
                )
            });
            return;
        }
        self.reveal_until = None;
        self.palette_visible.set(false);
        self.palette = Some(Palette::default());
    }
    fn toggle_reveal(&mut self) {
        if self.browser_origin.is_some()
            || self.session.is_none()
            || !matches!(self.mode, Mode::Browse | Mode::Edit)
            || (self.form.is_none() && self.selected_id().is_none())
        {
            return;
        }
        self.reveal_until = if self.reveal_until.is_some() {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(10))
        };
    }
    fn execute_command(&mut self, command: Command) {
        // Re-check at execution time; the palette never bypasses read-only/locked/browser restrictions.
        if !self.available_commands().contains(&command) {
            return;
        }
        self.palette = None;
        match command {
            Command::Language => {
                self.reveal_until = None;
                self.language_visible.set(false);
                self.language_choice = Some(usize::from(self.language == Language::Russian));
            }
            Command::Unlock => {} // Return to the masked input; never submit an existing master draft implicitly.
            Command::NewEntry => self.edit(true),
            Command::NewFolder => {
                self.folder = self.creation_folder();
                self.folder_name.clear();
                self.folder_error = None;
                self.reveal_until = None;
                self.clear_notice();
                self.folder_dialog_visible.set(false);
                self.mode = Mode::NewFolder;
            }
            Command::DeleteFolder => {
                self.pending_folder_delete = self.delete_folder_target();
                self.folder = self.pending_folder_delete;
                self.reveal_until = None;
                self.clear_notice();
                self.folder_dialog_visible.set(false);
                self.mode = Mode::DeleteFolder;
            }
            Command::OpenFolder | Command::MoveToFolder => {
                self.reveal_until = None;
                self.clear_notice();
                self.folder_selected = 0;
                self.folder_dialog_visible.set(false);
                self.mode = Mode::Folders(command == Command::MoveToFolder);
            }
            Command::NewGeneratedEntry => {
                self.edit(true);
                self.generate();
            }
            Command::EditEntry => self.edit(false),
            Command::SearchEntries => self.mode = Mode::Search,
            Command::CopyPassword => self.copy(false),
            Command::CopyUsername => self.copy(true),
            Command::RevealPassword => self.toggle_reveal(),
            Command::DeleteEntry => {
                self.reveal_until = None;
                self.mode = Mode::Delete;
            }
            Command::SaveEntry => {
                self.save();
            }
            Command::GeneratePassword => self.generate(),
            Command::Back => self.leave(Action::Browse),
            Command::CheckUpdates => {
                if self.update_rx.is_some() {
                    self.info(self.tr(
                        "An update check is already running.",
                        "Проверка обновлений уже идёт.",
                    ));
                } else {
                    self.update_return = Some(self.mode);
                    self.mode = Mode::UpdateConsent;
                    self.update_consent_visible.set(false);
                }
            }
            Command::UpdateResult => {
                self.update_return = Some(self.mode);
                self.mode = Mode::UpdateResult;
            }
            Command::ChangeMaster => self.begin_sensitive(SensitiveKind::ChangeMaster),
            Command::DeleteVault => self.begin_sensitive(SensitiveKind::DeleteVault),
            Command::Lock => self.leave(Action::Lock),
            Command::Quit => self.leave(Action::Quit),
        }
    }
    fn palette_key(&mut self, key: KeyEvent) {
        let commands = self
            .palette
            .as_ref()
            .unwrap()
            .filtered(&self.available_commands());
        match key.code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Enter if self.palette_visible.get() => {
                if let Some(command) = commands.get(self.palette.as_ref().unwrap().selected) {
                    self.execute_command(*command);
                }
            }
            _ => self.palette.as_mut().unwrap().key(key, commands.len()),
        }
    }
    fn folder_options(&self, moving: bool) -> Vec<(Option<GroupId>, String)> {
        let mut options = Vec::new();
        if !moving {
            options.push((None, self.tr("All entries", "Все записи").into()));
        }
        if let Some(session) = &self.session {
            options.extend(
                session
                    .folders()
                    .into_iter()
                    .map(|(id, name)| (Some(id), name)),
            );
        }
        options
    }
    fn folder_label(&self) -> String {
        self.folder
            .and_then(|id| {
                self.session
                    .as_ref()?
                    .folders()
                    .into_iter()
                    .find(|folder| folder.0 == id)
                    .map(|folder| folder.1)
            })
            .unwrap_or_else(|| self.tr("All entries", "Все записи").into())
    }
    fn submit_folder(&mut self) {
        if !self.folder_dialog_visible.get() {
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        match session.create_folder(self.folder, self.folder_name.text()) {
            Ok(id) => {
                self.folder = Some(id);
                self.expand_ancestors(id);
                self.explorer_focus = Some(ExplorerNode::Folder(id));
                self.folder_name.clear();
                self.query.clear();
                self.mode = Mode::Browse;
                self.refresh(None);
                self.info(self.tr("Folder created", "Папка создана"));
            }
            Err(error) => {
                self.folder_error = Some(error.to_string());
            }
        }
    }
    fn confirm_folder_delete(&mut self) {
        if !self.folder_dialog_visible.get() {
            return;
        }
        let Some(folder) = self.pending_folder_delete else {
            return;
        };
        let Some(session) = &mut self.session else {
            return;
        };
        match session.delete_folder(folder) {
            Ok(parent) => {
                self.pending_folder_delete = None;
                self.folder = Some(parent);
                self.explorer_focus = Some(ExplorerNode::Folder(parent));
                self.query.clear();
                self.mode = Mode::Browse;
                self.refresh(None);
                self.warning(self.tr(
                    "Folder and its contents deleted. Encrypted backup retained.",
                    "Папка и её содержимое удалены. Зашифрованный бэкап сохранён.",
                ));
            }
            Err(error) => {
                self.pending_folder_delete = None;
                self.mode = Mode::Browse;
                self.error(error.to_string());
            }
        }
    }
    fn choose_folder(&mut self, moving: bool) {
        if !self.folder_dialog_visible.get() {
            return;
        }
        let options = self.folder_options(moving);
        let Some((folder, _)) = options.get(self.folder_selected) else {
            return;
        };
        let folder = *folder;
        let preferred = if moving {
            let Some(id) = self.selected_id() else {
                return;
            };
            let Some(target) = folder else {
                return;
            };
            let Some(session) = &mut self.session else {
                return;
            };
            if let Err(error) = session.move_entry(id, target) {
                self.error(error.to_string());
                return;
            }
            self.info(self.tr("Entry moved", "Запись перемещена"));
            Some(id)
        } else {
            None
        };
        self.folder = folder;
        self.explorer_focus = None;
        if let Some(folder) = folder {
            self.expand_ancestors(folder);
        }
        self.query.clear();
        self.mode = Mode::Browse;
        self.refresh(preferred);
    }
    fn begin_sensitive(&mut self, kind: SensitiveKind) {
        self.reveal_until = None;
        self.clear_notice();
        self.delete_authorization = None;
        self.sensitive = Some(SensitiveForm::new(kind));
        self.sensitive_visible.set(false);
        self.mode = Mode::VaultAction;
    }
    fn cancel_sensitive(&mut self) {
        self.sensitive = None;
        self.delete_authorization = None;
        self.mode = Mode::Browse;
    }
    fn submit_sensitive(&mut self) {
        if !self.sensitive_visible.get() {
            self.warning(self.tr(
                "Enlarge the window to see the form and warnings.",
                "Увеличьте окно, чтобы увидеть форму и предупреждения.",
            ));
            return;
        }
        let Some(form) = self.sensitive.take() else {
            return;
        };
        let Some(session) = self.session.as_mut() else {
            self.lock();
            return;
        };
        let result = match form.kind {
            SensitiveKind::ChangeMaster => {
                if form.fields[1].text() != form.fields[2].text() {
                    Err(anyhow::anyhow!("New master passphrases do not match"))
                } else {
                    session
                        .change_master(form.fields[0].text(), form.fields[1].text())
                        .map(|_| None)
                }
            }
            SensitiveKind::DeleteVault => session
                .authorize_delete(form.fields[0].text(), form.fields[1].text())
                .map(Some),
        };
        match result {
            Ok(Some(authorization)) => {
                self.delete_authorization = Some(authorization);
                self.vault_delete_visible.set(false);
                self.mode = Mode::VaultDeleteConfirm;
            }
            Ok(None) => {
                self.mode = Mode::Browse;
                self.clipboard.clear();
                self.warning(self.tr(
                    "Master password changed. Previous backups require the OLD password.",
                    "Мастер-пароль изменён. Предыдущие бэкапы требуют СТАРЫЙ пароль.",
                ));
            }
            Err(error) => {
                let message = error.to_string();
                let mut retry = SensitiveForm::new(form.kind);
                retry.error = Some(message.clone());
                self.sensitive = Some(retry);
                self.error(message);
            }
        }
        // `form` drops here: all current/new master inputs use zeroizing buffers.
    }
    fn confirm_vault_delete(&mut self) {
        if !self.vault_delete_visible.get() {
            return;
        }
        let Some(authorization) = self.delete_authorization.take() else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            self.lock();
            return;
        };
        match session.remove_vault(&authorization) {
            Ok(backup) => {
                self.lock();
                self.deleted_backup = Some(backup);
                self.mode = Mode::VaultDeleted;
                self.info(self.tr(
                    "Active file removed. Encrypted backup retained.",
                    "Активный файл удалён. Зашифрованный бэкап сохранён.",
                ));
            }
            Err(error) => {
                self.mode = Mode::Browse;
                self.error(error.to_string());
            }
        }
    }
    fn close_update_dialog(&mut self) {
        self.mode = self.update_return.take().unwrap_or(Mode::Browse);
    }
    fn start_update_check(&mut self) {
        if !self.update_consent_visible.get() {
            return;
        }
        self.close_update_dialog();
        if self.update_rx.is_some() {
            return;
        }
        self.update_result = None;
        match updates::start_check() {
            Ok(receiver) => {
                self.update_rx = Some(receiver);
                self.info(self.tr(
                    "Checking GitHub for updates…",
                    "Проверяем обновления на GitHub…",
                ));
            }
            Err(error) => self.error(error.to_string()),
        }
    }
    fn poll_update(&mut self) -> bool {
        let Some(receiver) = &self.update_rx else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                Err("Update check was interrupted. Please retry.".into())
            }
        };
        self.update_rx = None;
        match result {
            Ok(status) => {
                self.info(match &status {
                    UpdateStatus::Available(version) => {
                        localized!(
                            self.language,
                            "Version {version} is available. See check result in Ctrl+P.",
                            "Доступна версия {version}. Результат проверки — в Ctrl+P."
                        )
                    }
                    _ => status.description(self.language),
                });
                self.update_result = Some(status);
            }
            Err(error) => self.error(error),
        }
        true
    }
    fn tick(&mut self, now: Instant) -> bool {
        self.clipboard.tick(now);
        let mut changed = self.poll_update();
        if self.status_until.is_some_and(|deadline| now >= deadline) {
            self.clear_notice();
            changed = true;
        }
        if self.session.is_some()
            && now.saturating_duration_since(self.last_activity) >= self.lock_after
        {
            self.lock();
            self.warning(self.tr(
                "Auto-locked after inactivity. Unsaved entry drafts were discarded.",
                "Автоблокировка из-за бездействия. Несохранённые черновики удалены.",
            ));
            return true;
        }
        if self.reveal_until.is_some_and(|deadline| now >= deadline) {
            self.reveal_until = None;
            return true;
        }
        changed
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
                'з' => 'p',
                'щ' => 'o',
                c => c,
            }),
            _ => None,
        };
        if ctrl && letter == Some('q') {
            self.leave(Action::Quit);
            return;
        }
        if ctrl && letter == Some('p') {
            if self.language_choice.take().is_some() {
                return;
            }
            if self.palette.is_some() {
                self.palette = None;
            } else {
                self.open_palette();
            }
            return;
        }
        if ctrl && letter == Some('l') && self.session.is_some() {
            self.leave(Action::Lock);
            return;
        }
        if let Some(choice) = &mut self.language_choice {
            match key.code {
                KeyCode::Esc => self.language_choice = None,
                KeyCode::Up | KeyCode::BackTab => *choice = choice.saturating_sub(1),
                KeyCode::Down | KeyCode::Tab => *choice = (*choice + 1).min(1),
                KeyCode::Enter => self.choose_language(),
                _ => {}
            }
            return;
        }
        if self.palette.is_some() {
            self.palette_key(key);
            return;
        }
        match self.mode {
            Mode::DeleteFolder => {
                match key.code {
                    KeyCode::Esc => {
                        self.pending_folder_delete = None;
                        self.mode = Mode::Browse;
                    }
                    KeyCode::Char('d')
                        if key.kind == KeyEventKind::Press && key.modifiers.is_empty() =>
                    {
                        self.confirm_folder_delete()
                    }
                    _ => {}
                }
                return;
            }
            Mode::NewFolder => {
                match key.code {
                    KeyCode::Esc => {
                        self.folder_name.clear();
                        self.folder_error = None;
                        self.mode = Mode::Browse;
                    }
                    KeyCode::Enter => self.submit_folder(),
                    _ => {
                        let fits = match key.code {
                            KeyCode::Char(character) => {
                                self.folder_name.text().len() + character.len_utf8() <= 128
                            }
                            _ => true,
                        };
                        if (fits || ctrl) && self.folder_name.key(key, false) {
                            self.folder_error = None;
                        }
                    }
                }
                return;
            }
            Mode::Folders(moving) => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Up | KeyCode::BackTab => {
                        self.folder_selected = self.folder_selected.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        self.folder_selected = (self.folder_selected + 1)
                            .min(self.folder_options(moving).len().saturating_sub(1))
                    }
                    KeyCode::Enter => self.choose_folder(moving),
                    _ => {}
                }
                return;
            }
            Mode::VaultDeleted => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                    self.exit = true;
                }
                return;
            }
            Mode::VaultAction => {
                if let Some(form) = &mut self.sensitive {
                    match form.key(key) {
                        FormEvent::Cancel => self.cancel_sensitive(),
                        FormEvent::Submit => self.submit_sensitive(),
                        FormEvent::None => {}
                    }
                }
                return;
            }
            Mode::VaultDeleteConfirm => {
                match key.code {
                    KeyCode::Esc => self.cancel_sensitive(),
                    KeyCode::Char('d') if key.kind == KeyEventKind::Press => {
                        self.confirm_vault_delete()
                    }
                    _ => {}
                }
                return;
            }
            Mode::UpdateConsent => {
                match key.code {
                    KeyCode::Esc => self.close_update_dialog(),
                    KeyCode::Enter => self.start_update_check(),
                    _ => {}
                }
                return;
            }
            Mode::UpdateResult => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.close_update_dialog();
                }
                return;
            }
            _ => {}
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
        if self.browser_origin.is_some() {
            match (self.mode, key.code) {
                (_, KeyCode::Esc | KeyCode::Char('q')) => self.action(Action::Quit),
                (Mode::Browse, KeyCode::Up) => self.selected = self.selected.saturating_sub(1),
                (Mode::Browse, KeyCode::Down) => {
                    self.selected = (self.selected + 1).min(self.visible.len().saturating_sub(1));
                }
                (Mode::Browse, KeyCode::Enter) if self.selected_id().is_some() => {
                    self.mode = Mode::BrowserConfirm;
                }
                (Mode::BrowserConfirm, KeyCode::Char('y')) if self.browser_review_visible.get() => {
                    if let Some(data) = self
                        .selected_id()
                        .and_then(|id| self.session.as_ref()?.entry(id))
                    {
                        let target = self.browser_origin.as_ref().unwrap();
                        if crate::browser::matches(&data.url, target)
                            && data.username.len() <= 1024
                            && data.password.len() <= 1024
                        {
                            self.browser_credentials =
                                Some(Zeroizing::new(crate::browser::Credentials {
                                    origin: target.clone(),
                                    username: data.username.to_string(),
                                    password: data.password.to_string(),
                                }));
                            self.action(Action::Quit);
                        }
                    }
                }
                (Mode::BrowserConfirm, KeyCode::Char('n')) => self.mode = Mode::Browse,
                _ => {}
            }
            return;
        }
        if ctrl && letter == Some('o') {
            self.toggle_reveal();
            return;
        }
        match self.mode {
            Mode::VaultAction
            | Mode::NewFolder
            | Mode::DeleteFolder
            | Mode::Folders(_)
            | Mode::VaultDeleteConfirm
            | Mode::VaultDeleted
            | Mode::UpdateConsent
            | Mode::UpdateResult => {}
            Mode::BrowserConfirm => {}
            Mode::Edit => {
                if ctrl && letter == Some('s') {
                    self.save();
                } else if ctrl && letter == Some('g') {
                    self.generate();
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
                                self.warning(self.tr("Entry removed. Encrypted backups still contain its previous versions.", "Запись удалена. Зашифрованные бэкапы содержат её прежние версии."));
                                self.mode = Mode::Browse;
                            }
                            Err(error) => self.error(error.to_string()),
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
                KeyCode::Up if self.explorer_active() => self.move_explorer(-1),
                KeyCode::Down if self.explorer_active() => self.move_explorer(1),
                KeyCode::Right if self.explorer_active() => self.explorer_right(false),
                KeyCode::Left if self.explorer_active() => self.explorer_left(),
                KeyCode::Enter | KeyCode::Char(' ') if self.explorer_active() => {
                    self.explorer_right(true)
                }
                KeyCode::Delete | KeyCode::Char('d') if self.delete_folder_target().is_some() => {
                    self.execute_command(Command::DeleteFolder)
                }
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
                KeyCode::Char('f') => self.execute_command(Command::OpenFolder),
                KeyCode::Char('F') => self.execute_command(Command::NewFolder),
                KeyCode::Char('m') => self.execute_command(Command::MoveToFolder),
                KeyCode::Char('g') => {
                    self.edit(true);
                    if self.mode == Mode::Edit {
                        self.generate();
                    }
                }
                KeyCode::Enter => self.edit(false),
                KeyCode::Delete | KeyCode::Char('d') if self.selected_id().is_some() => {
                    self.mode = Mode::Delete
                }
                KeyCode::Char('p') if key.modifiers.is_empty() => {
                    self.toggle_reveal();
                }
                KeyCode::Char('c') => self.copy(false),
                KeyCode::Char('u') => self.copy(true),
                _ => {}
            },
        }
    }
    fn mouse(&mut self, event: MouseEvent) {
        if !matches!(
            event.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollUp
        ) {
            return;
        }
        self.last_activity = Instant::now();
        let point = (event.column, event.row).into();
        if self.language_choice.is_some() {
            if event.kind == MouseEventKind::Down(MouseButton::Left) && self.language_visible.get()
            {
                let choice = self
                    .language_hits
                    .borrow()
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .map(|(_, choice)| *choice);
                if let Some(choice) = choice {
                    self.language_choice = Some(choice);
                    self.choose_language();
                }
            }
            return;
        }
        if self.palette.is_some() {
            if event.kind == MouseEventKind::Down(MouseButton::Left) {
                let command = self
                    .palette_hits
                    .borrow()
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .map(|(_, command)| *command);
                if let Some(command) = command {
                    self.execute_command(command);
                }
            }
            return;
        }
        if self.mode != Mode::Browse || !self.explorer_active() {
            return;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let node = self
                    .explorer_hits
                    .borrow()
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .map(|(_, node)| *node);
                if let Some(node) = node {
                    self.select_explorer(node);
                    if matches!(node, ExplorerNode::Folder(_)) {
                        self.explorer_right(true);
                    }
                }
            }
            MouseEventKind::ScrollDown
                if self
                    .explorer_hits
                    .borrow()
                    .iter()
                    .any(|(rect, _)| event.column >= rect.x && event.column < rect.right()) =>
            {
                self.move_explorer(3)
            }
            MouseEventKind::ScrollUp
                if self
                    .explorer_hits
                    .borrow()
                    .iter()
                    .any(|(rect, _)| event.column >= rect.x && event.column < rect.right()) =>
            {
                self.move_explorer(-3)
            }
            _ => {}
        }
    }
    fn paste(&mut self, text: &str) {
        self.last_activity = Instant::now();
        if self.language_choice.is_some() {
            return;
        }
        if self.mode == Mode::NewFolder {
            if self.folder_name.text().len().saturating_add(text.len()) <= 128
                && self.folder_name.insert(text, false)
            {
                self.folder_error = None;
            } else {
                self.folder_error =
                    Some("Paste rejected: maximum 128 bytes, no control characters.".into());
            }
            return;
        }
        if let Some(palette) = &mut self.palette {
            palette.paste(text);
            return;
        }
        if self.mode == Mode::VaultAction {
            if let Some(form) = &mut self.sensitive {
                form.paste(text);
            }
            return;
        }
        if self.session.is_none() {
            let field = if self.auth_field == 0 {
                &mut self.master
            } else {
                &mut self.confirmation
            };
            if !field.insert(text, false) {
                self.error("Paste rejected: unsupported characters or input too large");
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
                        self.error("Paste rejected: unsupported characters or input too large");
                    }
                }
            }
            Mode::Search if self.query.insert(text, false) => self.filter(None),
            _ => {}
        }
    }
    fn draw(&self, frame: &mut Frame) {
        self.language_visible.set(false);
        self.language_hits.borrow_mut().clear();
        self.explorer_hits.borrow_mut().clear();
        self.palette_hits.borrow_mut().clear();
        self.folder_dialog_visible.set(false);
        self.browser_review_visible.set(false);
        self.palette_visible.set(false);
        self.sensitive_visible.set(false);
        self.vault_delete_visible.set(false);
        self.update_consent_visible.set(false);
        let area = frame.area();
        frame.render_widget(Block::default().style(base()), area);
        if area.width < 60 || area.height < 20 {
            let text = if matches!(self.mode, Mode::Discard(_)) {
                self.tr(
                    "Unsaved entry\n[s] Save  [d] Discard\n[c/Esc] Cancel",
                    "Несохранённая запись\n[s] Сохранить  [d] Отбросить\n[c/Esc] Отмена",
                )
            } else {
                self.tr(
                    "TermVault — resize to 60 × 20\nCtrl+L locks · Ctrl+Q exits",
                    "TermVault — увеличьте окно до 60 × 20\nCtrl+L блокировка · Ctrl+Q выход",
                )
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
                Constraint::Length(1),
            ])
            .split(area);
        let state = if self.mode == Mode::VaultDeleted {
            self.tr("REMOVED", "УДАЛЕНО")
        } else if self.session.is_some() {
            self.tr("UNLOCKED", "ОТКРЫТО")
        } else {
            self.tr("LOCKED", "ЗАКРЫТО")
        };
        let state_color = if self.session.is_some() {
            theme::SUCCESS
        } else {
            theme::PRIMARY
        };
        let mut header = vec![Line::from(vec![
            Span::styled(
                " ◆ TermVault",
                base().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", base().fg(theme::BORDER)),
            Span::styled(state, base().fg(state_color)),
        ])];
        if let Some(origin) = &self.browser_origin {
            header.push(Line::styled(
                format!(
                    " {} → {}",
                    self.tr("BROWSER FILL", "АВТОЗАПОЛНЕНИЕ"),
                    input::display(origin)
                ),
                base().fg(theme::WARNING),
            ));
        }
        if self.session.is_some()
            || self.mode == Mode::VaultDeleted
            || self.browser_origin.is_some()
        {
            frame.render_widget(Paragraph::new(header).style(base()), rows[0]);
        }
        if self.mode == Mode::VaultDeleted {
            let backup = self
                .deleted_backup
                .as_ref()
                .map(|path| input::display(&path.display().to_string()))
                .unwrap_or_default();
            frame.render_widget(Paragraph::new(localized!(self.language, "The active file no longer exists.\n\nRetained encrypted backup:\n{backup}\n\nOpen it with the previous master password. Other backups are unchanged.\nThis is not secure erasure.\n\nEnter / Esc — close the application.", "Активный файл больше не существует.\n\nСохранённый зашифрованный бэкап:\n{backup}\n\nЕго можно открыть прежним мастер-паролем. Остальные бэкапы не изменены.\nЭто не безопасное стирание данных.\n\nEnter / Esc — закрыть приложение.")).block(panel(self.tr(" Vault removed ", " Хранилище удалено "))).wrap(Wrap { trim: false }), rows[1]);
        } else if self.session.is_none() {
            self.draw_auth(frame);
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
                        .block(
                            panel(&format!(
                                " {}{}{} ",
                                if active { "› " } else { "" },
                                self.field_label(i),
                                if form.dirty && i == 0 {
                                    self.tr(" · unsaved", " · не сохранено")
                                } else {
                                    ""
                                }
                            ))
                            .border_style(Style::default().fg(if active {
                                theme::PRIMARY
                            } else {
                                theme::BORDER
                            }))
                            .title_style(
                                Style::default().fg(if active {
                                    theme::PRIMARY
                                } else {
                                    theme::MUTED
                                }),
                            ),
                        )
                        .wrap(Wrap { trim: false }),
                    parts[i],
                );
            }
        } else {
            let parts = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
                .split(rows[1]);
            if self.explorer_active() {
                self.draw_explorer(frame, parts[0]);
            } else {
                let items: Vec<ListItem> = self
                    .visible
                    .iter()
                    .map(|i| {
                        let (id, title) = &self.entries[*i];
                        ListItem::new(self.entry_label(*id, title))
                    })
                    .collect();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(
                            panel(&format!(
                                " {} · {} · {} ",
                                self.tr("Entries", "Записи"),
                                self.visible.len(),
                                self.folder_label()
                            ))
                            .padding(Padding::new(0, 0, 1, 0)),
                        )
                        .highlight_style(invert())
                        .highlight_symbol("› "),
                    parts[0],
                    &mut ListState::default()
                        .with_selected((!self.visible.is_empty()).then_some(self.selected)),
                );
            }
            let data = self
                .selected_id()
                .and_then(|id| self.session.as_ref()?.entry(id));
            let text = if let Some(data) = data {
                let password = if self.reveal_until.is_some() {
                    input::display(&data.password)
                } else {
                    self.tr(
                        "••••••••  [Ctrl+O reveals for 10 seconds]",
                        "••••••••  [Ctrl+O показать на 10 секунд]",
                    )
                    .into()
                };
                let field = |label: &str, value: String| {
                    Line::from(vec![
                        Span::styled(label.to_owned(), theme::surface().fg(theme::MUTED)),
                        Span::styled(value, theme::surface()),
                    ])
                };
                let mut lines = vec![
                    Line::styled(
                        self.entry_label(self.selected_id().unwrap(), &input::display(&data.title)),
                        theme::surface()
                            .fg(theme::SECONDARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Line::default(),
                    field(
                        self.tr("Username: ", "Логин: "),
                        input::display(&data.username),
                    ),
                    Line::default(),
                    field(self.tr("Password: ", "Пароль: "), password),
                    Line::default(),
                    field("URL: ", input::display(&data.url)),
                    Line::default(),
                    Line::styled(
                        self.tr("Notes:", "Заметки:"),
                        theme::surface().fg(theme::MUTED),
                    ),
                ];
                lines.extend(
                    input::display(&data.notes)
                        .lines()
                        .map(|line| Line::from(line.to_owned())),
                );
                lines
            } else {
                if self.browser_origin.is_some() {
                    vec![Line::from(self.tr("No matching entries.", "Нет подходящих записей.")), Line::default(), Line::from(self.tr("Set an entry's URL in the normal TermVault editor to an HTTPS address with the exact same host and port. Subdomains are not shared automatically.", "Укажите в редакторе TermVault HTTPS-адрес с точно таким же хостом и портом. Поддомены автоматически не объединяются.")), Line::default(), Line::from(self.tr("Esc cancels.", "Esc — отмена."))]
                } else if let Some(ExplorerNode::Folder(folder)) =
                    self.explorer_focus.filter(|_| self.explorer_active())
                {
                    let (entries, folders) = self
                        .session
                        .as_ref()
                        .and_then(|session| session.folder_contents(folder))
                        .unwrap_or_default();
                    vec![
                        Line::styled(
                            self.tr("Folder", "Папка"),
                            theme::surface().fg(theme::SECONDARY),
                        ),
                        Line::default(),
                        Line::from(localized!(
                            self.language,
                            "Entries: {entries} · Nested folders: {folders}",
                            "Записей: {entries} · Вложенных папок: {folders}"
                        )),
                        Line::default(),
                        Line::from(self.tr(
                            "Enter / → expand · ← collapse",
                            "Enter / → раскрыть · ← свернуть",
                        )),
                        Line::from(self.tr(
                            "n new entry · Shift+F new subfolder",
                            "n создать запись · Shift+F создать подпапку",
                        )),
                        Line::from(self.tr(
                            "Delete remove folder with confirmation",
                            "Delete удалить папку с подтверждением",
                        )),
                    ]
                } else {
                    vec![
                        Line::styled(self.tr("No entries.", "Нет записей."), theme::surface().fg(theme::SECONDARY)),
                        Line::default(),
                        Line::from(
                            self.tr("Press n to add a record, or g to start with a generated password.", "Нажмите n, чтобы создать запись, или g для записи со случайным паролем."),
                        ),
                        Line::from(self.tr("Your data is saved only when you press Ctrl+S.", "Данные сохраняются только по Ctrl+S.")),
                        Line::default(),
                        Line::styled(
                            self.tr("This MVP is not audited. Use test credentials first.", "MVP не проходил аудит. Сначала используйте тестовые данные."),
                            theme::surface().fg(theme::WARNING),
                        ),
                    ]
                }
            };
            frame.render_widget(
                Paragraph::new(text)
                    .block(
                        panel(self.tr(
                            " Entry · passwords hidden by default ",
                            " Запись · пароли скрыты по умолчанию ",
                        ))
                        .padding(Padding::new(0, 0, 1, 0)),
                    )
                    .wrap(Wrap { trim: false }),
                parts[1],
            );
        }
        frame.render_widget(
            Paragraph::new(if self.mode == Mode::VaultDeleted {
                self.tr(" Enter / Esc exit ", " Enter / Esc выход ")
            } else if self.mode == Mode::VaultAction {
                self.tr(" Tab field · Enter continue · Esc cancel ", " Tab поле · Enter далее · Esc отмена ")
            } else if matches!(self.mode, Mode::VaultDeleteConfirm | Mode::DeleteFolder) {
                self.tr(" d confirm deletion · Esc cancel ", " d подтвердить удаление · Esc отмена ")
            } else if matches!(self.mode, Mode::UpdateConsent | Mode::UpdateResult) {
                self.tr(" Enter continue · Esc back ", " Enter далее · Esc назад ")
            } else if self.session.is_none() && self.browser_origin.is_some() {
                self.tr(" Enter unlock · Esc cancel ", " Enter разблокировать · Esc отмена ")
            } else if self.session.is_none() {
                self.tr(" Ctrl+P commands · Enter continue · Tab field · Esc quit ", " Ctrl+P команды · Enter далее · Tab поле · Esc выход ")
            } else if self.browser_origin.is_some() {
                self.tr(" ↑↓ select · Enter review · y approve in confirmation · Esc cancel ", " ↑↓ выбор · Enter проверить · y одобрить в диалоге · Esc отмена ")
            } else if self.form.is_some() {
                self.tr(" Ctrl+P commands · Ctrl+O reveal · Tab field · Ctrl+S save · Esc back ", " Ctrl+P команды · Ctrl+O показать · Tab поле · Ctrl+S сохранить · Esc назад ")
            } else if self.explorer_active() {
                self.tr(" Ctrl+P commands · ↑↓ select · ←→ folders · Enter open/edit · Delete remove · n new entry ", " Ctrl+P команды · ↑↓ выбор · ←→ папки · Enter открыть · Delete удалить · n новая запись ")
            } else {
                self.tr(" Ctrl+P commands · Ctrl+O reveal · ↑↓ select · Enter edit · Ctrl+L lock ", " Ctrl+P команды · Ctrl+O показать · ↑↓ выбор · Enter изменить · Ctrl+L блокировка ")
            })
            .style(base().fg(theme::MUTED)),
            rows[2],
        );
        // Dialogs are rendered afterward: a toast must never obscure approval or discard prompts.
        self.draw_notice(frame);
        match self.mode {
            Mode::DeleteFolder => self.draw_folder_delete(frame),
            Mode::NewFolder => {
                let parent = if self.folder.is_some() { self.folder_label() } else { "/".into() };
                popup(frame, &localized!(self.language, "Parent: {parent}\n\nFolder name:\n{}\n\nUp to 128 bytes, no / or \\.\n{}\n\nEnter create · Esc cancel", "Родитель: {parent}\n\nИмя папки:\n{}\n\nДо 128 байт, без / и \\.\n{}\n\nEnter создать · Esc отмена", self.folder_name.rendered(false, true), i18n::error(self.language, self.folder_error.as_deref().unwrap_or(""))), self.tr(" New folder ", " Новая папка "), 16);
                self.folder_dialog_visible.set(true);
            }
            Mode::Folders(moving) => self.draw_folders(frame, moving),
            Mode::VaultAction => self.draw_sensitive(frame),
            Mode::VaultDeleteConfirm => self.draw_vault_delete_confirmation(frame),
            Mode::UpdateConsent => {
                popup(frame, self.tr("Send a request to api.github.com?\n\nGitHub receives your IP and User-Agent with the TermVault version.\nPasswords, the vault, its path and the current site are NOT sent.\n\nChecks run only on this command.\nTimeout: 10 seconds. No automatic installation.\n\n[Enter] Check   [Esc] Cancel", "Отправить запрос к api.github.com?\n\nGitHub получит ваш IP и User-Agent с версией TermVault.\nПароли, база, её путь и текущий сайт НЕ передаются.\n\nПроверка выполняется только по этой команде.\nТайм-аут — 10 секунд. Автоустановки нет.\n\n[Enter] Проверить   [Esc] Отмена"), self.tr(" Check for updates ", " Проверить обновления "), 18);
                self.update_consent_visible.set(true);
            }
            Mode::UpdateResult => {
                let text = self.update_result.as_ref().map(|status| status.description(self.language)).unwrap_or_else(|| self.tr("No update check result.", "Результат проверки отсутствует.").into());
                popup(frame, &localized!(self.language, "{text}\n\n[Enter / Esc] Back", "{text}\n\n[Enter / Esc] Назад"), self.tr(" Updates ", " Обновления "), 18);
            }
            Mode::BrowserConfirm => {
                let title = self.selected_id().and_then(|id| self.session.as_ref()?.entry(id))
                    .map(|entry| input::display(&entry.title).replace('\n', " ").chars().take(80).collect::<String>()).unwrap_or_default();
                self.browser_review_visible.set(true);
                popup(frame, &localized!(self.language, "Send login and password for:\n{title}\nTo: {}\n\n[y] Approve once   [n] Back   [Esc] Cancel", "Отправить логин и пароль записи:\n{title}\nСайту: {}\n\n[y] Одобрить один раз   [n] Назад   [Esc] Отмена", self.browser_origin.as_deref().unwrap_or("")), self.tr(" Confirm browser fill ", " Подтвердить автозаполнение "), 18);
            }
            Mode::Search=>popup(frame,&localized!(self.language, "{}\nEnter applies · Esc clears", "{}\nEnter применить · Esc очистить", if self.query.text().is_empty() { self.tr("Search entry titles…", "Поиск по названиям…").to_owned() } else { self.query.rendered(false,true) }),self.tr(" Filter entry titles ", " Поиск по названиям "),5),
            Mode::Discard(_)=>popup(frame,self.tr("Unsaved changes\n\n[s] Save and continue   [d] Discard   [c/Esc] Cancel\nInactivity auto-lock discards unsaved drafts.", "Несохранённые изменения\n\n[s] Сохранить и продолжить   [d] Отбросить   [c/Esc] Отмена\nАвтоблокировка удаляет несохранённые черновики."),self.tr(" Unsaved entry ", " Несохранённая запись "),7),
            Mode::Delete=>popup(frame,self.tr("Delete the selected entry?\n\n[d] Delete   [Esc] Cancel\nEncrypted backups retain previous versions; this is not secure erasure.", "Удалить выбранную запись?\n\n[d] Удалить   [Esc] Отмена\nЗашифрованные бэкапы хранят прежние версии; это не безопасное стирание."),self.tr(" Confirm deletion ", " Подтвердить удаление "),7),
            _=>{}
        }
        self.draw_palette(frame);
        self.draw_language(frame);
    }

    fn field_label(&self, index: usize) -> &'static str {
        match index {
            0 => self.tr("Title", "Название"),
            1 => self.tr("Username", "Логин"),
            2 => self.tr("Password", "Пароль"),
            3 => "URL",
            _ => self.tr("Notes", "Заметки"),
        }
    }

    fn draw_language(&self, frame: &mut Frame) {
        let Some(choice) = self.language_choice else {
            return;
        };
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(52);
        let height = 8;
        if width < 30 || area.height < height + 4 {
            return;
        }
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        dim_backdrop(frame);
        frame.render_widget(Clear, rect);
        let block = panel(self.tr(" Language ", " Язык "));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
        frame.render_stateful_widget(
            List::new([ListItem::new("English"), ListItem::new("Русский")])
                .style(theme::surface())
                .highlight_style(theme::selected())
                .highlight_symbol("› "),
            rows[1],
            &mut ListState::default().with_selected(Some(choice)),
        );
        frame.render_widget(
            Paragraph::new(self.tr(
                "↑↓ select · Enter apply · Esc cancel",
                "↑↓ выбор · Enter применить · Esc отмена",
            ))
            .style(theme::surface().fg(theme::MUTED)),
            rows[3],
        );
        for index in 0..2 {
            self.language_hits.borrow_mut().push((
                Rect::new(rows[1].x, rows[1].y + index as u16, rows[1].width, 1),
                index,
            ));
        }
        self.language_visible.set(true);
    }

    fn draw_explorer(&self, frame: &mut Frame, area: Rect) {
        let rows = self.explorer_rows();
        let selected = rows
            .iter()
            .position(|row| Some(row.node) == self.explorer_focus);
        let block = panel(self.tr(" Explorer ", " Проводник ")).padding(Padding::new(0, 0, 1, 0));
        let inner = block.inner(area);
        let items: Vec<_> = rows
            .iter()
            .map(|row| {
                let indent = "  ".repeat(row.depth);
                let (icon, color) = match row.node {
                    ExplorerNode::Folder(id) => (
                        if self.expanded_folders.contains(&id) {
                            "▾ ▰ "
                        } else {
                            "▸ ▰ "
                        },
                        theme::MUTED,
                    ),
                    ExplorerNode::Entry(_) => ("  · ", theme::MUTED),
                };
                ListItem::new(Line::from(vec![
                    Span::raw(indent),
                    Span::styled(icon, Style::default().fg(color)),
                    Span::raw(row.label.as_str()),
                ]))
            })
            .collect();
        let mut state = ListState::default().with_selected(selected);
        frame.render_stateful_widget(
            List::new(items)
                .block(block)
                .style(theme::surface())
                .highlight_style(Style::default().bg(theme::TOAST_EDGE).fg(theme::TEXT)),
            area,
            &mut state,
        );
        for (index, row) in rows
            .iter()
            .enumerate()
            .skip(state.offset())
            .take(usize::from(inner.height))
        {
            self.explorer_hits.borrow_mut().push((
                Rect::new(
                    inner.x,
                    inner.y + (index - state.offset()) as u16,
                    inner.width,
                    1,
                ),
                row.node,
            ));
        }
    }

    fn draw_folder_delete(&self, frame: &mut Frame) {
        let Some((entries, folders)) = self
            .pending_folder_delete
            .and_then(|id| self.session.as_ref()?.folder_contents(id))
        else {
            return;
        };
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(90);
        let text = localized!(self.language, "Folder: {}\n\nEntries to delete: {entries}\nNested folders: {folders}\n\nAll contents of nested folders will also be deleted.\nAn encrypted backup will be kept. This is not secure erasure.\n\n[d] Delete folder and contents   [Esc] Cancel", "Папка: {}\n\nБудут удалены записи: {entries}\nВложенные папки: {folders}\n\nВсё содержимое вложенных папок тоже будет удалено.\nЗашифрованный бэкап сохранится. Это не безопасное стирание.\n\n[d] Удалить папку и содержимое   [Esc] Отмена", self.folder_label());
        let lines = wrap_plain_text(&text, usize::from(width.saturating_sub(2)));
        let height = lines.len() + 2;
        if height > usize::from(area.height.saturating_sub(4)) {
            popup(frame, self.tr("The path and warnings do not fit.\nEnlarge the window to confirm.\n\nEsc — cancel", "Путь и предупреждения не помещаются.\nУвеличьте окно для подтверждения.\n\nEsc — отмена"), self.tr(" Deletion blocked ", " Удаление заблокировано "), 8);
            return;
        }
        let height = height as u16;
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        dim_backdrop(frame);
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .block(panel(self.tr(" Delete folder? ", " Удалить папку? ")))
                .style(theme::surface().fg(theme::WARNING)),
            rect,
        );
        self.folder_dialog_visible.set(true);
    }

    fn draw_folders(&self, frame: &mut Frame, moving: bool) {
        let area = frame.area();
        let options = self.folder_options(moving);
        let width = area.width.saturating_sub(6).min(82);
        let height = (options.len() as u16 + 4)
            .clamp(8, 24)
            .min(area.height.saturating_sub(4));
        if width < 30 || height < 8 {
            return;
        }
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        dim_backdrop(frame);
        frame.render_widget(Clear, rect);
        let block = panel(if moving {
            self.tr(" Move entry to folder ", " Переместить запись в папку ")
        } else {
            self.tr(" Open folder ", " Открыть папку ")
        });
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
        let items: Vec<_> = options
            .iter()
            .map(|(_, name)| ListItem::new(name.as_str()))
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .style(theme::surface())
                .highlight_style(theme::selected())
                .highlight_symbol("› "),
            rows[0],
            &mut ListState::default().with_selected(Some(self.folder_selected)),
        );
        frame.render_widget(
            Paragraph::new(if moving {
                self.tr(
                    "↑↓ select · Enter move · Esc back",
                    "↑↓ выбор · Enter переместить · Esc назад",
                )
            } else {
                self.tr(
                    "↑↓ select · Enter open · Esc back",
                    "↑↓ выбор · Enter открыть · Esc назад",
                )
            })
            .style(theme::surface().fg(theme::MUTED)),
            rows[1],
        );
        self.folder_dialog_visible.set(true);
    }

    fn draw_auth(&self, frame: &mut Frame) {
        let area = frame.area();
        let width = area.width.saturating_sub(8).min(72);
        let height = if self.creating { 15 } else { 12 };
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("term", base().fg(theme::MUTED).add_modifier(Modifier::BOLD)),
                Span::styled(
                    "vault",
                    base().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center),
            Rect::new(rect.x, rect.y, width, 1),
        );
        frame.render_widget(
            Paragraph::new(if self.creating {
                self.tr("Create vault", "Создать хранилище")
            } else {
                self.tr("Your passwords. Yours alone.", "Ваши пароли. Только у вас.")
            })
            .alignment(Alignment::Center)
            .style(base().fg(theme::MUTED)),
            Rect::new(rect.x, rect.y + 2, width, 1),
        );
        for index in 0..if self.creating { 2 } else { 1 } {
            let field_rect = Rect::new(rect.x, rect.y + 4 + index * 3, width, 3);
            let active = self.auth_field == usize::from(index);
            let color = if active {
                theme::PRIMARY
            } else {
                theme::BORDER
            };
            let input = if index == 0 {
                &self.master
            } else {
                &self.confirmation
            };
            let label = if index == 0 {
                self.tr("Master password", "Мастер-пароль")
            } else {
                self.tr("Confirm master password", "Повтор мастер-пароля")
            };
            frame.render_widget(
                Block::default()
                    .style(theme::surface())
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .border_style(Style::default().fg(color)),
                field_rect,
            );
            let value = if input.text().is_empty() {
                label.to_owned()
            } else {
                input.rendered(true, active)
            };
            frame.render_widget(
                Paragraph::new(value).style(theme::surface().fg(if input.text().is_empty() {
                    theme::MUTED
                } else {
                    theme::TEXT
                })),
                Rect::new(field_rect.x + 3, field_rect.y + 1, width - 6, 1),
            );
        }
        let hint_y = rect.y + if self.creating { 11 } else { 8 };
        frame.render_widget(
            Paragraph::new(if self.creating {
                self.tr(
                    "At least 12 characters · Enter create · Tab field",
                    "Не менее 12 символов · Enter создать · Tab поле",
                )
            } else {
                self.tr(
                    "Enter unlock · Ctrl+P commands · Esc quit",
                    "Enter войти · Ctrl+P команды · Esc выйти",
                )
            })
            .style(base().fg(theme::MUTED))
            .alignment(Alignment::Center),
            Rect::new(rect.x, hint_y, width, 1),
        );
        frame.render_widget(
            Paragraph::new(input::display(&self.path.display().to_string()))
                .style(base().fg(theme::MUTED))
                .alignment(Alignment::Center),
            Rect::new(rect.x, hint_y + 2, width, 1),
        );
        frame.render_widget(
            Paragraph::new(if self.browser_origin.is_some() {
                self.tr(
                    "Browser fill request: check the destination above before unlocking.",
                    "Автозаполнение: проверьте сайт выше перед разблокировкой.",
                )
            } else {
                self.tr("No password recovery", "Нет восстановления пароля")
            })
            .style(base().fg(theme::WARNING))
            .alignment(Alignment::Center),
            Rect::new(rect.x, hint_y + 3, width, 1),
        );
    }

    fn draw_sensitive(&self, frame: &mut Frame) {
        let Some(form) = &self.sensitive else {
            return;
        };
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(90);
        let height = 18.min(area.height.saturating_sub(2));
        if width < 56 || height < 18 {
            return;
        }
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let (title, description) = match form.kind {
            SensitiveKind::ChangeMaster => (
                self.tr(" Change master password ", " Сменить мастер-пароль "),
                self.tr("New password: at least 12 characters.\nIMPORTANT: previous backups require the OLD password.", "Минимум 12 символов в новом пароле.\nВАЖНО: прежние бэкапы требуют СТАРЫЙ пароль.")
                    .to_string(),
            ),
            SensitiveKind::DeleteVault => {
                let active_path = self
                    .session
                    .as_ref()
                    .map(|session| session.path())
                    .unwrap_or(&self.path);
                let filename = active_path
                    .file_name()
                    .map(|name| input::display(&name.to_string_lossy()))
                    .unwrap_or_default();
                let shown: String = filename.chars().take(80).collect();
                (self.tr(" Delete vault ", " Удалить хранилище "), localized!(self.language, "Filename: {shown}\nAfter entering the password and exact filename, another confirmation follows.\nAn encrypted backup will be kept.", "Имя файла: {shown}\nПосле ввода пароля и точного имени будет ещё одно подтверждение.\nЗашифрованный бэкап сохранится."))
            }
        };
        let block = panel(title);
        let inner = block.inner(rect);
        dim_backdrop(frame);
        frame.render_widget(Clear, rect);
        frame.render_widget(block, rect);
        let labels = form.labels(self.language);
        let mut constraints = vec![Constraint::Length(if labels.len() == 3 { 3 } else { 6 })];
        constraints.extend(std::iter::repeat_n(Constraint::Length(3), labels.len()));
        constraints.extend([Constraint::Length(3), Constraint::Length(1)]);
        let rows = Layout::vertical(constraints).split(inner);
        frame.render_widget(
            Paragraph::new(description)
                .wrap(Wrap { trim: false })
                .style(theme::surface().fg(theme::WARNING)),
            rows[0],
        );
        for (i, label) in labels.iter().enumerate() {
            let active = form.field == i;
            frame.render_widget(
                Paragraph::new(form.fields[i].rendered(form.masked(i), active))
                    .block(
                        panel(&format!(" {}{} ", if active { "› " } else { "" }, label))
                            .border_style(Style::default().fg(if active {
                                theme::PRIMARY
                            } else {
                                theme::BORDER
                            }))
                            .title_style(Style::default().fg(if active {
                                theme::PRIMARY
                            } else {
                                theme::MUTED
                            })),
                    )
                    .style(theme::surface()),
                rows[i + 1],
            );
        }
        frame.render_widget(
            Paragraph::new(i18n::error(
                self.language,
                form.error.as_deref().unwrap_or(""),
            ))
            .wrap(Wrap { trim: false })
            .style(theme::surface().fg(theme::ERROR)),
            rows[labels.len() + 1],
        );
        frame.render_widget(
            Paragraph::new(self.tr(
                "Tab field · Enter next/confirm · Esc cancel",
                "Tab поле · Enter далее/подтвердить · Esc отмена",
            ))
            .style(theme::surface().fg(theme::MUTED)),
            rows[labels.len() + 2],
        );
        self.sensitive_visible.set(true);
    }
    fn draw_vault_delete_confirmation(&self, frame: &mut Frame) {
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(100);
        let active_path = self
            .session
            .as_ref()
            .map(|session| session.path())
            .unwrap_or(&self.path);
        let text = localized!(self.language, "Active file:\n{}\n\nIt will be moved to an encrypted backup.\nOther backups will remain. This is not secure erasure.\nConfirmation expires after 60 seconds.\n\n[d] Delete active file   [Esc] Cancel", "Активный файл:\n{}\n\nОн будет перемещён в зашифрованный бэкап.\nОстальные бэкапы сохранятся. Это не безопасное стирание.\nПодтверждение действует 60 секунд.\n\n[d] Удалить активный файл   [Esc] Отмена", input::display(&active_path.display().to_string()));
        let lines = wrap_plain_text(&text, usize::from(width.saturating_sub(2)));
        let height = lines.len() + 2;
        if width < 20 || height > usize::from(area.height.saturating_sub(4).min(26)) {
            popup(frame, self.tr("The full path and warnings do not fit.\nEnlarge the window to confirm deletion.\n\n[Esc] Cancel", "Полный путь и предупреждения не помещаются.\nУвеличьте окно, чтобы подтвердить удаление.\n\n[Esc] Отмена"), self.tr(" Deletion blocked ", " Удаление заблокировано "), 8);
            return;
        }
        let height = height as u16;
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        dim_backdrop(frame);
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .block(panel(
                    self.tr(" Final confirmation ", " Окончательное подтверждение "),
                ))
                .style(theme::surface().fg(theme::WARNING)),
            rect,
        );
        self.vault_delete_visible.set(true);
    }

    fn draw_palette(&self, frame: &mut Frame) {
        let Some(palette) = &self.palette else {
            return;
        };
        let available = self.available_commands();
        if available.is_empty() {
            return;
        }
        let commands = palette.filtered(&available);
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(76);
        let groups = available
            .iter()
            .map(|command| command.group(self.language))
            .fold((0u16, ""), |(count, previous), group| {
                (count + u16::from(group != previous), group)
            })
            .0;
        // Keep the dialog size stable while filtering, and center on both axes.
        let height = (available.len() as u16 + groups * 2 + 7).min(area.height.saturating_sub(4));
        if width < 20 || height < 8 {
            return;
        }
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let muted = theme::MUTED;
        let accent = theme::PRIMARY;
        let normal = theme::surface();
        // Terminal cells have no alpha channel: dim the existing screen in place.
        dim_backdrop(frame);
        frame.render_widget(Clear, rect);
        frame.render_widget(Block::default().style(normal), rect);
        let inner = Rect::new(rect.x + 2, rect.y + 1, rect.width - 4, rect.height - 2);
        let rows = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
        let title = Rect::new(inner.x, inner.y, inner.width.saturating_sub(4), 1);
        frame.render_widget(
            Paragraph::new(self.tr("Commands", "Команды"))
                .style(normal.add_modifier(Modifier::BOLD)),
            title,
        );
        frame.render_widget(
            Paragraph::new("esc").style(normal.fg(muted)),
            Rect::new(inner.right() - 3, inner.y, 3, 1),
        );
        let query = if palette.query.text().is_empty() {
            Line::from(vec![
                Span::styled(self.tr("S", "П"), normal.fg(theme::INK).bg(accent)),
                Span::styled(self.tr("earch commands", "оиск команды"), normal.fg(muted)),
            ])
        } else {
            Line::from(palette.query.rendered(false, true))
        };
        frame.render_widget(
            Paragraph::new(query).style(normal),
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
        );
        if commands.is_empty() {
            frame.render_widget(
                Paragraph::new(self.tr("No matching commands", "Нет подходящих команд"))
                    .style(normal.fg(muted)),
                rows[1],
            );
        } else {
            let mut lines = Vec::new();
            let mut previous = "";
            let mut selected_row = 0;
            let mut command_rows = Vec::new();
            for (index, command) in commands.iter().enumerate() {
                if command.group(self.language) != previous {
                    if !previous.is_empty() {
                        lines.push(Line::default());
                    }
                    lines.push(Line::styled(
                        command.group(self.language),
                        normal.fg(theme::SECONDARY).add_modifier(Modifier::BOLD),
                    ));
                    previous = command.group(self.language);
                }
                let selected = index == palette.selected.min(commands.len() - 1);
                if selected {
                    selected_row = lines.len();
                }
                let style = if selected { theme::selected() } else { normal };
                command_rows.push((lines.len(), *command));
                let shortcut = command.shortcut();
                let label_width = usize::from(rows[1].width).saturating_sub(shortcut.len() + 1);
                let label: String = command
                    .label(self.language)
                    .chars()
                    .take(label_width)
                    .collect();
                let padding = usize::from(rows[1].width)
                    .saturating_sub(Line::from(label.as_str()).width() + shortcut.len());
                lines.push(Line::from(vec![
                    Span::styled(format!("{label}{}", " ".repeat(padding)), style),
                    Span::styled(shortcut, if selected { style } else { style.fg(muted) }),
                ]));
            }
            let offset = (selected_row + 1).saturating_sub(usize::from(rows[1].height));
            for (index, command) in command_rows {
                if index >= offset && index < offset + usize::from(rows[1].height) {
                    self.palette_hits.borrow_mut().push((
                        Rect::new(
                            rows[1].x,
                            rows[1].y + (index - offset) as u16,
                            rows[1].width,
                            1,
                        ),
                        command,
                    ));
                }
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .style(normal)
                    .scroll((offset as u16, 0)),
                rows[1],
            );
        }
        frame.render_widget(
            Paragraph::new(self.tr(
                "↑↓ select · Enter run · Esc cancel",
                "↑↓ выбор · Enter выполнить · Esc отмена",
            ))
            .style(normal.fg(muted)),
            rows[2],
        );
        self.palette_visible.set(true);
    }

    fn draw_notice(&self, frame: &mut Frame) {
        if self.status_until.is_none() || self.status.is_empty() {
            return;
        }
        let area = frame.area();
        // Keep the title and browser destination header visible. Do not draw over the footer.
        let width = (area.width / 2)
            .clamp(30, 52)
            .min(area.width.saturating_sub(2));
        if width < 8 || area.height < 7 {
            return;
        }
        let text = input::display(&self.status);
        let inner_width = usize::from(width - 6);
        let mut lines = wrap_plain_text(&text, inner_width);
        let height = (lines.len() + 2).min(8).min(usize::from(area.height - 3)) as u16;
        let rect = Rect::new(area.right() - width - 1, area.y + 2, width, height);
        let color = match self.status_kind {
            NoticeKind::Info => theme::TOAST_EDGE,
            NoticeKind::Warning => theme::WARNING,
            NoticeKind::Error => theme::ERROR,
        };
        let style = theme::surface().bg(theme::TOAST);
        frame.render_widget(Clear, rect);
        frame.render_widget(Block::default().style(style), rect);
        for x in [rect.x, rect.right() - 1] {
            frame.render_widget(
                Paragraph::new((0..height).map(|_| Line::from("▏")).collect::<Vec<_>>())
                    .style(style.fg(theme::TOAST_EDGE)),
                Rect::new(x, rect.y, 1, height),
            );
        }
        frame.render_widget(
            Paragraph::new((0..height).map(|_| Line::from("▎")).collect::<Vec<_>>())
                .style(style.fg(color)),
            Rect::new(rect.x + 1, rect.y, 1, height),
        );
        let content = Rect::new(rect.x + 3, rect.y + 1, width - 6, height - 2);
        let max_lines = usize::from(content.height);
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            if let Some(last) = lines.last_mut() {
                last.pop();
                last.push('…');
            }
        }
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()).style(style),
            content,
        );
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.lock();
    }
}
fn invert() -> Style {
    theme::selected()
}
fn panel(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_owned())
        .style(theme::surface())
        .border_style(Style::default().fg(theme::BORDER))
        .title_style(Style::default().fg(theme::SECONDARY))
}
fn dim_backdrop(frame: &mut Frame) {
    let area = frame.area();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            frame.buffer_mut()[(x, y)].set_style(
                Style::default()
                    .fg(theme::DIMMED)
                    .bg(theme::INK)
                    .remove_modifier(Modifier::BOLD | Modifier::REVERSED | Modifier::DIM),
            );
        }
    }
}
fn wrap_plain_text(text: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    for line in text.lines() {
        let mut row = String::new();
        let mut used = 0;
        for character in line.chars() {
            let cells = Line::from(character.to_string()).width();
            if used + cells > width.max(1) && !row.is_empty() {
                result.push(std::mem::take(&mut row));
                used = 0;
            }
            row.push(character);
            used += cells;
        }
        result.push(row);
    }
    result
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
    dim_backdrop(frame);
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
            DisableMouseCapture,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

pub fn run(
    path: PathBuf,
    creating: bool,
    lock_after: Duration,
    settings_file: PathBuf,
) -> Result<()> {
    let mut app = App::new(path, creating, lock_after);
    app.load_settings(settings_file);
    run_app(app).map(|_| ())
}

#[cfg(windows)]
pub fn run_browser(
    path: PathBuf,
    origin: String,
) -> Result<Option<Zeroizing<crate::browser::Credentials>>> {
    let mut app = App::new(path, false, Duration::from_secs(120));
    app.load_settings(settings::settings_file()?);
    app.browser_origin = Some(origin);
    run_app(app)
}

fn run_app(mut app: App) -> Result<Option<Zeroizing<crate::browser::Credentials>>> {
    enable_raw_mode()?;
    let _guard = Guard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    // Deliberately omit panic payloads: upstream formatting could contain secrets.
    std::panic::set_hook(Box::new(|_| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
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
                Event::Mouse(mouse) => app.mouse(mouse),
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
    Ok(app.browser_credentials.take())
}

#[cfg(test)]
#[path = "screenshots.rs"]
mod screenshots;

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
    fn screen_rows(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }
    fn switch_language(app: &mut App, language: Language) {
        palette_command(app, "language");
        assert!(app.language_choice.is_some());
        let text = screen(app);
        assert!(text.contains("English"));
        assert!(text.contains("Русский"));
        app.language_choice = Some(usize::from(language == Language::Russian));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.language, language);
        assert!(app.language_choice.is_none());
    }

    #[test]
    fn constructor_never_loads_settings_and_explicit_preferences_preserve_master_drafts() {
        let root = tempfile::tempdir().unwrap();
        settings::save_language(root.path(), Language::Russian).unwrap();
        let path = root.path().join("not-created.kdbx");
        let mut app = App::new(path.clone(), true, Duration::from_secs(120));
        assert_eq!(app.language, Language::English);
        assert!(app.settings_file.is_none());
        assert!(screen(&app).contains("Master password"));
        app.master = Input::from("DUMMY_MASTER_DRAFT");
        app.confirmation = Input::from("DUMMY_CONFIRM_DRAFT");
        app.auth_field = 1;
        app.load_preferences(root.path().to_owned());
        assert_eq!(app.language, Language::Russian);
        assert!(screen(&app).contains("Создать хранилище"));
        switch_language(&mut app, Language::English);
        assert_eq!(app.master.text(), "DUMMY_MASTER_DRAFT");
        assert_eq!(app.confirmation.text(), "DUMMY_CONFIRM_DRAFT");
        assert_eq!(app.auth_field, 1);
        assert!(!screen(&app).contains("DUMMY_MASTER"));
        assert!(!screen(&app).contains("DUMMY_CONFIRM"));
        assert!(!path.exists());
        assert_eq!(settings::language(root.path()), Language::English);
    }

    #[test]
    fn language_switch_preserves_every_draft_field_cursor_selection_query_and_expanded_tree() {
        let (root, mut app, work, mail, id) = explorer_fixture();
        let settings_root = root.path().join("preferences");
        app.load_preferences(settings_root.clone());
        app.expand_ancestors(mail);
        app.select_explorer(ExplorerNode::Entry(id));
        app.edit(false);
        let form = app.form.as_mut().unwrap();
        form.fields[0] = Input::from("My title Язык");
        form.fields[0].key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), false);
        form.fields[1] = Input::from("username");
        form.fields[2] = Input::from("DUMMY_LANGUAGE_SECRET");
        form.fields[3] = Input::from("https://example.test/path");
        form.fields[4] = Input::from("Title is required (maximum 256 bytes)\nNotes stay unchanged");
        form.field = 4;
        form.dirty = true;
        let fields: Vec<_> = form
            .fields
            .iter()
            .map(|field| field.rendered(false, true))
            .collect();
        let expanded = app.expanded_folders.clone();
        let focus = app.explorer_focus;
        let folder = app.folder;
        app.query = Input::from("retained search");
        let visible = app.visible.clone();
        let bytes = std::fs::read(&app.path).unwrap();
        for language in [Language::Russian, Language::English] {
            switch_language(&mut app, language);
            let form = app.form.as_ref().unwrap();
            assert_eq!(app.mode, Mode::Edit);
            assert_eq!(form.id, Some(id));
            assert_eq!(form.field, 4);
            assert!(form.dirty);
            assert_eq!(
                form.fields
                    .iter()
                    .map(|field| field.rendered(false, true))
                    .collect::<Vec<_>>(),
                fields
            );
            assert_eq!(app.expanded_folders, expanded);
            assert!(app.expanded_folders.contains(&work));
            assert_eq!(app.explorer_focus, focus);
            assert_eq!(app.folder, folder);
            assert_eq!(app.query.text(), "retained search");
            assert_eq!(app.visible, visible);
            assert_eq!(std::fs::read(&app.path).unwrap(), bytes);
            assert!(!screen(&app).contains("DUMMY_LANGUAGE_SECRET"));
            assert_eq!(settings::language(&settings_root), language);
        }
    }

    #[test]
    fn language_overlay_requires_visibility_cancels_and_consumes_paste_without_editing_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.paste("Keep this draft");
        app.execute_command(Command::Language);
        app.language_choice = Some(1);
        app.paste("Do not insert this");
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.language, Language::English);
        assert_eq!(
            app.form.as_ref().unwrap().fields[0].text(),
            "Keep this draft"
        );
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.language, Language::English);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.language_choice.is_none());
        assert_eq!(app.mode, Mode::Edit);
        app.execute_command(Command::Language);
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.language, Language::Russian);
        assert!(screen(&app).contains("Название"));
        app.execute_command(Command::Language);
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.language_choice.is_none());
        assert!(app.form.is_none());
        assert!(app.session.is_none());
    }

    #[test]
    fn language_persistence_failure_keeps_config_and_draft_and_reports_localized_warning() {
        let (root, mut app) = app();
        let settings_root = root.path().join("preferences");
        let folder = settings_root.join("TermVault");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("settings.json");
        std::fs::write(&path, "invalid configuration").unwrap();
        app.load_preferences(settings_root);
        app.edit(true);
        app.paste("Keep draft");
        switch_language(&mut app, Language::Russian);
        assert!(app.status.contains("Не удалось сохранить язык"));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "invalid configuration"
        );
        assert_eq!(app.form.as_ref().unwrap().fields[0].text(), "Keep draft");
    }

    #[test]
    fn language_never_bypasses_browser_readonly_or_confirmation_guards() {
        let (_root, mut app) = app();
        app.session.as_mut().unwrap().readonly = true;
        switch_language(&mut app, Language::Russian);
        assert!(app.session.as_ref().unwrap().readonly);
        for command in [
            Command::NewEntry,
            Command::DeleteVault,
            Command::ChangeMaster,
            Command::NewFolder,
        ] {
            app.execute_command(command);
            assert_eq!(app.mode, Mode::Browse);
            assert!(app.form.is_none());
            assert!(app.sensitive.is_none());
        }
        for mode in [
            Mode::Delete,
            Mode::Discard(Action::Quit),
            Mode::VaultAction,
            Mode::VaultDeleteConfirm,
            Mode::DeleteFolder,
            Mode::BrowserConfirm,
            Mode::UpdateConsent,
        ] {
            app.mode = mode;
            app.execute_command(Command::Language);
            assert!(app.language_choice.is_none());
            assert_eq!(app.mode, mode);
        }
        app.mode = Mode::Browse;
        app.browser_origin = Some("https://example.test".into());
        app.execute_command(Command::Language);
        assert!(app.language_choice.is_none());
        assert!(app.available_commands().is_empty());
    }

    #[test]
    fn russian_ui_localizes_forms_search_notices_and_stable_errors_but_not_entry_data() {
        let (_root, mut app) = app();
        app.language = Language::Russian;
        app.edit(true);
        assert!(screen(&app).contains("Название"));
        assert!(!app.save());
        assert!(app.status.contains("Укажите название"));
        let form = app.form.as_mut().unwrap();
        form.fields[0] = Input::from("No entries.");
        form.fields[1] = Input::from("Username");
        form.fields[4] = Input::from("Current master passphrase is incorrect");
        form.dirty = true;
        assert!(app.save());
        assert_eq!(app.status, "Запись сохранена");
        let text = screen(&app);
        assert!(text.contains("No entries."));
        assert!(text.contains("Username"));
        assert!(text.contains("Current master passphrase is incorrect"));
        assert!(text.contains("Логин:"));
        app.mode = Mode::Search;
        assert!(screen(&app).contains("Поиск по названиям…"));
        app.mode = Mode::Browse;
        app.open_palette();
        assert!(screen(&app).contains("Команды"));
        assert!(screen(&app).contains("Поиск команды"));
        app.palette = None;
        app.begin_sensitive(SensitiveKind::ChangeMaster);
        let text = screen(&app);
        assert!(text.contains("Текущий мастер-пароль"));
        assert!(text.contains("СТАРЫЙ пароль"));
    }

    #[test]
    fn localized_folder_confirmations_keep_visibility_guards_and_never_translate_folder_names() {
        let (_root, mut app, work, _, _) = explorer_fixture();
        app.language = Language::Russian;
        app.execute_command(Command::DeleteFolder);
        let bytes = std::fs::read(&app.path).unwrap();
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), bytes);
        let text = screen(&app);
        assert!(text.contains("Удалить папку?"));
        assert!(text.contains("Будут удалены записи: 1"));
        assert!(text.contains("Work"));
        assert!(app.folder_dialog_visible.get());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app
            .session
            .as_ref()
            .unwrap()
            .folder_contents(work)
            .is_some());
        app.mode = Mode::Delete;
        assert!(screen(&app).contains("Удалить выбранную запись?"));
        app.mode = Mode::Discard(Action::Quit);
        let text = screen_rows(&app, 40, 10).join("");
        assert!(text.contains("Сохранить"));
        assert!(text.contains("Отбросить"));
        assert!(!app.exit);
    }

    #[test]
    fn localized_update_and_browser_dialogs_are_render_only_and_preserve_consent() {
        let (_root, mut app) = app();
        app.language = Language::Russian;
        app.mode = Mode::UpdateConsent;
        let text = screen(&app);
        assert!(text.contains("Отправить запрос к api.github.com?"));
        assert!(app.update_rx.is_none());
        app.mode = Mode::UpdateResult;
        app.update_result = Some(UpdateStatus::Available("99.0.0".into()));
        assert!(screen(&app).contains("Доступна версия 99.0.0"));
        app.mode = Mode::BrowserConfirm;
        app.browser_origin = Some("https://example.test".into());
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.browser_credentials.is_none());
        let text = screen(&app);
        assert!(text.contains("Подтвердить автозаполнение"));
        assert!(text.contains("Одобрить один раз"));
        assert!(app.browser_credentials.is_none());
    }

    #[test]
    fn localized_vault_deletion_confirmation_cannot_execute_without_full_visible_warning() {
        let (_root, mut app) = app();
        app.language = Language::Russian;
        app.delete_authorization = Some(
            app.session
                .as_ref()
                .unwrap()
                .authorize_delete("test master passphrase", "test.kdbx")
                .unwrap(),
        );
        app.mode = Mode::VaultDeleteConfirm;
        let before = std::fs::read(&app.path).unwrap();
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let text = screen(&app);
        assert!(text.contains("Окончательное подтверждение"));
        assert!(text.contains("Это не безопасное стирание"));
        assert!(text.contains("60 секунд"));
        assert!(app.vault_delete_visible.get());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.delete_authorization.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
    }

    #[test]
    fn language_selection_is_mouse_accessible_without_activating_background_commands() {
        let (_root, mut app) = app();
        app.edit(true);
        app.paste("Retained draft");
        app.execute_command(Command::Language);
        let _ = screen(&app);
        let hit = app
            .language_hits
            .borrow()
            .iter()
            .find(|(_, choice)| *choice == 1)
            .unwrap()
            .0;
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 1,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.language, Language::Russian);
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(
            app.form.as_ref().unwrap().fields[0].text(),
            "Retained draft"
        );
        assert!(app.form.as_ref().unwrap().dirty);
        assert!(!app.exit);
    }

    #[test]
    fn language_preserves_active_search_and_literal_user_titles_matching_ui_labels() {
        let (_root, mut app) = app();
        let id = app
            .session
            .as_mut()
            .unwrap()
            .update(
                None,
                &EntryData {
                    title: Zeroizing::new("Untitled".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        app.refresh(Some(id));
        app.mode = Mode::Search;
        app.query = Input::from("Untitled");
        app.filter(Some(id));
        let selected = app.selected;
        switch_language(&mut app, Language::Russian);
        assert_eq!(app.mode, Mode::Search);
        assert_eq!(app.query.text(), "Untitled");
        assert_eq!(app.selected, selected);
        assert_eq!(app.selected_id(), Some(id));
        assert_eq!(app.entry_label(id, "Untitled"), "Untitled");
        assert!(screen(&app).contains("Untitled"));
        assert_eq!(
            &*app.session.as_ref().unwrap().entry(id).unwrap().title,
            "Untitled"
        );
    }
    #[test]
    fn notifications_are_top_right_and_not_duplicated_in_footer() {
        let (_root, app) = app();
        let rows = screen_rows(&app, 110, 32);
        let (y, row) = rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.contains("Vault unlocked"))
            .unwrap();
        assert!(y < 8);
        assert!(row.find("Vault unlocked").unwrap() >= 55);
        assert!(!rows[28..].join("").contains("Vault unlocked"));
        assert_eq!(rows.join("").matches("Enter edit").count(), 1);
        assert_eq!(rows.join("").matches("Ctrl+L lock").count(), 1);
    }
    fn palette_command(app: &mut App, query: &str) {
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.palette.is_some());
        app.paste(query);
        assert!(screen(app).contains(app.tr("Commands", "Команды")));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    #[test]
    fn folder_commands_create_nested_folders_save_filter_and_move_records() {
        let (_root, mut app) = app();
        palette_command(&mut app, "new folder");
        assert_eq!(app.mode, Mode::NewFolder);
        app.paste("Работа");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let work = app.folder.unwrap();
        assert_eq!(app.folder_label(), "/Работа");
        palette_command(&mut app, "new folder");
        app.paste("Почта");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mail = app.folder.unwrap();
        assert_ne!(mail, work);
        assert_eq!(app.folder_label(), "/Работа / Почта");
        app.edit(true);
        let form = app.form.as_mut().unwrap();
        form.fields[0] = Input::from("Folder entry");
        form.fields[2] = Input::from("FOLDER_DUMMY_SECRET");
        form.dirty = true;
        assert!(app.save());
        let id = app.selected_id().unwrap();
        assert_eq!(app.session.as_ref().unwrap().entry_folder(id), Some(mail));
        palette_command(&mut app, "open folders");
        let options = app.folder_options(false);
        app.folder_selected = options
            .iter()
            .position(|item| item.0 == Some(work))
            .unwrap();
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.visible.is_empty()); // A folder shows its own records, not subfolder records.
        palette_command(&mut app, "open folders");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // All entries.
        assert!(app.folder.is_none());
        assert!(app
            .explorer_rows()
            .iter()
            .any(|row| row.node == ExplorerNode::Entry(id)));
        app.select_explorer(ExplorerNode::Entry(id));
        assert_eq!(app.selected_id(), Some(id));
        palette_command(&mut app, "move entry folder");
        let options = app.folder_options(true);
        app.folder_selected = options
            .iter()
            .position(|item| item.0 == Some(work))
            .unwrap();
        assert!(!screen(&app).contains("FOLDER_DUMMY_SECRET"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.folder, Some(work));
        assert_eq!(app.selected_id(), Some(id));
        let reopened = Session::unlock(&app.path, "test master passphrase").unwrap();
        assert_eq!(reopened.entry_folder(id), Some(work));
        assert_eq!(
            &*reopened.entry(id).unwrap().password,
            "FOLDER_DUMMY_SECRET"
        );
    }
    fn explorer_fixture() -> (tempfile::TempDir, App, GroupId, GroupId, EntryId) {
        let (root, mut app) = app();
        let session = app.session.as_mut().unwrap();
        let work = session.create_folder(None, "Work").unwrap();
        let mail = session.create_folder(Some(work), "Mail").unwrap();
        let id = session
            .update_in_folder(
                None,
                &EntryData {
                    title: Zeroizing::new("Nested login".into()),
                    password: Zeroizing::new("TREE_DUMMY_SECRET".into()),
                    url: Zeroizing::new("https://example.test".into()),
                    ..Default::default()
                },
                Some(mail),
            )
            .unwrap();
        app.clear_notice();
        app.refresh(None);
        app.select_explorer(ExplorerNode::Folder(work));
        (root, app, work, mail, id)
    }

    #[test]
    fn explorer_expands_collapses_and_navigates_parents_without_revealing_passwords() {
        let (_root, mut app, work, mail, id) = explorer_fixture();
        assert_eq!(app.explorer_focus, Some(ExplorerNode::Folder(work)));
        assert_eq!(app.explorer_rows().len(), 2);
        assert!(app.selected_id().is_none());
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.explorer_rows().len(), 3);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.explorer_focus, Some(ExplorerNode::Folder(mail)));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen(&app).contains("Nested login"));
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.selected_id(), Some(id));
        assert!(!screen(&app).contains("TREE_DUMMY_SECRET"));
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.explorer_focus, Some(ExplorerNode::Folder(mail)));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.explorer_rows().len(), 3);
        assert!(!screen(&app).contains("Nested login"));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::DeleteFolder);
        assert_eq!(app.pending_folder_delete, Some(work));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.pending_folder_delete.is_none());
    }

    #[test]
    fn clicking_delete_folder_in_palette_never_opens_create_folder() {
        let (_root, mut app, work, _, _) = explorer_fixture();
        let before = std::fs::read(&app.path).unwrap();
        app.open_palette();
        app.paste("пап");
        let _ = screen(&app);
        let hit = app
            .palette_hits
            .borrow()
            .iter()
            .find(|(_, command)| *command == Command::DeleteFolder)
            .unwrap()
            .0;
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 1,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.mode, Mode::DeleteFolder);
        assert_eq!(app.pending_folder_delete, Some(work));
        assert!(screen(&app).contains("Delete folder?"));
        assert!(!screen(&app).contains("New folder"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.open_palette();
        app.paste("пап");
        let commands = app
            .palette
            .as_ref()
            .unwrap()
            .filtered(&app.available_commands());
        let index = commands
            .iter()
            .position(|command| *command == Command::DeleteFolder)
            .unwrap();
        for _ in 0..index {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::DeleteFolder);
        assert_eq!(app.pending_folder_delete, Some(work));
    }

    #[test]
    fn explorer_mouse_toggles_only_clicked_folder_and_search_and_browser_include_collapsed_entries()
    {
        let (_root, mut app, work, _, id) = explorer_fixture();
        let _ = screen(&app);
        let hit = app
            .explorer_hits
            .borrow()
            .iter()
            .find(|(_, node)| *node == ExplorerNode::Folder(work))
            .unwrap()
            .0;
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 1,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.expanded_folders.contains(&work));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(!app.expanded_folders.contains(&work));
        app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.paste("Nested");
        assert!(!app.explorer_active());
        assert_eq!(app.selected_id(), Some(id));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.browser_origin = Some("https://example.test".into());
        app.filter(None);
        assert!(!app.explorer_active());
        assert_eq!(app.selected_id(), Some(id));
        app.browser_origin = None;
        app.lock();
        assert!(app.explorer_focus.is_none());
        assert!(app.expanded_folders.is_empty());
        assert!(app.explorer_hits.borrow().is_empty());
    }
    #[test]
    fn folder_deletion_requires_a_visible_confirmation_and_can_be_cancelled() {
        let (_root, mut app) = app();
        let session = app.session.as_mut().unwrap();
        let folder = session.create_folder(None, "Delete me").unwrap();
        let nested = session.create_folder(Some(folder), "Nested").unwrap();
        session
            .update_in_folder(
                None,
                &EntryData {
                    title: Zeroizing::new("Dummy".into()),
                    ..Default::default()
                },
                Some(nested),
            )
            .unwrap();
        app.folder = Some(folder);
        app.refresh(None);
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "delete folder");
        assert_eq!(app.mode, Mode::DeleteFolder);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let text = screen(&app);
        assert!(text.contains("Entries to delete: 1"));
        assert!(text.contains("Nested folders: 1"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::DeleteFolder);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        palette_command(&mut app, "delete folder");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Browse);
        let session = app.session.as_ref().unwrap();
        assert!(session.folder_contents(folder).is_none());
        assert_eq!(app.folder, Some(session.root_folder()));
    }

    #[test]
    fn delete_folder_is_unavailable_for_all_entries_root_readonly_browser_and_drafts() {
        let (_root, mut app) = app();
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
        let root = app.session.as_ref().unwrap().root_folder();
        app.folder = Some(root);
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
        let folder = app
            .session
            .as_mut()
            .unwrap()
            .create_folder(None, "Folder")
            .unwrap();
        app.folder = Some(folder);
        assert!(app.available_commands().contains(&Command::DeleteFolder));
        app.session.as_mut().unwrap().readonly = true;
        app.execute_command(Command::DeleteFolder);
        assert_eq!(app.mode, Mode::Browse);
        app.session.as_mut().unwrap().readonly = false;
        app.browser_origin = Some("https://example.test".into());
        app.execute_command(Command::DeleteFolder);
        assert_eq!(app.mode, Mode::Browse);
        app.browser_origin = None;
        app.edit(true);
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
    }

    #[test]
    fn folder_forms_require_visibility_cancel_cleanly_and_respect_session_restrictions() {
        let (_root, mut app) = app();
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "new folder");
        app.paste("Cancelled");
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.folder_name.text().is_empty());
        app.session.as_mut().unwrap().readonly = true;
        assert!(app.available_commands().contains(&Command::OpenFolder));
        for command in [Command::NewFolder, Command::MoveToFolder] {
            app.execute_command(command);
            assert_eq!(app.mode, Mode::Browse);
        }
        app.browser_origin = Some("https://example.test".into());
        app.execute_command(Command::OpenFolder);
        assert_eq!(app.mode, Mode::Browse);
        app.browser_origin = None;
        app.session.as_mut().unwrap().readonly = false;
        palette_command(&mut app, "new folder");
        app.paste("AUTO_LOCK_FOLDER");
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.folder_name.text().is_empty());
        assert!(app.folder.is_none());
        assert!(app.session.is_none());
    }
    #[test]
    fn palette_is_centered_dims_background_and_keeps_geometry_when_filtering() {
        let (_root, mut app) = app();
        app.open_palette();
        for (width, height) in [(110, 40), (80, 24), (60, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let bounds = |buffer: &ratatui::buffer::Buffer| {
                let cells: Vec<_> = (0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .filter(|&(x, y)| buffer[(x, y)].bg == theme::SURFACE)
                    .collect();
                (
                    cells.iter().map(|c| c.0).min().unwrap(),
                    cells.iter().map(|c| c.1).min().unwrap(),
                    cells.iter().map(|c| c.0).max().unwrap(),
                    cells.iter().map(|c| c.1).max().unwrap(),
                )
            };
            let before = bounds(buffer);
            assert!((i32::from(before.0) - i32::from(width - before.2 - 1)).abs() <= 1);
            assert!((i32::from(before.1) - i32::from(height - before.3 - 1)).abs() <= 1);
            assert_eq!(buffer[(0, 0)].fg, theme::DIMMED);
            assert!(buffer
                .content()
                .iter()
                .any(|cell| cell.bg == theme::PRIMARY && cell.fg == theme::INK));
            app.paste("quit");
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert_eq!(bounds(terminal.backend().buffer()), before);
            app.palette.as_mut().unwrap().query.clear();
        }
    }

    #[test]
    fn palette_scrolls_to_selected_command_without_selecting_group_headers() {
        let (_root, mut app) = app();
        app.open_palette();
        let last = app.available_commands().len() - 1;
        app.palette.as_mut().unwrap().selected = last;
        let rows = screen_rows(&app, 60, 20);
        assert!(rows.join("").contains("Quit"));
        assert!(app.palette_visible.get());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.exit);
    }
    #[test]
    fn ctrl_p_opens_commands_and_ctrl_o_reveals_without_changing_the_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.form.as_mut().unwrap().fields[2] = Input::from("PALETTE_DUMMY_SECRET");
        app.form.as_mut().unwrap().dirty = true;
        app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(screen(&app).contains("PALETTE_DUMMY_SECRET"));
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.palette.is_some());
        assert!(app.reveal_until.is_none());
        assert!(!screen(&app).contains("PALETTE_DUMMY_SECRET"));
        app.paste("generate");
        assert_eq!(
            app.form.as_ref().unwrap().fields[2].text(),
            "PALETTE_DUMMY_SECRET"
        );
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.palette.is_none());
        assert_eq!(app.mode, Mode::Edit);
        app.key(KeyEvent::new(KeyCode::Char('щ'), KeyModifiers::CONTROL));
        assert!(app.reveal_until.is_some());
        app.key(KeyEvent::new(KeyCode::Char('щ'), KeyModifiers::CONTROL));
        assert!(app.reveal_until.is_none());
        app.key(KeyEvent::new(KeyCode::Char('з'), KeyModifiers::CONTROL));
        assert_eq!(app.palette.as_ref().unwrap().query.text(), "");
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.palette.is_none());
    }
    #[test]
    fn palette_can_save_but_never_discards_a_dirty_form_on_lock_exit_or_back() {
        let (_root, mut app) = app();
        app.edit(true);
        let form = app.form.as_mut().unwrap();
        form.fields[0] = Input::from("Palette demo");
        form.dirty = true;
        for (query, action) in [
            ("lock", Action::Lock),
            ("quit", Action::Quit),
            ("back", Action::Browse),
        ] {
            palette_command(&mut app, query);
            assert!(app.palette.is_none());
            assert_eq!(app.mode, Mode::Discard(action));
            assert!(app.form.as_ref().unwrap().dirty);
            assert!(app.session.is_some());
            assert!(!app.exit);
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        }
        palette_command(&mut app, "save");
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.form.is_none());
        assert_eq!(app.entries.len(), 1);
        assert_eq!(app.status, "Entry saved");
    }
    #[test]
    fn palette_delete_opens_confirmation_instead_of_deleting_a_record() {
        let (_root, mut app) = app();
        app.session
            .as_mut()
            .unwrap()
            .update(
                None,
                &EntryData {
                    title: Zeroizing::new("Keep me".into()),
                    ..EntryData::default()
                },
            )
            .unwrap();
        app.refresh(None);
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "delete");
        assert_eq!(app.mode, Mode::Delete);
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.palette.is_none());
        assert_eq!(app.mode, Mode::Delete);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.entries.len(), 1);
    }
    #[test]
    fn palette_respects_readonly_and_browser_restrictions_at_execution_time() {
        let (_root, mut app) = app();
        app.session.as_mut().unwrap().readonly = true;
        let before = std::fs::read(&app.path).unwrap();
        assert!(!app.available_commands().contains(&Command::NewEntry));
        app.execute_command(Command::NewEntry);
        app.execute_command(Command::DeleteEntry);
        assert!(app.form.is_none());
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.browser_origin = Some("https://example.com".into());
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(app.palette.is_none());
        assert!(app.reveal_until.is_none());
        assert!(app.available_commands().is_empty());
    }
    #[test]
    fn auto_lock_closes_palette_and_discards_its_query_and_dirty_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.form.as_mut().unwrap().dirty = true;
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.paste("DUMMY_QUERY_MUST_DISAPPEAR");
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.palette.is_none());
        assert!(app.form.is_none());
        assert!(app.session.is_none());
        assert!(!screen(&app).contains("DUMMY_QUERY_MUST_DISAPPEAR"));
    }
    #[test]
    fn locked_palette_returns_to_masked_input_without_submitting_master_password() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().join("new.kdbx"), true, Duration::from_secs(120));
        app.master.insert("DUMMY_MASTER", false);
        palette_command(&mut app, "unlock");
        assert!(app.palette.is_none());
        assert!(app.session.is_none());
        assert_eq!(app.master.text(), "DUMMY_MASTER");
        assert!(!screen(&app).contains("DUMMY_MASTER"));
        assert!(!app.path.exists());
    }
    #[test]
    fn invisible_palette_and_unknown_commands_do_not_execute_actions() {
        let (_root, mut app) = app();
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.form.is_none());
        assert!(app.palette.is_some());
        app.paste("powershell.exe remove-item");
        assert!(screen(&app).contains("No matching commands"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.form.is_none());
        assert!(!app.exit);
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(app.palette.is_none());
        assert!(app.session.is_none());
    }
    fn fill_sensitive(app: &mut App, values: &[&str]) {
        for value in values {
            app.paste(value);
            assert!(!screen(app).contains("UI_DUMMY_MASTER"));
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
    }
    #[test]
    fn master_change_uses_a_separate_masked_form_and_retries_without_retaining_inputs() {
        let (_root, mut app) = app();
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "change master");
        assert_eq!(app.mode, Mode::VaultAction);
        fill_sensitive(
            &mut app,
            &[
                "test master passphrase",
                "UI_DUMMY_MASTER_new",
                "mismatched new master",
            ],
        );
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let form = app.sensitive.as_ref().unwrap();
        assert!(form.error.is_some());
        assert!(form.fields.iter().all(|field| field.text().is_empty()));
        fill_sensitive(
            &mut app,
            &[
                "test master passphrase",
                "UI_DUMMY_MASTER_new",
                "UI_DUMMY_MASTER_new",
            ],
        );
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.sensitive.is_none());
        assert!(Session::unlock(&app.path, "UI_DUMMY_MASTER_new").is_ok());
        assert!(Session::unlock(&app.path, "test master passphrase").is_err());
        assert!(!screen(&app).contains("UI_DUMMY_MASTER"));
    }
    #[test]
    fn sensitive_inputs_cannot_be_revealed_or_sent_to_the_palette_and_auto_lock_drops_them() {
        let (_root, mut app) = app();
        palette_command(&mut app, "change master");
        app.paste("UI_DUMMY_MASTER_current");
        app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.palette.is_none());
        assert!(app.reveal_until.is_none());
        assert!(!screen(&app).contains("UI_DUMMY_MASTER"));
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.sensitive.is_none());
        assert!(app.delete_authorization.is_none());
        assert!(app.session.is_none());
    }
    #[test]
    fn whole_vault_deletion_requires_reauthentication_filename_and_a_visible_final_confirmation() {
        let (_root, mut app) = app();
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "delete vault");
        assert_eq!(app.mode, Mode::VaultAction);
        fill_sensitive(&mut app, &["test master passphrase", "wrong.kdbx"]);
        assert!(app.delete_authorization.is_none());
        assert!(app
            .sensitive
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .all(|field| field.text().is_empty()));
        fill_sensitive(&mut app, &["test master passphrase", "test.kdbx"]);
        assert_eq!(app.mode, Mode::VaultDeleteConfirm);
        assert!(app.sensitive.is_none());
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)); // No rendered confirmation yet.
        assert!(app.path.exists());
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(app.path.exists());
        assert!(screen(&app).contains("Final confirmation"));
        assert!(app.vault_delete_visible.get());
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::VaultDeleted);
        assert!(app.session.is_none());
        assert!(app.delete_authorization.is_none());
        assert!(!app.path.exists());
        assert_eq!(
            std::fs::read(app.deleted_backup.as_ref().unwrap()).unwrap(),
            before
        );
        assert_eq!(app.available_commands(), [Command::Quit]);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.exit);
    }
    #[test]
    fn deleting_vault_can_be_cancelled_and_does_not_run_from_an_invisible_form() {
        let (_root, mut app) = app();
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "delete vault");
        let form = app.sensitive.as_mut().unwrap();
        form.fields[0] = Input::from("test master passphrase");
        form.fields[1] = Input::from("test.kdbx");
        form.field = 1;
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.delete_authorization.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::VaultDeleteConfirm);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.delete_authorization.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
    }
    #[test]
    fn update_checks_require_visible_consent_and_results_do_not_discard_an_entry_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.form.as_mut().unwrap().fields[0] = Input::from("Keep this draft");
        app.form.as_mut().unwrap().dirty = true;
        palette_command(&mut app, "check updates");
        assert_eq!(app.mode, Mode::UpdateConsent);
        assert!(app.update_rx.is_none());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // Unrendered consent must not start a request.
        assert!(app.update_rx.is_none());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Edit);
        let (sender, receiver) = std::sync::mpsc::channel();
        app.update_rx = Some(receiver);
        sender
            .send(Ok(UpdateStatus::Available("99.0.0".into())))
            .unwrap();
        assert!(app.tick(Instant::now()));
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(
            app.form.as_ref().unwrap().fields[0].text(),
            "Keep this draft"
        );
        palette_command(&mut app, "update result");
        assert_eq!(app.mode, Mode::UpdateResult);
        assert!(screen(&app).contains("99.0.0"));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Edit);
        assert!(app.form.as_ref().unwrap().dirty);
    }
    #[test]
    fn readonly_and_browser_modes_never_offer_sensitive_commands() {
        let (_root, mut app) = app();
        app.session.as_mut().unwrap().readonly = true;
        for command in [Command::ChangeMaster, Command::DeleteVault] {
            assert!(!app.available_commands().contains(&command));
            app.execute_command(command);
            assert!(app.sensitive.is_none());
        }
        assert!(app.available_commands().contains(&Command::CheckUpdates));
        app.browser_origin = Some("https://example.com".into());
        assert!(app.available_commands().is_empty());
        app.execute_command(Command::CheckUpdates);
        assert!(app.update_rx.is_none());
        assert_eq!(app.mode, Mode::Browse);
    }
    #[test]
    fn notifications_expire_without_input_or_extending_unlock_activity() {
        let (_root, mut app) = app();
        let deadline = app.status_until.unwrap();
        let activity = app.last_activity;
        assert!(!app.tick(deadline - Duration::from_millis(1)));
        assert!(!app.status.is_empty());
        assert!(app.tick(deadline));
        assert!(app.status.is_empty());
        assert!(app.status_until.is_none());
        assert!(app.session.is_some());
        assert_eq!(app.last_activity, activity);
        assert!(!screen(&app).contains("Уведомление"));
    }
    #[test]
    fn notification_cards_use_semantic_colors_and_a_borderless_surface() {
        let (_root, mut app) = app();
        for (kind, color) in [
            (NoticeKind::Info, theme::TOAST_EDGE),
            (NoticeKind::Warning, theme::WARNING),
            (NoticeKind::Error, theme::ERROR),
        ] {
            app.notify("Test notification", kind);
            let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(57, 2)].fg, theme::TOAST_EDGE);
            assert_eq!(buffer[(108, 2)].fg, theme::TOAST_EDGE);
            assert_eq!(buffer[(58, 2)].symbol(), "▎");
            assert_eq!(buffer[(58, 2)].fg, color);
            assert_eq!(buffer[(59, 2)].bg, theme::TOAST);
            assert_eq!(buffer[(60, 3)].fg, theme::TEXT);
            assert!(screen(&app).contains("Test notification"));
            assert!(!screen(&app).contains("Уведомление"));
            assert_eq!(buffer[(0, 31)].bg, theme::BACKGROUND);
        }
    }

    #[test]
    fn centered_auth_keeps_both_master_inputs_masked_and_handles_small_windows() {
        let (_root, mut app) = app();
        app.lock();
        app.clear_notice();
        for creating in [false, true] {
            app.creating = creating;
            app.master = Input::from("AUTH_DUMMY_SECRET");
            app.confirmation = Input::from("CONFIRM_DUMMY_SECRET");
            for (width, height) in [(110, 32), (80, 24), (60, 20)] {
                let rows = screen_rows(&app, width, height);
                let text = rows.join("");
                assert!(text.contains("termvault"));
                assert!(!text.contains("AUTH_DUMMY_SECRET"));
                assert!(!text.contains("CONFIRM_DUMMY_SECRET"));
                let title = rows.iter().find(|row| row.contains("termvault")).unwrap();
                let left = title.chars().take_while(|c| *c == ' ').count();
                assert!((left as i32 - (i32::from(width) - 9) / 2).abs() <= 1);
                assert_eq!(text.matches("termvault").count(), 1);
            }
        }
    }

    #[test]
    fn editor_focus_and_modal_backdrops_use_the_shared_theme() {
        let (_root, mut app) = app();
        app.edit(true);
        let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(terminal.backend().buffer()[(0, 2)].fg, theme::PRIMARY);
        assert_eq!(terminal.backend().buffer()[(0, 5)].fg, theme::BORDER);
        app.form.as_mut().unwrap().dirty = true;
        app.leave(Action::Quit);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(terminal.backend().buffer()[(0, 0)].fg, theme::DIMMED);
        assert!(!app.exit);
    }
    #[test]
    fn saved_and_copied_notifications_never_contain_credentials() {
        let (_root, mut app) = app();
        app.clipboard = Clipboard::memory_for_test();
        app.edit(true);
        assert!(app.status_until.is_none());
        let form = app.form.as_mut().unwrap();
        form.fields[0] = Input::from("Demo");
        form.fields[1] = Input::from("dummy-user");
        form.fields[2] = Input::from("dummy-secret");
        form.dirty = true;
        assert!(app.save());
        assert_eq!(app.status, "Entry saved");
        assert!(screen(&app).contains("Entry saved"));
        app.copy(false);
        assert!(app.status.starts_with("Password copied"));
        assert!(!app.status.contains("dummy-secret"));
        assert!(!screen(&app).contains("dummy-secret"));
        app.copy(true);
        assert!(app.status.starts_with("Username copied"));
        assert!(!app.status.contains("dummy-user"));
        app.lock();
        assert_eq!(app.status, "Vault locked");
    }
    #[test]
    fn notifications_fit_small_terminals_and_are_replaced_by_new_events() {
        let (_root, mut app) = app();
        let info_deadline = app.status_until.unwrap();
        app.error("Cannot save. The vault changed externally; reopen it before retrying.");
        assert_eq!(app.status_kind, NoticeKind::Error);
        assert!(app.status_until.unwrap() > info_deadline);
        for (width, height) in [(1, 1), (40, 10), (60, 20), (80, 24), (110, 32)] {
            let rows = screen_rows(&app, width, height);
            assert_eq!(rows.len(), usize::from(height));
        }
        app.info("Запись сохранена");
        assert_eq!(app.status_kind, NoticeKind::Info);
        assert!(!screen(&app).contains("Cannot save"));
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

    #[test]
    fn browser_release_requires_exact_origin_and_two_explicit_actions() {
        let (_root, mut app) = app();
        for (title, url) in [
            ("Good", "https://example.com/login"),
            ("Bad", "https://example.com.evil.org"),
            ("Subdomain", "https://sub.example.com"),
        ] {
            app.session
                .as_mut()
                .unwrap()
                .update(
                    None,
                    &EntryData {
                        title: Zeroizing::new(title.into()),
                        url: Zeroizing::new(url.into()),
                        username: Zeroizing::new("test-user".into()),
                        password: Zeroizing::new("dummy-secret".into()),
                        ..EntryData::default()
                    },
                )
                .unwrap();
        }
        let before = std::fs::read(&app.path).unwrap();
        app.browser_origin = Some("https://example.com".into());
        app.refresh(None);
        assert_eq!(app.visible.len(), 1);
        assert!(screen(&app).contains("Good"));
        assert!(!screen(&app).contains("dummy-secret"));
        for key in ['n', 'd', 'g', 'p', 'c', 'u', 'y'] {
            app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        }
        assert!(app.form.is_none());
        assert!(app.reveal_until.is_none());
        assert!(app.browser_credentials.is_none());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::BrowserConfirm);
        assert!(app.browser_credentials.is_none());
        let mut small = Terminal::new(TestBackend::new(40, 10)).unwrap();
        small.draw(|frame| app.draw(frame)).unwrap();
        app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.browser_credentials.is_none());
        assert!(screen(&app).contains("https://example.com"));
        app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let result = app.browser_credentials.as_ref().unwrap();
        assert_eq!(result.password, "dummy-secret");
        assert_eq!(result.origin, "https://example.com");
        assert!(app.exit);
        assert!(app.session.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
    }

    #[test]
    fn browser_cancel_and_auto_lock_release_nothing() {
        let (_root, mut app) = app();
        app.browser_origin = Some("https://example.com".into());
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.session.is_none());
        assert!(app.browser_credentials.is_none());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.exit);
        assert!(app.browser_credentials.is_none());
    }
}
