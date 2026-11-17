//! Owned working set: replacing this aggregate is the lock boundary.
use super::*;
use std::{collections::HashSet, sync::mpsc::Receiver};

pub(super) struct RuntimeState {
    pub master: Input,
    pub confirmation: Input,
    pub session: Option<Session>,
    pub entries: Vec<(EntryId, String)>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub query: Input,
    pub folder: Option<GroupId>,
    pub folder_name: Input,
    pub folder_error: Option<String>,
    pub folder_selected: usize,
    pub expanded_folders: HashSet<GroupId>,
    pub explorer_focus: Option<ExplorerNode>,
    pub pending_folder_delete: Option<GroupId>,
    pub form: Option<Form>,
    pub mode: Mode,
    pub palette: Option<Palette>,
    pub sensitive: Option<SensitiveForm>,
    pub delete_authorization: Option<DeleteAuthorization>,
    pub deleted_backup: Option<PathBuf>,
    pub update_rx: Option<Receiver<updates::CheckResult>>,
    pub update_return: Option<Mode>,
    pub reveal_until: Option<Instant>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            master: Input::default(),
            confirmation: Input::default(),
            session: None,
            entries: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            query: Input::default(),
            folder: None,
            folder_name: Input::default(),
            folder_error: None,
            folder_selected: 0,
            expanded_folders: HashSet::new(),
            explorer_focus: None,
            pending_folder_delete: None,
            form: None,
            mode: Mode::Browse,
            palette: None,
            sensitive: None,
            delete_authorization: None,
            deleted_backup: None,
            update_rx: None,
            update_return: None,
            reveal_until: None,
        }
    }
}

impl Drop for RuntimeState {
    fn drop(&mut self) {
        for (_, title) in &mut self.entries {
            title.zeroize();
        }
        // Input, SensitiveForm, Session and authorization retain their own
        // zeroizing/drop implementations; never copy secrets during teardown.
    }
}

// Keep the incremental extraction source-compatible. Cross-field mutable
// operations borrow `runtime` explicitly rather than through this projection.
impl std::ops::Deref for App {
    type Target = RuntimeState;
    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}
impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
}
