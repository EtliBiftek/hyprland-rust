-- Example hyprland.lua for hyprland-rust (Hyprland >= 0.55 Lua config format).
-- Copy to ~/.config/hypr/hyprland.lua (it is used instead of hyprland.conf).

local mod = "SUPER"
local terminal = "kitty"

hl.on("hyprland.start", function()
    hl.exec_cmd("waybar")
end)

hl.config({
    general = {
        gaps_in = 5,
        gaps_out = 20,
        border_size = 2,
        col = {
            active_border = { colors = { "rgba(33ccffee)", "rgba(00ff99ee)" }, angle = 45 },
            inactive_border = "rgba(595959aa)",
        },
        layout = "dwindle",
    },
    dwindle = { preserve_split = true },
    input = { kb_layout = "tr", follow_mouse = 1 },
})

hl.bind(mod .. " + Return", hl.dsp.exec_cmd(terminal))
hl.bind(mod .. " + Q", hl.dsp.window.close())
hl.bind(mod .. " + SHIFT + E", hl.dsp.exit())
hl.bind(mod .. " + V", hl.dsp.window.float({ action = "toggle" }))
hl.bind(mod .. " + F", hl.dsp.window.fullscreen())
hl.bind(mod .. " + J", hl.dsp.layout("togglesplit"))
for _, d in ipairs({ "left", "right", "up", "down" }) do
    hl.bind(mod .. " + " .. d, hl.dsp.focus({ direction = d }))
end
for i = 1, 9 do
    hl.bind(mod .. " + " .. i, hl.dsp.focus({ workspace = i }))
    hl.bind(mod .. " + SHIFT + " .. i, hl.dsp.window.move({ workspace = i }))
end
hl.bind(mod .. " + mouse:272", hl.dsp.window.drag(), { mouse = true })
hl.bind(mod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })

hl.window_rule({ name = "float-pavucontrol", match = { class = "pavucontrol" }, float = true })
