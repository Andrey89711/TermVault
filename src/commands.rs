//! A fixed action palette, not a shell or a password/argument input.
use crate::{
    i18n::Language,
    input::{control_shortcut, Input},
};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Language,
    Unlock,
    NewEntry,
    NewFolder,
    OpenFolder,
    MoveToFolder,
    MoveMarkedEntries,
    DeleteMarkedEntries,
    DeleteFolder,
    NewGeneratedEntry,
    EditEntry,
    SearchEntries,
    CopyPassword,
    CopyUsername,
    RevealPassword,
    DeleteEntry,
    SaveEntry,
    GeneratePassword,
    Back,
    ImportCsv,
    ExportCsv,
    InstallUpdate,
    CheckUpdates,
    UpdateResult,
    ChangeMaster,
    DeleteVault,
    Lock,
    Quit,
}

impl Command {
    pub fn group(self, language: Language) -> &'static str {
        match self {
            Self::Language => language.text("Settings", "Настройки"),
            Self::NewFolder | Self::OpenFolder | Self::MoveToFolder | Self::DeleteFolder => {
                language.text("Folders", "Папки")
            }
            Self::ChangeMaster | Self::DeleteVault | Self::ImportCsv | Self::ExportCsv => {
                language.text("Vault", "Хранилище")
            }
            Self::CheckUpdates | Self::UpdateResult | Self::InstallUpdate => {
                language.text("Updates", "Обновления")
            }
            Self::Unlock | Self::Lock | Self::Quit => language.text("Session", "Сеанс"),
            _ => language.text("Entries", "Записи"),
        }
    }

    pub fn label(self, language: Language) -> &'static str {
        match self {
            Self::Language => language.text("Language", "Язык"),
            Self::Unlock => language.text("Enter master password", "Ввод мастер-пароля"),
            Self::NewEntry => language.text("Create entry", "Создать запись"),
            Self::NewFolder => language.text("Create folder", "Создать папку"),
            Self::OpenFolder => {
                language.text("Open folder / all entries", "Открыть папку / все записи")
            }
            Self::MoveToFolder => {
                language.text("Move entry to folder", "Переместить запись в папку")
            }
            Self::MoveMarkedEntries => language.text(
                "Move marked entries to folder",
                "Переместить отмеченные записи в папку",
            ),
            Self::DeleteMarkedEntries => {
                language.text("Delete marked entries", "Удалить отмеченные записи")
            }
            Self::DeleteFolder => {
                language.text("Delete selected folder", "Удалить выбранную папку")
            }
            Self::NewGeneratedEntry => language.text(
                "Create entry with random password",
                "Создать запись со случайным паролем",
            ),
            Self::EditEntry => {
                language.text("Edit selected entry", "Редактировать выбранную запись")
            }
            Self::SearchEntries => language.text("Find entry", "Найти запись"),
            Self::CopyPassword => language.text("Copy password", "Скопировать пароль"),
            Self::CopyUsername => language.text("Copy username", "Скопировать логин"),
            Self::RevealPassword => {
                language.text("Show / hide password", "Показать / скрыть пароль")
            }
            Self::DeleteEntry => language.text("Delete selected entry", "Удалить выбранную запись"),
            Self::SaveEntry => language.text("Save entry", "Сохранить запись"),
            Self::GeneratePassword => language.text("Generate password", "Сгенерировать пароль"),
            Self::Back => language.text("Return to list", "Вернуться к списку"),
            Self::ImportCsv => language.text("Import CSV (append)", "Импорт CSV (добавить)"),
            Self::ExportCsv => {
                language.text("Export CSV (plaintext)", "Экспорт CSV (открытый текст)")
            }
            Self::InstallUpdate => language.text(
                "Download and install update",
                "Скачать и установить обновление",
            ),
            Self::CheckUpdates => language.text("Check for updates", "Проверить обновления"),
            Self::UpdateResult => {
                language.text("Update check result", "Результат проверки обновлений")
            }
            Self::ChangeMaster => language.text("Change master password", "Сменить мастер-пароль"),
            Self::DeleteVault => language.text(
                "Delete vault (keep backup)",
                "Удалить хранилище (с бэкапом)",
            ),
            Self::Lock => language.text("Lock vault", "Заблокировать хранилище"),
            Self::Quit => language.text("Quit", "Выйти"),
        }
    }
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::Language => "",
            Self::Unlock => "Esc",
            Self::NewEntry => "n",
            Self::NewFolder => "Shift+F",
            Self::OpenFolder => "f",
            Self::MoveToFolder => "m",
            Self::DeleteFolder => "",
            Self::NewGeneratedEntry => "g",
            Self::EditEntry => "Enter",
            Self::SearchEntries => "/",
            Self::CopyPassword => "c",
            Self::CopyUsername => "u",
            Self::RevealPassword => "Ctrl+O",
            Self::DeleteEntry => "d",
            Self::SaveEntry => "Ctrl+S",
            Self::GeneratePassword => "Ctrl+G",
            Self::Back => "Esc",
            Self::CheckUpdates
            | Self::UpdateResult
            | Self::ChangeMaster
            | Self::DeleteVault
            | Self::ImportCsv
            | Self::ExportCsv
            | Self::MoveMarkedEntries
            | Self::DeleteMarkedEntries
            | Self::InstallUpdate => "",
            Self::Lock => "Ctrl+L",
            Self::Quit => "Ctrl+Q",
        }
    }
    fn aliases(self) -> &'static str {
        match self {
            Self::Language => "language locale english russian язык английский русский",
            Self::Unlock => "unlock разблокировать мастер пароль",
            Self::NewEntry => "new add create новая добавить",
            Self::NewFolder => "new create folder directory создать папку каталог",
            Self::OpenFolder => "open folders directory all открыть папки все записи",
            Self::MoveToFolder => "move entry folder переместить запись папка",
            Self::MoveMarkedEntries => "bulk batch move marked selected entries групповое перемещение отмеченных записей",
            Self::DeleteMarkedEntries => "bulk batch delete remove marked selected entries групповое удаление отмеченных записей",
            Self::DeleteFolder => {
                "delete remove folder directory удалить папку удаление папки каталог"
            }
            Self::NewGeneratedEntry => "new generated random создать случайный",
            Self::EditEntry => "edit change редактирование изменить",
            Self::SearchEntries => "search find filter поиск",
            Self::CopyPassword => "copy password буфер",
            Self::CopyUsername => "copy username login имя пользователя",
            Self::RevealPassword => "reveal show hide password показать скрыть пароль",
            Self::DeleteEntry => "delete remove запись удалить",
            Self::SaveEntry => "save сохранить",
            Self::GeneratePassword => "generate random password генератор случайный",
            Self::Back => "back cancel return назад отмена",
            Self::ImportCsv => "import csv append импорт добавить",
            Self::ExportCsv => "export csv plaintext экспорт",
            Self::InstallUpdate => "download install update скачать установить обновление",
            Self::CheckUpdates => "check updates version обновить проверка версия",
            Self::UpdateResult => "update result release версия результат релиз",
            Self::ChangeMaster => "change master password смена мастер пароль",
            Self::DeleteVault => "delete vault database remove удалить базу хранилище",
            Self::Lock => "lock блокировка заблокировать",
            Self::Quit => "quit exit выход выйти",
        }
    }
}

