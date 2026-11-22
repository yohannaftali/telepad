//! Telepad application layer, Model–View–Update style.
//!
//! - [`State`] = [`Model`] (changed only by [`update`]) + [`Drafts`] (form
//!   text, filter and toggles that views bind to directly).
//! - Views read the model and push [`Msg`]s.
//! - [`update`] applies one message and returns [`Effect`]s; it never does
//!   I/O and gets the time as a parameter, so it is fully unit-testable.
//! - The GUI runtime executes effects and feeds their results back as messages.
//!
//! This crate does not depend on any GUI toolkit.

pub mod db;
pub mod editor;
pub mod files;
pub mod hostsync;
mod msg;
mod state;
mod table;
mod update;
pub mod workspace;

pub use msg::{Effect, Msg};
pub use state::{
    AkHost, Drafts, Editor, HostState, ImportDialog, KeyHolders, KeyRun, KeysDraft, KeysTab,
    LeaveAsk, MESSAGE_TTL, Model, OverviewPage, PasteImport, PickerDraft, RunInfoView, State,
    UpdatePhase, UpdateState, key_groups, key_holders, key_selectable, matches_filter,
    matches_query,
};
pub use table::{
    ClusterKind, ClusterRow, Flag, FlagKind, HostGroup, Severity, Sort, SortKey, StackRow,
    can_reorder, cluster_rows, cluster_shown_by, docker_stacks, drop_index, fullest_disk,
    host_flags, host_list_groups, host_urgency, hottest, maintenance_flags, overview_rows, urgency,
    worst_usage,
};
pub use update::{background_transfers, leave_losses, update};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_db;
#[cfg(test)]
mod tests_editor;
#[cfg(test)]
mod tests_files;
#[cfg(test)]
mod tests_history;
#[cfg(test)]
mod tests_hostsync;
#[cfg(test)]
mod tests_keys;
#[cfg(test)]
mod tests_leave;
#[cfg(test)]
mod tests_workspace;
