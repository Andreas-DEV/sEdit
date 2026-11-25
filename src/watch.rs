use std::path::PathBuf;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
};

use crate::util::wide_os;

/// MsgWaitForMultipleObjects kan højst vente på 64 håndtag, og ét går til beskedkøen.
pub const MAX_WATCHES: usize = 60;

/// Én ændringsnotifikation pr. mappe med åbne filer. Håndtagene ventes på
/// direkte i hovedløkken, så der hverken er tråde eller polling.
pub struct Watcher {
    dirs: Vec<(String, HANDLE)>,
}

impl Watcher {
    pub fn new() -> Watcher {
        Watcher { dirs: Vec::new() }
    }

    /// Afstemmer de overvågede mapper med `wanted` (nøgle = mappesti i små bogstaver).
    pub unsafe fn sync(&mut self, wanted: &[(String, PathBuf)]) {
        self.dirs.retain(|(key, handle)| {
            let keep = wanted.iter().any(|(k, _)| k == key);
            if !keep {
                FindCloseChangeNotification(*handle);
            }
            keep
        });
        for (key, dir) in wanted {
            if self.dirs.len() >= MAX_WATCHES {
                break;
            }
            if self.dirs.iter().any(|(k, _)| k == key) {
                continue;
            }
            let path = wide_os(dir.as_os_str());
            let handle = FindFirstChangeNotificationW(
                path.as_ptr(),
                0,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE,
            );
            if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
                self.dirs.push((key.clone(), handle));
            }
        }
    }

    pub fn fill(&self, out: &mut [HANDLE; MAX_WATCHES]) -> usize {
        for (slot, (_, handle)) in out.iter_mut().zip(&self.dirs) {
            *slot = *handle;
        }
        self.dirs.len()
    }

    /// Genaktiverer notifikationen efter et signal; en mappe der er forsvundet droppes.
    pub unsafe fn rearm(&mut self, index: usize) {
        let Some(&(_, handle)) = self.dirs.get(index) else { return };
        if FindNextChangeNotification(handle) == 0 {
            FindCloseChangeNotification(handle);
            self.dirs.remove(index);
        }
    }

    pub unsafe fn clear(&mut self) {
        for (_, handle) in self.dirs.drain(..) {
            FindCloseChangeNotification(handle);
        }
    }
}
