//! Files being written into Upload, held in the staging area until their upload (SPEC.md, "Uploads").
//!
//! Each staged file is a private file in the account's `staging/` directory, and every `open` or `create` of it gets
//! a handle of its own, which remembers whether it may write and in which round of writing it was opened.
//! - **Uploads start only from a writer's close** (FUSE `flush` on a writable handle that wrote): a reader closing,
//!   such as a thumbnailer, never uploads a half-written file.
//! - **Every close waits for the same result.** A close during an upload waits for it; a close after a failure gets
//!   the failure. So no close reports success while the server does not have the file.
//! - **After an upload the name stays as a receipt** for a few seconds, keeping its bytes, for the copy tools that
//!   set the file's times after closing it (GIO, KIO, rsync). It may be deleted before then, which changes nothing in
//!   Immich; it is not renamed.
//! - **A handle opened before an upload cannot write after it:** its writes would start a new file with a hole.
//!   A fresh open of the receipt's name is a new round, and a new upload.
//! - **What did not upload is never thrown away.** A file whose upload failed, and what a dead daemon left behind,
//!   go to the not-uploaded folder under their own names: Dolphin's move (KIO) deletes its source although the close
//!   failed. Only what the user dropped is deleted: a cancelled copy that ends short, an empty file, a file removed or
//!   replaced in Upload, an uploaded file's receipt.

use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use immich_mount_core::tree::NodeKey;
use tokio::sync::watch;

/// How long an uploaded file stays in Upload after its upload, as a receipt: copy tools set its times right after
/// closing it (GIO, KIO, rsync), and those must find it. Then it goes: Immich has it, in its date folder.
pub const RECEIPT_LIFETIME: Duration = Duration::from_secs(10);

/// An upload's outcome as the closing program sees it: success, or an error number.
pub type UploadResult = Result<(), i32>;

/// Where a staged file is in its life.
#[derive(Debug, Clone)]
pub enum StagedState {
    /// Being written.
    Open,
    /// Its upload is running; the receiver yields its result.
    Uploading(watch::Receiver<Option<UploadResult>>),
    /// Uploaded; shown until the moment given.
    Receipt(Instant),
    /// The upload failed with this error. Hidden from listings, and kept in the not-uploaded folder once the last
    /// handle is closed.
    Failed(i32),
}

/// One staged file, as the filesystem shows it.
#[derive(Debug, Clone)]
pub struct StagedFile {
    pub folder: NodeKey,
    pub name: String,
    pub size: u64,
    pub modified: SystemTime,
    pub state: StagedState,
    /// Written since its last upload.
    pub dirty: bool,
    /// Its name is a temporary one (GIO's or rsync's): it uploads only once renamed to its real name.
    pub awaiting_rename: bool,
    /// Counts completed uploads; a handle opened in an earlier round may no longer write.
    round: u64,
    open_handles: usize,
    path: PathBuf,
    /// Cut short by a cancelled copy: deleted rather than kept when it goes.
    cut_short: bool,
}

impl StagedFile {
    pub fn is_receipt(&self) -> bool {
        matches!(self.state, StagedState::Receipt(_))
    }

    pub fn has_open_writer(&self) -> bool {
        self.open_handles > 0 && matches!(self.state, StagedState::Open)
    }
}

/// An open handle on a staged file.
#[derive(Debug, Clone, Copy)]
struct Handle {
    file: u64,
    writable: bool,
    round: u64,
}

/// What a writer's close must do.
pub enum CloseAction {
    /// Nothing to upload: report success.
    Done,
    /// Report this error (an earlier upload of the file failed).
    Fail(i32),
    /// Another close started the upload: wait for its result.
    Wait(watch::Receiver<Option<UploadResult>>),
    /// Upload it, then report the result through the sender with [`Staging::finish_upload`].
    Upload(PendingUpload),
}

/// What an upload needs, handed out by [`Staging::close`].
pub struct PendingUpload {
    pub file: u64,
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
    sender: watch::Sender<Option<UploadResult>>,
}

/// Wait for an upload's result.
pub async fn wait_for(mut receiver: watch::Receiver<Option<UploadResult>>) -> UploadResult {
    loop {
        if let Some(result) = *receiver.borrow_and_update() {
            return result;
        }
        if receiver.changed().await.is_err() {
            return Err(libc::EIO);
        }
    }
}

