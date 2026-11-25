use std::ptr::null_mut;

use windows_sys::w;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use crate::util::rgb;

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub strip: u32,
    pub hover: u32,
    pub close_hover: u32,
    pub text: u32,
    pub dim: u32,
    pub sep: u32,
    pub accent: u32,
    /// Tekstfarve oven på accentfarven.
    pub on_accent: u32,
    pub editor_bg: u32,
    pub editor_fg: u32,
    // Farver til kode.
    pub keyword: u32,
    pub string: u32,
    pub number: u32,
    pub comment: u32,
    pub type_name: u32,
    pub function: u32,
}

const LIGHT: Theme = Theme {
    dark: false,
    strip: rgb(236, 236, 236),
    hover: rgb(224, 224, 224),
    close_hover: rgb(204, 204, 204),
    text: rgb(26, 26, 26),
    dim: rgb(92, 92, 92),
    sep: rgb(206, 206, 206),
    accent: rgb(0, 103, 192),
    on_accent: rgb(255, 255, 255),
    editor_bg: rgb(255, 255, 255),
    editor_fg: rgb(20, 20, 20),
    keyword: rgb(0, 92, 197),
    string: rgb(163, 84, 10),
    number: rgb(24, 122, 62),
    comment: rgb(120, 128, 136),
    type_name: rgb(0, 128, 128),
    function: rgb(124, 93, 0),
};

const DARK: Theme = Theme {
    dark: true,
    strip: rgb(32, 32, 32),
    hover: rgb(46, 46, 46),
    close_hover: rgb(72, 72, 72),
    text: rgb(255, 255, 255),
    dim: rgb(170, 170, 170),
    sep: rgb(66, 66, 66),
    accent: rgb(76, 194, 255),
    on_accent: rgb(0, 0, 0),
    editor_bg: rgb(39, 39, 39),
    editor_fg: rgb(232, 232, 232),
    keyword: rgb(108, 182, 255),
    string: rgb(224, 179, 122),
    number: rgb(165, 214, 167),
    comment: rgb(127, 135, 144),
    type_name: rgb(95, 208, 192),
    function: rgb(236, 208, 112),
};

impl Theme {
    /// Temaet for valget i indstillingerne: 1 er lyst, 2 er mørkt, alt andet følger Windows.
    pub fn for_mode(mode: u32) -> Theme {
        match mode {
            1 => LIGHT,
            2 => DARK,
            _ => Theme::detect(),
        }
    }

    /// Følger Windows' indstilling for lyst/mørkt app-tema.
    pub fn detect() -> Theme {
        let mut value: u32 = 1;
        let mut size: u32 = 4;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                null_mut(),
                &mut value as *mut u32 as *mut _,
                &mut size,
            )
        };
        if status == 0 && value == 0 {
            DARK
        } else {
            LIGHT
        }
    }
}
