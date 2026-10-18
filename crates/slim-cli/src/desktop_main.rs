#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    if let Err(error) = slim_cli::run_app() {
        let message = format!("SLIM could not start.\n\n{error:#}");
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show($env:SLIM_START_ERROR, 'SLIM') | Out-Null"])
            .env("SLIM_START_ERROR", &message).status();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display alert \"SLIM could not start\" message (item 1 of argv)",
                "-e",
                "end run",
                &message,
            ])
            .status();
        eprintln!("{message}");
    }
}
