//! Dock LED indicator, location finding, and physical presence commands.

use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;
use slim_core::model::led::*;
use slim_core::service::{GridRepository, LedRepository};
use slim_db::repo::grid::SqliteGridRepo;
use slim_db::repo::led::SqliteLedRepo;
use time::OffsetDateTime;

use crate::cli::{
    LedClearArgs, LedCommands, LedEvaluateArgs, LedLocateArgs, LedSimulatePresenceArgs,
    LedStatusArgs,
};

/// Handle `slim led` subcommands.
pub fn handle_led(conn: &Connection, cmd: LedCommands, json: bool) -> Result<()> {
    let repo = SqliteLedRepo::new(conn);
    let grid_repo = SqliteGridRepo::new(conn);
    let now = OffsetDateTime::now_utc();

    // Clean expired states on any command execution
    let _ = repo.clear_expired_leds(now);

    match cmd {
        LedCommands::Status(LedStatusArgs { base }) => {
            let base_id = if let Some(ref b) = base {
                let gb = grid_repo
                    .get_base_by_code(b)?
                    .ok_or_else(|| anyhow!("Gridfinity baseplate '{}' not found", b))?;
                Some(gb.id)
            } else {
                None
            };

            let states = repo.list_dock_leds(base_id.as_ref())?;

            if json {
                println!("{}", serde_json::to_string_pretty(&states)?);
            } else if states.is_empty() {
                println!("No active LED states. (All dock LEDs idle / off)");
            } else {
                println!(
                    "{:<18} {:<12} {:<16} {:<24} Expiration",
                    "Dock ID", "Mode", "RGB Color", "Reason"
                );
                println!("{}", "─".repeat(84));
                for st in &states {
                    let rgb_str = format!("[{}, {}, {}]", st.r, st.g, st.b);
                    let exp_str = match st.expires_at {
                        Some(exp) => {
                            let rem = (exp - now).whole_seconds();
                            if rem > 0 {
                                format!("in {}s", rem)
                            } else {
                                "expired".to_string()
                            }
                        }
                        None => "indefinite".to_string(),
                    };
                    let reason_str = st.reason.as_deref().unwrap_or("─");

                    println!(
                        "{:<18} {:<12} {:<16} {:<24} {}",
                        st.dock_id, st.mode, rgb_str, reason_str, exp_str
                    );
                }
            }
        }

        LedCommands::Locate(LedLocateArgs { target, ttl, clear }) => {
            if clear {
                // Clear any active locate for target
                let states = repo.list_dock_leds(None)?;
                let mut cleared = 0;
                for st in states {
                    if st.mode == LedMode::Locate
                        && st
                            .reason
                            .as_deref()
                            .map(|r| r.contains(&target))
                            .unwrap_or(false)
                    {
                        repo.clear_dock_led(&st.dock_id)?;
                        cleared += 1;
                    }
                }
                if json {
                    println!("{{\"cleared\": {cleared}}}");
                } else {
                    println!("Cleared {cleared} locate indicator(s) for '{target}'.");
                }
            } else {
                let states = repo.locate_target(&target, ttl, now)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&states)?);
                } else {
                    let duration = ttl.unwrap_or(DEFAULT_LOCATE_TTL_SECS);
                    println!(
                        "💡 Illuminating physical dock LEDs for '{}' (TTL: {}s):",
                        target, duration
                    );
                    for st in &states {
                        println!(
                            "  - Dock {} [X: {}, Y: {}, RGB: {:?}]",
                            st.dock_id,
                            st.x,
                            st.y,
                            st.rgb()
                        );
                    }
                }
            }
        }

        LedCommands::Clear(LedClearArgs { dock, base, all }) => {
            if let Some(ref d) = dock {
                repo.clear_dock_led(d)?;
                if json {
                    println!("{{\"cleared_dock\": \"{d}\"}}");
                } else {
                    println!("Cleared LED state for dock '{d}'.");
                }
            } else if let Some(ref b) = base {
                let gb = grid_repo
                    .get_base_by_code(b)?
                    .ok_or_else(|| anyhow!("Gridfinity baseplate '{}' not found", b))?;
                let states = repo.list_dock_leds(Some(&gb.id))?;
                for st in &states {
                    repo.clear_dock_led(&st.dock_id)?;
                }
                if json {
                    println!("{{\"cleared_base\": \"{b}\", \"count\": {}}}", states.len());
                } else {
                    println!(
                        "Cleared {} dock LED state(s) on baseplate '{b}'.",
                        states.len()
                    );
                }
            } else if all {
                let states = repo.list_dock_leds(None)?;
                for st in &states {
                    repo.clear_dock_led(&st.dock_id)?;
                }
                if json {
                    println!("{{\"cleared_all\": true, \"count\": {}}}", states.len());
                } else {
                    println!("Cleared all {} active dock LED states.", states.len());
                }
            } else {
                bail!("Specify --dock <DOCK_ID>, --base <BASE_CODE>, or --all");
            }
        }

        LedCommands::SimulatePresence(LedSimulatePresenceArgs {
            dock_id,
            occupied,
            status,
        }) => {
            let (base_code, x, y) = parse_dock_id(&dock_id)
                .ok_or_else(|| anyhow!("Invalid dock ID '{}'. Expected 'BASE:X:Y'", dock_id))?;

            let base = grid_repo
                .get_base_by_code(&base_code)?
                .ok_or_else(|| anyhow!("Baseplate '{}' not found", base_code))?;

            let presence = DockPresenceState {
                dock_id: dock_id.clone(),
                base_id: base.id,
                x,
                y,
                occupied,
                last_seen_at: now,
                status: status.clone(),
            };

            repo.set_dock_presence(&presence)?;

            // Automatically re-evaluate baseplate LEDs so missing bins immediately illuminate
            let updated = repo.evaluate_base_leds(&base.id, now)?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "presence": presence,
                        "updated_leds": updated,
                    })
                );
            } else {
                let occ_str = if occupied { "OCCUPIED" } else { "EMPTY" };
                println!(
                    "Updated dock '{}' presence: {} (status: {})",
                    dock_id, occ_str, status
                );
                if !occupied || status == "missing" {
                    println!("⚠️ Raised Missing Bin warning on dock '{}'.", dock_id);
                }
            }
        }

        LedCommands::Evaluate(LedEvaluateArgs { base, serial }) => {
            let bases = if let Some(ref b) = base {
                let gb = grid_repo
                    .get_base_by_code(b)?
                    .ok_or_else(|| anyhow!("Baseplate '{}' not found", b))?;
                vec![gb]
            } else {
                grid_repo.list_bases(None)?
            };

            let mut all_states = Vec::new();
            for b in &bases {
                let states = repo.evaluate_base_leds(&b.id, now)?;
                all_states.extend(states);
            }

            if serial {
                // Emit versioned serial protocol frames (Version 1)
                for st in &all_states {
                    let frame = if st.mode == LedMode::Off {
                        LedProtocolFrame::clear_led(&st.dock_id)
                    } else {
                        let ttl_ms = st
                            .expires_at
                            .map(|exp| (exp - now).whole_milliseconds().max(0) as u64);
                        LedProtocolFrame::set_led(&st.dock_id, st.mode, st.rgb(), ttl_ms)
                    };
                    print!("{}", frame.to_json_line()?);
                }
            } else if json {
                println!("{}", serde_json::to_string_pretty(&all_states)?);
            } else {
                println!(
                    "Evaluated {} dock LED state(s) across {} baseplate(s):",
                    all_states.len(),
                    bases.len()
                );
                for st in &all_states {
                    println!(
                        "  - Dock {}: mode={}, RGB={:?}, reason={}",
                        st.dock_id,
                        st.mode,
                        st.rgb(),
                        st.reason.as_deref().unwrap_or("─")
                    );
                }
            }
        }
    }

    Ok(())
}
