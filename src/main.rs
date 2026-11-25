//! sEdit: en lille, hurtig notesblok med faner til .env-filer og anden ren tekst.

#![windows_subsystem = "windows"]
#![allow(clippy::missing_safety_doc)]

mod dialog;
mod doc;
mod draw;
mod find;
mod highlight;
mod lang;
mod session;
mod settings;
mod syntax;
mod theme;
mod ui;
mod util;
mod watch;

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::w;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::Threading::{CreateMutexW, Sleep};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowExW, GetWindowThreadProcessId, MessageBoxW,
    SendMessageTimeoutW, HWND_MESSAGE, MB_ICONERROR, MB_OK, SMTO_ABORTIFHUNG, WM_COPYDATA,
};

fn main() {
    std::panic::set_hook(Box::new(|info| unsafe {
        let text = util::wide(&format!("{}\n\n{info}", lang::s().internal_error));
        MessageBoxW(null_mut(), text.as_ptr(), w!("sEdit"), MB_OK | MB_ICONERROR);
    }));

    // Windows' sprog gælder, indtil brugerens eget valg er læst sammen med indstillingerne.
    lang::select(0);

    let paths: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect();

    // SEDIT_INSTANCE giver en selvstændig sEdit, der ikke deler vindue med den
    // almindelige. Testene bruger det, så de aldrig rører den instans man arbejder i.
    let instance = std::env::var("SEDIT_INSTANCE").unwrap_or_default();
    let mutex_name = util::wide(&format!("sEdit.SingleInstance{instance}"));
    let ipc_class = util::wide(&format!("sEdit.Ipc{instance}"));

    unsafe {
        // Kører sEdit allerede, sendes filerne dertil og åbner som faner.
        CreateMutexW(null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS && forward(&paths, &ipc_class) {
            return;
        }
        // Sessionen (åbne faner og ugemt tekst) ligger pr. bruger, uden for programmappen.
        let data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        ui::run(paths, &ipc_class, data.join("sEdit").join(format!("session{instance}")));
    }
}

/// Afleverer stierne til den kørende instans. False hvis den ikke svarer.
unsafe fn forward(paths: &[PathBuf], ipc_class: &[u16]) -> bool {
    // Den anden instans kan være midt i sin opstart, så giv den et øjeblik til at få sit vindue op.
    for _ in 0..150 {
        let target = FindWindowExW(HWND_MESSAGE, null_mut(), ipc_class.as_ptr(), null());
        if target.is_null() {
            Sleep(10);
            continue;
        }
        let mut data: Vec<u16> = Vec::new();
        for path in paths {
            data.extend(path.as_os_str().encode_wide());
            data.push(b'\n' as u16);
        }
        let payload = COPYDATASTRUCT {
            dwData: ui::IPC_MAGIC,
            cbData: (data.len() * 2) as u32,
            lpData: data.as_ptr() as *mut _,
        };
        let mut process = 0u32;
        GetWindowThreadProcessId(target, &mut process);
        AllowSetForegroundWindow(process);
        let mut reply = 0usize;
        let sent = SendMessageTimeoutW(
            target,
            WM_COPYDATA,
            0,
            &payload as *const COPYDATASTRUCT as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            &mut reply,
        );
        return sent != 0 && reply == 1;
    }
    false
}
