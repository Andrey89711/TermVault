use anyhow::{bail, Result};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

trait Backend {
    fn write(&mut self, text: &str) -> Result<()>;
    fn read(&mut self) -> Result<Zeroizing<String>>;
    fn clear(&mut self) -> Result<()>;
}
struct Native(arboard::Clipboard);
impl Backend for Native {
    fn write(&mut self, text: &str) -> Result<()> {
        #[cfg(windows)]
        {
            use arboard::SetExtWindows;
            self.0
                .set()
                .exclude_from_history()
                .exclude_from_cloud()
                .text(text)
                .map_err(|_| anyhow::anyhow!("Cannot write to system clipboard"))?;
        }
        #[cfg(not(windows))]
        self.0
            .set_text(text)
            .map_err(|_| anyhow::anyhow!("Cannot write to system clipboard"))?;
        Ok(())
    }
    fn read(&mut self) -> Result<Zeroizing<String>> {
        self.0
            .get_text()
            .map(Zeroizing::new)
            .map_err(|_| anyhow::anyhow!("Cannot read system clipboard"))
    }
    fn clear(&mut self) -> Result<()> {
        self.0
            .clear()
            .map_err(|_| anyhow::anyhow!("Cannot clear system clipboard"))
    }
}

pub struct Clipboard {
    backend: Option<Box<dyn Backend>>,
    owned: Option<(Zeroizing<String>, Instant)>,
}
impl Clipboard {
    pub fn new() -> Self {
        Self {
            backend: None,
            owned: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn memory_for_test() -> Self {
        struct Memory(Zeroizing<String>);
        impl Backend for Memory {
            fn write(&mut self, text: &str) -> Result<()> {
                self.0 = Zeroizing::new(text.to_owned());
                Ok(())
            }
            fn read(&mut self) -> Result<Zeroizing<String>> {
                Ok(self.0.clone())
            }
            fn clear(&mut self) -> Result<()> {
                self.0 = Zeroizing::new(String::new());
                Ok(())
            }
        }
        Self {
            backend: Some(Box::new(Memory(Zeroizing::new(String::new())))),
            owned: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn owns_text_for_test(&self) -> bool {
        self.owned.is_some()
    }
    pub fn copy(&mut self, text: &str, now: Instant) -> Result<()> {
        if self.backend.is_none() {
            self.backend = arboard::Clipboard::new()
                .ok()
                .map(|backend| Box::new(Native(backend)) as Box<dyn Backend>);
        }
        let Some(clipboard) = &mut self.backend else {
            bail!("System clipboard unavailable");
        };
        clipboard.write(text)?;
        self.owned = Some((
            Zeroizing::new(text.to_owned()),
            now + Duration::from_secs(20),
        ));
        Ok(())
    }
    pub fn tick(&mut self, now: Instant) {
        if self
            .owned
            .as_ref()
            .is_some_and(|(_, deadline)| now >= *deadline)
        {
            self.clear();
        }
    }
    pub fn clear(&mut self) {
        let Some((expected, _)) = self.owned.take() else {
            return;
        };
        if let Some(clipboard) = &mut self.backend {
            if let Ok(current) = clipboard.read() {
                // Never clear something the user copied after our password.
                if *current == *expected {
                    let _ = clipboard.clear();
                }
            }
        }
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    struct Fake {
        text: Rc<RefCell<String>>,
        fail: bool,
    }
    impl Backend for Fake {
        fn write(&mut self, text: &str) -> Result<()> {
            if self.fail {
                bail!("write failed");
            }
            *self.text.borrow_mut() = text.into();
            Ok(())
        }
        fn read(&mut self) -> Result<Zeroizing<String>> {
            Ok(Zeroizing::new(self.text.borrow().clone()))
        }
        fn clear(&mut self) -> Result<()> {
            self.text.borrow_mut().clear();
            Ok(())
        }
    }
    #[test]
    fn expiry_clears_own_text_but_not_later_user_copy() {
        let text = Rc::new(RefCell::new(String::new()));
        let mut clip = Clipboard {
            backend: Some(Box::new(Fake {
                text: text.clone(),
                fail: false,
            })),
            owned: None,
        };
        let now = Instant::now();
        clip.copy("dummy", now).unwrap();
        clip.tick(now + Duration::from_secs(19));
        assert_eq!(&*text.borrow(), "dummy");
        clip.tick(now + Duration::from_secs(21));
        assert!(text.borrow().is_empty());
        clip.copy("dummy", now).unwrap();
        *text.borrow_mut() = "user's later copy".into();
        clip.clear();
        assert_eq!(&*text.borrow(), "user's later copy");
    }
    #[test]
    fn failed_copy_never_claims_ownership_or_clears_user_data() {
        let text = Rc::new(RefCell::new("existing".into()));
        let mut clip = Clipboard {
            backend: Some(Box::new(Fake {
                text: text.clone(),
                fail: true,
            })),
            owned: None,
        };
        assert!(clip.copy("dummy", Instant::now()).is_err());
        assert!(clip.owned.is_none());
        clip.clear();
        assert_eq!(&*text.borrow(), "existing");
    }
}
