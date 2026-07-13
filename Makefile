SHELL := /bin/bash

RUST_LOG ?= info
DESKTOP_MANIFEST := apps/desktop/src-tauri/Cargo.toml
DAEMON_BIN := target/debug/llm-meterd
CLI_BIN := target/debug/llm-meter
DESKTOP_BIN := apps/desktop/src-tauri/target/debug/llm-meter-desktop
HYPR_CONFIG_HOME ?= $(HOME)/.config/hypr
NOCTALIA_CONFIG_HOME ?= $(HOME)/.config/noctalia
NOCTALIA_ASSET_DIR := crates/cli/assets/noctalia
HYPRLAND_ASSET_DIR := crates/cli/assets/hyprland

.PHONY: help run dev daemon gui popup install setup install-dev-bins sync-desktop sync-bar sync-noctalia sync-noctalia-config sync-hyprland sync-waybar sync-waybar-config build build-daemon build-gui check test clean

help:
	@echo "LLM Meter development commands:"
	@echo "  make run      Build, sync the current top bar, and run the daemon"
	@echo "  make daemon   Start only the daemon"
	@echo "  make gui      Start only the main GUI"
	@echo "  make popup    Start only the popup GUI"
	@echo "  make install  Install development binaries and open the deployment wizard"
	@echo "  make sync-desktop   Install binaries, current top-bar module, and Hyprland rules"
	@echo "  make sync-bar       Auto-detect and sync Noctalia or Waybar"
	@echo "  make sync-noctalia  Install the native Noctalia panel"
	@echo "  make sync-hyprland  Install window rules and reload Hyprland"
	@echo "  make sync-waybar    Install/reload the Waybar module"
	@echo "  make build    Build the Rust workspace and desktop app"
	@echo "  make check    Check the Rust workspace and desktop app"
	@echo "  make test     Run workspace tests"

# Start both processes. If a daemon is already listening, reuse it. Otherwise,
# stop the daemon started here when the GUI exits or the command is interrupted.
run: sync-desktop
	@set -euo pipefail; \
	if [[ -z "$${XDG_RUNTIME_DIR:-}" ]]; then \
		echo "error: XDG_RUNTIME_DIR is not set" >&2; \
		exit 1; \
	fi; \
	socket="$$XDG_RUNTIME_DIR/llm-meter/daemon.sock"; \
	daemon_pid=""; \
	cleanup() { \
		if [[ -n "$$daemon_pid" ]]; then \
			kill "$$daemon_pid" 2>/dev/null || true; \
			wait "$$daemon_pid" 2>/dev/null || true; \
		fi; \
	}; \
	trap cleanup EXIT INT TERM; \
	if [[ -S "$$socket" ]] && $(CLI_BIN) status >/dev/null 2>&1; then \
		echo "Reusing daemon at $$socket"; \
	else \
		echo "Starting llm-meterd (RUST_LOG=$(RUST_LOG))..."; \
		RUST_LOG="$(RUST_LOG)" $(DAEMON_BIN) & \
		daemon_pid=$$!; \
		for ((attempt = 0; attempt < 100; attempt++)); do \
			if [[ -S "$$socket" ]] && $(CLI_BIN) status >/dev/null 2>&1; then break; fi; \
			if ! kill -0 "$$daemon_pid" 2>/dev/null; then \
				wait "$$daemon_pid"; \
				exit 1; \
			fi; \
			sleep 0.1; \
		done; \
		if [[ ! -S "$$socket" ]] || ! $(CLI_BIN) status >/dev/null 2>&1; then \
			echo "error: daemon did not create $$socket" >&2; \
			exit 1; \
		fi; \
	fi; \
	if [[ -n "$$daemon_pid" ]]; then \
		echo "LLM Meter is running; press Ctrl-C to stop the development daemon"; \
		wait "$$daemon_pid"; \
	else \
		echo "LLM Meter is ready (daemon was already running)"; \
	fi

