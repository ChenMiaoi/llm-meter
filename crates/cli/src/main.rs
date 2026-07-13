use clap::{Parser, Subcommand};
use llm_meter_daemon::{
    ipc::{Client, call},
    socket_path,
};
use llm_meter_secret_store::NativeSecretStore;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    time::Duration,
};

mod setup;
mod update;

#[derive(Parser)]
#[command(
    name = "llm-meter",
    version,
    about = "Local-first LLM usage and quota monitor"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Interactively install desktop and top-bar integration.
    #[command(alias = "install")]
    Setup {
        /// Desktop environment to integrate with.
        #[arg(long, value_enum)]
        desktop: Option<setup::DesktopEnvironment>,
        /// Top bar to integrate with.
        #[arg(long, value_enum)]
        bar: Option<setup::TopBar>,
        /// Do not prompt; both --desktop and --bar are required.
        #[arg(long)]
        non_interactive: bool,
        /// Show the selected installation plan without changing files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Update to the latest stable crates.io release and refresh integrations.
    Update,
    /// Inspect or change daemon startup with the user session.
    Autostart {
        #[arg(value_enum)]
        action: AutostartAction,
    },
    #[command(name = "_post-update", hide = true)]
    PostUpdate,
    Status,
    Connections,
    Diagnostics,
    Refresh {
        connection_id: String,
    },
    /// Refresh every configured connection and the local Codex collector.
    RefreshAll,
    Remove {
        connection_id: String,
    },
    Add {
        #[arg(value_enum)]
        kind: AddKind,
        #[arg(long)]
        device: bool,
        /// Open the provider login page in the default browser.
        #[arg(long)]
        open: bool,
        /// Provider to add a connection for.
        #[arg(long, value_enum, default_value_t = Provider::Openai)]
        provider: Provider,
        #[arg(long, default_value = "OpenAI")]
        name: String,
        /// Read the secret credential from one line on standard input.
        #[arg(long)]
        secret_stdin: bool,
    },
    Budget {
        connection_id: String,
        amount: String,
        #[arg(long, default_value = "USD")]
        currency: String,
    },
    Waybar {
        #[arg(long)]
        watch: bool,
    },
    Ui {
        #[arg(long,conflicts_with_all=["hide","main"])]
        toggle: bool,
        #[arg(long)]
        hide: bool,
        #[arg(long)]
        main: bool,
    },
}
#[derive(Clone, Copy, clap::ValueEnum, Default)]
enum Provider {
    #[default]
    Openai,
    Kimi,
}

