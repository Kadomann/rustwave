//! Bildschirmtastatur für Touch-Bedienung (Windows): Touch-Tastatur (TabTip) einblenden,
//! sonst die klassische Bildschirmtastatur (osk.exe) starten.

#[cfg(windows)]
mod imp {
    use windows::core::{interface, IUnknown, IUnknown_Vtbl, GUID, HRESULT};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    /// undokumentierte, aber seit Windows 10 übliche Schnittstelle der Touch-Tastatur
    #[interface("37c994e7-432b-4834-a2f7-dce1f13b834b")]
    unsafe trait ITipInvocation: IUnknown {
        fn toggle(&self, wnd: HWND) -> HRESULT;
    }
    const CLSID_UIHOST_NO_LAUNCH: GUID = GUID::from_u128(0x4ce576fa_83dc_4f88_951c_9d0782b4e376);

    fn tabtip() -> Result<(), String> {
        let exe = r"C:\Program Files\Common Files\microsoft shared\ink\TabTip.exe";
        if !std::path::Path::new(exe).exists() { return Err("keine Touch-Tastatur installiert".into()); }
        // der Dienst muss laufen, bevor er sich umschalten lässt
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", exe]).spawn();
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let tip: ITipInvocation = CoCreateInstance(&CLSID_UIHOST_NO_LAUNCH, None, CLSCTX_LOCAL_SERVER).map_err(|e| e.to_string())?;
            tip.toggle(GetDesktopWindow()).ok().map_err(|e| e.to_string())
        }
    }
    pub fn show() -> Result<(), String> {
        tabtip().or_else(|_| std::process::Command::new("osk.exe").spawn().map(|_| ()).map_err(|e| e.to_string()))
    }
}

/// Tastatur einblenden (eigener Thread, damit die Oberfläche nicht hängt)
pub fn show(tx: std::sync::mpsc::Sender<crate::UiEv>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        let r = imp::show();
        #[cfg(not(windows))]
        let r: Result<(), String> = Err("nur unter Windows".into());
        if let Err(e) = r { let _ = tx.send(crate::UiEv::Log(format!("Bildschirmtastatur: {e}"))); }
    });
}
