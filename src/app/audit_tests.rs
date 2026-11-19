//! Audit regressions: isolated files, in-memory clipboard, no HTTP.
use super::*;
use ratatui::backend::TestBackend;

const MASTER: &str = "audit test master passphrase";
const PASSWORD: &str = "AUDIT_PASSWORD_NOT_FOR_DISPLAY";

#[test]
fn login_at_sign_is_editable_with_shift_or_altgr_and_survives_save_and_reopen() {
    for modifiers in [
        KeyModifiers::NONE,
        KeyModifiers::SHIFT,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
    ] {
        let (_root, mut app) = fixture();
        let id = app.selected_id().unwrap();
        app.edit(false);
        let form = app.runtime.form.as_mut().unwrap();
        form.field = 1;
        form.fields[1] = Input::from("user");
        app.key(KeyEvent::new(KeyCode::Char('@'), modifiers));
        app.paste("example.test");
        assert_eq!(
            app.runtime.form.as_ref().unwrap().fields[1].text(),
            "user@example.test"
        );
        assert!(app.runtime.form.as_ref().unwrap().dirty);
        // AltGr letters must remain text, never trigger quit, lock, palette, or Ctrl+U.
        for c in ['q', 'l', 'p', 'u'] {
            app.key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ));
        }
        assert!(!app.exit);
        assert!(app.runtime.session.is_some());
        assert!(app.runtime.palette.is_none());
        let expected = "user@example.testqlpu";
        assert_eq!(
            app.runtime.form.as_ref().unwrap().fields[1].text(),
            expected
        );
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.runtime.form.is_none());
        assert_eq!(
            app.runtime
                .session
                .as_ref()
                .unwrap()
                .entry(id)
                .unwrap()
                .username
                .as_str(),
            expected
        );
        app.lock();
        let reopened = Session::unlock(&app.path, MASTER).unwrap();
        assert_eq!(reopened.entry(id).unwrap().username.as_str(), expected);
    }
}

