use serde_json::{Map, Value, json};
use std::{
    env, fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const NOCTALIA_ASSETS: [(&str, &str); 4] = [
    (
        "manifest.json",
        include_str!("../assets/noctalia/manifest.json"),
    ),
    ("Main.qml", include_str!("../assets/noctalia/Main.qml")),
    (
        "BarWidget.qml",
        include_str!("../assets/noctalia/BarWidget.qml"),
    ),
    ("Panel.qml", include_str!("../assets/noctalia/Panel.qml")),
];
const HYPRLAND_LUA: &str = include_str!("../assets/hyprland/llm-meter.lua");
const HYPRLAND_CONF: &str = include_str!("../assets/hyprland/llm-meter.conf");

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum DesktopEnvironment {
    Hyprland,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum TopBar {
    Noctalia,
}

pub struct Options {
    pub desktop: Option<DesktopEnvironment>,
    pub bar: Option<TopBar>,
    pub non_interactive: bool,
    pub dry_run: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub active: bool,
}

pub fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    println!("LLM Meter desktop integration setup");
    println!("Currently supported: Hyprland + Noctalia\n");

    let desktop = choose_desktop(options.desktop, options.non_interactive)?;
    let bar = choose_bar(options.bar, options.non_interactive)?;
    println!("\nInstallation plan:");
    println!("  Desktop environment: {}", desktop.label());
    println!("  Top bar:             {}", bar.label());
    println!("  Daemon:              systemd user service (when llm-meterd is on PATH)");

    preflight(desktop, bar)?;

    if options.dry_run {
        println!("\nDry run complete; no files were changed.");
        return Ok(());
    }
    if !options.non_interactive && !confirm("Continue with this installation?", true)? {
        println!("Installation cancelled.");
        return Ok(());
    }

    match desktop {
        DesktopEnvironment::Hyprland => install_hyprland()?,
    }
    match bar {
        TopBar::Noctalia => install_noctalia()?,
    }
    set_autostart(true)?;

    println!("\nLLM Meter integration is installed.");
    println!("Open the Noctalia bar item to connect an account and choose displayed fields.");
    Ok(())
}

pub fn autostart_status() -> AutostartStatus {
    AutostartStatus {
        enabled: command_success(
            "systemctl",
            &["--user", "is-enabled", "--quiet", "llm-meterd.service"],
        ),
        active: command_success(
            "systemctl",
            &["--user", "is-active", "--quiet", "llm-meterd.service"],
        ),
    }
}

pub fn set_autostart(enabled: bool) -> Result<AutostartStatus, Box<dyn std::error::Error>> {
    if enabled {
        write_daemon_service()?;
        require_command(
            "systemctl",
            &["--user", "daemon-reload"],
            "could not reload the systemd user manager",
        )?;
        require_command(
            "systemctl",
            &["--user", "enable", "--now", "llm-meterd.service"],
            "could not enable the llm-meterd user service",
        )?;
    } else {
        let current = autostart_status();
        if current.enabled || current.active {
            require_command(
                "systemctl",
                &["--user", "disable", "--now", "llm-meterd.service"],
                "could not disable the llm-meterd user service",
            )?;
        }
    }
    Ok(autostart_status())
}

/// Run from the newly installed CLI after `cargo install` replaces the current
/// executable. Existing integration choices are refreshed without changing the
/// user's autostart preference.
pub fn post_update() -> Result<(), Box<dyn std::error::Error>> {
    let before = autostart_status();
    let hyprland = hyprland_config_home()?;
    if hyprland.join("hyprland.lua").is_file() || hyprland.join("hyprland.conf").is_file() {
        install_hyprland()?;
    }
    if noctalia_config_home()?.join("settings.json").is_file() {
        install_noctalia()?;
    }

    write_daemon_service()?;
    require_command(
        "systemctl",
        &["--user", "daemon-reload"],
        "could not reload the systemd user manager",
    )?;
    if before.active {
        require_command(
            "systemctl",
            &["--user", "restart", "llm-meterd.service"],
            "the package was updated, but the daemon could not be restarted",
        )?;
    }
    println!("Updated desktop integration and daemon service definition.");
    Ok(())
}

fn preflight(desktop: DesktopEnvironment, bar: TopBar) -> Result<(), Box<dyn std::error::Error>> {
    match desktop {
        DesktopEnvironment::Hyprland => {
            let config = hyprland_config_home()?;
            if !config.join("hyprland.lua").is_file() && !config.join("hyprland.conf").is_file() {
                return Err(format!(
                    "no hyprland.lua or hyprland.conf was found under {}",
                    config.display()
                )
                .into());
            }
        }
    }
    match bar {
        TopBar::Noctalia => {
            let settings = noctalia_config_home()?.join("settings.json");
            if !settings.is_file() {
                return Err(format!(
                    "Noctalia settings were not found at {}; start Noctalia once, then rerun setup",
                    settings.display()
                )
                .into());
            }
        }
    }
    Ok(())
}

impl DesktopEnvironment {
    fn label(self) -> &'static str {
        match self {
            Self::Hyprland => "Hyprland",
        }
    }
}

