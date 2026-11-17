#![cfg(any(windows, unix))]
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

struct Pty {
    child: Box<dyn Child + Send + Sync>,
    writer: Option<Box<dyn Write + Send>>,
    master: Option<Box<dyn MasterPty + Send>>,
    reader: Option<thread::JoinHandle<()>>,
    rx: Receiver<Vec<u8>>,
    output: Vec<u8>,
    answered: usize,
    screen: vt100::Parser,
}
impl Pty {
    fn start(path: &Path, create: bool) -> Self {
        assert!(path.is_absolute());
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 34,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tvault"));
        command.arg(path);
        // ConPTY can ignore CommandBuilder.env overrides. The explicit CLI path
        // is required: an old binary rejects the flag instead of reading real config.
        command.arg("--settings-file");
        command.arg(
            path.parent()
                .unwrap()
                .join("isolated-settings/TermVault/settings.json"),
        );
        if create {
            command.arg("--create");
        }
        command.env("TERM", "xterm-256color");
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let (tx, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut buffer = [0; 4096];
            let mut total = 0;
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                total += count;
                if total > 2 * 1024 * 1024 || tx.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            writer: Some(writer),
            master: Some(pair.master),
            reader: Some(thread),
            rx,
            output: Vec::new(),
            answered: 0,
            screen: vt100::Parser::new(34, 120, 0),
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        let writer = self.writer.as_mut().unwrap();
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    }
    fn receive(&mut self) {
        if let Ok(bytes) = self.rx.recv_timeout(Duration::from_millis(50)) {
            assert!(self.output.len() + bytes.len() < 2 * 1024 * 1024);
            self.screen.process(&bytes);
            self.output.extend(bytes);
            let queries = self.output.windows(4).filter(|w| *w == b"\x1b[6n").count();
            while self.answered < queries {
                self.send(b"\x1b[1;1R");
                self.answered += 1;
            }
        }
    }
    fn expect(&mut self, _from: usize, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let output = self.screen.screen().contents().replace(' ', "");
            if output.contains(text) {
                return;
            }
            if text == "Entrysaved" {
                for marker in [
                    "Titleisrequired",
                    "KDBXencryptionfailed",
                    "Encryptedsaveverificationfailed",
                    "Vaultchangedondisk",
                    "Afieldexceeds",
                    "Cannotcreateencryptedbackup",
                ] {
                    assert!(
                        !output.contains(marker),
                        "Public UI error during save: {marker}"
                    );
                }
            }
            assert!(Instant::now()<deadline,"Timed out waiting for UI marker {text:?} (captured terminal output omitted to avoid leaking secrets)");
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Vault process exited unexpectedly"
            );
            self.receive();
        }
    }
    fn paste(&mut self, text: &str) {
        self.send(format!("\x1b[200~{text}\x1b[201~").as_bytes());
    }
    fn action(&mut self, bytes: &[u8], marker: &str) {
        let start = self.output.len();
        self.send(bytes);
        self.expect(start, marker);
    }
    fn exit(&mut self) {
        self.send(b"\x11");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline);
            self.receive();
        }
    }
    fn assert_absent(&self, secret: &str) {
        assert!(
            !self
                .output
                .windows(secret.len())
                .any(|bytes| bytes == secret.as_bytes()),
            "A dummy secret was emitted by the terminal (value omitted)"
        );
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                let _ = self.child.wait();
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(self.writer.take());
        drop(self.master.take());
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if reader.is_finished() {
                let _ = reader.join();
            } else if !thread::panicking() {
                panic!("PTY reader did not terminate");
            }
        }
    }
}
#[test]
fn real_terminal_create_edit_mask_save_lock_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.kdbx");
    let master = "dummy master phrase 123";
    let secret = "DUMMY_PASSWORD_MUST_NOT_ECHO";
    {
        let mut term = Pty::start(&path, true);
        term.expect(0, "Masterpassword");
        term.paste(master);
        term.send(b"\r");
        term.paste(master);
        term.action(b"\r", "Vaultunlocked");
        term.action(b"\x10", "Commands"); // Ctrl+P opens the palette, not password reveal.
        term.paste("new");
        term.action(b"\r", "Title");
        term.paste("Demo account");
        term.send(b"\t");
        term.paste("demo@example.test");
        term.send(b"\t");
        term.paste(secret);
        term.action(b"\x13", "Entrysaved");
        term.assert_absent(master);
        term.assert_absent(secret);
        term.action(b"\x0c", "Vaultlocked");
        term.assert_absent(secret);
        term.exit();
    }
    assert!(path.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    for file in fs::read_dir(directory.path()).unwrap() {
        let bytes = fs::read(file.unwrap().path()).unwrap();
        assert!(!bytes
            .windows(secret.len())
            .any(|value| value == secret.as_bytes()));
    }
    {
        let mut term = Pty::start(&path, false);
        term.expect(0, "Masterpassword");
        term.paste("wrong master password");
        term.action(b"\r", "Cannotunlock");
        term.paste(master);
        term.action(b"\r", "Vaultunlocked");
        term.expect(0, "Demoaccount");
        term.assert_absent(master);
        term.assert_absent(secret);
        term.exit();
    }
}

