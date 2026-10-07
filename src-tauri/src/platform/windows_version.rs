#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsVersion {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) build: u32,
    pub(crate) workstation: bool,
}

impl WindowsVersion {
    pub(crate) fn supports_conpty(self) -> bool {
        self.major > 10
            || (self.major == 10 && (self.minor > 0 || (self.minor == 0 && self.build >= 17_763)))
    }
}

#[cfg(windows)]
pub(crate) fn current_windows_version() -> Option<WindowsVersion> {
    use windows::Win32::System::SystemInformation::OSVERSIONINFOEXW;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

    const VER_NT_WORKSTATION: u8 = 1;
    type RtlGetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOEXW) -> i32;

    let mut version = OSVERSIONINFOEXW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOEXW>() as u32,
        ..Default::default()
    };

    let rtl_get_version = unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
        if ntdll.is_null() {
            None
        } else {
            GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast())
        }
    }?;

    let rtl_get_version: RtlGetVersion = unsafe { std::mem::transmute(rtl_get_version) };

    // RtlGetVersion reports the real Windows version without manifest-based version shims.
    if unsafe { rtl_get_version(&mut version) } < 0 {
        return None;
    }

    Some(WindowsVersion {
        major: version.dwMajorVersion,
        minor: version.dwMinorVersion,
        build: version.dwBuildNumber,
        workstation: version.wProductType == VER_NT_WORKSTATION,
    })
}

#[cfg(test)]
mod tests {
    use super::WindowsVersion;

    fn workstation(build: u32) -> WindowsVersion {
        WindowsVersion {
            major: 10,
            minor: 0,
            build,
            workstation: true,
        }
    }

    #[test]
    fn conpty_requires_windows_10_1809_or_later() {
        assert!(!workstation(16_299).supports_conpty());
        assert!(!workstation(17_134).supports_conpty());
        assert!(workstation(17_763).supports_conpty());
        assert!(workstation(19_045).supports_conpty());
        assert!(workstation(22_000).supports_conpty());
        assert!(workstation(26_100).supports_conpty());
    }
}
