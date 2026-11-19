//! Read-only presentation and last-frame feedback, separate from the working set.
//! A frame must reset its feedback before drawing, including on early returns.
use super::*;
use crate::theme::{self, base};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame,
};
use std::cell::{Cell, RefCell};

/// Last-frame consent visibility and hit geometry. Interior mutability preserves
/// `draw(&self)` without permitting presentation to mutate the vault.
#[derive(Default)]
pub(super) struct RenderFeedback {
    pub language_visible: Cell<bool>,
    pub language_hits: RefCell<Vec<(Rect, usize)>>,
    pub explorer_hits: RefCell<Vec<(Rect, ExplorerNode)>>,
    pub palette_hits: RefCell<Vec<(Rect, Command)>>,
    pub folder_dialog_visible: Cell<bool>,
    pub browser_review_visible: Cell<bool>,
    pub palette_visible: Cell<bool>,
    pub sensitive_visible: Cell<bool>,
    pub vault_delete_visible: Cell<bool>,
    pub update_consent_visible: Cell<bool>,
    pub csv_visible: Cell<bool>,
    pub install_visible: Cell<bool>,
}

impl RenderFeedback {
    pub fn begin_frame(&self) {
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
        self.csv_visible.set(false);
        self.install_visible.set(false);
    }
}

impl App {
    pub(super) fn entry_label(&self, id: EntryId, title: &str) -> String {
        if self
            .runtime
            .session
            .as_ref()
            .is_some_and(|session| !session.entry_has_title(id))
        {
            self.tr("Untitled", "Без названия").into()
        } else {
            title.into()
        }
    }

    pub(super) fn folder_label(&self) -> String {
        self.runtime
            .folder
            .and_then(|id| {
                self.runtime
                    .session
                    .as_ref()?
                    .folders()
                    .into_iter()
                    .find(|folder| folder.0 == id)
                    .map(|folder| folder.1)
            })
            .unwrap_or_else(|| self.tr("All entries", "Все записи").into())
    }

