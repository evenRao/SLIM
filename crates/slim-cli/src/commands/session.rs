//! Authentication session subcommands (login, badge, whoami, logout).

use anyhow::Result;
use rusqlite::Connection;
use slim_core::service::AuthService;
use slim_db::repo::auth::SqliteAuthService;

use crate::cli::SessionCommands;
use crate::config::Config;

pub fn handle_session(conn: &Connection, cmd: SessionCommands, json: bool) -> Result<()> {
    let auth = SqliteAuthService::new(conn);

    match cmd {
        SessionCommands::Login { name, pin } => {
            let final_pin = match pin {
                Some(p) => p,
                None => {
                    print!("Enter PIN for {}: ", name);
                    use std::io::{self, Write};
                    io::stdout().flush()?;
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    input.trim().to_string()
                }
            };

            let session = auth.login_with_pin(&name, &final_pin)?;
            Config::save_session_token(&session.token)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&session)?);
            } else {
                println!("Successfully authenticated as '{}'", name);
                println!("  Session Token: {}", session.token);
                println!("  Expires at:    {}", session.expires_at);
            }
        }
        SessionCommands::Badge { token } => {
            let session = auth.login_with_badge(&token)?;
            Config::save_session_token(&session.token)?;

            let validated = auth.validate_session(&session.token)?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "session": session,
                        "actor": validated.actor,
                    })
                );
            } else {
                println!(
                    "Badge recognized! Logged in as '{}' [{}]",
                    validated.actor.display_name, validated.actor.role
                );
                println!("  Session Token: {}", session.token);
                println!("  Expires at:    {}", session.expires_at);
            }
        }
        SessionCommands::Whoami => {
            let token = Config::load_session_token();
            match token {
                Some(t) => match auth.validate_session(&t) {
                    Ok(auth_session) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "authenticated": true,
                                    "actor": auth_session.actor,
                                    "session": auth_session.session,
                                })
                            );
                        } else {
                            println!(
                                "Active Session: {} [{}]",
                                auth_session.actor.display_name, auth_session.actor.role
                            );
                            println!("  Actor ID:    {}", auth_session.actor.id);
                            println!("  Role:        {}", auth_session.actor.role);
                            println!("  Expires at:  {}", auth_session.session.expires_at);
                        }
                    }
                    Err(_) => {
                        let _ = Config::clear_session_token();
                        if json {
                            println!("{}", serde_json::json!({ "authenticated": false }));
                        } else {
                            println!(
                                "Previous session expired. Please log in again with 'slim session login'."
                            );
                        }
                    }
                },
                None => {
                    // Fall back to resolved actor without an explicit session
                    let fallback = crate::commands::resolve::resolve_actor(conn, None, None);
                    match fallback {
                        Ok(actor) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::json!({
                                        "authenticated": false,
                                        "fallback_actor": actor,
                                    })
                                );
                            } else {
                                println!(
                                    "No active login session. Default actor: {} [{}]",
                                    actor.display_name, actor.role
                                );
                            }
                        }
                        Err(_) => {
                            if json {
                                println!("{}", serde_json::json!({ "authenticated": false }));
                            } else {
                                println!("No active login session and no default actor found.");
                            }
                        }
                    }
                }
            }
        }
        SessionCommands::Logout => {
            if let Some(token) = Config::load_session_token() {
                let _ = auth.terminate_session(&token);
                Config::clear_session_token()?;
                if json {
                    println!("{}", serde_json::json!({ "logged_out": true }));
                } else {
                    println!("Session logged out and cleared successfully.");
                }
            } else if json {
                println!("{}", serde_json::json!({ "logged_out": false }));
            } else {
                println!("No active session found.");
            }
        }
    }

    Ok(())
}