fn fixture() -> (tempfile::TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(
        root.path().join("audit.kdbx"),
        true,
        Duration::from_secs(120),
    );
    app.clipboard = Clipboard::memory_for_test();
    app.runtime.master = Input::from(MASTER);
    app.runtime.confirmation = Input::from(MASTER);
    app.unlock();
    assert!(app.runtime.session.is_some());
    app.edit(true);
    app.runtime.form.as_mut().unwrap().fields[0] = Input::from("Audit entry");
    app.runtime.form.as_mut().unwrap().fields[1] = Input::from("audit-user");
    app.runtime.form.as_mut().unwrap().fields[2] = Input::from(PASSWORD);
    app.runtime.form.as_mut().unwrap().fields[3] = Input::from("https://audit.example/login");
    app.runtime.form.as_mut().unwrap().dirty = true;
    assert!(app.save());
    (root, app)
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn capability_table_matches_keyboard_and_palette_in_every_context() {
    // Mutation, disclosure and navigation are all checked; global emergency
    // controls intentionally have a keyboard-only exception behind modals.
    let actions = [
        Command::NewEntry,
        Command::NewGeneratedEntry,
        Command::EditEntry,
        Command::DeleteEntry,
        Command::CopyPassword,
        Command::CopyUsername,
        Command::RevealPassword,
        Command::SearchEntries,
        Command::SaveEntry,
        Command::GeneratePassword,
        Command::ChangeMaster,
        Command::DeleteVault,
        Command::NewFolder,
        Command::MoveToFolder,
        Command::Language,
    ];
    for context in [
        "normal",
        "readonly",
        "locked",
        "browser",
        "modal",
        "edit",
        "readonly-edit",
    ] {
        let (_root, mut app) = fixture();
        if context.ends_with("edit") {
            app.edit(false);
        }
        if context.starts_with("readonly") {
            app.runtime.session.as_mut().unwrap().readonly = true;
        }
        match context {
            "locked" => app.lock(),
            "browser" => app.browser_origin = Some("https://audit.example".into()),
            "modal" => app.runtime.mode = Mode::Delete,
            _ => {}
        }
        for command in actions {
            let expected = match context {
                "normal" => !matches!(command, Command::SaveEntry | Command::GeneratePassword),
                "readonly" => matches!(
                    command,
                    Command::CopyPassword
                        | Command::CopyUsername
                        | Command::RevealPassword
                        | Command::SearchEntries
                        | Command::Language
                ),
                "locked" => command == Command::Language,
                "browser" | "modal" => false,
                "edit" => matches!(
                    command,
                    Command::SaveEntry
                        | Command::GeneratePassword
                        | Command::RevealPassword
                        | Command::Language
                ),
                "readonly-edit" => matches!(command, Command::RevealPassword | Command::Language),
                _ => unreachable!(),
            };
            assert_eq!(
                app.command_allowed(command, Invocation::Keyboard),
                expected,
                "{context}: keyboard {command:?}"
            );
            assert_eq!(
                app.command_allowed(command, Invocation::Palette),
                expected,
                "{context}: palette {command:?}"
            );
        }
        assert!(app.command_allowed(Command::Quit, Invocation::Keyboard));
        assert_eq!(
            app.command_allowed(Command::Lock, Invocation::Keyboard),
            context != "locked"
        );
    }
}

// Independent reference implementation of the pre-predicate palette ordering.
// Keep this procedural oracle separate from policy's fixed-order enumeration.
fn legacy_palette_order(app: &App) -> Vec<Command> {
    if app.language_choice.is_some()
        || app.browser_origin.is_some()
        || matches!(
            app.runtime.mode,
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
    if app.runtime.mode == Mode::VaultDeleted {
        return vec![Command::Quit];
    }
    let Some(session) = &app.runtime.session else {
        let mut commands = vec![Command::Unlock, Command::Language, Command::CheckUpdates];
        if app.update_result.is_some() {
            commands.push(Command::UpdateResult);
        }
        commands.push(Command::Quit);
        return commands;
    };
    let mut commands = Vec::new();
    match app.runtime.mode {
        Mode::Browse => {
            if !session.readonly {
                commands.extend([Command::NewEntry, Command::NewGeneratedEntry]);
            }
            commands.push(Command::SearchEntries);
            if app.selected_id().is_some() {
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
                if app.delete_folder_target().is_some() {
                    commands.push(Command::DeleteFolder);
                }
                if app.selected_id().is_some() {
                    commands.push(Command::MoveToFolder);
                }
                commands.extend([Command::ChangeMaster, Command::DeleteVault]);
            }
        }
        Mode::Edit if app.runtime.form.is_some() => {
            if !session.readonly {
                commands.extend([Command::SaveEntry, Command::GeneratePassword]);
            }
            commands.extend([Command::RevealPassword, Command::Back]);
        }
        Mode::Search => commands.push(Command::Back),
        _ => {}
    }
    commands.extend([Command::Language, Command::CheckUpdates]);
    if app.update_result.is_some() {
        commands.push(Command::UpdateResult);
    }
    commands.extend([Command::Lock, Command::Quit]);
    commands
}

#[test]
fn every_command_matches_legacy_palette_membership_and_order_across_contexts() {
    // Enumerate every Command independently of policy's presentation order.
    let all_commands = [
        Command::Language,
        Command::Unlock,
        Command::NewEntry,
        Command::NewFolder,
        Command::OpenFolder,
        Command::MoveToFolder,
        Command::DeleteFolder,
        Command::NewGeneratedEntry,
        Command::EditEntry,
        Command::SearchEntries,
        Command::CopyPassword,
        Command::CopyUsername,
        Command::RevealPassword,
        Command::DeleteEntry,
        Command::SaveEntry,
        Command::GeneratePassword,
        Command::Back,
        Command::CheckUpdates,
        Command::UpdateResult,
        Command::ChangeMaster,
        Command::DeleteVault,
        Command::Lock,
        Command::Quit,
    ];
    let modes = [
        Mode::Browse,
        Mode::Search,
        Mode::Edit,
        Mode::Delete,
        Mode::Discard(Action::Browse),
        Mode::Discard(Action::Lock),
        Mode::Discard(Action::Quit),
        Mode::BrowserConfirm,
        Mode::VaultAction,
        Mode::VaultDeleteConfirm,
        Mode::VaultDeleted,
        Mode::UpdateConsent,
        Mode::UpdateResult,
        Mode::NewFolder,
        Mode::DeleteFolder,
        Mode::Folders(false),
        Mode::Folders(true),
    ];
    for session_state in ["normal", "readonly", "locked"] {
        let (_root, mut app) = fixture();
        let entry = app.runtime.entries[0].0;
        let root = app.runtime.session.as_ref().unwrap().root_folder();
        let folder = app
            .runtime
            .session
            .as_mut()
            .unwrap()
            .create_folder(None, "Audit folder")
            .unwrap();
        match session_state {
            "readonly" => app.runtime.session.as_mut().unwrap().readonly = true,
            "locked" => app.lock(),
            _ => {}
        }
        for mode in modes {
            for has_form in [false, true] {
                for target in ["entry", "folder", "root", "all", "search"] {
                    for overlay in ["none", "language", "browser"] {
                        for has_update in [false, true] {
                            app.runtime.mode = mode;
                            app.runtime.form =
                                has_form.then(|| Form::new(None, EntryData::default()));
                            app.runtime.folder = match target {
                                "folder" => Some(folder),
                                "root" => Some(root),
                                _ => None,
                            };
                            app.runtime.explorer_focus = match target {
                                "entry" => Some(ExplorerNode::Entry(entry)),
                                "folder" => Some(ExplorerNode::Folder(folder)),
                                "root" => Some(ExplorerNode::Folder(root)),
                                _ => None,
                            };
                            app.runtime.visible = if session_state != "locked" && target == "entry"
                            {
                                vec![0]
                            } else {
                                Vec::new()
                            };
                            app.runtime.selected = 0;
                            app.runtime.query =
                                Input::from(if target == "search" { "Audit" } else { "" });
                            app.language_choice = (overlay == "language").then_some(0);
                            app.browser_origin =
                                (overlay == "browser").then(|| "https://audit.example".to_owned());
                            app.update_result = has_update.then_some(UpdateStatus::UpToDate);
                            let expected = legacy_palette_order(&app);
                            let actual = app.available_commands();
                            assert_eq!(actual, expected,
                                "{session_state}/{mode:?}/form={has_form}/{target}/{overlay}/update={has_update}");
                            assert_eq!(Palette::default().filtered(&actual), expected);
                            for command in all_commands {
                                let member = expected.contains(&command);
                                assert_eq!(
                                    app.command_allowed(command, Invocation::Palette),
                                    member,
                                    "palette {command:?}: {session_state}/{mode:?}/{target}/{overlay}"
                                );
                                let keyboard_expected = match command {
                                    Command::Quit => true,
                                    Command::Lock => app.runtime.session.is_some(),
                                    _ => member,
                                };
                                assert_eq!(
                                    app.command_allowed(command, Invocation::Keyboard),
                                    keyboard_expected,
                                    "keyboard {command:?}: {session_state}/{mode:?}/{target}/{overlay}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn semantic_shortcuts_and_palette_have_equivalent_effects() {
    for (command, code) in [
        (Command::NewEntry, KeyCode::Char('n')),
        (Command::NewGeneratedEntry, KeyCode::Char('g')),
        (Command::EditEntry, KeyCode::Enter),
        (Command::DeleteEntry, KeyCode::Char('d')),
        (Command::SearchEntries, KeyCode::Char('/')),
        (Command::RevealPassword, KeyCode::Char('p')),
        (Command::CopyPassword, KeyCode::Char('c')),
        (Command::CopyUsername, KeyCode::Char('u')),
    ] {
        for readonly in [false, true] {
            let (_left_root, mut keyboard) = fixture();
            let (_right_root, mut palette) = fixture();
            keyboard.runtime.session.as_mut().unwrap().readonly = readonly;
            palette.runtime.session.as_mut().unwrap().readonly = readonly;
            keyboard.key(KeyEvent::new(code, KeyModifiers::NONE));
            palette.open_palette();
            draw(&palette, 110, 32);
            // Same guarded semantic dispatch used by palette Enter and mouse.
            palette.execute_command(command);
            assert_eq!(
                keyboard.runtime.mode, palette.runtime.mode,
                "{command:?}, readonly={readonly}"
            );
            assert_eq!(
                keyboard.runtime.form.is_some(),
                palette.runtime.form.is_some()
            );
            assert_eq!(
                keyboard.runtime.reveal_until.is_some(),
                palette.runtime.reveal_until.is_some()
            );
            assert_eq!(
                keyboard.clipboard.owns_text_for_test(),
                palette.clipboard.owns_text_for_test()
            );
            assert_eq!(
                keyboard.runtime.session.as_ref().unwrap().entries().len(),
                1
            );
        }
    }
}

#[test]
fn modal_emergency_shortcuts_still_require_unsaved_confirmation() {
    for command in [Command::Lock, Command::Quit] {
        for mode in [
            Mode::Delete,
            Mode::VaultAction,
            Mode::UpdateConsent,
            Mode::BrowserConfirm,
        ] {
            let (_root, mut app) = fixture();
            app.edit(false);
            app.runtime.form.as_mut().unwrap().dirty = true;
            app.runtime.mode = mode;
            assert!(!app.command_allowed(command, Invocation::Palette));
            let code = if command == Command::Lock { 'l' } else { 'q' };
            app.key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL));
            assert_eq!(
                app.runtime.mode,
                Mode::Discard(if command == Command::Lock {
                    Action::Lock
                } else {
                    Action::Quit
                })
            );
            assert!(app.runtime.session.is_some());
            assert!(!app.exit);
            app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
            assert!(app.runtime.session.is_none());
            assert_eq!(app.exit, command == Command::Quit);
        }
    }
}

#[test]
fn editor_control_shortcuts_recheck_readonly_capabilities_like_palette_actions() {
    for (command, code) in [
        (Command::SaveEntry, 's'),
        (Command::GeneratePassword, 'g'),
        (Command::RevealPassword, 'o'),
    ] {
        for readonly in [false, true] {
            let (_left_root, mut keyboard) = fixture();
            let (_right_root, mut palette) = fixture();
            for app in [&mut keyboard, &mut palette] {
                app.edit(false);
                app.runtime.form.as_mut().unwrap().fields[0] = Input::from("Changed title");
                app.runtime.form.as_mut().unwrap().dirty = true;
                app.runtime.session.as_mut().unwrap().readonly = readonly;
            }
            keyboard.key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL));
            palette.open_palette();
            draw(&palette, 110, 32);
            palette.execute_command(command);
            assert_eq!(
                keyboard.runtime.mode, palette.runtime.mode,
                "{command:?}, readonly={readonly}"
            );
            assert_eq!(
                keyboard.runtime.form.is_some(),
                palette.runtime.form.is_some()
            );
            assert_eq!(
                keyboard.runtime.reveal_until.is_some(),
                palette.runtime.reveal_until.is_some()
            );
            if readonly {
                for app in [&keyboard, &palette] {
                    let form = app.runtime.form.as_ref().unwrap();
                    assert_eq!(form.fields[0].text(), "Changed title");
                    assert_eq!(form.fields[2].text(), PASSWORD);
                    assert!(form.dirty);
                    assert_eq!(
                        app.runtime.session.as_ref().unwrap().entries()[0].1,
                        "Audit entry"
                    );
                }
            }
        }
    }
}

#[test]
fn lock_drops_the_complete_working_set_from_every_modal_and_sensitive_state() {
    for mode in [
        Mode::Browse,
        Mode::Search,
        Mode::Edit,
        Mode::Delete,
        Mode::Discard(Action::Lock),
        Mode::BrowserConfirm,
        Mode::VaultAction,
        Mode::VaultDeleteConfirm,
        Mode::VaultDeleted,
        Mode::UpdateConsent,
        Mode::UpdateResult,
        Mode::NewFolder,
        Mode::DeleteFolder,
        Mode::Folders(false),
        Mode::Folders(true),
    ] {
        for automatic in [false, true] {
            let (_root, mut app) = fixture();
            let folder = app.runtime.session.as_ref().unwrap().root_folder();
            app.edit(false);
            app.runtime.form.as_mut().unwrap().dirty = true;
            app.runtime.master = Input::from("MASTER_DRAFT");
            app.runtime.confirmation = Input::from("CONFIRMATION_DRAFT");
            app.runtime.query = Input::from("PRIVATE_SEARCH");
            app.runtime.folder_name = Input::from("PRIVATE_FOLDER");
            app.runtime.folder = Some(folder);
            app.runtime.folder_error = Some("folder validation".into());
            app.runtime.folder_selected = 4;
            app.runtime.expanded_folders.insert(folder);
            app.runtime.explorer_focus = Some(ExplorerNode::Folder(folder));
            app.runtime.pending_folder_delete = Some(folder);
            app.runtime.palette = Some(Palette::default());
            app.runtime.palette.as_mut().unwrap().query = Input::from("PRIVATE_PALETTE_QUERY");
            app.runtime.sensitive = Some(SensitiveForm::new(SensitiveKind::ChangeMaster));
            app.runtime.sensitive.as_mut().unwrap().fields[0] = Input::from(MASTER);
            app.runtime.delete_authorization = Some(
                app.runtime
                    .session
                    .as_ref()
                    .unwrap()
                    .authorize_delete(MASTER, "audit.kdbx")
                    .unwrap(),
            );
            app.runtime.deleted_backup = Some(app.path.with_extension("backup"));
            let (_sender, receiver) = std::sync::mpsc::channel();
            app.runtime.update_rx = Some(receiver); // Channel only: never start an HTTP worker.
            app.runtime.update_return = Some(Mode::Edit);
            app.runtime.reveal_until = Some(Instant::now() + Duration::from_secs(10));
            app.language_choice = Some(0);
            app.clipboard.copy(PASSWORD, Instant::now()).unwrap();
            app.runtime.mode = mode;
            if automatic {
                assert!(app.tick(app.last_activity + app.lock_after));
            } else {
                app.lock();
            }
            assert!(app.runtime.session.is_none(), "{mode:?}");
            assert!(app.runtime.entries.is_empty());
            assert!(app.runtime.visible.is_empty());
            assert_eq!(app.runtime.selected, 0);
            assert!(app.runtime.master.text().is_empty());
            assert!(app.runtime.confirmation.text().is_empty());
            assert!(app.runtime.query.text().is_empty());
            assert!(app.runtime.folder_name.text().is_empty());
            assert!(app.runtime.folder.is_none());
            assert!(app.runtime.folder_error.is_none());
            assert_eq!(app.runtime.folder_selected, 0);
            assert!(app.runtime.expanded_folders.is_empty());
            assert!(app.runtime.explorer_focus.is_none());
            assert!(app.runtime.pending_folder_delete.is_none());
            assert!(app.runtime.form.is_none());
            assert!(app.runtime.palette.is_none());
            assert!(app.runtime.sensitive.is_none());
            assert!(app.runtime.delete_authorization.is_none());
            assert!(app.runtime.deleted_backup.is_none());
            assert!(app.runtime.update_rx.is_none());
            assert!(app.runtime.update_return.is_none());
            assert!(app.runtime.reveal_until.is_none());
            assert!(app.language_choice.is_none());
            assert!(!app.clipboard.owns_text_for_test());
            assert!(app.browser_credentials.is_none());
            assert_eq!(app.runtime.mode, Mode::Browse);
            let rendered = draw(&app, 110, 32);
            for secret in [
                PASSWORD,
                MASTER,
                "PRIVATE_SEARCH",
                "PRIVATE_FOLDER",
                "PRIVATE_PALETTE_QUERY",
            ] {
                assert!(!rendered.contains(secret));
            }
        }
    }
}

#[test]
fn resized_hidden_overlays_revoke_feedback_and_cannot_activate_background_hits() {
    let (_root, mut app) = fixture();
    app.execute_command(Command::Language);
    draw(&app, 110, 32);
    let (old_hit, _) = app.render.language_hits.borrow()[1];
    assert!(app.render.language_visible.get());
    draw(&app, 40, 10);
    assert!(!app.render.language_visible.get());
    assert!(app.render.language_hits.borrow().is_empty());
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: old_hit.x,
        row: old_hit.y,
        modifiers: KeyModifiers::NONE,
    });
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.language, Language::English);
    assert!(app.language_choice.is_some());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.open_palette();
    draw(&app, 110, 32);
    let old_hit = app
        .render
        .palette_hits
        .borrow()
        .iter()
        .find(|(_, command)| *command == Command::NewEntry)
        .unwrap()
        .0;
    draw(&app, 40, 10);
    assert!(!app.render.palette_visible.get());
    assert!(app.render.palette_hits.borrow().is_empty());
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: old_hit.x,
        row: old_hit.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.runtime.form.is_none());
    assert!(app.runtime.palette.is_some());
}

#[test]
fn browser_approval_requires_a_new_visible_review_after_returning_or_resizing() {
    for resize in [false, true] {
        let (_root, mut app) = fixture();
        app.browser_origin = Some("https://audit.example".into());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!draw(&app, 110, 32).contains(PASSWORD));
        assert!(app.render.browser_review_visible.get());
        if resize {
            draw(&app, 40, 10);
        } else {
            app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.browser_credentials.is_none());
        assert!(app.runtime.session.is_some());
        assert!(!draw(&app, 110, 32).contains(PASSWORD));
        app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.runtime.session.is_none());
        assert!(app.exit);
        let approved = app.browser_credentials.take().unwrap();
        assert_eq!(approved.password, PASSWORD);
        assert_eq!(approved.origin, "https://audit.example");
        app.lock(); // Approved output remains independent of the dropped runtime.
        assert!(app.browser_credentials.is_none());
    }
}

