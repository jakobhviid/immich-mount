//! Who is calling: the thread behind a FUSE request, and its process's ancestors.
//!
//! FUSE reports the calling thread's id. `/proc/<id>/comm` then gives the thread's name, which tells a file
//! manager's search crawl from a person browsing, and the ancestors' names tell a thumbnailer, even one sandboxed
//! under `bwrap` (ARCHITECTURE.md, "Seeding the freedesktop cache" and "The safety net"). Names are truncated by the
//! kernel to 15 bytes, so the lists hold truncated names. KDE's thumbnailer is told apart by its command line.

/// Threads that crawl folders for a search. Their listings never trigger seeding.
const SEARCH_CRAWLERS: &[&str] = &["nautilus-search"];

/// Indexers that walk every folder for file names. Their listings never trigger seeding either: one crawl of the
/// mount would otherwise fetch a server thumbnail for every photo (seen with Baloo in a Plasma VM).
const INDEX_CRAWLERS: &[&str] = &[
    "baloo_file",      // KDE
    "localsearch-3",   // GNOME
    "tracker-miner-f", // tracker-miner-fs-3, older GNOME
];

/// Programs that make thumbnails by reading originals. They get `EIO` instead of the bytes.
const THUMBNAILERS: &[&str] = &[
    "glycin-thumbnai", // glycin-thumbnailer, GNOME 50 images
    "gst-video-thumb", // gst-video-thumbnailer, GNOME 50 videos
    "gdk-pixbuf-thum", // gdk-pixbuf-thumbnailer
    "totem-video-thu", // totem-video-thumbnailer
    "ffmpegthumbnail", // ffmpegthumbnailer
    "evince-thumbnai", // evince-thumbnailer
    "gnome-epub-thum", // gnome-epub-thumbnailer
];

/// Desktop indexers that read what is in files. They get `EIO` instead of the bytes, always: indexing the mount's
/// contents would download the whole library. Their listings (file names) are answered as usual. KDE's Baloo reads
/// originals once the mount is among its folders (seen in a Plasma VM); GNOME's are also kept out by
/// `.trackerignore`.
const INDEXERS: &[&str] = &[
    "baloo_file_extr", // baloo_file_extractor, KDE
    "localsearch-ext", // localsearch-extractor-3, GNOME
    "tracker-extract", // tracker-extract-3, older GNOME
];

/// The indexer the caller is, if any: the thread, or the process it belongs to (the extractors name their threads).
pub fn indexer(thread_id: u32) -> Option<String> {
    [Some(thread_id), process_of(thread_id)].into_iter().flatten().filter_map(name_of).find(|name| INDEXERS.contains(&name.as_str()))
}

/// Whether a process is a KDE `kioworker` running the given plugin (`thumbnail.so`, `kio_filenamesearch.so`, as KF6
/// names them; both spellings are accepted). Workers share the process name (`file` copies files, `trash`, `tags`),
/// so the plugin on the command line decides.
fn is_kio_worker(process_id: u32, name: &str, plugin: &str) -> bool {
    let (plain, prefixed) = (format!("/kio/{plugin}.so"), format!("/kio/kio_{plugin}.so"));
    name == "kioworker"
        && std::fs::read(format!("/proc/{process_id}/cmdline")).is_ok_and(|command_line| {
            command_line.split(|byte| *byte == 0).any(|argument| argument.ends_with(plain.as_bytes()) || argument.ends_with(prefixed.as_bytes()))
        })
}

/// How many ancestors to look through: a sandboxed loader sits a few levels below the thumbnailer.
const ANCESTOR_DEPTH: usize = 6;

/// A thread's or process's name, from `/proc`.
pub fn name_of(id: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{id}/comm")).ok()?;
    Some(comm.trim_end_matches('\n').to_owned())
}

/// The parent process of a thread or process, from `/proc/<id>/stat`.
fn parent_of(id: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{id}/stat")).ok()?;
    parent_from_stat(&stat)
}

/// The fourth field of a `stat` line. The second is the name in parentheses, and may itself hold spaces or `)`.
fn parent_from_stat(stat: &str) -> Option<u32> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().nth(1)?.parse().ok()
}

