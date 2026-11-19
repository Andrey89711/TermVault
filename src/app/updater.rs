use super::*;

impl App {
    pub(super) fn open_update_download(&mut self) {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            self.warning(self.tr("In-app installation supports Windows x86_64 only. Use the official release page.", "Установка в приложении доступна только для Windows x86_64. Используйте официальную страницу релиза."));
            return;
        }
        self.runtime.mode = if self.runtime.staged_update.is_some() {
            Mode::UpdateInstall
        } else {
            Mode::UpdateDownload
        };
        self.render.install_visible.set(false);
        self.clear_notice();
    }
    pub(super) fn update_install_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.runtime.staged_update = None;
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
                    self.info(self.tr("Downloading and verifying; no installation yet.", "Скачиваем и проверяем; установка ещё не выполняется."));
                }
                Err(_) => self.error(self.tr("Cannot stage update. Use a complete, setup-installed Windows package and retry.", "Не удалось подготовить обновление. Нужна полная установка Windows через setup; повторите.")),
            }
        } else if let Some(staged) = self.runtime.staged_update.take() {
            // Final consent has already been visible. Tear down BEFORE spawn.
            self.lock();
            match staged.apply() {
                Ok(()) => self.exit = true,
                Err(_) => self.error(self.tr("Update could not start. Vault is locked; installation unchanged. Unlock and retry.", "Не удалось начать обновление. Хранилище заблокировано; установка не изменена. Разблокируйте и повторите.")),
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
                self.runtime.staged_update = Some(staged);
                if self.runtime.mode == Mode::UpdateDownload {
                    self.runtime.mode = Mode::UpdateInstall;
                    self.render.install_visible.set(false);
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
        if self.runtime.mode == Mode::UpdateInstall {
            let Some(staged) = &self.runtime.staged_update else {
                return String::new();
            };
            localized!(self.language,
                "INSTALL version {}?\nDestination:\n{}\n\nOfficial release ZIP verified against SHA256SUMS.txt.\nUNSIGNED: checksums are not publisher signatures.\nReplace only owned program files and the owned browser-host copy.\nVaults, backups, settings, browser config, registry and PATH are untouched.\nClose browser bridge processes before continuing.\nVault and clipboard will be cleared; TermVault will EXIT.\nRollback is attempted on failure. No administrator rights or auto-relaunch.\nReopen TermVault yourself; reload the unpacked browser extension.\n\n[Enter] Install and exit   [Esc] Cancel and remove staging",
                "УСТАНОВИТЬ версию {}?\nКаталог:\n{}\n\nОфициальный ZIP проверен по SHA256SUMS.txt.\nБЕЗ ПОДПИСИ: контрольные суммы не являются подписью издателя.\nЗаменяются только файлы программы и принадлежащая ей копия браузерного хоста.\nБазы, бэкапы, настройки, конфиг браузера, реестр и PATH не меняются.\nЗакройте процессы браузерного моста перед продолжением.\nХранилище и буфер будут очищены; TermVault ЗАВЕРШИТСЯ.\nПри ошибке выполняется откат. Без прав администратора и автозапуска.\nОткройте TermVault сами; перезагрузите распакованное расширение.\n\n[Enter] Установить и выйти   [Esc] Отмена и удаление подготовки",
                staged.version, input::display(&staged.destination.display().to_string()))
        } else {
            let version = match &self.update_result {
                Some(UpdateStatus::Available(v)) => v.as_str(),
                _ => "",
            };
            let progress = if self.runtime.download_rx.is_some() {
                self.tr(
                    "\nDownload running; Enter does nothing.",
                    "\nИдёт скачивание; Enter ничего не делает.",
                )
            } else {
                ""
            };
            localized!(self.language,
                "DOWNLOAD version {version} from official TermVault GitHub releases?\n\nGitHub/release asset hosts receive your IP and application User-Agent.\nNo vault data, credentials, paths or sites are sent.\nHTTPS only; bounded downloads and redirects restricted to GitHub asset hosts.\nExact Windows x86_64 ZIP + SHA256SUMS.txt; complete payload allowlist.\nUNSIGNED: checksums detect corruption, NOT a compromised publisher.\nNetwork download does NOT authorize installation.\nA SEPARATE final install confirmation follows verification.\nNo startup checks, no automatic installation.\n{progress}\n[Enter] Download and verify   [Esc] Cancel",
                "СКАЧАТЬ версию {version} из официальных релизов TermVault на GitHub?\n\nGitHub/серверы файлов получат IP и User-Agent приложения.\nБаза, пароли, пути и сайты не отправляются.\nТолько HTTPS; ограничены размеры, редиректы — лишь серверы GitHub.\nТочный ZIP Windows x86_64 + SHA256SUMS.txt; полный список файлов.\nБЕЗ ПОДПИСИ: суммы выявляют повреждение, НЕ взлом издателя.\nРазрешение скачивания НЕ разрешает установку.\nПосле проверки потребуется ОТДЕЛЬНОЕ подтверждение установки.\nБез проверок при запуске и автоматической установки.\n{progress}\n[Enter] Скачать и проверить   [Esc] Отмена")
        }
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
        assert!(app.poll_download());
        path
    }
    #[test]
    fn worker_completion_requires_separate_new_visible_install_consent_and_resize_revokes_it() {
        let (_root, mut app) = fixture();
        // An injected worker result avoids all HTTP and does not authorize apply.
        app.runtime.mode = Mode::UpdateDownload;
        app.render.install_visible.set(true);
        let stage = deliver_staged(&mut app);
        assert_eq!(app.runtime.mode, Mode::UpdateInstall);
        assert!(!app.render.install_visible.get());
        enter(&mut app);
        assert!(stage.exists());
        assert!(app.runtime.session.is_some());
        for language in [Language::English, Language::Russian] {
            app.language = language;
            let text = draw(&app, 120, 50);
            assert!(app.render.install_visible.get());
            assert!(!text.contains("updater test master passphrase"));
        }
        draw(&app, 50, 10);
        assert!(!app.render.install_visible.get());
        enter(&mut app);
        assert!(stage.exists());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!stage.exists());
        assert!(app.runtime.staged_update.is_none());
        assert_eq!(app.runtime.mode, Mode::Browse);
    }
    #[test]
    fn lock_inactivity_and_quit_drop_staging_and_worker_receivers() {
        for action in ["lock", "timeout", "quit"] {
            let (_root, mut app) = fixture();
            app.runtime.mode = Mode::UpdateDownload;
            let stage = deliver_staged(&mut app);
            let (sender, receiver) = std::sync::mpsc::channel();
            app.runtime.download_rx = Some(receiver);
            match action {
                "lock" => app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                "quit" => app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
                _ => {
                    app.tick(app.last_activity + app.lock_after);
                }
            }
            assert!(!stage.exists());
            assert!(app.runtime.session.is_none());
            assert!(app.runtime.staged_update.is_none());
            assert!(app.runtime.download_rx.is_none());
            assert!(sender.send(Err("cancelled".into())).is_err());
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
        draw(&app, 120, 50);
        // The test harness is not an installed tvault.exe, so apply's ownership
        // preflight fails before spawning anything or touching program files.
        enter(&mut app);
        assert!(app.runtime.session.is_none());
        assert!(app.runtime.staged_update.is_none());
        assert!(!stage.exists());
        assert!(!app.exit);
        assert!(!app.status.contains("PRIVATE_CLIPBOARD"));
    }
}
