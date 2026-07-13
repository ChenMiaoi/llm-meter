# Hyprland integration

1. Install and enable `llm-meterd.service` with `systemctl --user enable --now llm-meterd`.
2. Check `hyprctl version`. Copy or source `hyprland/llm-meter.lua` on Hyprland 0.55+, or `hyprland/llm-meter.conf` on 0.54.
3. Merge `waybar/llm-meter.jsonc` into the selected bar and add `custom/llm-meter` to a modules list. Import the optional CSS.
4. Reload Hyprland and Waybar, then verify the released window identity with `hyprctl clients`.

Manual installers should also place `desktop/io.github.llmmeter.desktop` under
`~/.local/share/applications/` and
`icons/io.github.llmmeter.png` under
`~/.local/share/icons/hicolor/512x512/apps/`, then refresh the desktop and icon
caches. Distribution packages should install the same files to their system
data directories.

The Waybar-first profile does not need a prestarted desktop process. For the
optional tray profile, install `systemd/llm-meter-desktop.service`, ensure the
bar exposes a `tray` module, and enable it with
`systemctl --user enable --now llm-meter-desktop.service`. Set
`LLM_METER_NO_TRAY=1` only when launching the desktop without a tray icon.

For UWSM key bindings, change the command to `uwsm app -- llm-meter ui --toggle`. These templates are examples: installation never rewrites user Hyprland or Waybar configuration.