#[derive(Default)]
struct Inner {
    files: HashMap<u64, StagedFile>,
    handles: HashMap<u64, Handle>,
    next_file: u64,
    next_handle: u64,
}

/// The staging area of one account.
pub struct Staging {
    directory: PathBuf,
    /// Where files that were not uploaded are kept (`~/.local/share/immich-mount/not-uploaded`).
    kept: PathBuf,
    inner: Mutex<Inner>,
}

/// The file beside a staged file that holds its name, so what a dead daemon left behind is kept under it.
fn name_file(staged: &Path) -> PathBuf {
    staged.with_extension("name")
}

/// Move `staged` into `kept` as `name`, numbered (`IMG_1 (2).JPG`) if the name is taken there. Never loses the bytes:
/// when the move fails they stay where they are, and the next start tries again.
fn keep(kept: &Path, staged: &Path, name: &str) {
    let name = Path::new(name).file_name().map_or_else(|| "not-uploaded".to_owned(), |name| name.to_string_lossy().into_owned());
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_owned(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    if let Err(error) = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(kept) {
        log::error!("could not keep {name} that was not uploaded: {}: {error}", kept.display());
        return;
    }
    let Some(target) = (1..10_000)
        .map(|counter| if counter == 1 { kept.join(&name) } else { kept.join(format!("{stem} ({counter}){extension}")) })
        .find(|candidate| !candidate.exists())
    else {
        return;
    };
    // A rename where both lie on one filesystem, else a copy.
    let moved = std::fs::rename(staged, &target).or_else(|_| std::fs::copy(staged, &target).and_then(|_| std::fs::remove_file(staged)));
    match moved {
        Ok(()) => log::warn!("{name} was not uploaded; it is kept as {}", target.display()),
        Err(error) => log::error!("could not keep {name} that was not uploaded, in {}: {error}", kept.display()),
    }
}

/// Why a staging operation was refused, as an error number for FUSE.
pub type Refusal = i32;

impl Staging {
    /// The staging area in `directory`, keeping what is not uploaded in `kept`. Files a dead daemon left behind are
    /// not uploaded, since their writer's close never succeeded, but they are kept, not deleted: Dolphin's move deletes
    /// its source even then.
    pub fn new(directory: &Path, kept: &Path) -> std::io::Result<Staging> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(directory)?;
        for leftover in std::fs::read_dir(directory)?.flatten() {
            let path = leftover.path();
            if path.extension().is_some_and(|extension| extension == "staged") {
                let fallback = format!("recovered-{}", path.file_stem().unwrap_or_default().to_string_lossy());
                let name = std::fs::read_to_string(name_file(&path)).ok().filter(|name| !name.is_empty()).unwrap_or(fallback);
                if leftover.metadata().is_ok_and(|metadata| metadata.len() > 0) {
                    keep(kept, &path, &name);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        for leftover in std::fs::read_dir(directory)?.flatten() {
            if leftover.path().extension().is_some_and(|extension| extension == "name") {
                let _ = std::fs::remove_file(leftover.path());
            }
        }
        Ok(Staging {
            directory: directory.to_path_buf(),
            kept: kept.to_path_buf(),
            inner: Mutex::new(Inner { next_file: 1, next_handle: 1, ..Inner::default() }),
        })
    }

    /// Where files that were not uploaded are kept.
    pub fn kept_directory(&self) -> &Path {
        &self.kept
    }

    /// Let a staged file go: kept if its upload failed (the user may have nothing else), deleted otherwise.
    fn dispose(&self, file: StagedFile) {
        if matches!(file.state, StagedState::Failed(_)) && !file.cut_short && file.size > 0 {
            keep(&self.kept, &file.path, &file.name);
        } else {
            let _ = std::fs::remove_file(&file.path);
        }
        let _ = std::fs::remove_file(name_file(&file.path));
    }

    /// The state, with expired receipts gone (their bytes deleted too).
    fn lock(&self) -> MutexGuard<'_, Inner> {
        let mut inner = self.inner.lock().expect("staging lock");
        let now = Instant::now();
        let expired: Vec<u64> = inner
            .files
            .iter()
            .filter(|(_, file)| matches!(file.state, StagedState::Receipt(until) if until <= now) && file.open_handles == 0)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(file) = inner.files.remove(&id) {
                self.dispose(file);
            }
        }
        inner
    }

    /// A new, empty staged file named `name` in `folder`, opened for writing. Replaces a receipt of that name
    /// (writing it again is a new upload). Returns the file's id and the handle.
    pub fn create(&self, folder: &NodeKey, name: &str, awaiting_rename: bool) -> std::io::Result<(u64, u64)> {
        let mut inner = self.lock();
        let id = inner.next_file;
        inner.next_file += 1;
        let path = self.directory.join(format!("{id}.staged"));
        std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&path)?;
        std::fs::write(name_file(&path), name)?;
        let replaced: Vec<u64> = inner.files.iter().filter(|(_, file)| file.folder == *folder && file.name == name).map(|(id, _)| *id).collect();
        for old in replaced {
            if let Some(file) = inner.files.remove(&old) {
                self.dispose(file);
            }
        }
        inner.files.insert(
            id,
            StagedFile {
                folder: folder.clone(),
                name: name.to_owned(),
                size: 0,
                modified: SystemTime::now(),
                state: StagedState::Open,
                dirty: true,
                awaiting_rename,
                round: 0,
                open_handles: 0,
                path,
                cut_short: false,
            },
        );
        let handle = Self::new_handle(&mut inner, id, true);
        Ok((id, handle))
    }

    fn new_handle(inner: &mut Inner, file: u64, writable: bool) -> u64 {
        let handle = inner.next_handle;
        inner.next_handle += 1;
        let round = match inner.files.get_mut(&file) {
            Some(staged) => {
                staged.open_handles += 1;
                staged.round
            }
            None => 0,
        };
        inner.handles.insert(handle, Handle { file, writable, round });
        handle
    }

    /// Open a staged file or receipt; returns the handle.
    pub fn open(&self, file: u64, writable: bool) -> Option<u64> {
        let mut inner = self.lock();
        if !inner.files.contains_key(&file) {
            return None;
        }
        Some(Self::new_handle(&mut inner, file, writable))
    }

    /// Close a handle (FUSE `release`). A failed file goes once nobody holds it, and so does an empty one with its real
    /// name (one that waits for a rename goes after it).
    pub fn release(&self, handle: u64) {
        let mut inner = self.lock();
        let Some(Handle { file, .. }) = inner.handles.remove(&handle) else { return };
        let remove = match inner.files.get_mut(&file) {
            Some(staged) => {
                staged.open_handles = staged.open_handles.saturating_sub(1);
                let empty = staged.size == 0 && !staged.awaiting_rename && matches!(staged.state, StagedState::Open);
                staged.open_handles == 0 && (matches!(staged.state, StagedState::Failed(_)) || empty)
            }
            None => false,
        };
        if remove {
            if let Some(staged) = inner.files.remove(&file) {
                self.dispose(staged);
            }
        }
    }

    pub fn get(&self, file: u64) -> Option<StagedFile> {
        self.lock().files.get(&file).cloned()
    }

    /// The staged file named `name` in `folder`, if any (failed ones are gone from view).
    pub fn find(&self, folder: &NodeKey, name: &str) -> Option<(u64, StagedFile)> {
        self.lock()
            .files
            .iter()
            .find(|(_, file)| file.folder == *folder && file.name == name && !matches!(file.state, StagedState::Failed(_)))
            .map(|(id, file)| (*id, file.clone()))
    }

    /// The staged files and receipts in `folder`, by name.
    pub fn in_folder(&self, folder: &NodeKey) -> Vec<(u64, StagedFile)> {
        let mut found: Vec<(u64, StagedFile)> = self
            .lock()
            .files
            .iter()
            .filter(|(_, file)| file.folder == *folder && !matches!(file.state, StagedState::Failed(_)))
            .map(|(id, file)| (*id, file.clone()))
            .collect();
        found.sort_by(|first, second| first.1.name.cmp(&second.1.name));
        found
    }

    /// The file a handle may write to, starting a new round on a receipt, or the refusal.
    fn writable_path(inner: &mut Inner, handle: u64) -> Result<(u64, PathBuf), Refusal> {
        let Handle { file, writable, round } = *inner.handles.get(&handle).ok_or(libc::EBADF)?;
        if !writable {
            return Err(libc::EBADF);
        }
        let staged = inner.files.get_mut(&file).ok_or(libc::EIO)?;
        if round != staged.round {
            // Opened before the last upload: writing now would make a new file with a hole.
            return Err(libc::EIO);
        }
        match staged.state {
            StagedState::Uploading(_) => Err(libc::EBUSY),
            StagedState::Failed(errno) => Err(errno),
            StagedState::Receipt(_) | StagedState::Open => {
                staged.state = StagedState::Open;
                Ok((file, staged.path.clone()))
            }
        }
    }

    /// Write bytes at `offset` through a handle. The lock is held across the local write, so an upload can never
    /// start between the check and the bytes landing.
    pub fn write(&self, handle: u64, offset: u64, data: &[u8]) -> Result<usize, Refusal> {
        let mut inner = self.lock();
        let (file, path) = Self::writable_path(&mut inner, handle)?;
        let target = std::fs::OpenOptions::new().write(true).open(&path).map_err(|_| libc::EIO)?;
        target.write_all_at(data, offset).map_err(|_| libc::EIO)?;
        if let Some(staged) = inner.files.get_mut(&file) {
            staged.size = staged.size.max(offset + data.len() as u64);
            staged.modified = SystemTime::now();
            staged.dirty = true;
        }
        Ok(data.len())
    }

    /// Read bytes back: a writer checking its work, or a receipt read back (an undo), which keeps its bytes.
    pub fn read(&self, file: u64, offset: u64, length: u32) -> std::io::Result<Vec<u8>> {
        let path = self.lock().files.get(&file).map(|staged| staged.path.clone()).ok_or(std::io::ErrorKind::NotFound)?;
        let source = std::fs::File::open(path)?;
        let mut buffer = vec![0u8; length as usize];
        let mut filled = 0;
        while filled < buffer.len() {
            let count = source.read_at(&mut buffer[filled..], offset + filled as u64)?;
            if count == 0 {
                break;
            }
            filled += count;
        }
        buffer.truncate(filled);
        Ok(buffer)
    }

    /// A truncate through a writable handle, or (any handle) the modification time GIO sets from the source.
    pub fn set_attributes(&self, file: u64, handle: Option<u64>, size: Option<u64>, modified: Option<SystemTime>) -> Result<(), Refusal> {
        let mut inner = self.lock();
        if let Some(size) = size {
            // A truncate without a handle (`truncate(1)`) acts as a fresh writer of the current round, for this call only.
            let (handle, temporary) = match handle {
                Some(handle) => (handle, false),
                None => (Self::new_handle(&mut inner, file, true), true),
            };
            let truncated = Self::writable_path(&mut inner, handle).and_then(|(_, path)| {
                std::fs::OpenOptions::new().write(true).open(path).and_then(|target| target.set_len(size)).map_err(|_| libc::EIO)
            });
            if temporary {
                inner.handles.remove(&handle);
                if let Some(staged) = inner.files.get_mut(&file) {
                    staged.open_handles = staged.open_handles.saturating_sub(1);
                }
            }
            truncated?;
        }
        let staged = inner.files.get_mut(&file).ok_or(libc::ENOENT)?;
        if let Some(size) = size {
            staged.size = size;
            staged.dirty = true;
        }
        if let Some(modified) = modified {
            staged.modified = modified;
        }
        Ok(())
    }

    /// Give a staged file its real name (from a temporary one). Receipts are not renamed: nothing on the server
    /// would change.
    pub fn rename(&self, file: u64, folder: &NodeKey, name: &str, awaiting_rename: bool) -> Result<(), Refusal> {
        let mut inner = self.lock();
        if inner.files.get(&file).is_some_and(StagedFile::is_receipt) {
            return Err(libc::EPERM);
        }
        let replaced: Vec<u64> =
            inner.files.iter().filter(|(other, staged)| **other != file && staged.folder == *folder && staged.name == name).map(|(id, _)| *id).collect();
        for old in replaced {
            if let Some(staged) = inner.files.remove(&old) {
                self.dispose(staged);
            }
        }
        let staged = inner.files.get_mut(&file).ok_or(libc::ENOENT)?;
        staged.folder = folder.clone();
        staged.name = name.to_owned();
        staged.awaiting_rename = awaiting_rename;
        let _ = std::fs::write(name_file(&staged.path), name);
        Ok(())
    }

    /// Remove a staged file: one not uploaded (a cancelled copy cleaning up), or a receipt going early. Not one being
    /// uploaded, whose close is waiting.
    pub fn remove(&self, file: u64) -> Result<(), Refusal> {
        let mut inner = self.lock();
        match inner.files.get(&file).map(|staged| &staged.state) {
            // A receipt may go early: Immich keeps the photo, and the receipt was leaving anyway.
            Some(StagedState::Uploading(_)) => Err(libc::EBUSY),
            Some(_) => {
                if let Some(staged) = inner.files.remove(&file) {
                    self.dispose(staged);
                }
                Ok(())
            }
            None => Err(libc::ENOENT),
        }
    }

    /// What closing `handle` (FUSE `flush`) must do. Only a writable handle of the current round can start an
    /// upload, and only of a file with its real name, written since its last upload.
    pub fn close(&self, handle: u64) -> CloseAction {
        let mut inner = self.lock();
        let Some(Handle { file, writable, round }) = inner.handles.get(&handle).copied() else {
            return CloseAction::Done;
        };
        let Some(staged) = inner.files.get_mut(&file) else { return CloseAction::Done };
        match &staged.state {
            StagedState::Failed(errno) => return CloseAction::Fail(*errno),
            StagedState::Uploading(receiver) => return CloseAction::Wait(receiver.clone()),
            StagedState::Receipt(_) => return CloseAction::Done,
            StagedState::Open => {}
        }
        // An empty file never uploads: a shell's `>` closes a copy of the descriptor before anything is written, and
        // `touch` writes nothing at all.
        if !writable || round != staged.round || !staged.dirty || staged.awaiting_rename || staged.size == 0 {
            return CloseAction::Done;
        }
        let (sender, receiver) = watch::channel(None);
        staged.state = StagedState::Uploading(receiver);
        staged.dirty = false;
        CloseAction::Upload(PendingUpload { file, path: staged.path.clone(), name: staged.name.clone(), modified: staged.modified, sender })
    }

    /// Start the upload of a closed file just given its real name (rsync renames after closing). `None` if there is
    /// nothing to upload or someone still writes it (then the writer's close uploads); readers do not hold it back.
    /// An empty file is not uploaded, and goes once nobody holds it.
    pub fn upload_after_rename(&self, file: u64) -> Option<PendingUpload> {
        let mut inner = self.lock();
        let round = inner.files.get(&file)?.round;
        let writing = inner.handles.values().any(|handle| handle.file == file && handle.writable && handle.round == round);
        let staged = inner.files.get_mut(&file)?;
        if staged.size == 0 && staged.open_handles == 0 && matches!(staged.state, StagedState::Open) {
            if let Some(staged) = inner.files.remove(&file) {
                self.dispose(staged);
            }
            return None;
        }
        if writing || !staged.dirty || staged.awaiting_rename || staged.size == 0 || !matches!(staged.state, StagedState::Open) {
            return None;
        }
        let (sender, receiver) = watch::channel(None);
        staged.state = StagedState::Uploading(receiver);
        staged.dirty = false;
        Some(PendingUpload { file, path: staged.path.clone(), name: staged.name.clone(), modified: staged.modified, sender })
    }

    /// End an upload that was refused because the file ends before its own structure does: a cancelled copy, whose
    /// source is still there. It is deleted when it goes, not kept.
    pub fn finish_cut_short(&self, pending: PendingUpload) {
        if let Some(staged) = self.lock().files.get_mut(&pending.file) {
            staged.cut_short = true;
        }
        self.finish_upload(pending, Err(libc::EIO));
    }

    /// Record an upload's end and tell every waiting close. A success leaves a receipt, keeping its bytes, and ends
    /// the round; a failure is remembered for the other closes, and the file goes once nobody holds it.
    pub fn finish_upload(&self, pending: PendingUpload, result: UploadResult) {
        let mut inner = self.lock();
        let mut remove = false;
        if let Some(staged) = inner.files.get_mut(&pending.file) {
            match result {
                Ok(()) => {
                    staged.state = StagedState::Receipt(Instant::now() + RECEIPT_LIFETIME);
                    staged.round += 1;
                }
                Err(errno) => {
                    staged.state = StagedState::Failed(errno);
                    remove = staged.open_handles == 0;
                }
            }
        }
        if remove {
            if let Some(staged) = inner.files.remove(&pending.file) {
                self.dispose(staged);
            }
        }
        let _ = pending.sender.send(Some(result));
    }

    /// Files being written or uploaded: name, phase (`writing` or `uploading`) and bytes.
    /// Whether nothing staged is open or uploading, so the mount may go away without cutting a copy short.
    pub fn is_idle(&self) -> bool {
        self.lock().files.values().all(|staged| staged.open_handles == 0 && !matches!(staged.state, StagedState::Uploading(_)))
    }

    pub fn in_flight(&self) -> Vec<(String, String, u64)> {
        self.lock()
            .files
            .values()
            .filter_map(|staged| match staged.state {
                StagedState::Open if staged.dirty && staged.open_handles > 0 => Some((staged.name.clone(), "writing".to_owned(), staged.size)),
                StagedState::Uploading(_) => Some((staged.name.clone(), "uploading".to_owned(), staged.size)),
                _ => None,
            })
            .collect()
    }

    /// The directory the staged bytes live in, for free-space reporting.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use immich_mount_core::tree::SectionKey;

    fn photos() -> NodeKey {
        NodeKey::Section(SectionKey::Photos)
    }

    fn upload(action: CloseAction) -> PendingUpload {
        match action {
            CloseAction::Upload(pending) => pending,
            _ => panic!("expected an upload"),
        }
    }

    #[tokio::test]
    async fn only_a_writers_close_uploads_and_every_close_gets_its_result() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), "IMG_0001.JPG", false).unwrap();
        staging.write(writer, 0, b"hello world").unwrap();
        let reader = staging.open(file, false).unwrap();
        assert!(matches!(staging.close(reader), CloseAction::Done), "a reader's close never uploads");

        let pending = upload(staging.close(writer));
        assert_eq!(std::fs::read(&pending.path).unwrap(), b"hello world");
        let CloseAction::Wait(receiver) = staging.close(writer) else { panic!("a second close waits") };
        assert_eq!(staging.write(writer, 0, b"x"), Err(libc::EBUSY), "no writes while uploading");
        staging.finish_upload(pending, Ok(()));
        assert_eq!(wait_for(receiver).await, Ok(()));

        let receipt = staging.get(file).unwrap();
        assert!(receipt.is_receipt());
        assert_eq!(staging.read(file, 0, 64).unwrap(), b"hello world", "a receipt keeps its bytes");
        assert_eq!(staging.write(writer, 11, b"!"), Err(libc::EIO), "a handle from before the upload cannot write");
        let fresh = staging.open(file, true).unwrap();
        staging.write(fresh, 0, b"again").unwrap();
        assert!(matches!(staging.close(fresh), CloseAction::Upload(_)), "a fresh open of the name is a new upload");
    }

    #[tokio::test]
    async fn a_receipt_may_go_before_its_time() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), "IMG_0003.JPG", false).unwrap();
        staging.write(writer, 0, b"bytes").unwrap();
        let pending = upload(staging.close(writer));
        let path = pending.path.clone();
        staging.finish_upload(pending, Ok(()));
        assert!(staging.get(file).unwrap().is_receipt());
        assert_eq!(staging.remove(file), Ok(()), "Immich keeps the photo; the receipt was leaving anyway");
        assert!(staging.get(file).is_none());
        assert!(!path.exists(), "its bytes go with it, as Immich has them");
    }

    #[tokio::test]
    async fn a_failure_reaches_every_close_and_the_file_goes_with_its_last_handle() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), "IMG_0002.JPG", false).unwrap();
        staging.write(writer, 0, b"bytes").unwrap();
        let pending = upload(staging.close(writer));
        staging.finish_upload(pending, Err(libc::EACCES));
        assert!(matches!(staging.close(writer), CloseAction::Fail(libc::EACCES)), "a later close gets the failure");
        assert!(staging.in_folder(&photos()).is_empty(), "a failed file is not shown");
        staging.release(writer);
        assert!(staging.get(file).is_none(), "and goes with its last handle");
    }

    #[test]
    fn a_temporary_name_uploads_once_renamed_and_closed() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), ".goutputstream-ABC123", true).unwrap();
        staging.write(writer, 0, b"bytes").unwrap();
        assert!(matches!(staging.close(writer), CloseAction::Done), "a temporary name does not upload on close");
        staging.rename(file, &photos(), "IMG_0003.JPG", false).unwrap();
        assert!(staging.upload_after_rename(file).is_none(), "still open: the writer's close uploads");
        staging.release(writer);
        assert!(staging.upload_after_rename(file).is_some(), "closed already (rsync): the rename uploads");
    }

    #[test]
    fn an_empty_file_never_uploads_and_goes_with_its_last_handle() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), "touched.jpg", false).unwrap();
        assert!(matches!(staging.close(writer), CloseAction::Done), "nothing written, nothing uploaded");
        staging.release(writer);
        assert!(staging.get(file).is_none(), "an empty file goes when closed");
    }

    #[test]
    fn a_shell_redirect_uploads_once_written() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (_, writer) = staging.create(&photos(), "IMG_0004.JPG", false).unwrap();
        assert!(matches!(staging.close(writer), CloseAction::Done), "the shell closes its copy of the descriptor first");
        staging.write(writer, 0, b"bytes").unwrap();
        assert!(matches!(staging.close(writer), CloseAction::Upload(_)), "the writer's own close uploads");
    }

    #[test]
    fn a_reader_does_not_hold_back_the_upload_after_a_rename() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), ".IMG_0005.JPG.a1b2c3", true).unwrap();
        staging.write(writer, 0, b"bytes").unwrap();
        assert!(matches!(staging.close(writer), CloseAction::Done));
        staging.release(writer);
        let _reader = staging.open(file, false).unwrap();
        staging.rename(file, &photos(), "IMG_0005.JPG", false).unwrap();
        assert!(staging.upload_after_rename(file).is_some(), "only a writer would close it and upload later");
    }

    #[test]
    fn a_truncate_without_a_handle_holds_nothing_open() {
        let directory = tempfile::tempdir().unwrap();
        let staging = Staging::new(directory.path(), &directory.path().join("kept")).unwrap();
        let (file, writer) = staging.create(&photos(), ".IMG_0006.JPG.d4e5f6", true).unwrap();
        staging.write(writer, 0, b"bytes and more").unwrap();
        staging.release(writer);
        staging.set_attributes(file, None, Some(5), None).unwrap();
        assert_eq!(staging.get(file).unwrap().open_handles, 0);
        assert_eq!(staging.read(file, 0, 64).unwrap(), b"bytes");
    }

    #[test]
    fn leftovers_of_a_dead_daemon_are_kept_under_their_names_not_uploaded() {
        let directory = tempfile::tempdir().unwrap();
        let (staging_directory, kept) = (directory.path().join("staging"), directory.path().join("kept"));
        std::fs::create_dir(&staging_directory).unwrap();
        std::fs::write(staging_directory.join("7.staged"), b"half").unwrap();
        std::fs::write(staging_directory.join("7.name"), "IMG_0007.JPG").unwrap();
        std::fs::write(staging_directory.join("8.staged"), b"").unwrap();
        let staging = Staging::new(&staging_directory, &kept).unwrap();
        assert!(std::fs::read_dir(&staging_directory).unwrap().next().is_none(), "the staging area starts empty");
        assert!(staging.in_folder(&photos()).is_empty());
        assert_eq!(std::fs::read(kept.join("IMG_0007.JPG")).unwrap(), b"half", "kept: a move may have lost its source");
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 1, "an empty leftover is not kept");
    }

    #[tokio::test]
    async fn a_failed_upload_is_kept_and_a_cancelled_copy_is_not() {
        let directory = tempfile::tempdir().unwrap();
        let kept = directory.path().join("kept");
        let staging = Staging::new(&directory.path().join("staging"), &kept).unwrap();
        let (_, writer) = staging.create(&photos(), "IMG_0008.JPG", false).unwrap();
        staging.write(writer, 0, b"bytes").unwrap();
        staging.finish_upload(upload(staging.close(writer)), Err(libc::EACCES));
        staging.release(writer);
        assert_eq!(std::fs::read(kept.join("IMG_0008.JPG")).unwrap(), b"bytes");

        let (_, again) = staging.create(&photos(), "IMG_0008.JPG", false).unwrap();
        staging.write(again, 0, b"other").unwrap();
        staging.finish_upload(upload(staging.close(again)), Err(libc::EIO));
        staging.release(again);
        assert_eq!(std::fs::read(kept.join("IMG_0008 (2).JPG")).unwrap(), b"other", "a taken name is numbered");

        let (_, cancelled) = staging.create(&photos(), "IMG_0009.JPG", false).unwrap();
        staging.write(cancelled, 0, b"par").unwrap();
        staging.finish_cut_short(upload(staging.close(cancelled)));
        staging.release(cancelled);
        assert!(!kept.join("IMG_0009.JPG").exists(), "a cancelled copy still has its source");
    }
}