#[derive(Default)]
pub struct Palette {
    pub query: Input,
    pub selected: usize,
}

impl Palette {
    pub fn filtered(&self, available: &[Command]) -> Vec<Command> {
        let query = self.query.text().to_lowercase();
        available
            .iter()
            .copied()
            .filter(|command| {
                let searchable = format!(
                    "{} {} {}",
                    command.label(Language::English).to_lowercase(),
                    command.label(Language::Russian).to_lowercase(),
                    command.aliases()
                );
                query
                    .split_whitespace()
                    .all(|word| searchable.contains(word))
            })
            .collect()
    }
    pub fn key(&mut self, key: KeyEvent, count: usize) {
        match key.code {
            KeyCode::Up | KeyCode::BackTab => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(count.saturating_sub(1))
            }
            _ => {
                let fits = match key.code {
                    KeyCode::Char(c) => self.query.text().len() + c.len_utf8() <= 256,
                    _ => true,
                };
                if (fits || control_shortcut(key)) && self.query.key(key, false) {
                    self.selected = 0;
                }
            }
        }
    }
    pub fn paste(&mut self, text: &str) {
        if self.query.text().len().saturating_add(text.len()) <= 256
            && self.query.insert(text, false)
        {
            self.selected = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    #[test]
    fn filters_in_russian_and_english_without_parsing_shell_commands() {
        let mut palette = Palette::default();
        let commands = [Command::NewEntry, Command::CopyPassword, Command::Quit];
        palette.paste("COPY password");
        assert_eq!(palette.filtered(&commands), [Command::CopyPassword]);
        palette.query.clear();
        palette.paste("Создать");
        assert_eq!(palette.filtered(&commands), [Command::NewEntry]);
        palette.query.clear();
        palette.paste("powershell.exe -Command remove-item");
        assert!(palette.filtered(&commands).is_empty());
    }
    #[test]
    fn language_command_has_localized_labels_and_bilingual_search() {
        assert_eq!(Command::Language.label(Language::English), "Language");
        assert_eq!(Command::Language.label(Language::Russian), "Язык");
        for query in ["language", "язык", "English", "Русский"] {
            let mut palette = Palette::default();
            palette.paste(query);
            assert_eq!(
                palette.filtered(&[Command::Quit, Command::Language]),
                [Command::Language]
            );
        }
    }
    #[test]
    fn bounds_selection_and_rejects_multiline_or_oversized_paste() {
        let mut palette = Palette::default();
        palette.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 0);
        assert_eq!(palette.selected, 0);
        palette.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 2);
        palette.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 2);
        assert_eq!(palette.selected, 1);
        palette.paste("line\nbreak");
        palette.paste(&"a".repeat(257));
        assert!(palette.query.text().is_empty());
        palette.paste("save");
        assert_eq!(palette.selected, 0);
        palette.query.clear();
        palette.paste(&"a".repeat(256));
        palette.key(KeyEvent::new(KeyCode::Char('я'), KeyModifiers::NONE), 0);
        assert_eq!(palette.query.text().len(), 256);
        palette.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), 0);
        assert!(palette.query.text().is_empty());
    }
    #[test]
    fn altgr_text_cannot_bypass_the_palette_byte_limit_or_clear_its_query() {
        let mut palette = Palette::default();
        palette.paste(&"a".repeat(255));
        let modifiers = KeyModifiers::CONTROL | KeyModifiers::ALT;
        palette.key(KeyEvent::new(KeyCode::Char('@'), modifiers), 0);
        assert_eq!(palette.query.text().len(), 256);
        palette.key(KeyEvent::new(KeyCode::Char('u'), modifiers), 0);
        assert_eq!(palette.query.text().len(), 256);
        palette.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), 0);
        assert!(palette.query.text().is_empty());
    }
}