dev: run

daemon: build-daemon
	RUST_LOG="$(RUST_LOG)" $(DAEMON_BIN)

gui: build-gui
	$(DESKTOP_BIN) --main

popup: sync-desktop
	$(DESKTOP_BIN)

install: install-dev-bins
	$(HOME)/.local/bin/llm-meter setup

setup: install

install-dev-bins: build
	@mkdir -p "$(HOME)/.local/bin"
	@install -m 0755 $(DAEMON_BIN) "$(HOME)/.local/bin/llm-meterd"
	@install -m 0755 $(CLI_BIN) "$(HOME)/.local/bin/llm-meter"
	@install -m 0755 $(DESKTOP_BIN) "$(HOME)/.local/bin/llm-meter-desktop"
	@echo "Installed development binaries under $(HOME)/.local/bin"

sync-desktop: sync-hyprland sync-bar

sync-bar: install-dev-bins
	@set -euo pipefail; \
	if [[ -f "$(NOCTALIA_CONFIG_HOME)/settings.json" ]] && command -v qs >/dev/null 2>&1; then \
		$(MAKE) --no-print-directory sync-noctalia-config; \
	elif command -v waybar >/dev/null 2>&1 || [[ -d "$(HOME)/.config/waybar" ]]; then \
		$(MAKE) --no-print-directory sync-waybar-config; \
	else \
		echo "warning: no supported top bar detected (Noctalia or Waybar)" >&2; \
	fi

sync-noctalia: install-dev-bins sync-noctalia-config