impl TopBar {
    fn label(self) -> &'static str {
        match self {
            Self::Noctalia => "Noctalia",
        }
    }
}

fn choose_desktop(
    selected: Option<DesktopEnvironment>,
    non_interactive: bool,
) -> Result<DesktopEnvironment, Box<dyn std::error::Error>> {
    if let Some(value) = selected {
        return Ok(value);
    }
    if non_interactive {
        return Err("--desktop is required with --non-interactive".into());
    }
    println!("Select your desktop environment:");
    println!("  1) Hyprland (supported)");
    println!("  0) Cancel");
    choose_one("Desktop [1]: ", DesktopEnvironment::Hyprland)
}

fn choose_bar(
    selected: Option<TopBar>,
    non_interactive: bool,
) -> Result<TopBar, Box<dyn std::error::Error>> {
    if let Some(value) = selected {
        return Ok(value);
    }
    if non_interactive {
        return Err("--bar is required with --non-interactive".into());
    }
    println!("\nSelect your top bar:");
    println!("  1) Noctalia (supported)");
    println!("  0) Cancel");
    choose_one("Top bar [1]: ", TopBar::Noctalia)
}

fn choose_one<T: Copy>(prompt: &str, only: T) -> Result<T, Box<dyn std::error::Error>> {
    loop {
        let answer = read_line(prompt)?;
        match answer.trim() {
            "" | "1" => return Ok(only),
            "0" | "q" | "Q" => return Err("installation cancelled".into()),
            _ => eprintln!("Please enter 1, or 0 to cancel."),
        }
    }
}

fn confirm(prompt: &str, default: bool) -> io::Result<bool> {
    let suffix = if default { " [Y/n]: " } else { " [y/N]: " };
    let answer = read_line(&format!("{prompt}{suffix}"))?;
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" => true,
        _ => false,
    })
}

fn read_line(prompt: &str) -> io::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    if io::stdin().read_line(&mut value)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "interactive input ended; use --non-interactive with explicit choices",
        ));
    }
    Ok(value)
}