    pub(super) fn draw(&self, frame: &mut Frame) {
        self.render.begin_frame();
        let area = frame.area();
        frame.render_widget(Block::default().style(base()), area);
        if area.width < 60 || area.height < 20 {
            let text = if matches!(self.runtime.mode, Mode::Discard(_)) {
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
        let state = if self.runtime.mode == Mode::VaultDeleted {
            self.tr("REMOVED", "УДАЛЕНО")
        } else if self.runtime.session.is_some() {
            self.tr("UNLOCKED", "ОТКРЫТО")
        } else {
            self.tr("LOCKED", "ЗАКРЫТО")
        };
        let state_color = if self.runtime.session.is_some() {
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
        if self.runtime.session.is_some()
            || self.runtime.mode == Mode::VaultDeleted
            || self.browser_origin.is_some()
        {
            frame.render_widget(Paragraph::new(header).style(base()), rows[0]);
        }
        if self.runtime.mode == Mode::VaultDeleted {
            let backup = self
                .runtime
                .deleted_backup
                .as_ref()
                .map(|path| input::display(&path.display().to_string()))
                .unwrap_or_default();
            frame.render_widget(Paragraph::new(localized!(self.language, "The active file no longer exists.\n\nRetained encrypted backup:\n{backup}\n\nOpen it with the previous master password. Other backups are unchanged.\nThis is not secure erasure.\n\nEnter / Esc — close the application.", "Активный файл больше не существует.\n\nСохранённый зашифрованный бэкап:\n{backup}\n\nЕго можно открыть прежним мастер-паролем. Остальные бэкапы не изменены.\nЭто не безопасное стирание данных.\n\nEnter / Esc — закрыть приложение.")).block(panel(self.tr(" Vault removed ", " Хранилище удалено "))).wrap(Wrap { trim: false }), rows[1]);
        } else if self.runtime.session.is_none() {
            self.draw_auth(frame);
        } else if let Some(form) = &self.runtime.form {
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
                let masked = i == 2 && self.runtime.reveal_until.is_none();
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
                    .runtime
                    .visible
                    .iter()
                    .map(|i| {
                        let (id, title) = &self.runtime.entries[*i];
                        ListItem::new(self.entry_label(*id, title))
                    })
                    .collect();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(
                            panel(&format!(
                                " {} · {} · {} ",
                                self.tr("Entries", "Записи"),
                                self.runtime.visible.len(),
                                self.folder_label()
                            ))
                            .padding(Padding::new(0, 0, 1, 0)),
                        )
                        .highlight_style(invert())
                        .highlight_symbol("› "),
                    parts[0],
                    &mut ListState::default().with_selected(
                        (!self.runtime.visible.is_empty()).then_some(self.runtime.selected),
                    ),
                );
            }
            let data = self
                .selected_id()
                .and_then(|id| self.runtime.session.as_ref()?.entry(id));
            let text = if let Some(data) = data {
                let password = if self.runtime.reveal_until.is_some() {
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
                } else if let Some(ExplorerNode::Folder(folder)) = self
                    .runtime
                    .explorer_focus
                    .filter(|_| self.explorer_active())
                {
                    let (entries, folders) = self
                        .runtime
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
            Paragraph::new(if self.runtime.mode == Mode::VaultDeleted {
                self.tr(" Enter / Esc exit ", " Enter / Esc выход ")
            } else if self.runtime.mode == Mode::VaultAction {
                self.tr(" Tab field · Enter continue · Esc cancel ", " Tab поле · Enter далее · Esc отмена ")
            } else if matches!(self.runtime.mode, Mode::VaultDeleteConfirm | Mode::DeleteFolder) {
                self.tr(" d confirm deletion · Esc cancel ", " d подтвердить удаление · Esc отмена ")
            } else if matches!(self.runtime.mode, Mode::UpdateConsent | Mode::UpdateResult) {
                self.tr(" Enter continue · Esc back ", " Enter далее · Esc назад ")
            } else if self.runtime.session.is_none() && self.browser_origin.is_some() {
                self.tr(" Enter unlock · Esc cancel ", " Enter разблокировать · Esc отмена ")
            } else if self.runtime.session.is_none() {
                self.tr(" Ctrl+P commands · Enter continue · Tab field · Esc quit ", " Ctrl+P команды · Enter далее · Tab поле · Esc выход ")
            } else if self.browser_origin.is_some() {
                self.tr(" ↑↓ select · Enter review · y approve in confirmation · Esc cancel ", " ↑↓ выбор · Enter проверить · y одобрить в диалоге · Esc отмена ")
            } else if self.runtime.form.is_some() {
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
        match self.runtime.mode {
            Mode::Csv => {
                let visible = self.draw_full_consent(frame, &self.csv_text(), self.tr(" CSV transfer ", " Перенос CSV "));
                self.render.csv_visible.set(visible);
            }
            Mode::UpdateDownload | Mode::UpdateInstall => {
                let visible = self.draw_full_consent(frame, &self.install_text(), self.tr(" Update ", " Обновление "));
                self.render.install_visible.set(visible);
            }
            Mode::DeleteFolder => self.draw_folder_delete(frame),
            Mode::NewFolder => {
                let parent = if self.runtime.folder.is_some() { self.folder_label() } else { "/".into() };
                popup(frame, &localized!(self.language, "Parent: {parent}\n\nFolder name:\n{}\n\nUp to 128 bytes, no / or \\.\n{}\n\nEnter create · Esc cancel", "Родитель: {parent}\n\nИмя папки:\n{}\n\nДо 128 байт, без / и \\.\n{}\n\nEnter создать · Esc отмена", self.runtime.folder_name.rendered(false, true), i18n::error(self.language, self.runtime.folder_error.as_deref().unwrap_or(""))), self.tr(" New folder ", " Новая папка "), 16);
                self.render.folder_dialog_visible.set(true);
            }
            Mode::Folders(moving) => self.draw_folders(frame, moving),
            Mode::VaultAction => self.draw_sensitive(frame),
            Mode::VaultDeleteConfirm => self.draw_vault_delete_confirmation(frame),
            Mode::UpdateConsent => {
                popup(frame, self.tr("Send a request to api.github.com?\n\nGitHub receives your IP and User-Agent with the TermVault version.\nPasswords, the vault, its path and the current site are NOT sent.\n\nChecks run only on this command.\nTimeout: 10 seconds. No automatic installation.\n\n[Enter] Check   [Esc] Cancel", "Отправить запрос к api.github.com?\n\nGitHub получит ваш IP и User-Agent с версией TermVault.\nПароли, база, её путь и текущий сайт НЕ передаются.\n\nПроверка выполняется только по этой команде.\nТайм-аут — 10 секунд. Автоустановки нет.\n\n[Enter] Проверить   [Esc] Отмена"), self.tr(" Check for updates ", " Проверить обновления "), 18);
                self.render.update_consent_visible.set(true);
            }
            Mode::UpdateResult => {
                let text = self.update_result.as_ref().map(|status| status.description(self.language)).unwrap_or_else(|| self.tr("No update check result.", "Результат проверки отсутствует.").into());
                popup(frame, &localized!(self.language, "{text}\n\n[Enter / Esc] Back", "{text}\n\n[Enter / Esc] Назад"), self.tr(" Updates ", " Обновления "), 18);
            }
            Mode::BrowserConfirm => {
                let title = self.selected_id().and_then(|id| self.runtime.session.as_ref()?.entry(id))
                    .map(|entry| input::display(&entry.title).replace('\n', " ").chars().take(80).collect::<String>()).unwrap_or_default();
                self.render.browser_review_visible.set(true);
                popup(frame, &localized!(self.language, "Send login and password for:\n{title}\nTo: {}\n\n[y] Approve once   [n] Back   [Esc] Cancel", "Отправить логин и пароль записи:\n{title}\nСайту: {}\n\n[y] Одобрить один раз   [n] Назад   [Esc] Отмена", self.browser_origin.as_deref().unwrap_or("")), self.tr(" Confirm browser fill ", " Подтвердить автозаполнение "), 18);
            }
            Mode::Search=>popup(frame,&localized!(self.language, "{}\nEnter applies · Esc clears", "{}\nEnter применить · Esc очистить", if self.runtime.query.text().is_empty() { self.tr("Search entry titles…", "Поиск по названиям…").to_owned() } else { self.runtime.query.rendered(false,true) }),self.tr(" Filter entry titles ", " Поиск по названиям "),5),
            Mode::Discard(_)=>popup(frame,self.tr("Unsaved changes\n\n[s] Save and continue   [d] Discard   [c/Esc] Cancel\nInactivity auto-lock discards unsaved drafts.", "Несохранённые изменения\n\n[s] Сохранить и продолжить   [d] Отбросить   [c/Esc] Отмена\nАвтоблокировка удаляет несохранённые черновики."),self.tr(" Unsaved entry ", " Несохранённая запись "),7),
            Mode::Delete=>popup(frame,self.tr("Delete the selected entry?\n\n[d] Delete   [Esc] Cancel\nEncrypted backups retain previous versions; this is not secure erasure.", "Удалить выбранную запись?\n\n[d] Удалить   [Esc] Отмена\nЗашифрованные бэкапы хранят прежние версии; это не безопасное стирание."),self.tr(" Confirm deletion ", " Подтвердить удаление "),7),
            _=>{}
        }
        self.draw_palette(frame);
        self.draw_language(frame);
    }

    fn draw_full_consent(&self, frame: &mut Frame, text: &str, title: &str) -> bool {
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(100);
        let lines = wrap_plain_text(text, usize::from(width.saturating_sub(2)));
        if width < 40 || lines.len() + 2 > usize::from(area.height.saturating_sub(4)) {
            popup(frame, self.tr("Full paths and warnings do not fit.\nEnlarge the terminal to continue.\n[Esc] Cancel", "Полные пути и предупреждения не помещаются.\nУвеличьте окно для продолжения.\n[Esc] Отмена"), title, 8);
            return false;
        }
        let height = lines.len() as u16 + 2;
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
                .block(panel(title))
                .style(theme::surface().fg(theme::WARNING)),
            rect,
        );
        true
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
            &mut ListState::default().with_selected(Some(self.runtime.folder_selected)),
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
        self.render.folder_dialog_visible.set(true);
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
                &self.runtime.master
            } else {
                &self.runtime.confirmation
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

    fn draw_palette(&self, frame: &mut Frame) {
        let Some(palette) = &self.runtime.palette else {
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
                    self.render.palette_hits.borrow_mut().push((
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
        self.render.palette_visible.set(true);
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

    pub(super) fn draw_sensitive(&self, frame: &mut Frame) {
        let Some(form) = &self.runtime.sensitive else {
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
                let active_path = self.runtime
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
        self.render.sensitive_visible.set(true);
    }

    pub(super) fn draw_vault_delete_confirmation(&self, frame: &mut Frame) {
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(100);
        let active_path = self
            .runtime
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
        self.render.vault_delete_visible.set(true);
    }

    pub(super) fn draw_folder_delete(&self, frame: &mut Frame) {
        let Some((entries, folders)) = self
            .runtime
            .pending_folder_delete
            .and_then(|id| self.runtime.session.as_ref()?.folder_contents(id))
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
        self.render.folder_dialog_visible.set(true);
    }
    pub(super) fn draw_language(&self, frame: &mut Frame) {
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
            self.render.language_hits.borrow_mut().push((
                Rect::new(rows[1].x, rows[1].y + index as u16, rows[1].width, 1),
                index,
            ));
        }
        self.render.language_visible.set(true);
    }

    pub(super) fn draw_explorer(&self, frame: &mut Frame, area: Rect) {
        let rows = self.explorer_rows();
        let selected = rows
            .iter()
            .position(|row| Some(row.node) == self.runtime.explorer_focus);
        let block = panel(self.tr(" Explorer ", " Проводник ")).padding(Padding::new(0, 0, 1, 0));
        let inner = block.inner(area);
        let items: Vec<_> = rows
            .iter()
            .map(|row| {
                let indent = "  ".repeat(row.depth);
                let (icon, color) = match row.node {
                    ExplorerNode::Folder(id) => (
                        if self.runtime.expanded_folders.contains(&id) {
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
            self.render.explorer_hits.borrow_mut().push((
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
}

pub(super) fn invert() -> Style {
    theme::selected()
}
pub(super) fn panel(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_owned())
        .style(theme::surface())
        .border_style(Style::default().fg(theme::BORDER))
        .title_style(Style::default().fg(theme::SECONDARY))
}
pub(super) fn dim_backdrop(frame: &mut Frame) {
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
pub(super) fn wrap_plain_text(text: &str, width: usize) -> Vec<String> {
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
pub(super) fn popup(frame: &mut Frame, text: &str, title: &str, height: u16) {
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