#[test]
fn real_terminal_folder_tree_and_delete_command_are_distinct_from_creation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("folders.kdbx");
    let mut term = Pty::start(&path, true);
    term.expect(0, "Masterpassword");
    term.paste("dummy master phrase 123");
    term.send(b"\r");
    term.paste("dummy master phrase 123");
    term.action(b"\r", "Vaultunlocked");
    term.action(b"F", "Newfolder");
    term.paste("Work");
    term.action(b"\r", "Foldercreated");
    term.expect(0, "Explorer");
    term.action(b"n", "Title");
    term.paste("Nested account");
    term.action(b"\x13", "Entrysaved");
    term.action(b"\x1b[D", "Folder"); // Left selects the parent folder.
    term.action(b"\x10", "Commands");
    term.paste("удаление папки");
    term.action(b"\r", "Deletefolder?");
    term.expect(0, "Entriestodelete:1");
    // Wait for the browse footer, not the Explorer title (which is visible
    // behind the dialog too). Unix terminals may otherwise decode Esc followed
    // immediately by Ctrl+Q as one Alt+Ctrl+Q event instead of cancel then quit.
    term.action(b"\x1b", "Enteropen/edit");
    term.exit();
}

#[test]
fn real_terminal_language_choice_preserves_a_draft_and_persists_in_isolated_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("localized.kdbx");
    let master = "dummy localized master 123";
    let secret = "DUMMY_LOCALIZED_PASSWORD_NO_ECHO";
    {
        let mut term = Pty::start(&path, true);
        term.expect(0, "Masterpassword");
        term.paste(master);
        term.send(b"\r");
        term.paste(master);
        term.action(b"\r", "Vaultunlocked");
        term.action(b"n", "Title");
        term.paste("Language draft unchanged");
        term.send(b"\t\t");
        term.paste(secret);
        term.action(b"\x10", "Commands");
        term.paste("language");
        term.action(b"\r", "Русский");
        term.action(b"\x1b[B\r", "Название");
        term.expect(0, "Languagedraftunchanged");
        term.action(b"\x1b", "Несохранённаязапись");
        term.action(b"c", "Название");
        term.action(b"\x13", "Записьсохранена");
        term.assert_absent(master);
        term.assert_absent(secret);
        term.exit();
    }
    let config = directory
        .path()
        .join("isolated-settings/TermVault/settings.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    assert_eq!(value["language"], "ru");
    {
        let mut term = Pty::start(&path, false);
        term.expect(0, "Мастер-пароль");
        term.paste("wrong dummy master");
        term.action(b"\r", "Неудалосьразблокировать");
        term.paste(master);
        term.action(b"\r", "Хранилищеразблокировано");
        term.expect(0, "Languagedraftunchanged");
        term.action(b"\x10", "Команды");
        term.paste("language");
        term.action(b"\r", "Русский");
        term.action(b"\x1b[A\r", "Username:");
        term.assert_absent(master);
        term.assert_absent(secret);
        term.exit();
    }
}
