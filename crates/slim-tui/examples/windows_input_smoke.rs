//! Exercise actual Win32 console records through Crossterm into SLIM.
//! Run from an isolated terminal: cargo run -p slim-tui --example windows_input_smoke
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use crossterm::{
        event::{self, Event, KeyEventKind},
        terminal::{disable_raw_mode, enable_raw_mode},
    };
    use slim_tui::{App, app::Modal, event::handle_key_event};
    use std::{ffi::c_void, time::Duration};
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeyRecord {
        down: i32,
        repeat: u16,
        virtual_key: u16,
        scan: u16,
        unicode: u16,
        modifiers: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct InputRecord {
        event_type: u16,
        padding: u16,
        key: KeyRecord,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(id: u32) -> *mut c_void;
        fn WriteConsoleInputW(
            handle: *mut c_void,
            records: *const InputRecord,
            count: u32,
            written: *mut u32,
        ) -> i32;
    }
    struct RawGuard;
    impl Drop for RawGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
        }
    }
    let conn = slim_db::connection::open_memory()?;
    slim_db::migrate::run_migrations(&conn)?;
    let mut app = App::new_locked(&conn)?;
    enable_raw_mode()?;
    let guard = RawGuard;
    let mut presses = 0;
    let mut releases = 0;
    for expected in ["a", "aa", "aaa"] {
        let records = [true, false].map(|down| InputRecord {
            event_type: 1,
            padding: 0,
            key: KeyRecord {
                down: i32::from(down),
                repeat: 1,
                virtual_key: 0x41,
                scan: 0x1e,
                unicode: u16::from(b'a'),
                modifiers: 0,
            },
        });
        let mut written = 0;
        // The only handle modified is this smoke process's dedicated terminal input.
        let succeeded = unsafe {
            WriteConsoleInputW(
                GetStdHandle((-10i32) as u32),
                records.as_ptr(),
                records.len() as u32,
                &mut written,
            )
        };
        anyhow::ensure!(
            succeeded != 0 && written == 2,
            "Cannot inject test records into this terminal: {}",
            std::io::Error::last_os_error()
        );
        let mut seen = 0;
        while seen < 2 {
            anyhow::ensure!(
                event::poll(Duration::from_secs(3))?,
                "Timed out waiting for console record"
            );
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    presses += 1;
                }
                if key.kind == KeyEventKind::Release {
                    releases += 1;
                }
                handle_key_event(&mut app, key);
                seen += 1;
            }
        }
        let Some(Modal::Catalog(form)) = app.active_modal.as_ref() else {
            anyhow::bail!("Setup form closed unexpectedly")
        };
        anyhow::ensure!(
            form.fields[0].value == expected,
            "Expected {expected:?}, got {:?}",
            form.fields[0].value
        );
    }
    drop(guard);
    println!(
        "Windows input smoke passed: {presses} presses + {releases} releases produced exactly aaa; one tap produced a."
    );
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    println!("Win32 console input smoke runs on Windows only.");
}