sync-noctalia-config:
	@set -euo pipefail; \
	settings="$(NOCTALIA_CONFIG_HOME)/settings.json"; \
	plugins="$(NOCTALIA_CONFIG_HOME)/plugins.json"; \
	if ! command -v qs >/dev/null 2>&1 || [[ ! -f "$$settings" ]]; then \
		echo "error: Noctalia/Quickshell settings were not found" >&2; \
		exit 1; \
	fi; \
	if ! command -v jq >/dev/null 2>&1; then \
		echo "error: jq is required to update Noctalia settings safely" >&2; \
		exit 1; \
	fi; \
	plugin_dir="$(NOCTALIA_CONFIG_HOME)/plugins/llm-meter"; \
	was_installed=0; \
	if [[ -f "$$plugin_dir/manifest.json" ]] \
		&& jq -e 'any(.bar.widgets.right[]?; .id == "plugin:llm-meter")' "$$settings" >/dev/null \
		&& [[ -f "$$plugins" ]] \
		&& jq -e '.states["llm-meter"].enabled == true' "$$plugins" >/dev/null; then \
		was_installed=1; \
	fi; \
	mkdir -p "$$plugin_dir"; \
	plugin_changed=0; \
	for file in manifest.json Main.qml BarWidget.qml Panel.qml; do \
		if ! cmp -s "$(NOCTALIA_ASSET_DIR)/$$file" "$$plugin_dir/$$file"; then \
			install -m 0644 "$(NOCTALIA_ASSET_DIR)/$$file" "$$plugin_dir/$$file"; \
			plugin_changed=1; \
		fi; \
	done; \
	settings_tmp="$$(mktemp "$$settings.llm-meter.XXXXXX")"; \
	plugins_tmp="$$(mktemp "$$plugins.llm-meter.XXXXXX")"; \
	trap 'rm -f "$$settings_tmp" "$$plugins_tmp"' EXIT; \
	jq '(.bar.widgets.right //= []) | ((.bar.widgets.right | map(select(.id == "plugin:llm-meter")) | .[0]) // {"id":"plugin:llm-meter"}) as $$widget | .bar.widgets.right |= map(select(.ipcIdentifier != "llm-meter" and .id != "plugin:llm-meter")) | .bar.widgets.right = ([$$widget] + .bar.widgets.right)' "$$settings" > "$$settings_tmp"; \
	chmod --reference="$$settings" "$$settings_tmp"; \
	if [[ -f "$$plugins" ]]; then \
		jq '.states["llm-meter"] = ((.states["llm-meter"] // {}) + {"enabled":true})' "$$plugins" > "$$plugins_tmp"; \
		chmod --reference="$$plugins" "$$plugins_tmp"; \
	else \
		printf '%s\n' '{"version":2,"sources":[],"states":{"llm-meter":{"enabled":true}}}' > "$$plugins_tmp"; \
	fi; \
	if cmp -s "$$settings_tmp" "$$settings"; then rm -f "$$settings_tmp"; else mv "$$settings_tmp" "$$settings"; fi; \
	if [[ -f "$$plugins" ]] && cmp -s "$$plugins_tmp" "$$plugins"; then rm -f "$$plugins_tmp"; else mv "$$plugins_tmp" "$$plugins"; fi; \
	trap - EXIT; \
	if pgrep -f '^qs -c noctalia-shell$$' >/dev/null 2>&1 && [[ "$$was_installed" == 1 ]]; then \
		if [[ "$$plugin_changed" == 0 ]]; then \
			echo "Native Noctalia LLM Meter plugin is already up to date"; \
		elif qs ipc -c noctalia-shell show 2>/dev/null | grep -q '^target llmMeterDev$$'; then \
			qs ipc -c noctalia-shell call llmMeterDev reload >/dev/null; \
			sleep 1; \
			echo "Hot-reloaded the native Noctalia LLM Meter plugin"; \
		else \
			echo "Synced LLM Meter; one Noctalia restart is required to enable future plugin-only hot reloads"; \
		fi; \
		exit 0; \
	fi; \
	if pgrep -f '^qs -c noctalia-shell$$' >/dev/null 2>&1; then \
		qs kill -c noctalia-shell >/dev/null 2>&1 || true; \
		for ((attempt = 0; attempt < 50; attempt++)); do \
			if ! pgrep -f '^qs -c noctalia-shell$$' >/dev/null 2>&1; then break; fi; \
			sleep 0.1; \
		done; \
	fi; \
	sleep 0.3; \
	if command -v hyprctl >/dev/null 2>&1 && [[ -f "$(HYPR_CONFIG_HOME)/hyprland.lua" ]]; then \
		hyprctl dispatch 'hl.dsp.exec_cmd("qs -c noctalia-shell")' >/dev/null; \
	elif command -v hyprctl >/dev/null 2>&1; then \
		hyprctl dispatch exec 'qs -c noctalia-shell' >/dev/null; \
	else \
		systemd-run --user --quiet --collect --unit=noctalia-shell qs -c noctalia-shell; \
	fi; \
	for ((attempt = 0; attempt < 100; attempt++)); do \
		if pgrep -f '^qs -c noctalia-shell$$' >/dev/null 2>&1; then break; fi; \
		sleep 0.1; \
	done; \
	if ! pgrep -f '^qs -c noctalia-shell$$' >/dev/null 2>&1; then \
		echo "error: Noctalia did not remain running after startup" >&2; \
		exit 1; \
	fi; \
	echo "Installed the native Noctalia LLM Meter panel and started Noctalia"