/// The kernel's `PF_SIGNALED` task flag: the task is dying from a signal.
const PF_SIGNALED: u64 = 0x0000_0400;

/// Whether the calling thread is closing its files because a signal killed it (Ctrl+C on `cp`). Such a close comes
/// with half-written data, so it must not upload.
pub fn was_killed(thread_id: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{thread_id}/stat")) else { return false };
    flags_from_stat(&stat).is_some_and(|flags| flags & PF_SIGNALED != 0)
}

/// The ninth field of a `stat` line (the task flags), counted after the name in parentheses.
fn flags_from_stat(stat: &str) -> Option<u64> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().nth(6)?.parse().ok()
}

/// Whether the calling thread is a file manager's search crawl: Files' `nautilus-search` thread, or Dolphin's
/// `filenamesearch` worker.
pub fn is_search_crawl(thread_id: u32) -> bool {
    if name_of(thread_id).is_some_and(|name| SEARCH_CRAWLERS.contains(&name.as_str()) || INDEX_CRAWLERS.contains(&name.as_str())) {
        return true;
    }
    process_of(thread_id).is_some_and(|process| {
        name_of(process).is_some_and(|name| INDEX_CRAWLERS.contains(&name.as_str()) || is_kio_worker(process, &name, "filenamesearch"))
    })
}

/// The process a thread belongs to (its thread group), from `/proc/<id>/status`.
fn process_of(thread_id: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{thread_id}/status")).ok()?;
    status.lines().find_map(|line| line.strip_prefix("Tgid:")).and_then(|value| value.trim().parse().ok())
}

/// The thumbnailer the caller is, or runs under, if any. The caller is often a worker thread with a name of its own
/// (glycin's `blocking-1`), so the search starts from the thread's process and walks up its ancestors.
pub fn thumbnailer(thread_id: u32) -> Option<String> {
    if let Some(name) = name_of(thread_id).filter(|name| THUMBNAILERS.contains(&name.as_str())) {
        return Some(name);
    }
    let mut current = process_of(thread_id)?;
    for _ in 0..=ANCESTOR_DEPTH {
        if let Some(name) = name_of(current) {
            if THUMBNAILERS.contains(&name.as_str()) {
                return Some(name);
            }
            if is_kio_worker(current, &name, "thumbnail") {
                return Some("kioworker thumbnail".to_owned());
            }
        }
        current = match parent_of(current) {
            Some(parent) if parent > 1 && parent != current => parent,
            _ => return None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_comes_after_the_name_even_when_the_name_has_spaces_and_parentheses() {
        assert_eq!(parent_from_stat("4242 (bash) S 4100 4242 4242 0"), Some(4100));
        assert_eq!(parent_from_stat("77 (my (odd) name) R 12 77 77 0"), Some(12));
        assert_eq!(parent_from_stat("garbage"), None);
    }

    #[test]
    fn the_task_flags_come_after_the_name() {
        // pid (comm) state ppid pgrp session tty_nr tpgid flags
        assert_eq!(flags_from_stat("4242 (cp) S 4100 4242 4242 0 -1 4194624 0 0"), Some(4_194_624));
        assert_eq!(flags_from_stat("77 (odd name)) R 1 1 1 0 -1 1024"), Some(1024));
        assert!(!was_killed(std::process::id()), "this test is alive");
    }

    #[test]
    fn a_thread_belongs_to_its_process() {
        let own_id = std::process::id();
        assert_eq!(process_of(own_id), Some(own_id));
        let worker = std::thread::Builder::new()
            .name("blocking-1".into())
            .spawn(move || {
                let thread_id = unsafe { libc::gettid() } as u32;
                (name_of(thread_id), process_of(thread_id))
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(worker, (Some("blocking-1".to_owned()), Some(own_id)));
    }

    #[test]
    fn this_test_is_neither_a_crawler_nor_a_thumbnailer_nor_an_indexer() {
        let own_id = std::process::id();
        assert!(name_of(own_id).is_some());
        assert!(!is_search_crawl(own_id));
        assert_eq!(thumbnailer(own_id), None);
        assert_eq!(indexer(own_id), None);
    }
}
