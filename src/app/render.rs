//! Presentation feedback belongs here, not in the sensitive working set.
//! A frame must reset its feedback before drawing, including on early returns.
use super::*;
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
    }
}

impl App {
    pub(super) fn draw_sensitive(&self, frame: &mut Frame) {
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
        self.render.sensitive_visible.set(true);
    }

    pub(super) fn draw_vault_delete_confirmation(&self, frame: &mut Frame) {
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
        self.render.vault_delete_visible.set(true);
    }

    pub(super) fn draw_folder_delete(&self, frame: &mut Frame) {
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