#[test]
fn resizing_revokes_a_previously_visible_full_deletion_warning() {
    let (_root, mut app) = fixture();
    let before = std::fs::read(&app.path).unwrap();
    app.runtime.delete_authorization = Some(
        app.runtime
            .session
            .as_ref()
            .unwrap()
            .authorize_delete(MASTER, "audit.kdbx")
            .unwrap(),
    );
    app.runtime.mode = Mode::VaultDeleteConfirm;
    draw(&app, 110, 40);
    assert!(app.render.vault_delete_visible.get());
    let text = draw(&app, 40, 10);
    assert!(text.contains("resize"));
    assert!(!app.render.vault_delete_visible.get());
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.runtime.delete_authorization.is_some());
    assert_eq!(std::fs::read(&app.path).unwrap(), before);
    draw(&app, 110, 40);
    assert!(app.render.vault_delete_visible.get());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.runtime.delete_authorization.is_none());
}

#[test]
fn rendering_and_explicit_runtime_access_remain_separate_from_the_controller() {
    let controller = include_str!("../app.rs");
    assert!(!controller.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("fn draw") || line.starts_with("pub(super) fn draw")
    }));
    assert!(!controller.contains("render_widget("));
    assert!(!controller.contains("render_stateful_widget("));
    for source in [
        controller,
        include_str!("runtime.rs"),
        include_str!("render.rs"),
    ] {
        assert!(
            !source.contains("Deref"),
            "do not restore runtime projection"
        );
    }
}

