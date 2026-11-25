use super::*;

impl App {
    pub(super) fn open_update_download(&mut self) {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            self.warning(self.tr("In-app installation supports Windows x86_64 only. Use the official release page.", "Установка в приложении доступна только для Windows x86_64. Используйте официальную страницу релиза."));
            return;
        }
        self.runtime.mode = Mode::UpdateDownload;
        self.render.install_visible.set(false);
        self.clear_notice();
    }
    pub(super) fn update_install_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            // Dropping the receiver means a completed worker result is dropped
            // and its owned staging directory is removed. No worker can apply.
            self.runtime.download_rx = None;
            self.runtime.mode = Mode::Browse;
            return;
        }
        if key.kind != KeyEventKind::Press
            || key.code != KeyCode::Enter
            || !self.render.install_visible.replace(false)
        {
            return;
        }
        if self.runtime.session.is_none()
            || self.runtime.form.is_some()
            || self.browser_origin.is_some()
        {
            return;
        }
        if self.runtime.mode == Mode::UpdateDownload {
            if self.runtime.download_rx.is_some() {
                return;
            }
            let Some(UpdateStatus::Available(version)) = &self.update_result else {
                return;
            };
            match updates::start_download(version.clone()) {
                Ok(receiver) => {
                    self.runtime.download_rx = Some(receiver);
                    self.info(self.tr("Downloading and verifying…", "Скачиваем и проверяем…"));
                }
                Err(_) => self.error(self.tr("Cannot stage update. Use a complete, setup-installed Windows package and retry.", "Не удалось подготовить обновление. Нужна полная установка Windows через setup; повторите.")),
            }
        }
    }
    pub(super) fn poll_download(&mut self) -> bool {
        let Some(receiver) = &self.runtime.download_rx else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Update staging was interrupted; retry.".into()),
        };
        self.runtime.download_rx = None;
        match result {
            Ok(staged) => {
                if self.runtime.mode == Mode::UpdateDownload
                    && self.runtime.session.is_some()
                    && self.runtime.form.is_none()
                    && self.browser_origin.is_none()
                {
                    // The visible consent covers both stages. Lock before spawning.
                    self.lock();
                    match staged.apply(&self.path, self.settings_file.as_deref(), self.lock_after) {
                        Ok(()) => self.exit = true,
                        Err(_) => self.error(self.tr("Update could not start. Vault is locked; installation unchanged. Unlock and retry.", "Не удалось начать обновление. Хранилище заблокировано; установка не изменена. Разблокируйте и повторите.")),
                    }
                }
            }
            Err(_) => self.error(self.tr(
                "Update download/verification failed. Installation unchanged. Retry or cancel.",
                "Скачивание/проверка не выполнены. Установка не изменена. Повторите или отмените.",
            )),
        }
        true
    }
    pub(super) fn install_text(&self) -> String {
        if self.runtime.download_rx.is_some() {
            return self
                .tr(
                    "Downloading and verifying…\n\n[Esc] Cancel installation",
                    "Скачивание и проверка…\n\n[Esc] Отменить установку",
                )
                .into();
        }
        let version = match &self.update_result {
            Some(UpdateStatus::Available(v)) => v.as_str(),
            _ => "",
        };
        localized!(self.language,
                "Download and install version {version}, then restart?\n\nGitHub receives your IP and app version, not vault data.\nThe ZIP is unsigned; SHA-256 is not a publisher signature.\nClose active browser fill dialogs. Only owned app and bridge files change.\nVaults and settings stay intact. After restart, unlock again.\n\n[Enter] Download, install and restart   [Esc] Cancel",
                "Скачать и установить версию {version}, затем перезапустить?\n\nGitHub получит IP и версию приложения, но не данные базы.\nZIP не подписан; SHA-256 не заменяет подпись издателя.\nЗакройте диалоги автозаполнения. Меняются только файлы приложения и моста.\nБазы и настройки сохраняются. После перезапуска разблокируйте базу.\n\n[Enter] Скачать, установить и перезапустить   [Esc] Отмена")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn fixture() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(
            root.path().join("vault.kdbx"),
            true,
            Duration::from_secs(120),
        );
        app.clipboard = Clipboard::memory_for_test();
        app.runtime.master = Input::from("updater test master passphrase");
        app.runtime.confirmation = Input::from("updater test master passphrase");
        app.unlock();
        app.update_result = Some(UpdateStatus::Available("99.0.0".into()));
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
    fn deliver_staged(app: &mut App) -> PathBuf {
        let directory = tempfile::Builder::new()
            .prefix("termvault-update-")
            .tempdir()
            .unwrap();
        let path = directory.path().to_owned();
        let (sender, receiver) = std::sync::mpsc::channel();
        app.runtime.download_rx = Some(receiver);
        sender
            .send(Ok(updates::StagedUpdate::for_test(directory)))
            .unwrap();
        path
    }
    #[test]
    fn update_requires_visible_consent_and_cancellation_drops_the_worker_result() {
        let (_root, mut app) = fixture();
        app.runtime.mode = Mode::UpdateDownload;
        for language in [Language::English, Language::Russian] {
            app.language = language;
            let text = draw(&app, 120, 50);
            assert!(app.render.install_visible.get());
            assert!(!text.contains("updater test master passphrase"));
        }
        draw(&app, 50, 10);
        assert!(!app.render.install_visible.get());
        enter(&mut app);
        assert!(app.runtime.download_rx.is_none());
        let stage = deliver_staged(&mut app);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!stage.exists());
        assert!(app.runtime.session.is_some());
        assert!(!app.poll_download());
        assert_eq!(app.runtime.mode, Mode::Browse);
    }
    #[test]
    fn lock_inactivity_and_quit_drop_staging_and_worker_receivers() {
        for action in ["lock", "timeout", "quit"] {
            let (_root, mut app) = fixture();
            app.runtime.mode = Mode::UpdateDownload;
            let stage = deliver_staged(&mut app);
            match action {
                "lock" => app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                "quit" => app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
                _ => {
                    app.tick(app.last_activity + app.lock_after);
                }
            }
            assert!(!stage.exists());
            assert!(app.runtime.session.is_none());
            assert!(app.runtime.download_rx.is_none());
        }
    }
    #[test]
    fn updates_cannot_apply_from_browser_modal_or_any_entry_draft() {
        let (_root, mut app) = fixture();
        assert!(app.command_allowed(Command::InstallUpdate, Invocation::Palette));
        app.browser_origin = Some("https://example.test".into());
        assert!(!app.command_allowed(Command::InstallUpdate, Invocation::Palette));
        app.browser_origin = None;
        for mode in [
            Mode::Edit,
            Mode::Search,
            Mode::Delete,
            Mode::Csv,
            Mode::UpdateConsent,
            Mode::VaultAction,
        ] {
            app.runtime.mode = mode;
            assert!(!app.command_allowed(Command::InstallUpdate, Invocation::Palette));
        }
        app.runtime.mode = Mode::Browse;
        app.runtime.form = Some(Form::new(None, EntryData::default()));
        assert!(!app.command_allowed(Command::InstallUpdate, Invocation::Palette));
        app.runtime.form.as_mut().unwrap().dirty = true;
        assert!(!app.command_allowed(Command::InstallUpdate, Invocation::Palette));
    }
    #[test]
    fn failed_apply_preflight_still_clears_vault_clipboard_and_staging_before_returning() {
        let (_root, mut app) = fixture();
        app.clipboard
            .copy("PRIVATE_CLIPBOARD", Instant::now())
            .unwrap();
        app.runtime.mode = Mode::UpdateDownload;
        let stage = deliver_staged(&mut app);
        // The test harness is not an installed tvault.exe, so apply's ownership
        // preflight fails before spawning anything or touching program files.
        assert!(app.poll_download());
        assert!(app.runtime.session.is_none());
        assert!(!stage.exists());
        assert!(!app.exit);
        assert!(!app.status.contains("PRIVATE_CLIPBOARD"));
    }
}
