#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! What a web page that hosts a Craft app offers it. Today that is the host's own file storage:
//! a page that serves an app (a cloud drive, a team folder) can set `window.craftStorage`, and the
//! app's File › Open and Save offer it next to this device. The contract is the same for every
//! app, so each app's web shell calls this crate instead of its own copy.
//!
//! ```js
//! window.craftStorage = {
//!   // File › Open: {path, bytes} for a file in the host's storage, "device" for the browser's
//!   // own file picker, or null when the user cancelled.
//!   async chooseOpen(extensions /* ["psd", …] or null */) { … },
//!   // A save or export: a path in the host's storage, "device" for a download, or null.
//!   async chooseSave(suggested /* a file name, or the document's host path */) { … },
//!   // Writes a file the user saved to a host path. The save counts once it resolves; a
//!   // rejection leaves the document unsaved.
//!   async write(path, bytes /* Uint8Array */) { … },
//! };
//! ```
//!
//! Host paths have a scheme of their own, such as `cloud:home/art/photo.psd` ([`is_host_path`]):
//! a document opened from one keeps it, so File › Save writes back there without asking.
//!
//! The rules every app follows (the [`storage`] module carries them out):
//!
//! - Only the answer `"device"` opens the browser's picker or download. A host that rejects, or
//!   answers anything else, means nothing was chosen.
//! - Browsers open a file picker only during a click, so a host should answer `"device"` right
//!   from the user's click in its own dialog.
//! - Saves reach the host one at a time, in the order the user made them ([`storage::SaveQueue`]),
//!   so an earlier save never lands after a later one. So each `write` must settle: a host whose
//!   storage can hang should reject after its own timeout (only the host can tell a slow upload
//!   from a stuck one).
//!
//! What a save means for a document (its path, its saved state, closing it) stays in each app.

mod path;

pub use path::is_host_path;

#[cfg(target_arch = "wasm32")]
pub mod storage;