fn install_noctalia() -> Result<(), Box<dyn std::error::Error>> {
    let config = noctalia_config_home()?;
    let settings_path = config.join("settings.json");
    let plugin_dir = config.join("plugins/llm-meter");
    let plugins_path = config.join("plugins.json");
    let mut settings = read_json_object(&settings_path)?;
    let mut plugins = if plugins_path.is_file() {
        read_json_object(&plugins_path)?
    } else {
        json!({"version": 2, "sources": [], "states": {}})
    };
    let was_installed = plugin_dir.join("manifest.json").is_file()
        && settings["bar"]["widgets"]["right"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.get("id").and_then(Value::as_str) == Some("plugin:llm-meter"))
            })
        && plugins["states"]["llm-meter"]["enabled"].as_bool() == Some(true);

    fs::create_dir_all(&plugin_dir)?;
    let cli = env::current_exe()?.to_string_lossy().into_owned();
    let cli_qml = serde_json::to_string(&cli)?;
    let mut assets_changed = false;
    for (name, source) in NOCTALIA_ASSETS {
        let rendered = if name == "BarWidget.qml" {
            source
                .replace(
                    "Quickshell.env(\"HOME\") + \"/.local/bin/llm-meter-desktop\", \"--main\"",
                    &format!("{cli_qml}, \"ui\", \"--main\""),
                )
                .replace(
                    "Quickshell.env(\"HOME\") + \"/.local/bin/llm-meter\"",
                    &cli_qml,
                )
        } else {
            source.to_owned()
        };
        assets_changed |= write_if_changed(&plugin_dir.join(name), rendered.as_bytes(), 0o644)?;
    }

    let right = ensure_array_path(&mut settings, &["bar", "widgets", "right"])?;
    let existing = right
        .iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some("plugin:llm-meter"))
        .cloned()
        .unwrap_or_else(|| json!({"id": "plugin:llm-meter"}));
    right.retain(|item| {
        item.get("id").and_then(Value::as_str) != Some("plugin:llm-meter")
            && item.get("ipcIdentifier").and_then(Value::as_str) != Some("llm-meter")
    });
    right.insert(0, existing);
    write_json(&settings_path, &settings)?;

    ensure_object_path(&mut plugins, &["states"])?
        .insert("llm-meter".into(), json!({"enabled": true}));
    write_json(&plugins_path, &plugins)?;

    if process_running("^qs -c noctalia-shell$") {
        if !was_installed && restart_noctalia() {
            println!("Restarted Noctalia once to activate the new LLM Meter plugin.");
        } else if assets_changed
            && command_success(
                "qs",
                &[
                    "ipc",
                    "-c",
                    "noctalia-shell",
                    "call",
                    "llmMeterDev",
                    "reload",
                ],
            )
        {
            println!("Hot-reloaded the Noctalia LLM Meter plugin.");
        } else if assets_changed {
            println!("Noctalia is running; restart it once to load the newly installed plugin.");
        } else {
            println!("Noctalia LLM Meter plugin is already up to date.");
        }
    } else {
        println!("Installed the Noctalia plugin; start Noctalia to display it.");
    }
    Ok(())
}

fn install_hyprland() -> Result<(), Box<dyn std::error::Error>> {
    let config = hyprland_config_home()?;
    let lua_root = config.join("hyprland.lua");
    let conf_root = config.join("hyprland.conf");
    if lua_root.is_file() {
        let target = config.join("config/llm-meter.lua");
        write_if_changed(&target, HYPRLAND_LUA.as_bytes(), 0o644)?;
        append_once(&lua_root, "require(\"config.llm-meter\")")?;
        println!("Installed Hyprland Lua rules at {}.", target.display());
    } else if conf_root.is_file() {
        let target = config.join("llm-meter.conf");
        write_if_changed(&target, HYPRLAND_CONF.as_bytes(), 0o644)?;
        append_once(&conf_root, "source = ~/.config/hypr/llm-meter.conf")?;
        println!("Installed Hyprland rules at {}.", target.display());
    } else {
        return Err(format!(
            "no hyprland.lua or hyprland.conf was found under {}",
            config.display()
        )
        .into());
    }
    if command_success("hyprctl", &["reload"]) {
        println!("Reloaded Hyprland.");
    } else {
        println!("Hyprland is not running; the rules will load at its next start.");
    }
    Ok(())
}