#[derive(Clone, clap::ValueEnum)]
enum AddKind {
    Subscription,
    Admin,
    Standard,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum AutostartAction {
    Status,
    Enable,
    Disable,
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("llm-meter: {e}");
        std::process::exit(1)
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let a = Args::parse();
    if let Command::Setup {
        desktop,
        bar,
        non_interactive,
        dry_run,
    } = &a.command
    {
        return setup::run(setup::Options {
            desktop: *desktop,
            bar: *bar,
            non_interactive: *non_interactive,
            dry_run: *dry_run,
        });
    }
    match &a.command {
        Command::Update => return update::run(),
        Command::PostUpdate => return setup::post_update(),
        Command::Autostart { action } => {
            let status = match action {
                AutostartAction::Status => setup::autostart_status(),
                AutostartAction::Enable => setup::set_autostart(true)?,
                AutostartAction::Disable => setup::set_autostart(false)?,
            };
            print(serde_json::to_value(status)?);
            return Ok(());
        }
        _ => {}
    }
    let p = socket_path()?;
    match a.command {
        Command::Setup { .. } => unreachable!(),
        Command::Update | Command::PostUpdate | Command::Autostart { .. } => unreachable!(),
        Command::Status => print(call(&p, "snapshot/get", json!({})).await?),
        Command::Connections => print(call(&p, "connections/list", json!({})).await?),
        Command::Diagnostics => {
            let version = call(&p, "system/version", json!({})).await;
            let health = call(&p, "system/health", json!({})).await;
            let providers = call(&p, "providers/list", json!({})).await;
            let connections = call(&p, "connections/list", json!({})).await;
            let session_type =
                std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
            let socket_state = match std::fs::metadata(&p) {
                Ok(meta) if std::os::unix::fs::FileTypeExt::is_socket(&meta.file_type()) => {
                    "available"
                }
                Ok(_) => "not-a-socket",
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "permission-denied",
                Err(_) => "missing",
            };
            let user_bus_names =
                command_line("busctl", &["--user", "--no-pager", "--list"]).unwrap_or_default();
            let notifications = user_bus_names.contains("org.freedesktop.Notifications");
            let portal = user_bus_names.contains("org.freedesktop.portal.Desktop");
            let secret_service = NativeSecretStore.availability().await;
            let mut hints = Vec::new();
            if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
                hints.push("Import the graphical session D-Bus environment (UWSM is recommended on Hyprland).");
            }
            if secret_service != "available" {
                hints.push("Start and unlock a Secret Service provider such as GNOME Keyring or KeePassXC.");
            }
            if !notifications {
                hints.push("Start a notification daemon such as mako, dunst, fnott, or swaync.");
            }
            if !portal {
                hints.push(
                    "Install and start xdg-desktop-portal-hyprland plus xdg-desktop-portal-gtk.",
                );
            }
            print(json!({
                "version":version.ok(),"health":health.ok(),"os":std::env::consts::OS,
                "providers":providers.ok(),"connections":connections.ok(),
                "session":{"type":session_type,"desktop":desktop,"wayland_display":std::env::var_os("WAYLAND_DISPLAY").is_some(),"dbus":std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()},
                "hyprland":{"present":std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some(),"version":command_line("hyprctl", &["version"]).and_then(|s|s.lines().next().map(str::to_owned))},
                "daemon":{"service_active":command_success("systemctl", &["--user","is-active","llm-meterd.service"]),"socket":socket_state},
                "secret_service":{"state":secret_service},
                "notification_service":{"available":notifications},
                "portal":{"desktop":portal,"hyprland_service":command_success("systemctl", &["--user","is-active","xdg-desktop-portal-hyprland.service"])},
                "webkitgtk":{"version":command_line("pkg-config", &["--modversion","webkit2gtk-4.1"])},
                "window_identity":{"expected":"io.github.llmmeter"},
                "hints":hints
            }))
        }
        Command::Refresh { connection_id } => {
            let _: uuid::Uuid = connection_id.parse().map_err(|_| "invalid connection id")?;
            print(
                call(
                    &p,
                    "connections/refresh",
                    json!({"connection_id":connection_id}),
                )
                .await?,
            );
        }
        Command::RefreshAll => {
            let result = call(&p, "connections/refresh-all", json!({})).await?;
            let failed = result
                .get("failed")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            print(result);
            if failed > 0 {
                return Err(format!("{failed} connection(s) could not be refreshed").into());
            }
        }
        Command::Remove { connection_id } => {
            let _: uuid::Uuid = connection_id.parse().map_err(|_| "invalid connection id")?;
            print(
                call(
                    &p,
                    "connections/remove",
                    json!({"connection_id":connection_id}),
                )
                .await?,
            );
        }
        Command::Add {
            kind,
            device,
            open,
            provider,
            name,
            secret_stdin,
        } => match kind {
            AddKind::Subscription => {
                let (provider_id, connection_type, auth) = match provider {
                    Provider::Kimi => ("kimi", "kimi_code_subscription", "oauth_device_code"),
                    Provider::Openai => {
                        if device {
                            ("openai", "chatgpt_subscription", "oauth_device_code")
                        } else {
                            ("openai", "chatgpt_subscription", "oauth_browser")
                        }
                    }
                };
                let challenge = call(
                    &p,
                    "connections/add",
                    json!({"provider_id": provider_id, "connection_type": connection_type, "display_name": name, "auth_scheme": auth}),
                )
                .await?;
                let challenge_id = challenge
                    .get("state")
                    .and_then(Value::as_str)
                    .ok_or("daemon did not return login state")?;
                if let Some(url) = challenge.get("auth_url").and_then(Value::as_str) {
                    eprintln!("Open this URL to authenticate:\n{url}");
                    if open && !command_success("xdg-open", &[url]) {
                        return Err("could not open the login URL with xdg-open".into());
                    }
                }
                if let Some(url) = challenge.get("verification_url").and_then(Value::as_str) {
                    eprintln!(
                        "Open {url} and enter code {}",
                        challenge
                            .get("user_code")
                            .and_then(Value::as_str)
                            .unwrap_or("<missing>")
                    );
                    if open && !command_success("xdg-open", &[url]) {
                        return Err("could not open the verification URL with xdg-open".into());
                    }
                }
                print(
                    call(
                        &p,
                        "connections/auth/complete",
                        json!({"challenge_id":challenge_id}),
                    )
                    .await?,
                );
            }
            AddKind::Admin => {
                let challenge=call(&p,"connections/add",json!({"provider_id":"openai","connection_type":"platform_admin","display_name":name,"auth_scheme":"admin_api_key"})).await?;
                let challenge_id = challenge
                    .get("challenge_id")
                    .and_then(Value::as_str)
                    .ok_or("daemon did not return challenge id")?;
                let secret = read_secret(secret_stdin, "OpenAI Admin API Key: ")?;
                let result = call(
                    &p,
                    "connections/auth/complete",
                    json!({"challenge_id":challenge_id,"secret":secret}),
                )
                .await;
                drop(secret);
                print(result?);
            }
            AddKind::Standard => {
                let challenge=call(&p,"connections/add",json!({"provider_id":"openai","connection_type":"platform_standard","display_name":name,"auth_scheme":"api_key"})).await?;
                let challenge_id = challenge
                    .get("challenge_id")
                    .and_then(Value::as_str)
                    .ok_or("daemon did not return challenge id")?;
                let secret = read_secret(secret_stdin, "OpenAI API Key: ")?;
                let result = call(
                    &p,
                    "connections/auth/complete",
                    json!({"challenge_id":challenge_id,"secret":secret}),
                )
                .await;
                drop(secret);
                print(result?);
            }
        },
        Command::Budget {
            connection_id,
            amount,
            currency,
        } => {
            let _: uuid::Uuid = connection_id.parse().map_err(|_| "invalid connection id")?;
            let _: rust_decimal::Decimal = amount.parse().map_err(|_| "invalid decimal amount")?;
            print(call(&p,"budgets/set",json!({"id":uuid::Uuid::new_v4(),"connection_id":connection_id,"amount":amount,"currency":currency.to_uppercase(),"period":"monthly","warning_ratio":"0.8","critical_ratio":"1","enabled":true})).await?);
        }
        Command::Waybar { watch } => {
            let mut client = None;
            loop {
                if client.is_none() {
                    client = Client::connect(&p).await.ok();
                }
                let result = match client.as_mut() {
                    Some(value) => value.call("waybar/render", json!({})).await,
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        "daemon offline",
                    )),
                };
                match result {
                    Ok(v) => println!("{}", serde_json::to_string(&v)?),
                    Err(_) => {
                        client = None;
                        println!(
                            "{}",
                            json!({"text":"LLM offline","tooltip":"llm-meter daemon is unavailable","class":["daemon-offline"]})
                        )
                    }
                }
                std::io::stdout().flush()?;
                if !watch {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        Command::Ui { toggle, hide, main } => {
            let action = if hide {
                "hide"
            } else if main {
                "main"
            } else if toggle {
                "toggle"
            } else {
                "show"
            };
            std::process::Command::new("llm-meter-desktop")
                .arg(format!("--{action}"))
                .spawn()
                .map_err(|error| {
                    format!("could not activate llm-meter-desktop ({action}): {error}")
                })?;
        }
    }
    Ok(())
}
fn read_secret(from_stdin: bool, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    if !from_stdin {
        return Ok(rpassword::prompt_password(prompt)?);
    }
    read_secret_line(std::io::stdin().lock())
}

fn read_secret_line(mut reader: impl BufRead) -> Result<String, Box<dyn std::error::Error>> {
    let mut secret = String::new();
    reader.read_line(&mut secret)?;
    while matches!(secret.chars().last(), Some('\n' | '\r')) {
        secret.pop();
    }
    if secret.is_empty() {
        return Err("standard input did not contain a secret".into());
    }
    Ok(secret)
}

fn print(v: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| "null".into())
    )
}
fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}
fn command_success(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|v| v.status.success())
}

#[cfg(test)]
mod tests {
    use super::read_secret_line;

    #[test]
    fn reads_secret_from_one_line_without_line_ending() {
        let secret = read_secret_line("api-secret\r\ntrailing".as_bytes()).unwrap();
        assert_eq!(secret, "api-secret");
    }

    #[test]
    fn rejects_empty_standard_input_secret() {
        assert!(read_secret_line("\n".as_bytes()).is_err());
    }
}
