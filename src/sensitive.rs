//! Separate masked forms for master-passphrase changes and vault deletion.
use crate::{
    i18n::Language,
    input::{control_shortcut, Input},
};
#[cfg(test)]
use crossterm::event::KeyModifiers;
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensitiveKind {
    ChangeMaster,
    DeleteVault,
}

pub enum FormEvent {
    None,
    Cancel,
    Submit,
}

pub struct SensitiveForm {
    pub kind: SensitiveKind,
    pub fields: [Input; 3],
    pub field: usize,
    pub error: Option<String>,
}

impl SensitiveForm {
    pub fn new(kind: SensitiveKind) -> Self {
        Self {
            kind,
            fields: std::array::from_fn(|_| Input::default()),
            field: 0,
            error: None,
        }
    }
    pub fn labels(&self, language: Language) -> &'static [&'static str] {
        match (self.kind, language) {
            (SensitiveKind::ChangeMaster, Language::English) => &[
                "Current master password",
                "New master password",
                "Confirm new master password",
            ],
            (SensitiveKind::DeleteVault, Language::English) => {
                &["Current master password", "Exact filename (with extension)"]
            }
            (SensitiveKind::ChangeMaster, Language::Russian) => &[
                "Текущий мастер-пароль",
                "Новый мастер-пароль",
                "Повтор нового мастер-пароля",
            ],
            (SensitiveKind::DeleteVault, Language::Russian) => {
                &["Текущий мастер-пароль", "Точное имя файла (с расширением)"]
            }
        }
    }
    pub fn masked(&self, index: usize) -> bool {
        self.kind == SensitiveKind::ChangeMaster || index == 0
    }
    pub fn key(&mut self, key: KeyEvent) -> FormEvent {
        let count = self.labels(Language::English).len();
        match key.code {
            KeyCode::Esc => return FormEvent::Cancel,
            KeyCode::Tab => self.field = (self.field + 1) % count,
            KeyCode::BackTab => self.field = (self.field + count - 1) % count,
            KeyCode::Enter if self.field + 1 == count => return FormEvent::Submit,
            KeyCode::Enter => self.field += 1,
            _ => {
                let fits = match key.code {
                    KeyCode::Char(c) => self.fields[self.field].text().len() + c.len_utf8() <= 1024,
                    _ => true,
                };
                if (fits || control_shortcut(key)) && self.fields[self.field].key(key, false) {
                    self.error = None;
                }
            }
        }
        FormEvent::None
    }
    pub fn paste(&mut self, text: &str) -> bool {
        if self.fields[self.field]
            .text()
            .len()
            .saturating_add(text.len())
            <= 1024
            && self.fields[self.field].insert(text, false)
        {
            self.error = None;
            true
        } else {
            self.error = Some("Paste rejected: unsupported characters or input too large".into());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn altgr_master_text_stays_masked_and_obeys_the_field_byte_limit() {
        let mut form = SensitiveForm::new(SensitiveKind::ChangeMaster);
        let modifiers = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert!(form.paste(&"x".repeat(1023)));
        form.key(KeyEvent::new(KeyCode::Char('@'), modifiers));
        assert_eq!(form.fields[0].text().len(), 1024);
        assert!(form.fields[0].text().ends_with('@'));
        form.key(KeyEvent::new(KeyCode::Char('u'), modifiers));
        assert_eq!(form.fields[0].text().len(), 1024);
        assert!(!form.fields[0].rendered(true, false).contains('@'));
        form.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(form.fields[0].text().is_empty());
    }
    #[test]
    fn all_master_fields_stay_masked_and_never_accept_multiline_paste() {
        let mut form = SensitiveForm::new(SensitiveKind::ChangeMaster);
        for field in 0..3 {
            form.field = field;
            assert!(form.paste("DUMMY_SECRET"));
            assert!(!form.fields[field]
                .rendered(form.masked(field), true)
                .contains("DUMMY_SECRET"));
            assert!(!form.paste("\nINJECT"));
            assert!(!form.paste(&"x".repeat(1025)));
        }
    }
    #[test]
    fn deletion_filename_is_visible_but_master_is_not() {
        let form = SensitiveForm::new(SensitiveKind::DeleteVault);
        assert!(form.masked(0));
        assert!(!form.masked(1));
        assert_eq!(form.labels(Language::English).len(), 2);
    }
}
