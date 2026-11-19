//! Capability checks shared by semantic keyboard and palette dispatch.
use super::*;

// One fixed order preserves the historical ordering in browse, edit, search,
// locked and removed-vault contexts. Disallowed commands are simply skipped.
const PALETTE_ORDER: &[Command] = &[
    Command::NewEntry,
    Command::NewGeneratedEntry,
    Command::SearchEntries,
    Command::EditEntry,
    Command::CopyPassword,
    Command::CopyUsername,
    Command::SaveEntry,
    Command::GeneratePassword,
    Command::RevealPassword,
    Command::DeleteEntry,
    Command::OpenFolder,
    Command::NewFolder,
    Command::DeleteFolder,
    Command::MoveToFolder,
    Command::ChangeMaster,
    Command::DeleteVault,
    Command::ImportCsv,
    Command::ExportCsv,
    Command::InstallUpdate,
    Command::Back,
    Command::Unlock,
    Command::Language,
    Command::CheckUpdates,
    Command::UpdateResult,
    Command::Lock,
    Command::Quit,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Invocation {
    Keyboard,
    Palette,
}

impl App {
    pub(super) fn available_commands(&self) -> Vec<Command> {
        PALETTE_ORDER
            .iter()
            .copied()
            .filter(|command| self.command_allowed(*command, Invocation::Palette))
            .collect()
    }

    pub(super) fn command_allowed(&self, command: Command, invocation: Invocation) -> bool {
        // Emergency session controls remain available behind overlays and in
        // browser mode. They still use `leave`, including dirty-draft consent.
        if invocation == Invocation::Keyboard {
            match command {
                Command::Quit => return true,
                Command::Lock => return self.runtime.session.is_some(),
                _ => {}
            }
        }
        if self.language_choice.is_some()
            || self.browser_origin.is_some()
            || matches!(
                self.runtime.mode,
                Mode::Delete
                    | Mode::Discard(_)
                    | Mode::BrowserConfirm
                    | Mode::VaultAction
                    | Mode::VaultDeleteConfirm
                    | Mode::UpdateConsent
                    | Mode::UpdateResult
                    | Mode::Csv
                    | Mode::UpdateDownload
                    | Mode::UpdateInstall
                    | Mode::NewFolder
                    | Mode::DeleteFolder
                    | Mode::Folders(_)
            )
        {
            return false;
        }
        if self.runtime.mode == Mode::VaultDeleted {
            return command == Command::Quit;
        }
        let Some(session) = &self.runtime.session else {
            return match command {
                Command::Unlock | Command::Language | Command::CheckUpdates | Command::Quit => true,
                Command::UpdateResult => self.update_result.is_some(),
                _ => false,
            };
        };
        let browsing = self.runtime.mode == Mode::Browse;
        let editing = self.runtime.mode == Mode::Edit && self.runtime.form.is_some();
        match command {
            Command::InstallUpdate => {
                browsing
                    && self.runtime.form.is_none()
                    && matches!(self.update_result, Some(UpdateStatus::Available(_)))
            }
            Command::Language | Command::CheckUpdates | Command::Lock | Command::Quit => true,
            Command::UpdateResult => self.update_result.is_some(),
            Command::Unlock => false,
            Command::Back => editing || self.runtime.mode == Mode::Search,
            Command::SaveEntry | Command::GeneratePassword => editing && !session.readonly,
            Command::RevealPassword => editing || (browsing && self.selected_id().is_some()),
            Command::CopyPassword | Command::CopyUsername => {
                browsing && self.selected_id().is_some()
            }
            Command::SearchEntries | Command::OpenFolder => browsing,
            Command::NewEntry
            | Command::NewGeneratedEntry
            | Command::NewFolder
            | Command::ChangeMaster
            | Command::DeleteVault => browsing && !session.readonly,
            Command::ImportCsv | Command::ExportCsv => {
                browsing && !session.readonly && self.runtime.form.is_none()
            }
            Command::EditEntry | Command::DeleteEntry | Command::MoveToFolder => {
                browsing && !session.readonly && self.selected_id().is_some()
            }
            Command::DeleteFolder => {
                browsing && !session.readonly && self.delete_folder_target().is_some()
            }
        }
    }
}
