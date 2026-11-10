//! The clipboard, for a command that ends at once: Dolphin's Copy Immich Link runs `immich-mount web --copy`. A
//! Wayland or X11 clipboard is served by the program that set it, so the text is handed to one that stays.

use std::io::Write;

/// Put `text` on the clipboard so that it stays there after this command ends: with Plasma's clipboard (Klipper) where
/// it runs, else with `wl-copy` on Wayland or `xclip` or `xsel` on X11, which keep serving it.
pub fn copy(text: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())?;
    let klipper = runtime.block_on(async {
        let connection = zbus::Connection::session().await.ok()?;
        let proxy = zbus::Proxy::new(&connection, "org.kde.klipper", "/klipper", "org.kde.klipper.klipper").await.ok()?;
        tokio::time::timeout(std::time::Duration::from_secs(3), proxy.call::<_, _, ()>("setClipboardContents", &(text,))).await.ok()?.ok()
    });
    if klipper.is_some() {
        return Ok(());
    }
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let helpers: &[(&str, &[&str])] = if wayland {
        &[("wl-copy", &[])]
    } else {
        &[("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    };
    for (program, arguments) in helpers {
        let Ok(mut child) = std::process::Command::new(program)
            .args(*arguments)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        // The helper keeps serving the text once this command ends, in a process of its own.
        let written = child.stdin.take().is_some_and(|mut input| input.write_all(text.as_bytes()).is_ok());
        let succeeded = child.wait().is_ok_and(|status| status.success());
        return if written && succeeded { Ok(()) } else { Err(format!("{program} could not set the clipboard")) };
    }
    let install = if wayland { "wl-clipboard" } else { "xclip" };
    Err(format!("no clipboard to copy to: install {install}"))
}