fn write_daemon_service() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let sibling_daemon = env::current_exe().ok().and_then(|executable| {
        let daemon = executable.parent()?.join("llm-meterd");
        daemon.is_file().then_some(daemon)
    });
    let Some(daemon) = sibling_daemon.or_else(|| find_command("llm-meterd")) else {
        return Err(
            "llm-meterd is not on PATH; reinstall with `cargo install --locked llm-meter-cli`"
                .into(),
        );
    };
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().expect("home already resolved").join(".config"));
    let service_path = config_home.join("systemd/user/llm-meterd.service");
    let service = format!(
        "[Unit]\nDescription=LLM Meter daemon\n\n[Service]\nType=simple\nExecStart={}\nRestart=on-failure\nRestartSec=3\nRuntimeDirectory=llm-meter\nRuntimeDirectoryMode=0700\nUMask=0077\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(&daemon)
    );
    write_if_changed(&service_path, service.as_bytes(), 0o644)?;
    Ok(service_path)
}

fn read_json_object(path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    if !value.is_object() {
        return Err(format!("{} must contain a JSON object", path.display()).into());
    }
    Ok(value)
}

fn ensure_object_path<'a>(
    root: &'a mut Value,
    path: &[&str],
) -> Result<&'a mut Map<String, Value>, Box<dyn std::error::Error>> {
    let mut value = root;
    for key in path {
        let object = value
            .as_object_mut()
            .ok_or_else(|| format!("configuration field before {key} is not an object"))?;
        value = object
            .entry((*key).to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    value
        .as_object_mut()
        .ok_or_else(|| "configuration field is not an object".into())
}

fn ensure_array_path<'a>(
    root: &'a mut Value,
    path: &[&str],
) -> Result<&'a mut Vec<Value>, Box<dyn std::error::Error>> {
    let (last, parents) = path.split_last().ok_or("array path cannot be empty")?;
    let parent = ensure_object_path(root, parents)?;
    parent
        .entry((*last).to_owned())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| format!("configuration field {last} is not an array").into())
}

fn write_json(path: &Path, value: &Value) -> Result<bool, Box<dyn std::error::Error>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(write_if_changed(path, &bytes, 0o600)?)
}

fn write_if_changed(path: &Path, bytes: &[u8], mode: u32) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("llm-meter-{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
    fs::rename(temporary, path)?;
    Ok(true)
}

fn append_once(path: &Path, line: &str) -> io::Result<()> {
    let mut content = fs::read_to_string(path)?;
    if content.lines().any(|existing| existing.trim() == line) {
        return Ok(());
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    content.push_str(line);
    content.push('\n');
    let mode = fs::metadata(path)?.permissions().mode();
    write_if_changed(path, content.as_bytes(), mode)?;
    Ok(())
}

fn find_command(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|path| path.join(name))
        .find(|path| path.is_file())
}

fn home_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn process_running(pattern: &str) -> bool {
    command_success("pgrep", &["-f", pattern])
}

fn restart_noctalia() -> bool {
    if !command_success("qs", &["kill", "-c", "noctalia-shell"]) {
        return false;
    }
    if find_command("hyprctl").is_some() {
        command_success("hyprctl", &["dispatch", "exec", "qs -c noctalia-shell"])
    } else {
        command_success(
            "systemd-run",
            &[
                "--user",
                "--quiet",
                "--collect",
                "--unit=noctalia-shell",
                "qs",
                "-c",
                "noctalia-shell",
            ],
        )
    }
}

fn noctalia_config_home() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(env::var_os("NOCTALIA_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config/noctalia")))
}

fn hyprland_config_home() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(env::var_os("HYPR_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config/hypr")))
}

fn command_success(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn require_command(
    program: &str,
    args: &[&str],
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if command_success(program, args) {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn systemd_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_nested_noctalia_widget_array() {
        let mut root = json!({});
        ensure_array_path(&mut root, &["bar", "widgets", "right"])
            .unwrap()
            .push(json!({"id": "plugin:llm-meter"}));
        assert_eq!(root["bar"]["widgets"]["right"][0]["id"], "plugin:llm-meter");
    }

    #[test]
    fn systemd_paths_are_quoted() {
        assert_eq!(
            systemd_quote(Path::new("/home/a user/.cargo/bin/llm-meterd")),
            "\"/home/a user/.cargo/bin/llm-meterd\""
        );
    }
}
