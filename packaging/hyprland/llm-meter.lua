-- WebKitGTK/Tauri currently exposes the binary name as the Wayland app_id.
-- Keep the bundle identifier too so packaged builds continue to match.
local llm_meter_class = [=[^(llm-meter-desktop|io\.github\.llmmeter)$]=]
local llm_meter_popup_title = [=[^LLM Meter Popup$]=]

hl.window_rule({
  name = "llm-meter-popup",
  match = { class = llm_meter_class, title = llm_meter_popup_title },
  float = true,
  size = {360, 440},
  move = {"monitor_w-window_w-12", "42"},
  no_max_size = true,
  focus_on_activate = true,
  animation = "popin 90%",
})

hl.bind(
  "SUPER + SHIFT + M",
  hl.dsp.exec_cmd("llm-meter ui --toggle"),
  { description = "Toggle LLM Meter" }
)
