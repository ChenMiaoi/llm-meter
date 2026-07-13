# LLM Meter

[![llm-meter-cli on crates.io](https://img.shields.io/crates/v/llm-meter-cli.svg?label=llm-meter-cli)](https://crates.io/crates/llm-meter-cli)
[![llm-meter-daemon on crates.io](https://img.shields.io/crates/v/llm-meter-daemon.svg?label=llm-meter-daemon)](https://crates.io/crates/llm-meter-daemon)
[![crates.io downloads](https://img.shields.io/crates/d/llm-meter-cli.svg?label=downloads)](https://crates.io/crates/llm-meter-cli)
[![license](https://img.shields.io/crates/l/llm-meter-cli.svg)](https://crates.io/crates/llm-meter-cli)

LLM Meter is a local-first usage, cost, and quota monitor for LLM services on
Linux. It combines a Rust daemon, CLI, native Noctalia/Waybar integration, and
an optional Tauri desktop application.

The current provider implementation supports OpenAI Platform accounts and
ChatGPT subscriptions. LLM Meter can also discover locally running Codex
sessions, split their token usage by model, and calculate an API-equivalent
cost estimate.

## Features

- ChatGPT subscription quota, weekly reset time, and reset-credit expiry.
- Estimated quota exhaustion time based on recent local activity.
- OpenAI Platform usage and cost collection through supported API credentials.
- Automatic discovery of running Codex sessions every two seconds.
- Per-model input, cached-input, output, and total-token accounting.
- Local-day Token and API-equivalent cost totals that survive Codex exits and
  daemon restarts.
- Native Noctalia popup with overview, connection, login, budget, reset-credit,
  and local-Codex pages.
- Configurable top-bar fields: account, quota, today's Tokens, today's cost,
  active Codex count, and seven-day trend.
- Waybar JSON output, CLI diagnostics, SQLite history, alerts, and budgets.
- Secrets stored in the operating-system credential store rather than SQLite.

> [!IMPORTANT]
> ChatGPT subscriptions are not billed per Token. Local Codex costs shown by
> LLM Meter are estimates using equivalent OpenAI API text-token prices, not an
> additional charge or an invoice. Prices can change; the embedded table records
> its source and effective date.

## Architecture

```text
OpenAI / Codex logs
        │
        ▼
   llm-meterd ─── SQLite + system Keyring
        │ private Unix socket
        ├──────── llm-meter CLI
        ├──────── Noctalia / Waybar
        └──────── Tauri desktop
```

`llm-meterd` owns provider access, normalization, local Codex collection,
storage, and alerts. Frontends consume allowlisted snapshots over a private
Unix socket; credentials and raw secret material are not exposed through IPC.

## Requirements

- Linux with a user session and `XDG_RUNTIME_DIR`.
- Rust stable with edition 2024 support.
- A Secret Service implementation such as GNOME Keyring or KeePassXC for API
  keys and OAuth secrets.
- `jq` for automatic Noctalia/Waybar configuration.
- Noctalia/Quickshell or Waybar for top-bar integration.
- Hyprland is supported directly; the daemon and CLI are compositor-neutral.

Building the optional desktop application additionally requires the normal
Tauri v2 Linux dependencies, including WebKitGTK 4.1 and JavaScriptCoreGTK 4.1.

## Installation

### Stable release with Cargo (recommended)

For normal use, install the latest published stable release from crates.io.
This avoids tracking incomplete changes from the repository's `main` branch.
Install the Rust stable toolchain with [rustup](https://rustup.rs/) first if
`cargo` is not already available:

```bash
rustup toolchain install stable
rustup default stable
```

Then install the application package using its published lockfile:

```bash
cargo install --locked llm-meter-cli
llm-meter setup
```

Package pages:

- [`llm-meter-cli`](https://crates.io/crates/llm-meter-cli) installs both
  `llm-meter` and `llm-meterd`, including the deployment wizard;
- [`llm-meter-daemon`](https://crates.io/crates/llm-meter-daemon) is the reusable
  daemon runtime library pulled in automatically as a dependency. It does not
  need to be installed separately.

Cargo installs the binaries under `~/.cargo/bin` by default. If the shell cannot
find `llm-meter`, add that directory to `PATH` and start a new shell:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

To install a specific stable version instead of the newest one:

```bash
cargo install --locked --version 0.1.0 llm-meter-cli
```

Upgrade an existing Cargo installation with:

```bash
llm-meter update
```

The updater installs the latest stable crates.io release, refreshes the embedded
desktop integration, and restarts the daemon only when it was already running.
On older releases that do not yet provide `update`, run
`cargo install --locked --force llm-meter-cli` once.

Cargo intentionally does not run package post-install hooks, so the explicit
`setup` step is what safely asks before modifying desktop configuration. The
wizard presents separate desktop-environment and top-bar choices. The current
release supports **Hyprland + Noctalia**; more integrations can be added to the
same selector later.

For unattended deployment, make the choices explicit:

```bash
llm-meter setup --desktop hyprland --bar noctalia --non-interactive
```

Use `--dry-run` to inspect the plan without writing files. The setup command
installs the embedded Noctalia plugin, adds the Hyprland rule, reloads supported
components when possible, and installs the `llm-meterd` systemd user service.
Its user-session startup state can be changed from the Noctalia settings panel
or from the command line:

```bash
llm-meter autostart status
llm-meter autostart enable
llm-meter autostart disable
```

The optional Tauri desktop binary is also attached to versioned GitHub Releases;
the Cargo installation is sufficient for the daemon, CLI, and native Noctalia
experience.

### Development version from source

Cloning `main` installs the newest development state and may include unfinished
or breaking changes. Use this path for contributing, testing an unreleased fix,
or working on the desktop application:

```bash
git clone https://github.com/llm-meter/llm-meter.git
cd llm-meter
make install
```

Run `make run` for the normal foreground development workflow.

`make run` builds the workspace and desktop application, installs development
binaries under `~/.local/bin`, detects the active supported top bar, synchronizes
its integration, reloads only the LLM Meter Noctalia plugin when necessary, and
runs the development daemon in the foreground.

Useful development commands:

```bash
make run              # build, sync desktop/bar integration, run daemon
make daemon           # build and run only llm-meterd
make popup            # open the compact popup application
make gui              # open the full desktop application
make sync-noctalia    # install/hot-reload the native Noctalia plugin
make sync-waybar      # install/reload the Waybar integration
make check
make test
```

## Accounts and authentication

The native popup provides browser-based ChatGPT login and connection
management. Equivalent CLI commands are available:

```bash
llm-meter add subscription --open
llm-meter add subscription --device
llm-meter add admin
llm-meter add standard
```

- `subscription` uses the local Codex/OpenAI account flow.
- `admin` requests an OpenAI Admin API key.
- `standard` requests a standard OpenAI API key.

API keys and OAuth secrets go to the system Keyring. The SQLite database stores
only credential references and non-secret account state. The ChatGPT
subscription adapter may reuse authentication owned by the locally installed
Codex application instead of duplicating it.

## CLI

```bash
llm-meter status
llm-meter connections
llm-meter diagnostics
llm-meter refresh CONNECTION_ID
llm-meter refresh-all
llm-meter remove CONNECTION_ID
llm-meter budget CONNECTION_ID 20 --currency USD
llm-meter waybar
llm-meter waybar --watch
```

`refresh-all` waits for every configured provider connection to finish a real
network refresh and immediately refreshes the local Codex collector. Automatic
provider refresh defaults to five minutes for ChatGPT subscriptions and ten
minutes for Platform usage. Local Codex discovery runs every two seconds.

## Noctalia and top-bar settings

Click the LLM Meter pill to open the native translucent popup. Settings, login,
budgets, and top-bar customization are available without opening the full Tauri
GUI.

Under **Settings → Top bar display**, each of these values can be enabled or
disabled independently:

- account name;
- remaining weekly quota;
- today's local Codex Tokens;
- today's API-equivalent cost;
- active Codex session count;
- seven-day Token trend.

Settings are persisted by Noctalia and update the bar immediately. Source syncs
use plugin-only hot reload with a QML cache-buster; Noctalia itself is not
restarted when files are unchanged or when a plugin reload is sufficient.

## Local Codex accounting

Codex session logs are read incrementally from:

```text
~/.codex/sessions/YYYY/MM/DD/*.jsonl
```

LLM Meter detects open session files through `/proc`, records model changes, and
attributes cumulative-token deltas to the model active at that point. It also
reads the current and adjacent UTC day directories so today's local total does
not disappear after a Codex process exits. A new local calendar day starts at
zero according to the machine's timezone.

The original Codex JSONL files remain owned by Codex; LLM Meter does not copy
them into its database. Closed historical sessions outside the local-day window
are therefore not presented as an all-time LLM Meter ledger.

## Files and data

Persistent state is organized under `~/.llm-meter`:

```text
~/.llm-meter/
├── config.toml
├── data/
│   ├── llm-meter.sqlite3
│   ├── llm-meter.sqlite3-wal
│   └── llm-meter.sqlite3-shm
└── logs/
    └── llm-meterd.log
```

Set `LLM_METER_HOME` to use a different persistent root. On first launch, the
daemon migrates the earlier XDG data/config layout when the new destination is
empty.

Other locations intentionally remain separate:

- runtime socket: `$XDG_RUNTIME_DIR/llm-meter/daemon.sock`;
- sensitive credentials: operating-system Keyring/Secret Service;
- Codex-owned session and authentication state: `~/.codex`;
- Noctalia plugin installation: `~/.config/noctalia/plugins/llm-meter`.

Directories are created with mode `0700`; database, configuration, and daemon
log files use mode `0600`.

## Configuration

The daemon creates `~/.llm-meter/config.toml` with safe defaults:

```toml
scheduler_tick_seconds = 30
subscription_sync_seconds = 300
platform_usage_sync_seconds = 600
manual_refresh_min_seconds = 30
stale_after_seconds = 1800

[retention]
raw_days = 30
hourly_days = 180
provider_events_days = 30
```

See [`config.example.toml`](https://github.com/llm-meter/llm-meter/blob/main/config.example.toml).
Configuration files must not contain credentials.

## systemd and Hyprland

Packaging templates are provided under `packaging/`:

- `packaging/systemd/llm-meterd.service`;
- `packaging/systemd/llm-meter-desktop.service`;
- `packaging/hyprland/llm-meter.lua` and `.conf`;
- `packaging/noctalia/llm-meter/`;
- `packaging/waybar/`.

After installing the daemon unit:

```bash
systemctl --user daemon-reload
systemctl --user enable --now llm-meterd.service
journalctl --user -u llm-meterd.service -f
```

See the [Hyprland integration guide](https://github.com/llm-meter/llm-meter/blob/main/packaging/README-hyprland.md)
for manual packaging details.

## Development

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml
```

The Rust workspace contains:

| Package                                                                             | Purpose                                             |
| ----------------------------------------------------------------------------------- | --------------------------------------------------- |
| [`llm-meter-core`](https://crates.io/crates/llm-meter-core)                         | Provider-neutral domain types and adapter contracts |
| [`llm-meter-storage`](https://crates.io/crates/llm-meter-storage)                   | SQLite repository, migrations, and retention        |
| [`llm-meter-secret-store`](https://crates.io/crates/llm-meter-secret-store)         | Native Keyring-backed secret storage                |
| [`llm-meter-provider-openai`](https://crates.io/crates/llm-meter-provider-openai)   | OpenAI subscription and Platform adapters           |
| [`llm-meter-provider-testkit`](https://crates.io/crates/llm-meter-provider-testkit) | Mock provider and adapter test helpers              |
| [`llm-meter-daemon`](https://crates.io/crates/llm-meter-daemon)                     | Reusable scheduler, collectors, and IPC runtime     |
| [`llm-meter-cli`](https://crates.io/crates/llm-meter-cli)                           | `llm-meter` and `llm-meterd` executable package     |

The Tauri desktop package is intentionally excluded from crates.io publishing.

## Publishing crates

Publishing must follow dependency order:

1. `llm-meter-core`;
2. `llm-meter-secret-store`, `llm-meter-storage`, and
   `llm-meter-provider-openai`;
3. `llm-meter-provider-testkit`;
4. `llm-meter-daemon`;
5. `llm-meter-cli`.

Wait for each newly published version to appear in the registry index before
continuing. Although `llm-meter-provider-testkit` is only a development helper,
the daemon declares it as a versioned development dependency, so publish it
before the daemon.

Before a release, run
`cargo publish --dry-run --registry crates-io -p PACKAGE --allow-dirty` for
each package. The explicit registry also works when a local Cargo configuration
replaces the default crates.io index with a mirror. Downstream dry-runs can
resolve internal crates only after their exact versions exist in the selected
registry; before the first real release, use `cargo package --no-verify` to
inspect their archives without uploading.

## GitHub releases

Pushing a semantic-version tag that matches every Cargo package version creates
a GitHub Release automatically:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow runs the workspace tests, builds the CLI, daemon, and
Tauri desktop binaries, and publishes a versioned Linux archive together with
`SHA256SUMS`. It does not publish crates to crates.io; crate publishing remains
the separate, ordered process described above.

## Documentation

The detailed architecture and desktop integration documents are indexed in
[`docs/README.md`](https://github.com/llm-meter/llm-meter/blob/main/docs/README.md).

## License

LLM Meter is licensed under the MIT License. See [`LICENSE`](LICENSE).