#[test]
fn every_frame_clears_all_consent_and_hit_feedback_even_on_early_return() {
    let root = tempfile::tempdir().unwrap();
    let app = App::new(
        root.path().join("unopened.kdbx"),
        false,
        Duration::from_secs(120),
    );
    for (width, height) in [(110, 32), (40, 10), (0, 0)] {
        let feedback = &app.render;
        let flags = [
            &feedback.language_visible,
            &feedback.folder_dialog_visible,
            &feedback.browser_review_visible,
            &feedback.palette_visible,
            &feedback.sensitive_visible,
            &feedback.vault_delete_visible,
            &feedback.update_consent_visible,
        ];
        for flag in flags {
            flag.set(true);
        }
        let hit = ratatui::layout::Rect::new(1, 1, 1, 1);
        feedback.language_hits.borrow_mut().push((hit, 0));
        feedback
            .explorer_hits
            .borrow_mut()
            .push((hit, ExplorerNode::Entry(EntryId::new())));
        feedback
            .palette_hits
            .borrow_mut()
            .push((hit, Command::Quit));

        draw(&app, width, height);

        for flag in flags {
            assert!(!flag.get(), "{width}x{height}: stale consent");
        }
        assert!(feedback.language_hits.borrow().is_empty());
        assert!(feedback.explorer_hits.borrow().is_empty());
        assert!(feedback.palette_hits.borrow().is_empty());
        assert!(app.runtime.session.is_none());
        assert!(!app.path.exists());
    }
}

#[test]
fn approved_browser_handoff_survives_runtime_teardown_until_taken_once() {
    let (_root, mut app) = fixture();
    let before = std::fs::read(&app.path).unwrap();
    app.browser_origin = Some("https://audit.example".into());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    draw(&app, 110, 32);
    app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.exit);
    assert!(app.runtime.session.is_none());
    assert!(app.runtime.entries.is_empty());
    assert!(app.runtime.form.is_none());
    assert!(app.browser_credentials.is_some());

    app.lock();
    assert!(app.browser_credentials.is_some());
    assert!(!draw(&app, 110, 32).contains(PASSWORD));
    let approved = app.browser_credentials.take().unwrap();
    assert_eq!(approved.origin, "https://audit.example");
    assert_eq!(approved.username, "audit-user");
    assert_eq!(approved.password, PASSWORD);
    assert!(app.browser_credentials.take().is_none());
    assert_eq!(std::fs::read(&app.path).unwrap(), before);
}
