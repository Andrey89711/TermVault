use anyhow::{bail, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use zeroize::Zeroizing;

pub const MAX_INPUT: usize = 8192;

#[derive(Default)]
pub struct Input {
    text: Zeroizing<String>,
    cursor: usize,
}

impl Input {
    pub fn from(text: &str) -> Self {
        Self {
            text: Zeroizing::new(text.to_owned()),
            cursor: text.len(),
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn clear(&mut self) {
        self.text = Zeroizing::new(String::new());
        self.cursor = 0;
    }
    pub fn insert(&mut self, text: &str, multiline: bool) -> bool {
        if self.text.len().saturating_add(text.len()) > MAX_INPUT || !valid(text, multiline) {
            return false;
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        true
    }
    pub fn key(&mut self, key: KeyEvent, multiline: bool) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if matches!(key.code, KeyCode::Char('u' | 'U')) {
                self.clear();
                return true;
            }
            return false;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return false;
        }
        match key.code {
            KeyCode::Left => {
                self.cursor = self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            KeyCode::Right => {
                self.cursor += self.text[self.cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0)
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
                return true;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                self.text.remove(self.cursor);
                return true;
            }
            KeyCode::Char(c) if !c.is_control() => return self.insert(&c.to_string(), multiline),
            _ => {}
        }
        false
    }
    pub fn rendered(&self, masked: bool, cursor: bool) -> String {
        if masked {
            return format!(
                "{}{}",
                "•".repeat(self.text.chars().count()),
                if cursor { "▏" } else { "" }
            );
        }
        if !cursor {
            return display(&self.text);
        }
        format!(
            "{}▏{}",
            display(&self.text[..self.cursor]),
            display(&self.text[self.cursor..])
        )
    }
}

pub fn valid(text: &str, multiline: bool) -> bool {
    text.chars().all(|c| (!c.is_control() || (multiline && c == '\n')) && !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{206f}'))
}

pub fn display(text: &str) -> String {
    text.chars().filter(|c| (!c.is_control() || *c == '\n') && !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{206f}')).collect()
}

pub fn generate(length: usize) -> Result<Zeroizing<String>> {
    if !(16..=128).contains(&length) {
        bail!("Password length must be 16–128");
    }
    const ALPHABET: &[u8] =
        b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*()-_=+[]{}:,.?";
    for _ in 0..128 {
        let mut output = Zeroizing::new(String::with_capacity(length));
        while output.len() < length {
            let mut byte = [0];
            getrandom::fill(&mut byte)
                .map_err(|_| anyhow::anyhow!("Operating-system randomness unavailable"))?;
            let ceiling = 256 - 256 % ALPHABET.len();
            if usize::from(byte[0]) < ceiling {
                output.push(ALPHABET[usize::from(byte[0]) % ALPHABET.len()] as char);
            }
        }
        if output.bytes().any(|c| c.is_ascii_uppercase())
            && output.bytes().any(|c| c.is_ascii_lowercase())
            && output.bytes().any(|c| c.is_ascii_digit())
            && output.bytes().any(|c| c.is_ascii_punctuation())
        {
            return Ok(output);
        }
    }
    bail!("Password generation failed; try again")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_editing_and_masks() {
        let mut input = Input::from("Привет");
        input.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), false);
        input.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), false);
        assert_eq!(input.text(), "Привт");
        assert!(!input.rendered(true, true).contains("Прив"));
        assert!(!input.insert("\x1b[31m", false));
        assert!(!input.insert("\n", false));
    }
    #[test]
    fn generated_values_are_bounded_and_have_all_classes() {
        let a = generate(24).unwrap();
        let b = generate(24).unwrap();
        assert_eq!(a.len(), 24);
        assert_ne!(a, b);
        assert!(a.bytes().any(|c| c.is_ascii_uppercase()));
        assert!(a.bytes().any(|c| c.is_ascii_digit()));
        assert!(generate(1).is_err());
    }
}