sync-hyprland:
	@set -euo pipefail; \
	if ! command -v hyprctl >/dev/null 2>&1; then \
		echo "Hyprland not found; skipping window-rule installation"; \
		exit 0; \
	fi; \
	if [[ -f "$(HYPR_CONFIG_HOME)/hyprland.lua" ]]; then \
		mkdir -p "$(HYPR_CONFIG_HOME)/config"; \
		install -m 0644 $(HYPRLAND_ASSET_DIR)/llm-meter.lua "$(HYPR_CONFIG_HOME)/config/llm-meter.lua"; \
		if ! grep -Fq 'require("config.llm-meter")' "$(HYPR_CONFIG_HOME)/hyprland.lua"; then \
			printf '\nrequire("config.llm-meter")\n' >> "$(HYPR_CONFIG_HOME)/hyprland.lua"; \
			echo "Added config.llm-meter to hyprland.lua"; \
		fi; \
		echo "Installed Lua rule: $(HYPR_CONFIG_HOME)/config/llm-meter.lua"; \
	elif [[ -f "$(HYPR_CONFIG_HOME)/hyprland.conf" ]]; then \
		install -m 0644 $(HYPRLAND_ASSET_DIR)/llm-meter.conf "$(HYPR_CONFIG_HOME)/llm-meter.conf"; \
		if ! grep -Fq 'source = ~/.config/hypr/llm-meter.conf' "$(HYPR_CONFIG_HOME)/hyprland.conf"; then \
			printf '\nsource = ~/.config/hypr/llm-meter.conf\n' >> "$(HYPR_CONFIG_HOME)/hyprland.conf"; \
			echo "Added llm-meter.conf source to hyprland.conf"; \
		fi; \
		echo "Installed legacy rule: $(HYPR_CONFIG_HOME)/llm-meter.conf"; \
	else \
		echo "error: no hyprland.lua or hyprland.conf found under $(HYPR_CONFIG_HOME)" >&2; \
		exit 1; \
	fi; \
	if hyprctl reload >/dev/null 2>&1; then \
		echo "Reloaded Hyprland"; \
	else \
		echo "warning: rules installed, but Hyprland reload failed" >&2; \
	fi

sync-waybar: install-dev-bins sync-waybar-config

sync-waybar-config:
	@set -euo pipefail; \
	if ! command -v waybar >/dev/null 2>&1; then \
		echo "warning: Waybar is not installed; on Arch/CachyOS run: sudo pacman -S waybar" >&2; \
		exit 0; \
	fi; \
	config_home="$(HOME)/.config/waybar"; \
	mkdir -p "$$config_home"; \
	install -m 0644 packaging/waybar/llm-meter.css "$$config_home/llm-meter.css"; \
	if [[ ! -f "$$config_home/config" && ! -f "$$config_home/config.jsonc" ]]; then \
		install -m 0644 packaging/waybar/config.jsonc "$$config_home/config.jsonc"; \
		echo "Installed initial Waybar config with custom/llm-meter"; \
	elif ! grep -Rqs 'custom/llm-meter' "$$config_home/config" "$$config_home/config.jsonc" 2>/dev/null; then \
		install -m 0644 packaging/waybar/llm-meter.jsonc "$$config_home/llm-meter.jsonc"; \
		echo "warning: existing Waybar config detected; merge $$config_home/llm-meter.jsonc and add custom/llm-meter to a modules list" >&2; \
	fi; \
	if [[ ! -f "$$config_home/style.css" ]]; then \
		install -m 0644 packaging/waybar/style.css "$$config_home/style.css"; \
	elif ! grep -Fq '@import "llm-meter.css";' "$$config_home/style.css"; then \
		printf '\n@import "llm-meter.css";\n' >> "$$config_home/style.css"; \
	fi; \
	if pgrep -x waybar >/dev/null 2>&1; then \
		pkill -SIGUSR2 -x waybar; \
		echo "Reloaded Waybar"; \
	else \
		nohup waybar >"$${XDG_RUNTIME_DIR:-/tmp}/llm-meter-waybar.log" 2>&1 & \
		echo "Started Waybar"; \
	fi

build: build-daemon build-gui

build-daemon:
	cargo build --workspace

build-gui:
	cargo build --manifest-path $(DESKTOP_MANIFEST)

check:
	cargo check --workspace
	cargo check --manifest-path $(DESKTOP_MANIFEST)

test:
	cargo test --workspace

clean:
	cargo clean
	cargo clean --manifest-path $(DESKTOP_MANIFEST)
