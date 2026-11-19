//! CSV workflows own all temporary secrets inside the lock-scoped runtime.
use super::*;
use crate::csv_transfer::{self, ImportPreview};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CsvStep {
    Path,
    ImportConfirm,
    Master,
    ExportConfirm,
}

pub(super) struct CsvForm {
    pub export: bool,
    pub step: CsvStep,
    pub path: Input,
    pub master: Input,
    pub preview: Option<ImportPreview>,
    pub folder: Option<GroupId>,
}

// Keep the workflow's isolated fixtures beside its owned state definition.
#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    const MASTER: &str = "csv frontend master passphrase";
    const VALUES: [&str; 5] = [
        "CSV_PRIVATE_TITLE",
        "CSV_PRIVATE_USER",
        "CSV_PRIVATE_PASSWORD",
        "https://csv-private.test",
        "CSV_PRIVATE_NOTES",
    ];

    fn fixture() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(
            root.path().join("vault.kdbx"),
            true,
            Duration::from_secs(120),
        );
        app.clipboard = Clipboard::memory_for_test();
        app.runtime.master = Input::from(MASTER);
        app.runtime.confirmation = Input::from(MASTER);
        app.unlock();
        assert!(app.runtime.session.is_some());
        (root, app)
    }
    fn draw(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    fn enter(app: &mut App) {
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    fn csv_file(root: &Path) -> PathBuf {
        let path = root.join("source.csv");
        let mut writer = csv::Writer::from_path(&path).unwrap();
        writer
            .write_record(["title", "username", "password", "url", "notes"])
            .unwrap();
        writer.write_record(VALUES).unwrap();
        writer.flush().unwrap();
        path
    }
    #[test]
    fn preview_never_renders_fields_and_explicit_append_preserves_all_five() {
        let (root, mut app) = fixture();
        let source = csv_file(root.path());
        let folder = app
            .runtime
            .session
            .as_mut()
            .unwrap()
            .create_folder(None, "Import destination")
            .unwrap();
        app.runtime.folder = Some(folder);
        app.runtime.explorer_focus = Some(ExplorerNode::Folder(folder));
        app.execute_command(Command::ImportCsv);
        app.paste(source.to_str().unwrap());
        enter(&mut app); // no visible path yet
        assert!(app.runtime.csv.as_ref().unwrap().preview.is_none());
        draw(&app, 120, 50);
        enter(&mut app);
        assert_eq!(
            app.runtime
                .csv
                .as_ref()
                .unwrap()
                .preview
                .as_ref()
                .unwrap()
                .len(),
            1
        );
        for language in [Language::English, Language::Russian] {
            app.language = language;
            let screen = draw(&app, 120, 50);
            assert!(screen.contains("Import destination"));
            for value in VALUES {
                assert!(!screen.contains(value));
            }
        }
        draw(&app, 50, 10);
        enter(&mut app);
        assert!(app.runtime.session.as_ref().unwrap().entries().is_empty());
        draw(&app, 120, 50);
        enter(&mut app);
        assert!(app.runtime.csv.is_none());
        let session = app.runtime.session.as_ref().unwrap();
        let id = session.entries()[0].0;
        let data = session.entry(id).unwrap();
        for (actual, expected) in [
            &data.title,
            &data.username,
            &data.password,
            &data.url,
            &data.notes,
        ]
        .into_iter()
        .zip(VALUES)
        {
            assert_eq!(actual.as_str(), expected);
        }
        assert_eq!(session.entry_folder(id), Some(folder));
        assert!(!app.status.contains(VALUES[2]));
    }
    #[test]
    fn export_wrong_master_retries_separately_masked_and_never_overwrites() {
        let (root, mut app) = fixture();
        let source = csv_file(root.path());
        let preview = csv_transfer::read_import(&source).unwrap();
        app.runtime
            .session
            .as_mut()
            .unwrap()
            .import_csv(&preview, None)
            .unwrap();
        let target = root.path().join("export.csv");
        app.execute_command(Command::ExportCsv);
        app.paste(target.to_str().unwrap());
        draw(&app, 120, 50);
        enter(&mut app);
        assert_eq!(app.runtime.csv.as_ref().unwrap().step, CsvStep::Master);
        app.paste("wrong current master");
        let screen = draw(&app, 120, 50);
        assert!(!screen.contains("wrong current master"));
        enter(&mut app);
        assert!(!target.exists()); // masked authentication stage cannot write
        let screen = draw(&app, 120, 50);
        assert!(screen.contains("FINAL CONFIRMATION"));
        assert!(!screen.contains("wrong current master"));
        enter(&mut app);
        assert!(!target.exists());
        let form = app.runtime.csv.as_ref().unwrap();
        assert_eq!(form.step, CsvStep::Master);
        assert!(form.master.text().is_empty());
        assert_eq!(form.path.text(), target.to_str().unwrap());
        app.paste(MASTER);
        draw(&app, 120, 50);
        enter(&mut app);
        draw(&app, 50, 10);
        enter(&mut app);
        assert!(!target.exists());
        draw(&app, 120, 50);
        enter(&mut app);
        let exported = std::fs::read(&target).unwrap();
        let preview = csv_transfer::read_import(&target).unwrap();
        assert_eq!(preview.len(), 1);
        for value in VALUES {
            assert!(String::from_utf8_lossy(&exported).contains(value));
        }
        app.execute_command(Command::ExportCsv);
        app.paste(target.to_str().unwrap());
        draw(&app, 120, 50);
        enter(&mut app);
        app.paste(MASTER);
        draw(&app, 120, 50);
        enter(&mut app);
        draw(&app, 120, 50);
        enter(&mut app);
        assert_eq!(std::fs::read(target).unwrap(), exported);
        assert!(app.runtime.csv.as_ref().unwrap().master.text().is_empty());
    }
    #[test]
    fn cancel_lock_exit_and_inactivity_drop_paths_preview_and_master() {
        let (root, mut app) = fixture();
        let source = csv_file(root.path());
        for action in ["cancel", "lock", "exit", "timeout"] {
            if app.runtime.session.is_none() {
                app.runtime.master = Input::from(MASTER);
                app.unlock();
            }
            app.exit = false;
            app.open_csv(false);
            app.paste(source.to_str().unwrap());
            draw(&app, 120, 50);
            enter(&mut app);
            app.runtime.csv.as_mut().unwrap().master = Input::from(MASTER);
            match action {
                "cancel" => app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                "lock" => app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                "exit" => app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
                _ => {
                    app.tick(app.last_activity + app.lock_after);
                }
            }
            assert!(app.runtime.csv.is_none());
            assert!(!app.render.csv_visible.get());
            assert!(std::fs::read_to_string(&source)
                .unwrap()
                .contains(VALUES[2]));
        }
    }
    #[test]
    fn csv_commands_are_normal_writable_browse_only_and_master_limit_includes_altgr() {
        let (_root, mut app) = fixture();
        for mode in [
            Mode::Search,
            Mode::Edit,
            Mode::Delete,
            Mode::UpdateConsent,
            Mode::BrowserConfirm,
            Mode::NewFolder,
            Mode::Csv,
            Mode::UpdateInstall,
        ] {
            app.runtime.mode = mode;
            for command in [Command::ImportCsv, Command::ExportCsv] {
                assert!(!app.command_allowed(command, Invocation::Palette));
            }
        }
        app.runtime.mode = Mode::Browse;
        for command in [Command::ImportCsv, Command::ExportCsv] {
            app.runtime.session.as_mut().unwrap().readonly = true;
            assert!(!app.command_allowed(command, Invocation::Palette));
            app.runtime.session.as_mut().unwrap().readonly = false;
            app.browser_origin = Some("https://example.test".into());
            assert!(!app.command_allowed(command, Invocation::Palette));
            app.browser_origin = None;
            app.runtime.form = Some(Form::new(None, EntryData::default()));
            assert!(!app.command_allowed(command, Invocation::Palette));
            app.runtime.form = None;
            assert!(app.command_allowed(command, Invocation::Palette));
        }
        app.open_csv(true);
        let form = app.runtime.csv.as_mut().unwrap();
        form.step = CsvStep::Master;
        form.master = Input::from(&"a".repeat(1023));
        app.key(KeyEvent::new(
            KeyCode::Char('я'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        assert_eq!(app.runtime.csv.as_ref().unwrap().master.text().len(), 1023);
        app.key(KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        assert_eq!(app.runtime.csv.as_ref().unwrap().master.text().len(), 1024);
        app.paste("x");
        assert_eq!(app.runtime.csv.as_ref().unwrap().master.text().len(), 1024);
    }
}

impl App {
    pub(super) fn open_csv(&mut self, export: bool) {
        self.runtime.csv = Some(CsvForm {
            export,
            step: CsvStep::Path,
            path: Input::default(),
            master: Input::default(),
            preview: None,
            folder: self.creation_folder(),
        });
        self.runtime.mode = Mode::Csv;
        self.runtime.reveal_until = None;
        self.render.csv_visible.set(false);
        self.clear_notice();
    }
    fn cancel_csv(&mut self) {
        self.runtime.csv = None;
        self.runtime.mode = Mode::Browse;
        self.render.csv_visible.set(false);
        self.clear_notice();
    }
    pub(super) fn csv_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.cancel_csv();
            return;
        }
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.code != KeyCode::Enter {
            if let Some(form) = &mut self.runtime.csv {
                if key.code == KeyCode::BackTab {
                    form.master.clear();
                    form.step = CsvStep::Path;
                    form.preview = None;
                    self.render.csv_visible.set(false);
                    return;
                }
                let field = match form.step {
                    CsvStep::Path => Some(&mut form.path),
                    CsvStep::Master => Some(&mut form.master),
                    _ => None,
                };
                if let Some(field) = field {
                    let maximum = if form.step == CsvStep::Master {
                        1024
                    } else {
                        input::MAX_INPUT
                    };
                    let fits = match key.code {
                        KeyCode::Char(c) => {
                            field.text().len().saturating_add(c.len_utf8()) <= maximum
                        }
                        _ => true,
                    };
                    if fits || input::control_shortcut(key) {
                        field.key(key, false);
                    }
                }
            }
            // Any input invalidates the previous frame's consent.
            self.render.csv_visible.set(false);
            return;
        }
        if !self.render.csv_visible.replace(false) {
            return;
        }
        let Some(form) = &mut self.runtime.csv else {
            return;
        };
        match form.step {
            CsvStep::Path => {
                let path = Path::new(form.path.text());
                if !path.is_absolute()
                    || path
                        .extension()
                        .is_none_or(|ext| !ext.eq_ignore_ascii_case("csv"))
                {
                    self.error(self.tr(
                        "Use an absolute .csv file path.",
                        "Укажите абсолютный путь к файлу .csv.",
                    ));
                    return;
                }
                if form.export {
                    form.step = CsvStep::Master;
                } else {
                    match csv_transfer::read_import(path) {
                        Ok(preview) => {
                            form.preview = Some(preview);
                            form.step = CsvStep::ImportConfirm;
                        }
                        _ => self.error(self.tr("CSV could not be imported. Check the file and five headers; no entries were changed.", "Не удалось импортировать CSV. Проверьте файл и пять заголовков; записи не изменены.")),
                    }
                }
            }
            CsvStep::Master => {
                if form.master.text().is_empty() {
                    return;
                }
                form.step = CsvStep::ExportConfirm;
            }
            CsvStep::ImportConfirm => {
                let result = self
                    .runtime
                    .session
                    .as_mut()
                    .unwrap()
                    .import_csv(form.preview.as_ref().unwrap(), form.folder);
                match result {
                    Ok(ids) => {
                        let count = ids.len();
                        self.cancel_csv();
                        self.refresh(ids.first().copied());
                        self.info(localized!(
                            self.language,
                            "Imported {count} entries. Encrypted backup retained.",
                            "Добавлено записей: {count}. Зашифрованный бэкап сохранён."
                        ));
                    }
                    Err(_) => self.error(self.tr(
                        "Import failed; no entries changed. Retry or cancel.",
                        "Импорт не выполнен; записи не изменены. Повторите или отмените.",
                    )),
                }
            }
            CsvStep::ExportConfirm => {
                let result = self
                    .runtime
                    .session
                    .as_ref()
                    .unwrap()
                    .export_csv(Path::new(form.path.text()), form.master.text());
                form.master.clear();
                match result {
                    Ok(count) => {
                        self.cancel_csv();
                        self.warning(localized!(self.language, "Exported {count} entries. Protect the plaintext CSV; do not open it in a spreadsheet.", "Экспортировано записей: {count}. Защитите открытый CSV; не открывайте его в электронной таблице."));
                    }
                    Err(_) => {
                        form.step = CsvStep::Master;
                        self.error(self.tr("Export failed. Verify current master password and a new writable .csv path. Retry or cancel.", "Экспорт не выполнен. Проверьте текущий мастер-пароль и новый доступный путь .csv. Повторите или отмените."));
                    }
                }
            }
        }
    }
    pub(super) fn csv_paste(&mut self, text: &str) {
        if let Some(form) = &mut self.runtime.csv {
            match form.step {
                CsvStep::Path => {
                    form.path.insert(text, false);
                }
                CsvStep::Master if form.master.text().len().saturating_add(text.len()) <= 1024 => {
                    form.master.insert(text, false);
                }
                _ => {}
            }
        }
        self.render.csv_visible.set(false);
    }
    pub(super) fn csv_text(&self) -> String {
        let Some(form) = &self.runtime.csv else {
            return String::new();
        };
        let path = form.path.rendered(false, form.step == CsvStep::Path);
        let fields = self.tr("Fields: title, username, password, url, notes.\nFolders and other metadata are not preserved.", "Поля: title, username, password, url, notes.\nПапки и прочие метаданные не сохраняются.");
        let footer = self.tr(
            "[Enter] Continue / confirm   [Shift+Tab] Edit path   [Esc] Cancel",
            "[Enter] Далее / подтвердить   [Shift+Tab] Изменить путь   [Esc] Отмена",
        );
        if form.export {
            let warning = self.tr("DANGER: plaintext export of ALL entries in ALL folders, including passwords.\nAnyone with the CSV can read your secrets. Protect/delete the file yourself.\nCells are preserved exactly: spreadsheet formulas may execute when opened.\nDo NOT open in Excel or other spreadsheets. No existing file is overwritten.", "ОПАСНО: открытый экспорт ВСЕХ записей ВСЕХ папок, включая пароли.\nЛюбой владелец CSV сможет прочитать секреты. Защитите/удалите файл сами.\nЯчейки сохраняются точно: формулы могут выполниться в электронной таблице.\nНЕ открывайте в Excel и подобных программах. Существующие файлы не заменяются.");
            let step = match form.step {
                CsvStep::Path => self
                    .tr(
                        "Enter NEW absolute .csv destination:",
                        "Введите НОВЫЙ абсолютный путь .csv:",
                    )
                    .into(),
                CsvStep::Master => localized!(
                    self.language,
                    "Current master password (masked):\n{}",
                    "Текущий мастер-пароль (скрыт):\n{}",
                    form.master.rendered(true, true)
                ),
                _ => self
                    .tr(
                        "FINAL CONFIRMATION: write plaintext CSV now?",
                        "ПОСЛЕДНЕЕ ПОДТВЕРЖДЕНИЕ: записать открытый CSV?",
                    )
                    .into(),
            };
            format!("{warning}\n\n{fields}\n\n{path}\n\n{step}\n\n{footer}")
        } else {
            let count = form.preview.as_ref().map_or(0, ImportPreview::len);
            let empty = form.preview.as_ref().is_some_and(ImportPreview::is_empty);
            let folder = form
                .folder
                .and_then(|id| {
                    self.runtime
                        .session
                        .as_ref()?
                        .folders()
                        .into_iter()
                        .find(|x| x.0 == id)
                        .map(|x| x.1)
                })
                .unwrap_or_else(|| self.tr("Vault root", "Корень хранилища").into());
            let step = if form.step == CsvStep::Path {
                self.tr(
                    "Enter absolute source .csv path. The file contains plaintext secrets.",
                    "Введите абсолютный путь исходного .csv. Файл содержит открытые секреты.",
                )
                .into()
            } else {
                localized!(self.language, "Validated entries: {count}\nAppend ALL entries to folder: {folder}\nExisting entries are NEVER overwritten. Duplicates are appended.\nAn encrypted backup is created before the single transaction.\nConfirm append?", "Проверено записей: {count}\nДобавить ВСЕ записи в папку: {folder}\nСуществующие записи НЕ заменяются. Дубликаты добавляются.\nПеред единой транзакцией создаётся зашифрованный бэкап.\nПодтвердить добавление?")
            };
            let empty_note = if empty {
                self.tr(
                    "\nEmpty CSV: confirmation is a no-op.",
                    "\nПустой CSV: подтверждение ничего не меняет.",
                )
            } else {
                ""
            };
            format!("{fields}\n\n{path}\n\n{step}{empty_note}\n\n{footer}")
        }
    }
}
