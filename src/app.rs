use crate::{
    clipboard::Clipboard,
    commands::{Command, Palette},
    i18n::{self, localized, Language},
    input::{self, Input},
    sensitive::{FormEvent, SensitiveForm, SensitiveKind},
    settings,
    updates::{self, UpdateStatus},
    vault::{DeleteAuthorization, EntryData, Session},
};
use anyhow::Result;
#[cfg(test)]
use crossterm::event::KeyModifiers;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use keepass::db::{EntryId, GroupId};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io,
    path::PathBuf,
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

mod policy;
mod render;
mod runtime;
use policy::Invocation;
use render::RenderFeedback;
use runtime::RuntimeState;

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
    runtime: RuntimeState,
    render: RenderFeedback,
    language: Language,
    settings_file: Option<PathBuf>,
    language_choice: Option<usize>,
    path: PathBuf,
    creating: bool,
    auth_field: usize,
    update_result: Option<UpdateStatus>,
    clipboard: Clipboard,
    last_activity: Instant,
    lock_after: Duration,
    status: String,
    status_until: Option<Instant>,
    status_kind: NoticeKind,
    exit: bool,
    browser_origin: Option<String>,
    browser_credentials: Option<Zeroizing<crate::browser::Credentials>>,
}

impl App {
    fn new(path: PathBuf, creating: bool, lock_after: Duration) -> Self {
        Self {
            runtime: RuntimeState::default(),
            render: RenderFeedback::default(),
            language: Language::default(),
            settings_file: None,
            language_choice: None,
            path,
            creating,
            auth_field: 0,
            update_result: None,
            clipboard: Clipboard::new(),
            last_activity: Instant::now(),
            lock_after,
            status: String::new(),
            status_until: None,
            status_kind: NoticeKind::Info,
            exit: false,
            browser_origin: None,
            browser_credentials: None,
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
        if !self.render.language_visible.get() {
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
        if self.creating && self.runtime.master.text() != self.runtime.confirmation.text() {
            self.error("Master passwords do not match");
            self.runtime.master.clear();
            self.runtime.confirmation.clear();
            self.auth_field = 0;
            return;
        }
        let result = if self.creating {
            Session::create(&self.path, self.runtime.master.text())
        } else {
            Session::unlock(&self.path, self.runtime.master.text())
        };
        self.runtime.master.clear();
        self.runtime.confirmation.clear();
        self.auth_field = 0;
        match result {
            Ok(session) => {
                self.path = session.path().to_owned();
                let readonly = session.readonly;
                self.runtime.session = Some(session);
                self.creating = false;
                self.runtime.mode = Mode::Browse;
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
        for (_, title) in &mut self.runtime.entries {
            title.zeroize();
        }
        self.runtime.entries.clear();
        self.runtime.visible.clear();
        self.runtime.selected = 0;
    }
    fn refresh(&mut self, preferred: Option<EntryId>) {
        self.clear_entries();
        if let Some(session) = &self.runtime.session {
            self.runtime.entries = session.entries();
        }
        self.filter(preferred);
    }
    fn explorer_active(&self) -> bool {
        self.browser_origin.is_none()
            && self.runtime.query.text().is_empty()
            && self
                .runtime
                .session
                .as_ref()
                .is_some_and(Session::has_folders)
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
                if app.runtime.expanded_folders.contains(&id) {
                    append(app, session, id, depth + 1, rows);
                }
            }
            for (id, title) in &app.runtime.entries {
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
        if let Some(session) = &self.runtime.session {
            let root = session.root_folder();
            rows.push(ExplorerRow {
                node: ExplorerNode::Folder(root),
                parent: root,
                depth: 0,
                label: "TermVault".into(),
            });
            if self.runtime.expanded_folders.contains(&root) {
                append(self, session, root, 1, &mut rows);
            }
        }
        rows
    }
    fn expand_ancestors(&mut self, folder: GroupId) {
        let Some(session) = &self.runtime.session else {
            return;
        };
        let mut current = Some(folder);
        while let Some(id) = current {
            self.runtime.expanded_folders.insert(id);
            current = session.folder_parent(id);
        }
    }
    fn sync_explorer(&mut self, preferred: Option<EntryId>) {
        if !self.explorer_active() {
            return;
        }
        if self.runtime.explorer_focus.is_none() {
            self.runtime
                .expanded_folders
                .insert(self.runtime.session.as_ref().unwrap().root_folder());
        }
        if let Some(id) = preferred {
            if let Some(parent) = self
                .runtime
                .session
                .as_ref()
                .and_then(|session| session.entry_folder(id))
            {
                self.expand_ancestors(parent);
                self.runtime.explorer_focus = Some(ExplorerNode::Entry(id));
            }
        } else if self.runtime.explorer_focus.is_none() {
            if let Some(folder) = self.runtime.folder {
                self.expand_ancestors(folder);
                self.runtime.explorer_focus = Some(ExplorerNode::Folder(folder));
            }
        }
        let rows = self.explorer_rows();
        if !rows
            .iter()
            .any(|row| Some(row.node) == self.runtime.explorer_focus)
        {
            self.runtime.explorer_focus = rows.first().map(|row| row.node);
        }
    }
    fn select_explorer(&mut self, node: ExplorerNode) {
        self.runtime.explorer_focus = Some(node);
        self.runtime.folder = match node {
            ExplorerNode::Folder(id) => Some(id),
            ExplorerNode::Entry(id) => self
                .runtime
                .session
                .as_ref()
                .and_then(|session| session.entry_folder(id)),
        };
        self.runtime.reveal_until = None;
    }
    fn move_explorer(&mut self, delta: isize) {
        let rows = self.explorer_rows();
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|row| Some(row.node) == self.runtime.explorer_focus)
            .unwrap_or(0);
        let index = index.saturating_add_signed(delta).min(rows.len() - 1);
        self.select_explorer(rows[index].node);
    }
    fn explorer_right(&mut self, toggle: bool) {
        if let Some(ExplorerNode::Folder(folder)) = self.runtime.explorer_focus {
            if !self.runtime.expanded_folders.insert(folder) {
                if toggle {
                    self.runtime.expanded_folders.remove(&folder);
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
            self.runtime.reveal_until = None;
        } else if toggle {
            self.execute_shortcut(Command::EditEntry);
        }
    }
    fn explorer_left(&mut self) {
        let Some(focus) = self.runtime.explorer_focus else {
            return;
        };
        if let ExplorerNode::Folder(folder) = focus {
            if self.runtime.expanded_folders.remove(&folder) {
                self.runtime.reveal_until = None;
                return;
            }
        }
        if let Some(row) = self.explorer_rows().iter().find(|row| row.node == focus) {
            let parent = row.parent;
            if Some(ExplorerNode::Folder(parent)) != self.runtime.explorer_focus {
                self.select_explorer(ExplorerNode::Folder(parent));
            }
        }
    }
    fn creation_folder(&self) -> Option<GroupId> {
        if self.explorer_active() {
            match self.runtime.explorer_focus {
                Some(ExplorerNode::Folder(id)) => return Some(id),
                Some(ExplorerNode::Entry(id)) => {
                    return self.runtime.session.as_ref()?.entry_folder(id)
                }
                None => {}
            }
        }
        self.runtime.folder
    }
    fn delete_folder_target(&self) -> Option<GroupId> {
        if !self.runtime.query.text().is_empty() {
            return None;
        }
        let folder = if self.explorer_active() {
            match self.runtime.explorer_focus {
                Some(ExplorerNode::Folder(id)) => Some(id),
                Some(ExplorerNode::Entry(_)) => None,
                None => self.runtime.folder,
            }
        } else {
            self.runtime.folder
        }?;
        (folder != self.runtime.session.as_ref()?.root_folder()).then_some(folder)
    }
    fn filter(&mut self, preferred: Option<EntryId>) {
        let query = self.runtime.query.text().to_lowercase();
        self.runtime.visible = self
            .runtime
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (id, title))| {
                self.entry_label(*id, title).to_lowercase().contains(&query)
                    && (self.browser_origin.is_some()
                        || !query.is_empty()
                        || self.runtime.folder.is_none_or(|folder| {
                            self.runtime
                                .session
                                .as_ref()
                                .and_then(|session| session.entry_folder(*id))
                                == Some(folder)
                        }))
                    && self.browser_origin.as_ref().is_none_or(|target| {
                        self.runtime
                            .session
                            .as_ref()
                            .and_then(|session| session.entry(*id))
                            .is_some_and(|entry| crate::browser::matches(&entry.url, target))
                    })
            })
            .map(|(i, _)| i)
            .collect();
        self.runtime.selected = preferred
            .and_then(|id| {
                self.runtime
                    .visible
                    .iter()
                    .position(|i| self.runtime.entries[*i].0 == id)
            })
            .unwrap_or(0);
        self.runtime.reveal_until = None;
        self.sync_explorer(preferred);
    }
    fn selected_id(&self) -> Option<EntryId> {
        if self.explorer_active() {
            return match self.runtime.explorer_focus {
                Some(ExplorerNode::Entry(id)) => Some(id),
                Some(ExplorerNode::Folder(_)) => None,
                None => self
                    .runtime
                    .visible
                    .get(self.runtime.selected)
                    .map(|i| self.runtime.entries[*i].0),
            };
        }
        self.runtime
            .visible
            .get(self.runtime.selected)
            .map(|i| self.runtime.entries[*i].0)
    }
    fn lock(&mut self) {
        self.language_choice = None;
        self.render = RenderFeedback::default();
        // Drop the complete sensitive working set, including masked authentication
        // drafts. Approved browser output is deliberately owned separately.
        self.runtime = RuntimeState::default();
        self.clipboard.clear();
        self.info(self.tr("Vault locked", "Хранилище заблокировано"));
    }
    fn action(&mut self, action: Action) {
        match action {
            Action::Browse => {
                self.runtime.form = None;
                self.runtime.mode = Mode::Browse;
                self.runtime.reveal_until = None;
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
        self.runtime.pending_folder_delete = None;
        self.runtime.folder_name.clear();
        self.runtime.folder_error = None;
        self.runtime.sensitive = None;
        self.runtime.delete_authorization = None;
        self.runtime.update_return = None;
        self.runtime.palette = None;
        self.runtime.reveal_until = None;
        if self.runtime.form.as_ref().is_some_and(|form| form.dirty) {
            self.runtime.mode = Mode::Discard(action);
        } else {
            self.action(action);
        }
    }
    fn edit(&mut self, new: bool) {
        if new {
            self.runtime.folder = self.creation_folder();
        }
        let Some(session) = &self.runtime.session else {
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
        self.runtime.form = Some(Form::new(id, data));
        self.runtime.mode = Mode::Edit;
        self.runtime.reveal_until = None;
        self.clear_notice();
    }
    fn generate(&mut self) {
        match input::generate(24) {
            Ok(password) => {
                if let Some(form) = &mut self.runtime.form {
                    form.fields[2] = Input::from(&password);
                    form.dirty = true;
                    self.runtime.reveal_until = None;
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
        let Some(form) = &self.runtime.form else {
            return false;
        };
        if !form.dirty && form.id.is_some() {
            self.action(Action::Browse);
            return true;
        }
        let data = form.data();
        let id = form.id;
        let Some(session) = &mut self.runtime.session else {
            return false;
        };
        let result = match self.runtime.folder {
            Some(folder) => session.update_in_folder(id, &data, Some(folder)),
            None => session.update(id, &data),
        };
        match result {
            Ok(id) => {
                self.runtime.form = None;
                self.runtime.mode = Mode::Browse;
                self.runtime.query.clear();
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
            .and_then(|id| self.runtime.session.as_ref()?.entry(id));
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
        self.runtime.reveal_until = None;
        self.render.begin_frame();
        self.runtime.palette = Some(Palette::default());
    }
    fn toggle_reveal(&mut self) {
        if self.browser_origin.is_some()
            || self.runtime.session.is_none()
            || !matches!(self.runtime.mode, Mode::Browse | Mode::Edit)
            || (self.runtime.form.is_none() && self.selected_id().is_none())
        {
            return;
        }
        self.runtime.reveal_until = if self.runtime.reveal_until.is_some() {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(10))
        };
    }
    fn execute_command(&mut self, command: Command) {
        self.dispatch_command(command, Invocation::Palette);
    }
    fn execute_shortcut(&mut self, command: Command) {
        self.dispatch_command(command, Invocation::Keyboard);
    }
    fn dispatch_command(&mut self, command: Command, invocation: Invocation) {
        // Both entry points re-check the same capabilities at execution time.
        if !self.command_allowed(command, invocation) {
            return;
        }
        self.render.begin_frame();
        self.runtime.palette = None;
        match command {
            Command::Language => {
                self.runtime.reveal_until = None;
                self.render.language_visible.set(false);
                self.language_choice = Some(usize::from(self.language == Language::Russian));
            }
            Command::Unlock => {} // Return to the masked input; never submit an existing master draft implicitly.
            Command::NewEntry => self.edit(true),
            Command::NewFolder => {
                self.runtime.folder = self.creation_folder();
                self.runtime.folder_name.clear();
                self.runtime.folder_error = None;
                self.runtime.reveal_until = None;
                self.clear_notice();
                self.render.folder_dialog_visible.set(false);
                self.runtime.mode = Mode::NewFolder;
            }
            Command::DeleteFolder => {
                self.runtime.pending_folder_delete = self.delete_folder_target();
                self.runtime.folder = self.runtime.pending_folder_delete;
                self.runtime.reveal_until = None;
                self.clear_notice();
                self.render.folder_dialog_visible.set(false);
                self.runtime.mode = Mode::DeleteFolder;
            }
            Command::OpenFolder | Command::MoveToFolder => {
                self.runtime.reveal_until = None;
                self.clear_notice();
                self.runtime.folder_selected = 0;
                self.render.folder_dialog_visible.set(false);
                self.runtime.mode = Mode::Folders(command == Command::MoveToFolder);
            }
            Command::NewGeneratedEntry => {
                self.edit(true);
                self.generate();
            }
            Command::EditEntry => self.edit(false),
            Command::SearchEntries => self.runtime.mode = Mode::Search,
            Command::CopyPassword => self.copy(false),
            Command::CopyUsername => self.copy(true),
            Command::RevealPassword => self.toggle_reveal(),
            Command::DeleteEntry => {
                self.runtime.reveal_until = None;
                self.runtime.mode = Mode::Delete;
            }
            Command::SaveEntry => {
                self.save();
            }
            Command::GeneratePassword => self.generate(),
            Command::Back => {
                if self.runtime.mode == Mode::Search {
                    self.runtime.query.clear();
                    self.filter(None);
                }
                self.leave(Action::Browse);
            }
            Command::CheckUpdates => {
                if self.runtime.update_rx.is_some() {
                    self.info(self.tr(
                        "An update check is already running.",
                        "Проверка обновлений уже идёт.",
                    ));
                } else {
                    self.runtime.update_return = Some(self.runtime.mode);
                    self.runtime.mode = Mode::UpdateConsent;
                    self.render.update_consent_visible.set(false);
                }
            }
            Command::UpdateResult => {
                self.runtime.update_return = Some(self.runtime.mode);
                self.runtime.mode = Mode::UpdateResult;
            }
            Command::ChangeMaster => self.begin_sensitive(SensitiveKind::ChangeMaster),
            Command::DeleteVault => self.begin_sensitive(SensitiveKind::DeleteVault),
            Command::Lock => self.leave(Action::Lock),
            Command::Quit => self.leave(Action::Quit),
        }
    }
    fn palette_key(&mut self, key: KeyEvent) {
        let commands = self
            .runtime
            .palette
            .as_ref()
            .unwrap()
            .filtered(&self.available_commands());
        match key.code {
            KeyCode::Esc => self.runtime.palette = None,
            KeyCode::Enter if self.render.palette_visible.get() => {
                if let Some(command) = commands.get(self.runtime.palette.as_ref().unwrap().selected)
                {
                    self.execute_command(*command);
                }
            }
            _ => self
                .runtime
                .palette
                .as_mut()
                .unwrap()
                .key(key, commands.len()),
        }
    }
    fn folder_options(&self, moving: bool) -> Vec<(Option<GroupId>, String)> {
        let mut options = Vec::new();
        if !moving {
            options.push((None, self.tr("All entries", "Все записи").into()));
        }
        if let Some(session) = &self.runtime.session {
            options.extend(
                session
                    .folders()
                    .into_iter()
                    .map(|(id, name)| (Some(id), name)),
            );
        }
        options
    }
    fn submit_folder(&mut self) {
        if !self.render.folder_dialog_visible.get() {
            return;
        }
        let Some(session) = &mut self.runtime.session else {
            return;
        };
        match session.create_folder(self.runtime.folder, self.runtime.folder_name.text()) {
            Ok(id) => {
                self.runtime.folder = Some(id);
                self.expand_ancestors(id);
                self.runtime.explorer_focus = Some(ExplorerNode::Folder(id));
                self.runtime.folder_name.clear();
                self.runtime.query.clear();
                self.runtime.mode = Mode::Browse;
                self.refresh(None);
                self.info(self.tr("Folder created", "Папка создана"));
            }
            Err(error) => {
                self.runtime.folder_error = Some(error.to_string());
            }
        }
    }
    fn confirm_folder_delete(&mut self) {
        if !self.render.folder_dialog_visible.get() {
            return;
        }
        let Some(folder) = self.runtime.pending_folder_delete else {
            return;
        };
        let Some(session) = &mut self.runtime.session else {
            return;
        };
        match session.delete_folder(folder) {
            Ok(parent) => {
                self.runtime.pending_folder_delete = None;
                self.runtime.folder = Some(parent);
                self.runtime.explorer_focus = Some(ExplorerNode::Folder(parent));
                self.runtime.query.clear();
                self.runtime.mode = Mode::Browse;
                self.refresh(None);
                self.warning(self.tr(
                    "Folder and its contents deleted. Encrypted backup retained.",
                    "Папка и её содержимое удалены. Зашифрованный бэкап сохранён.",
                ));
            }
            Err(error) => {
                self.runtime.pending_folder_delete = None;
                self.runtime.mode = Mode::Browse;
                self.error(error.to_string());
            }
        }
    }
    fn choose_folder(&mut self, moving: bool) {
        if !self.render.folder_dialog_visible.get() {
            return;
        }
        let options = self.folder_options(moving);
        let Some((folder, _)) = options.get(self.runtime.folder_selected) else {
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
            let Some(session) = &mut self.runtime.session else {
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
        self.runtime.folder = folder;
        self.runtime.explorer_focus = None;
        if let Some(folder) = folder {
            self.expand_ancestors(folder);
        }
        self.runtime.query.clear();
        self.runtime.mode = Mode::Browse;
        self.refresh(preferred);
    }
    fn begin_sensitive(&mut self, kind: SensitiveKind) {
        self.runtime.reveal_until = None;
        self.clear_notice();
        self.runtime.delete_authorization = None;
        self.runtime.sensitive = Some(SensitiveForm::new(kind));
        self.render.sensitive_visible.set(false);
        self.runtime.mode = Mode::VaultAction;
    }
    fn cancel_sensitive(&mut self) {
        self.runtime.sensitive = None;
        self.runtime.delete_authorization = None;
        self.runtime.mode = Mode::Browse;
    }
    fn submit_sensitive(&mut self) {
        if !self.render.sensitive_visible.get() {
            self.warning(self.tr(
                "Enlarge the window to see the form and warnings.",
                "Увеличьте окно, чтобы увидеть форму и предупреждения.",
            ));
            return;
        }
        let Some(form) = self.runtime.sensitive.take() else {
            return;
        };
        let Some(session) = self.runtime.session.as_mut() else {
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
                self.runtime.delete_authorization = Some(authorization);
                self.render.vault_delete_visible.set(false);
                self.runtime.mode = Mode::VaultDeleteConfirm;
            }
            Ok(None) => {
                self.runtime.mode = Mode::Browse;
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
                self.runtime.sensitive = Some(retry);
                self.error(message);
            }
        }
        // `form` drops here: all current/new master inputs use zeroizing buffers.
    }
    fn confirm_vault_delete(&mut self) {
        if !self.render.vault_delete_visible.get() {
            return;
        }
        let Some(authorization) = self.runtime.delete_authorization.take() else {
            return;
        };
        let Some(session) = self.runtime.session.as_ref() else {
            self.lock();
            return;
        };
        match session.remove_vault(&authorization) {
            Ok(backup) => {
                self.lock();
                self.runtime.deleted_backup = Some(backup);
                self.runtime.mode = Mode::VaultDeleted;
                self.info(self.tr(
                    "Active file removed. Encrypted backup retained.",
                    "Активный файл удалён. Зашифрованный бэкап сохранён.",
                ));
            }
            Err(error) => {
                self.runtime.mode = Mode::Browse;
                self.error(error.to_string());
            }
        }
    }
    fn close_update_dialog(&mut self) {
        self.runtime.mode = self.runtime.update_return.take().unwrap_or(Mode::Browse);
    }
    fn start_update_check(&mut self) {
        if !self.render.update_consent_visible.get() {
            return;
        }
        self.close_update_dialog();
        if self.runtime.update_rx.is_some() {
            return;
        }
        self.update_result = None;
        match updates::start_check() {
            Ok(receiver) => {
                self.runtime.update_rx = Some(receiver);
                self.info(self.tr(
                    "Checking GitHub for updates…",
                    "Проверяем обновления на GitHub…",
                ));
            }
            Err(error) => self.error(error.to_string()),
        }
    }
    fn poll_update(&mut self) -> bool {
        let Some(receiver) = &self.runtime.update_rx else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                Err("Update check was interrupted. Please retry.".into())
            }
        };
        self.runtime.update_rx = None;
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
        if self.runtime.session.is_some()
            && now.saturating_duration_since(self.last_activity) >= self.lock_after
        {
            self.lock();
            self.warning(self.tr(
                "Auto-locked after inactivity. Unsaved entry drafts were discarded.",
                "Автоблокировка из-за бездействия. Несохранённые черновики удалены.",
            ));
            return true;
        }
        if self
            .runtime
            .reveal_until
            .is_some_and(|deadline| now >= deadline)
        {
            self.runtime.reveal_until = None;
            return true;
        }
        changed
    }
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        self.last_activity = Instant::now();
        let ctrl = input::control_shortcut(key);
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
            self.execute_shortcut(Command::Quit);
            return;
        }
        if ctrl && letter == Some('p') {
            if self.language_choice.take().is_some() {
                return;
            }
            if self.runtime.palette.is_some() {
                self.runtime.palette = None;
            } else {
                self.open_palette();
            }
            return;
        }
        if ctrl && letter == Some('l') && self.runtime.session.is_some() {
            self.execute_shortcut(Command::Lock);
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
        if self.runtime.palette.is_some() {
            self.palette_key(key);
            return;
        }
        match self.runtime.mode {
            Mode::DeleteFolder => {
                match key.code {
                    KeyCode::Esc => {
                        self.runtime.pending_folder_delete = None;
                        self.runtime.mode = Mode::Browse;
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
                        self.runtime.folder_name.clear();
                        self.runtime.folder_error = None;
                        self.runtime.mode = Mode::Browse;
                    }
                    KeyCode::Enter => self.submit_folder(),
                    _ => {
                        let fits = match key.code {
                            KeyCode::Char(character) => {
                                self.runtime.folder_name.text().len() + character.len_utf8() <= 128
                            }
                            _ => true,
                        };
                        if (fits || ctrl) && self.runtime.folder_name.key(key, false) {
                            self.runtime.folder_error = None;
                        }
                    }
                }
                return;
            }
            Mode::Folders(moving) => {
                match key.code {
                    KeyCode::Esc => self.runtime.mode = Mode::Browse,
                    KeyCode::Up | KeyCode::BackTab => {
                        self.runtime.folder_selected =
                            self.runtime.folder_selected.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        self.runtime.folder_selected = (self.runtime.folder_selected + 1)
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
                if let Some(form) = &mut self.runtime.sensitive {
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
        if self.runtime.session.is_none() {
            match key.code {
                KeyCode::Esc => self.exit = true,
                KeyCode::Tab | KeyCode::BackTab if self.creating => {
                    self.auth_field = 1 - self.auth_field
                }
                KeyCode::Enter if self.creating && self.auth_field == 0 => self.auth_field = 1,
                KeyCode::Enter => self.unlock(),
                _ => {
                    let field = if self.auth_field == 0 {
                        &mut self.runtime.master
                    } else {
                        &mut self.runtime.confirmation
                    };
                    field.key(key, false);
                }
            }
            return;
        }
        if self.browser_origin.is_some() {
            match (self.runtime.mode, key.code) {
                (_, KeyCode::Esc | KeyCode::Char('q')) => self.action(Action::Quit),
                (Mode::Browse, KeyCode::Up) => {
                    self.runtime.selected = self.runtime.selected.saturating_sub(1)
                }
                (Mode::Browse, KeyCode::Down) => {
                    self.runtime.selected = (self.runtime.selected + 1)
                        .min(self.runtime.visible.len().saturating_sub(1));
                }
                (Mode::Browse, KeyCode::Enter) if self.selected_id().is_some() => {
                    self.render.browser_review_visible.set(false);
                    self.runtime.mode = Mode::BrowserConfirm;
                }
                (Mode::BrowserConfirm, KeyCode::Char('y'))
                    if self.render.browser_review_visible.get() =>
                {
                    if let Some(data) = self
                        .selected_id()
                        .and_then(|id| self.runtime.session.as_ref()?.entry(id))
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
                (Mode::BrowserConfirm, KeyCode::Char('n')) => {
                    self.render.browser_review_visible.set(false);
                    self.runtime.mode = Mode::Browse;
                }
                _ => {}
            }
            return;
        }
        if ctrl && letter == Some('o') {
            self.execute_shortcut(Command::RevealPassword);
            return;
        }
        match self.runtime.mode {
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
                    self.execute_shortcut(Command::SaveEntry);
                } else if ctrl && letter == Some('g') {
                    self.execute_shortcut(Command::GeneratePassword);
                } else if key.code == KeyCode::Esc {
                    self.execute_shortcut(Command::Back);
                } else if let Some(form) = &mut self.runtime.form {
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
                KeyCode::Esc | KeyCode::Char('c') => self.runtime.mode = Mode::Edit,
                _ => {}
            },
            Mode::Delete => match key.code {
                KeyCode::Esc => self.runtime.mode = Mode::Browse,
                KeyCode::Char('d') => {
                    if let Some(id) = self.selected_id() {
                        let result = self.runtime.session.as_mut().unwrap().delete(id);
                        match result {
                            Ok(()) => {
                                self.refresh(None);
                                self.warning(self.tr("Entry removed. Encrypted backups still contain its previous versions.", "Запись удалена. Зашифрованные бэкапы содержат её прежние версии."));
                                self.runtime.mode = Mode::Browse;
                            }
                            Err(error) => self.error(error.to_string()),
                        }
                    }
                }
                _ => {}
            },
            Mode::Search => match key.code {
                KeyCode::Esc => {
                    self.execute_shortcut(Command::Back);
                }
                KeyCode::Enter => self.runtime.mode = Mode::Browse,
                _ => {
                    if self.runtime.query.key(key, false) {
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
                    self.execute_shortcut(Command::DeleteFolder)
                }
                KeyCode::Up => {
                    self.runtime.selected = self.runtime.selected.saturating_sub(1);
                    self.runtime.reveal_until = None;
                }
                KeyCode::Down => {
                    self.runtime.selected = (self.runtime.selected + 1)
                        .min(self.runtime.visible.len().saturating_sub(1));
                    self.runtime.reveal_until = None;
                }
                KeyCode::Char('q') => self.execute_shortcut(Command::Quit),
                KeyCode::Char('/') => self.execute_shortcut(Command::SearchEntries),
                KeyCode::Esc => {
                    self.runtime.query.clear();
                    self.filter(None);
                    self.runtime.reveal_until = None;
                }
                KeyCode::Char('n') => self.execute_shortcut(Command::NewEntry),
                KeyCode::Char('f') => self.execute_shortcut(Command::OpenFolder),
                KeyCode::Char('F') => self.execute_shortcut(Command::NewFolder),
                KeyCode::Char('m') => self.execute_shortcut(Command::MoveToFolder),
                KeyCode::Char('g') => self.execute_shortcut(Command::NewGeneratedEntry),
                KeyCode::Enter => self.execute_shortcut(Command::EditEntry),
                KeyCode::Delete | KeyCode::Char('d') if self.selected_id().is_some() => {
                    self.execute_shortcut(Command::DeleteEntry)
                }
                KeyCode::Char('p') if key.modifiers.is_empty() => {
                    self.execute_shortcut(Command::RevealPassword);
                }
                KeyCode::Char('c') => self.execute_shortcut(Command::CopyPassword),
                KeyCode::Char('u') => self.execute_shortcut(Command::CopyUsername),
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
            if event.kind == MouseEventKind::Down(MouseButton::Left)
                && self.render.language_visible.get()
            {
                let choice = self
                    .render
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
        if self.runtime.palette.is_some() {
            if event.kind == MouseEventKind::Down(MouseButton::Left) {
                let command = self
                    .render
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
        if self.runtime.mode != Mode::Browse || !self.explorer_active() {
            return;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let node = self
                    .render
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
                    .render
                    .explorer_hits
                    .borrow()
                    .iter()
                    .any(|(rect, _)| event.column >= rect.x && event.column < rect.right()) =>
            {
                self.move_explorer(3)
            }
            MouseEventKind::ScrollUp
                if self
                    .render
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
        if self.runtime.mode == Mode::NewFolder {
            if self
                .runtime
                .folder_name
                .text()
                .len()
                .saturating_add(text.len())
                <= 128
                && self.runtime.folder_name.insert(text, false)
            {
                self.runtime.folder_error = None;
            } else {
                self.runtime.folder_error =
                    Some("Paste rejected: maximum 128 bytes, no control characters.".into());
            }
            return;
        }
        if let Some(palette) = &mut self.runtime.palette {
            palette.paste(text);
            return;
        }
        if self.runtime.mode == Mode::VaultAction {
            if let Some(form) = &mut self.runtime.sensitive {
                form.paste(text);
            }
            return;
        }
        if self.runtime.session.is_none() {
            let field = if self.auth_field == 0 {
                &mut self.runtime.master
            } else {
                &mut self.runtime.confirmation
            };
            if !field.insert(text, false) {
                self.error("Paste rejected: unsupported characters or input too large");
            }
            return;
        }
        match self.runtime.mode {
            Mode::Edit => {
                if let Some(form) = &mut self.runtime.form {
                    let normalized = Zeroizing::new(text.replace("\r\n", "\n"));
                    if form.fields[form.field].insert(&normalized, form.field == 4) {
                        form.dirty = true;
                    } else {
                        self.error("Paste rejected: unsupported characters or input too large");
                    }
                }
            }
            Mode::Search if self.runtime.query.insert(text, false) => self.filter(None),
            _ => {}
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.lock();
    }
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
#[path = "app/audit_tests.rs"]
mod audit_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use ratatui::backend::TestBackend;
    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.kdbx");
        let mut app = App::new(path, true, Duration::from_secs(120));
        app.runtime.master.insert("test master passphrase", false);
        app.runtime
            .confirmation
            .insert("test master passphrase", false);
        app.unlock();
        assert!(app.runtime.session.is_some(), "{}", app.status);
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
        app.runtime.master = Input::from("DUMMY_MASTER_DRAFT");
        app.runtime.confirmation = Input::from("DUMMY_CONFIRM_DRAFT");
        app.auth_field = 1;
        app.load_preferences(root.path().to_owned());
        assert_eq!(app.language, Language::Russian);
        assert!(screen(&app).contains("Создать хранилище"));
        switch_language(&mut app, Language::English);
        assert_eq!(app.runtime.master.text(), "DUMMY_MASTER_DRAFT");
        assert_eq!(app.runtime.confirmation.text(), "DUMMY_CONFIRM_DRAFT");
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
        let form = app.runtime.form.as_mut().unwrap();
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
        let expanded = app.runtime.expanded_folders.clone();
        let focus = app.runtime.explorer_focus;
        let folder = app.runtime.folder;
        app.runtime.query = Input::from("retained search");
        let visible = app.runtime.visible.clone();
        let bytes = std::fs::read(&app.path).unwrap();
        for language in [Language::Russian, Language::English] {
            switch_language(&mut app, language);
            let form = app.runtime.form.as_ref().unwrap();
            assert_eq!(app.runtime.mode, Mode::Edit);
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
            assert_eq!(app.runtime.expanded_folders, expanded);
            assert!(app.runtime.expanded_folders.contains(&work));
            assert_eq!(app.runtime.explorer_focus, focus);
            assert_eq!(app.runtime.folder, folder);
            assert_eq!(app.runtime.query.text(), "retained search");
            assert_eq!(app.runtime.visible, visible);
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
            app.runtime.form.as_ref().unwrap().fields[0].text(),
            "Keep this draft"
        );
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.language, Language::English);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.language_choice.is_none());
        assert_eq!(app.runtime.mode, Mode::Edit);
        app.execute_command(Command::Language);
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.language, Language::Russian);
        assert!(screen(&app).contains("Название"));
        app.execute_command(Command::Language);
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.language_choice.is_none());
        assert!(app.runtime.form.is_none());
        assert!(app.runtime.session.is_none());
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
        assert_eq!(
            app.runtime.form.as_ref().unwrap().fields[0].text(),
            "Keep draft"
        );
    }

    #[test]
    fn language_never_bypasses_browser_readonly_or_confirmation_guards() {
        let (_root, mut app) = app();
        app.runtime.session.as_mut().unwrap().readonly = true;
        switch_language(&mut app, Language::Russian);
        assert!(app.runtime.session.as_ref().unwrap().readonly);
        for command in [
            Command::NewEntry,
            Command::DeleteVault,
            Command::ChangeMaster,
            Command::NewFolder,
        ] {
            app.execute_command(command);
            assert_eq!(app.runtime.mode, Mode::Browse);
            assert!(app.runtime.form.is_none());
            assert!(app.runtime.sensitive.is_none());
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
            app.runtime.mode = mode;
            app.execute_command(Command::Language);
            assert!(app.language_choice.is_none());
            assert_eq!(app.runtime.mode, mode);
        }
        app.runtime.mode = Mode::Browse;
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
        let form = app.runtime.form.as_mut().unwrap();
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
        app.runtime.mode = Mode::Search;
        assert!(screen(&app).contains("Поиск по названиям…"));
        app.runtime.mode = Mode::Browse;
        app.open_palette();
        assert!(screen(&app).contains("Команды"));
        assert!(screen(&app).contains("Поиск команды"));
        app.runtime.palette = None;
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
        assert!(app.render.folder_dialog_visible.get());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app
            .runtime
            .session
            .as_ref()
            .unwrap()
            .folder_contents(work)
            .is_some());
        app.runtime.mode = Mode::Delete;
        assert!(screen(&app).contains("Удалить выбранную запись?"));
        app.runtime.mode = Mode::Discard(Action::Quit);
        let text = screen_rows(&app, 40, 10).join("");
        assert!(text.contains("Сохранить"));
        assert!(text.contains("Отбросить"));
        assert!(!app.exit);
    }

    #[test]
    fn localized_update_and_browser_dialogs_are_render_only_and_preserve_consent() {
        let (_root, mut app) = app();
        app.language = Language::Russian;
        app.runtime.mode = Mode::UpdateConsent;
        let text = screen(&app);
        assert!(text.contains("Отправить запрос к api.github.com?"));
        assert!(app.runtime.update_rx.is_none());
        app.runtime.mode = Mode::UpdateResult;
        app.update_result = Some(UpdateStatus::Available("99.0.0".into()));
        assert!(screen(&app).contains("Доступна версия 99.0.0"));
        app.runtime.mode = Mode::BrowserConfirm;
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
        app.runtime.delete_authorization = Some(
            app.runtime
                .session
                .as_ref()
                .unwrap()
                .authorize_delete("test master passphrase", "test.kdbx")
                .unwrap(),
        );
        app.runtime.mode = Mode::VaultDeleteConfirm;
        let before = std::fs::read(&app.path).unwrap();
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let text = screen(&app);
        assert!(text.contains("Окончательное подтверждение"));
        assert!(text.contains("Это не безопасное стирание"));
        assert!(text.contains("60 секунд"));
        assert!(app.render.vault_delete_visible.get());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.runtime.delete_authorization.is_none());
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
            .render
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
        assert_eq!(app.runtime.mode, Mode::Edit);
        assert_eq!(
            app.runtime.form.as_ref().unwrap().fields[0].text(),
            "Retained draft"
        );
        assert!(app.runtime.form.as_ref().unwrap().dirty);
        assert!(!app.exit);
    }

    #[test]
    fn language_preserves_active_search_and_literal_user_titles_matching_ui_labels() {
        let (_root, mut app) = app();
        let id = app
            .runtime
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
        app.runtime.mode = Mode::Search;
        app.runtime.query = Input::from("Untitled");
        app.filter(Some(id));
        let selected = app.runtime.selected;
        switch_language(&mut app, Language::Russian);
        assert_eq!(app.runtime.mode, Mode::Search);
        assert_eq!(app.runtime.query.text(), "Untitled");
        assert_eq!(app.runtime.selected, selected);
        assert_eq!(app.selected_id(), Some(id));
        assert_eq!(app.entry_label(id, "Untitled"), "Untitled");
        assert!(screen(&app).contains("Untitled"));
        assert_eq!(
            &*app
                .runtime
                .session
                .as_ref()
                .unwrap()
                .entry(id)
                .unwrap()
                .title,
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
        assert!(app.runtime.palette.is_some());
        app.paste(query);
        assert!(screen(app).contains(app.tr("Commands", "Команды")));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    #[test]
    fn folder_commands_create_nested_folders_save_filter_and_move_records() {
        let (_root, mut app) = app();
        palette_command(&mut app, "new folder");
        assert_eq!(app.runtime.mode, Mode::NewFolder);
        app.paste("Работа");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let work = app.runtime.folder.unwrap();
        assert_eq!(app.folder_label(), "/Работа");
        palette_command(&mut app, "new folder");
        app.paste("Почта");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mail = app.runtime.folder.unwrap();
        assert_ne!(mail, work);
        assert_eq!(app.folder_label(), "/Работа / Почта");
        app.edit(true);
        let form = app.runtime.form.as_mut().unwrap();
        form.fields[0] = Input::from("Folder entry");
        form.fields[2] = Input::from("FOLDER_DUMMY_SECRET");
        form.dirty = true;
        assert!(app.save());
        let id = app.selected_id().unwrap();
        assert_eq!(
            app.runtime.session.as_ref().unwrap().entry_folder(id),
            Some(mail)
        );
        palette_command(&mut app, "open folders");
        let options = app.folder_options(false);
        app.runtime.folder_selected = options
            .iter()
            .position(|item| item.0 == Some(work))
            .unwrap();
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.runtime.visible.is_empty()); // A folder shows its own records, not subfolder records.
        palette_command(&mut app, "open folders");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // All entries.
        assert!(app.runtime.folder.is_none());
        assert!(app
            .explorer_rows()
            .iter()
            .any(|row| row.node == ExplorerNode::Entry(id)));
        app.select_explorer(ExplorerNode::Entry(id));
        assert_eq!(app.selected_id(), Some(id));
        palette_command(&mut app, "move entry folder");
        let options = app.folder_options(true);
        app.runtime.folder_selected = options
            .iter()
            .position(|item| item.0 == Some(work))
            .unwrap();
        assert!(!screen(&app).contains("FOLDER_DUMMY_SECRET"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.runtime.folder, Some(work));
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
        let session = app.runtime.session.as_mut().unwrap();
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
        assert_eq!(app.runtime.explorer_focus, Some(ExplorerNode::Folder(work)));
        assert_eq!(app.explorer_rows().len(), 2);
        assert!(app.selected_id().is_none());
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.explorer_rows().len(), 3);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.runtime.explorer_focus, Some(ExplorerNode::Folder(mail)));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen(&app).contains("Nested login"));
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.selected_id(), Some(id));
        assert!(!screen(&app).contains("TREE_DUMMY_SECRET"));
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.runtime.explorer_focus, Some(ExplorerNode::Folder(mail)));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.explorer_rows().len(), 3);
        assert!(!screen(&app).contains("Nested login"));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::DeleteFolder);
        assert_eq!(app.runtime.pending_folder_delete, Some(work));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.runtime.pending_folder_delete.is_none());
    }

    #[test]
    fn clicking_delete_folder_in_palette_never_opens_create_folder() {
        let (_root, mut app, work, _, _) = explorer_fixture();
        let before = std::fs::read(&app.path).unwrap();
        app.open_palette();
        app.paste("пап");
        let _ = screen(&app);
        let hit = app
            .render
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
        assert_eq!(app.runtime.mode, Mode::DeleteFolder);
        assert_eq!(app.runtime.pending_folder_delete, Some(work));
        assert!(screen(&app).contains("Delete folder?"));
        assert!(!screen(&app).contains("New folder"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.open_palette();
        app.paste("пап");
        let commands = app
            .runtime
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
        assert_eq!(app.runtime.mode, Mode::DeleteFolder);
        assert_eq!(app.runtime.pending_folder_delete, Some(work));
    }

    #[test]
    fn explorer_mouse_toggles_only_clicked_folder_and_search_and_browser_include_collapsed_entries()
    {
        let (_root, mut app, work, _, id) = explorer_fixture();
        let _ = screen(&app);
        let hit = app
            .render
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
        assert!(app.runtime.expanded_folders.contains(&work));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(!app.runtime.expanded_folders.contains(&work));
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
        assert!(app.runtime.explorer_focus.is_none());
        assert!(app.runtime.expanded_folders.is_empty());
        assert!(app.render.explorer_hits.borrow().is_empty());
    }
    #[test]
    fn folder_deletion_requires_a_visible_confirmation_and_can_be_cancelled() {
        let (_root, mut app) = app();
        let session = app.runtime.session.as_mut().unwrap();
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
        app.runtime.folder = Some(folder);
        app.refresh(None);
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "delete folder");
        assert_eq!(app.runtime.mode, Mode::DeleteFolder);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let text = screen(&app);
        assert!(text.contains("Entries to delete: 1"));
        assert!(text.contains("Nested folders: 1"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::DeleteFolder);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        palette_command(&mut app, "delete folder");
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::Browse);
        let session = app.runtime.session.as_ref().unwrap();
        assert!(session.folder_contents(folder).is_none());
        assert_eq!(app.runtime.folder, Some(session.root_folder()));
    }

    #[test]
    fn delete_folder_is_unavailable_for_all_entries_root_readonly_browser_and_drafts() {
        let (_root, mut app) = app();
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
        let root = app.runtime.session.as_ref().unwrap().root_folder();
        app.runtime.folder = Some(root);
        assert!(!app.available_commands().contains(&Command::DeleteFolder));
        let folder = app
            .runtime
            .session
            .as_mut()
            .unwrap()
            .create_folder(None, "Folder")
            .unwrap();
        app.runtime.folder = Some(folder);
        assert!(app.available_commands().contains(&Command::DeleteFolder));
        app.runtime.session.as_mut().unwrap().readonly = true;
        app.execute_command(Command::DeleteFolder);
        assert_eq!(app.runtime.mode, Mode::Browse);
        app.runtime.session.as_mut().unwrap().readonly = false;
        app.browser_origin = Some("https://example.test".into());
        app.execute_command(Command::DeleteFolder);
        assert_eq!(app.runtime.mode, Mode::Browse);
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
        assert!(app.runtime.folder_name.text().is_empty());
        app.runtime.session.as_mut().unwrap().readonly = true;
        assert!(app.available_commands().contains(&Command::OpenFolder));
        for command in [Command::NewFolder, Command::MoveToFolder] {
            app.execute_command(command);
            assert_eq!(app.runtime.mode, Mode::Browse);
        }
        app.browser_origin = Some("https://example.test".into());
        app.execute_command(Command::OpenFolder);
        assert_eq!(app.runtime.mode, Mode::Browse);
        app.browser_origin = None;
        app.runtime.session.as_mut().unwrap().readonly = false;
        palette_command(&mut app, "new folder");
        app.paste("AUTO_LOCK_FOLDER");
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.runtime.folder_name.text().is_empty());
        assert!(app.runtime.folder.is_none());
        assert!(app.runtime.session.is_none());
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
            app.runtime.palette.as_mut().unwrap().query.clear();
        }
    }

    #[test]
    fn palette_scrolls_to_selected_command_without_selecting_group_headers() {
        let (_root, mut app) = app();
        app.open_palette();
        let last = app.available_commands().len() - 1;
        app.runtime.palette.as_mut().unwrap().selected = last;
        let rows = screen_rows(&app, 60, 20);
        assert!(rows.join("").contains("Quit"));
        assert!(app.render.palette_visible.get());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.exit);
    }
    #[test]
    fn ctrl_p_opens_commands_and_ctrl_o_reveals_without_changing_the_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.runtime.form.as_mut().unwrap().fields[2] = Input::from("PALETTE_DUMMY_SECRET");
        app.runtime.form.as_mut().unwrap().dirty = true;
        app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(screen(&app).contains("PALETTE_DUMMY_SECRET"));
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.runtime.palette.is_some());
        assert!(app.runtime.reveal_until.is_none());
        assert!(!screen(&app).contains("PALETTE_DUMMY_SECRET"));
        app.paste("generate");
        assert_eq!(
            app.runtime.form.as_ref().unwrap().fields[2].text(),
            "PALETTE_DUMMY_SECRET"
        );
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.runtime.palette.is_none());
        assert_eq!(app.runtime.mode, Mode::Edit);
        app.key(KeyEvent::new(KeyCode::Char('щ'), KeyModifiers::CONTROL));
        assert!(app.runtime.reveal_until.is_some());
        app.key(KeyEvent::new(KeyCode::Char('щ'), KeyModifiers::CONTROL));
        assert!(app.runtime.reveal_until.is_none());
        app.key(KeyEvent::new(KeyCode::Char('з'), KeyModifiers::CONTROL));
        assert_eq!(app.runtime.palette.as_ref().unwrap().query.text(), "");
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.runtime.palette.is_none());
    }
    #[test]
    fn palette_can_save_but_never_discards_a_dirty_form_on_lock_exit_or_back() {
        let (_root, mut app) = app();
        app.edit(true);
        let form = app.runtime.form.as_mut().unwrap();
        form.fields[0] = Input::from("Palette demo");
        form.dirty = true;
        for (query, action) in [
            ("lock", Action::Lock),
            ("quit", Action::Quit),
            ("back", Action::Browse),
        ] {
            palette_command(&mut app, query);
            assert!(app.runtime.palette.is_none());
            assert_eq!(app.runtime.mode, Mode::Discard(action));
            assert!(app.runtime.form.as_ref().unwrap().dirty);
            assert!(app.runtime.session.is_some());
            assert!(!app.exit);
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        }
        palette_command(&mut app, "save");
        assert_eq!(app.runtime.mode, Mode::Browse);
        assert!(app.runtime.form.is_none());
        assert_eq!(app.runtime.entries.len(), 1);
        assert_eq!(app.status, "Entry saved");
    }
    #[test]
    fn palette_delete_opens_confirmation_instead_of_deleting_a_record() {
        let (_root, mut app) = app();
        app.runtime
            .session
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
        assert_eq!(app.runtime.mode, Mode::Delete);
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.runtime.palette.is_none());
        assert_eq!(app.runtime.mode, Mode::Delete);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.runtime.entries.len(), 1);
    }
    #[test]
    fn palette_respects_readonly_and_browser_restrictions_at_execution_time() {
        let (_root, mut app) = app();
        app.runtime.session.as_mut().unwrap().readonly = true;
        let before = std::fs::read(&app.path).unwrap();
        assert!(!app.available_commands().contains(&Command::NewEntry));
        app.execute_command(Command::NewEntry);
        app.execute_command(Command::DeleteEntry);
        assert!(app.runtime.form.is_none());
        assert_eq!(app.runtime.mode, Mode::Browse);
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        app.browser_origin = Some("https://example.com".into());
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(app.runtime.palette.is_none());
        assert!(app.runtime.reveal_until.is_none());
        assert!(app.available_commands().is_empty());
    }
    #[test]
    fn auto_lock_closes_palette_and_discards_its_query_and_dirty_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.runtime.form.as_mut().unwrap().dirty = true;
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.paste("DUMMY_QUERY_MUST_DISAPPEAR");
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.runtime.palette.is_none());
        assert!(app.runtime.form.is_none());
        assert!(app.runtime.session.is_none());
        assert!(!screen(&app).contains("DUMMY_QUERY_MUST_DISAPPEAR"));
    }
    #[test]
    fn locked_palette_returns_to_masked_input_without_submitting_master_password() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().join("new.kdbx"), true, Duration::from_secs(120));
        app.runtime.master.insert("DUMMY_MASTER", false);
        palette_command(&mut app, "unlock");
        assert!(app.runtime.palette.is_none());
        assert!(app.runtime.session.is_none());
        assert_eq!(app.runtime.master.text(), "DUMMY_MASTER");
        assert!(!screen(&app).contains("DUMMY_MASTER"));
        assert!(!app.path.exists());
    }
    #[test]
    fn invisible_palette_and_unknown_commands_do_not_execute_actions() {
        let (_root, mut app) = app();
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.runtime.form.is_none());
        assert!(app.runtime.palette.is_some());
        app.paste("powershell.exe remove-item");
        assert!(screen(&app).contains("No matching commands"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.runtime.form.is_none());
        assert!(!app.exit);
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(app.runtime.palette.is_none());
        assert!(app.runtime.session.is_none());
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
        assert_eq!(app.runtime.mode, Mode::VaultAction);
        fill_sensitive(
            &mut app,
            &[
                "test master passphrase",
                "UI_DUMMY_MASTER_new",
                "mismatched new master",
            ],
        );
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let form = app.runtime.sensitive.as_ref().unwrap();
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
        assert_eq!(app.runtime.mode, Mode::Browse);
        assert!(app.runtime.sensitive.is_none());
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
        assert!(app.runtime.palette.is_none());
        assert!(app.runtime.reveal_until.is_none());
        assert!(!screen(&app).contains("UI_DUMMY_MASTER"));
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.runtime.sensitive.is_none());
        assert!(app.runtime.delete_authorization.is_none());
        assert!(app.runtime.session.is_none());
    }
    #[test]
    fn whole_vault_deletion_requires_reauthentication_filename_and_a_visible_final_confirmation() {
        let (_root, mut app) = app();
        let before = std::fs::read(&app.path).unwrap();
        palette_command(&mut app, "delete vault");
        assert_eq!(app.runtime.mode, Mode::VaultAction);
        fill_sensitive(&mut app, &["test master passphrase", "wrong.kdbx"]);
        assert!(app.runtime.delete_authorization.is_none());
        assert!(app
            .runtime
            .sensitive
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .all(|field| field.text().is_empty()));
        fill_sensitive(&mut app, &["test master passphrase", "test.kdbx"]);
        assert_eq!(app.runtime.mode, Mode::VaultDeleteConfirm);
        assert!(app.runtime.sensitive.is_none());
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)); // No rendered confirmation yet.
        assert!(app.path.exists());
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(app.path.exists());
        assert!(screen(&app).contains("Final confirmation"));
        assert!(app.render.vault_delete_visible.get());
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::VaultDeleted);
        assert!(app.runtime.session.is_none());
        assert!(app.runtime.delete_authorization.is_none());
        assert!(!app.path.exists());
        assert_eq!(
            std::fs::read(app.runtime.deleted_backup.as_ref().unwrap()).unwrap(),
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
        let form = app.runtime.sensitive.as_mut().unwrap();
        form.fields[0] = Input::from("test master passphrase");
        form.fields[1] = Input::from("test.kdbx");
        form.field = 1;
        let _ = screen_rows(&app, 40, 10);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.runtime.delete_authorization.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
        let _ = screen(&app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::VaultDeleteConfirm);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::Browse);
        assert!(app.runtime.delete_authorization.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
    }
    #[test]
    fn update_checks_require_visible_consent_and_results_do_not_discard_an_entry_draft() {
        let (_root, mut app) = app();
        app.edit(true);
        app.runtime.form.as_mut().unwrap().fields[0] = Input::from("Keep this draft");
        app.runtime.form.as_mut().unwrap().dirty = true;
        palette_command(&mut app, "check updates");
        assert_eq!(app.runtime.mode, Mode::UpdateConsent);
        assert!(app.runtime.update_rx.is_none());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // Unrendered consent must not start a request.
        assert!(app.runtime.update_rx.is_none());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::Edit);
        let (sender, receiver) = std::sync::mpsc::channel();
        app.runtime.update_rx = Some(receiver);
        sender
            .send(Ok(UpdateStatus::Available("99.0.0".into())))
            .unwrap();
        assert!(app.tick(Instant::now()));
        assert_eq!(app.runtime.mode, Mode::Edit);
        assert_eq!(
            app.runtime.form.as_ref().unwrap().fields[0].text(),
            "Keep this draft"
        );
        palette_command(&mut app, "update result");
        assert_eq!(app.runtime.mode, Mode::UpdateResult);
        assert!(screen(&app).contains("99.0.0"));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::Edit);
        assert!(app.runtime.form.as_ref().unwrap().dirty);
    }
    #[test]
    fn readonly_and_browser_modes_never_offer_sensitive_commands() {
        let (_root, mut app) = app();
        app.runtime.session.as_mut().unwrap().readonly = true;
        for command in [Command::ChangeMaster, Command::DeleteVault] {
            assert!(!app.available_commands().contains(&command));
            app.execute_command(command);
            assert!(app.runtime.sensitive.is_none());
        }
        assert!(app.available_commands().contains(&Command::CheckUpdates));
        app.browser_origin = Some("https://example.com".into());
        assert!(app.available_commands().is_empty());
        app.execute_command(Command::CheckUpdates);
        assert!(app.runtime.update_rx.is_none());
        assert_eq!(app.runtime.mode, Mode::Browse);
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
        assert!(app.runtime.session.is_some());
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
            app.runtime.master = Input::from("AUTH_DUMMY_SECRET");
            app.runtime.confirmation = Input::from("CONFIRM_DUMMY_SECRET");
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
        app.runtime.form.as_mut().unwrap().dirty = true;
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
        let form = app.runtime.form.as_mut().unwrap();
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
        let form = app.runtime.form.as_mut().unwrap();
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
        app.runtime.master.insert("MASTER_MUST_BE_HIDDEN", false);
        let output = screen(&app);
        assert!(!output.contains("MASTER_MUST_BE_HIDDEN"));
        assert!(!output.contains("Example"));
    }
    #[test]
    fn auto_lock_discards_draft_and_manual_exit_requires_confirmation() {
        let (_root, mut app) = app();
        app.edit(true);
        app.runtime.form.as_mut().unwrap().dirty = true;
        app.leave(Action::Quit);
        assert_eq!(app.runtime.mode, Mode::Discard(Action::Quit));
        assert!(!app.exit);
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::Edit);
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.runtime.session.is_none());
        assert!(app.runtime.form.is_none());
        assert!(app.runtime.entries.is_empty());
    }
    #[test]
    fn failed_save_keeps_draft_and_never_shows_field_values_in_error() {
        let (_root, mut app) = app();
        app.edit(true);
        let form = app.runtime.form.as_mut().unwrap();
        form.fields[2] = Input::from("DO_NOT_PRINT_PASSWORD");
        form.dirty = true;
        assert!(!app.save());
        assert!(app.runtime.form.is_some());
        assert!(!app.status.contains("DO_NOT_PRINT_PASSWORD"));
    }

    #[test]
    fn cancelled_creation_does_not_write_and_confirmation_is_masked() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing/vault.kdbx");
        let mut app = App::new(path.clone(), true, Duration::from_secs(120));
        app.runtime.master.insert("first passphrase", false);
        app.runtime
            .confirmation
            .insert("different passphrase", false);
        let view = screen(&app);
        assert!(!view.contains("first passphrase"));
        assert!(!view.contains("different passphrase"));
        app.unlock();
        assert!(app.runtime.session.is_none());
        assert!(app.runtime.master.text().is_empty());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn small_terminal_still_shows_unsaved_confirmation() {
        let (_root, mut app) = app();
        app.edit(true);
        app.runtime.form.as_mut().unwrap().dirty = true;
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
        assert!(app.runtime.session.is_none());
    }

    #[test]
    fn browser_release_requires_exact_origin_and_two_explicit_actions() {
        let (_root, mut app) = app();
        for (title, url) in [
            ("Good", "https://example.com/login"),
            ("Bad", "https://example.com.evil.org"),
            ("Subdomain", "https://sub.example.com"),
        ] {
            app.runtime
                .session
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
        assert_eq!(app.runtime.visible.len(), 1);
        assert!(screen(&app).contains("Good"));
        assert!(!screen(&app).contains("dummy-secret"));
        for key in ['n', 'd', 'g', 'p', 'c', 'u', 'y'] {
            app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        }
        assert!(app.runtime.form.is_none());
        assert!(app.runtime.reveal_until.is_none());
        assert!(app.browser_credentials.is_none());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.runtime.mode, Mode::BrowserConfirm);
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
        assert!(app.runtime.session.is_none());
        assert_eq!(std::fs::read(&app.path).unwrap(), before);
    }

    #[test]
    fn browser_cancel_and_auto_lock_release_nothing() {
        let (_root, mut app) = app();
        app.browser_origin = Some("https://example.com".into());
        app.tick(app.last_activity + Duration::from_secs(121));
        assert!(app.runtime.session.is_none());
        assert!(app.browser_credentials.is_none());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.exit);
        assert!(app.browser_credentials.is_none());
    }
}
