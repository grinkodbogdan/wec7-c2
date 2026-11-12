-- phone: the shell for DragonFly BSD on the Xiaomi Mi 5.
--
-- Swipe-based, in the manner of the N9's: three home views side by side,
--
--   Events  |  Home  |  Open apps
--
-- panned with a horizontal swipe.  Home is laid out like an Android home
-- screen: a clock, a grid of apps, a dock, and the app drawer under a
-- swipe up.  In an app, a swipe in from the left or right edge leaves it
-- running and returns to the home views; a swipe down from the top edge
-- closes it.  A tap on the status bar, or a pull down from it at home,
-- opens quick settings.  The power button locks (screen off); a swipe
-- up unlocks.  Home goes home; volume steps the brightness.
--
-- Built-in apps are Lua (settings, files, system, calculator); the
-- terminal is Arcan's terminal frameserver running sh.  Later, Wayland
-- and Android clients will be windows here the same way.
--
-- Arguments: autorun=CMD opens the terminal and types CMD; test=VIEW
-- opens a view at start (QEMU screenshots); touchw=N is the touch
-- panel's width in pixels (1080).

local W, H
local SB = 30			-- status bar height
local touch_w = 1080
local tscale = 1

local KEY_HOME, KEY_VOLDOWN, KEY_VOLUP, KEY_POWER = 102, 114, 115, 116

local FONT = "DejaVuSans.ttf"
local FONT_B = "DejaVuSans-Bold.ttf"
local FONT_L = "DejaVuSans-ExtraLight.ttf"
local FONT_M = "DejaVuSansMono.ttf"

-- order bands
local O_WALL, O_HOME, O_DRAWER, O_APP = 1, 10, 60, 100
local O_KBD, O_SB, O_QS, O_BANNER, O_LOCK = 180, 200, 300, 350, 400

------------------------------------------------------------------------
-- drawing helpers

local shaders = {}

local function build_shaders()
	local head = [[
uniform float obj_opacity;
uniform vec3 obj_col;
uniform vec2 obj_output_sz;
varying vec2 texco;
]]
	-- rounded rectangle: radius a fraction of the short side, capped
	shaders.rrect = build_shader(nil, head .. [[
void main()
{
	vec2 sz = obj_output_sz;
	vec2 p = texco * sz;
	float r = min(0.24 * min(sz.x, sz.y), 20.0);
	vec2 q = abs(p - 0.5 * sz) - (0.5 * sz - vec2(r));
	float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
	gl_FragColor = vec4(obj_col, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]], "phone_rrect")
	shaders.circle = build_shader(nil, head .. [[
void main()
{
	vec2 sz = obj_output_sz;
	float r = 0.5 * min(sz.x, sz.y);
	float d = length(texco * sz - 0.5 * sz) - r;
	gl_FragColor = vec4(obj_col, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]], "phone_circle")
	-- the wallpaper: a night-blue to violet diagonal with a soft glow
	shaders.wall = build_shader(nil, head .. [[
void main()
{
	vec3 a = vec3(0.035, 0.055, 0.140);
	vec3 b = vec3(0.200, 0.075, 0.270);
	float t = clamp(0.65 * texco.y + 0.35 * texco.x, 0.0, 1.0);
	vec3 c = mix(a, b, t);
	float g = 1.0 - clamp(length(texco - vec2(0.78, 0.22)) * 1.6, 0.0, 1.0);
	c += vec3(0.10, 0.12, 0.22) * g * g;
	gl_FragColor = vec4(c, obj_opacity);
}]], "phone_wall")
end

-- a coloured box, optionally shaped, linked into parent at x, y
local function box(parent, x, y, w, h, col, shape, alpha, order)
	local v = color_surface(math.max(1, math.floor(w)),
		math.max(1, math.floor(h)), col[1], col[2], col[3])
	if not valid_vid(v) then
		return nil
	end
	alpha = alpha or 1.0
	if shape and shaders[shape] then
		image_shader(v, shaders[shape])
		-- a fully opaque object is drawn without blending, whatever
		-- force_image_blend says; just under 1 keeps the shape's edges
		alpha = math.min(alpha, 0.996)
	end
	if parent then
		link_image(v, parent)
		image_inherit_order(v, true)
	end
	move_image(v, x, y)
	order_image(v, order or 1)
	blend_image(v, alpha)
	if alpha < 1.0 then
		force_image_blend(v, BLEND_NORMAL)
	end
	image_mask_set(v, MASK_UNPICKABLE)
	return v
end

-- text: user content goes in raw, never as a format string
local function text(parent, x, y, str, size, color, font, order)
	local v, _, w, h = render_text({string.format([[\f%s,%d\#%s]],
		font or FONT, size, color or "ffffff"), tostring(str)})
	if not valid_vid(v) then
		return nil, 0, 0
	end
	local p = image_surface_properties(v)
	if parent then
		link_image(v, parent)
		image_inherit_order(v, true)
	end
	move_image(v, x, y)
	order_image(v, order or 2)
	show_image(v)
	image_mask_set(v, MASK_UNPICKABLE)
	return v, p.width, p.height
end

local function text_c(parent, cx, y, str, size, color, font, order)
	local v, w, h = text(parent, 0, y, str, size, color, font, order)
	if v then
		move_image(v, math.floor(cx - w / 2), y)
	end
	return v, w, h
end

local function anchor(parent, x, y, w, h, order)
	local v = null_surface(w or 1, h or 1)
	if parent then
		link_image(v, parent)
		image_inherit_order(v, true)
	end
	move_image(v, x or 0, y or 0)
	order_image(v, order or 1)
	show_image(v)
	image_mask_set(v, MASK_UNPICKABLE)
	return v
end

local function clear_children(v)
	for _, c in ipairs(image_children(v) or {}) do
		if valid_vid(c) then
			delete_image(c)
		end
	end
end

local function hit(r, x, y)
	return x >= r.x and x < r.x + r.w and y >= r.y and y < r.y + r.h
end

local function sh_quote(s)
	return "'" .. string.gsub(s, "'", "'\\''") .. "'"
end

local function run(cmd)
	if not io or not io.popen then	-- ARCAN_SYSTEM_IO unset
		return ""
	end
	local f = io.popen(cmd .. " 2>/dev/null")
	if not f then
		return ""
	end
	local out = f:read("*a") or ""
	f:close()
	return out
end

local function sysctl_set(name, val)
	run(string.format("sysctl -q %s=%s", name, tostring(val)))
end

------------------------------------------------------------------------
-- state

local sys = {battery = -1, charger = "unknown", backlight = 1400,
	led = 0, renderer = "", gpu = false}
local screen_off = false
local locked = true
local page = 2			-- 1 events, 2 home, 3 open apps
local strip, pages = nil, {}
local wall
local apps, app_by_id = {}, {}
local running, run_order = {}, {}
local focus = nil		-- id of the app in front, or nil
local drawer = {open = false}
local qs = {open = false}
local lock = {}
local sbar = {}
local events = {}
local g = nil			-- the gesture in progress
local redraw_pages
local open_app, leave_app, close_app, go_home, set_page
local add_event

------------------------------------------------------------------------
-- system status

local function read_status()
	local out = run("sysctl -n hw.mi5.battery hw.mi5.charger")
	local b, c = string.match(out, "^%s*(%-?%d+)%s+(%S+)")
	local old = {battery = sys.battery, charger = sys.charger}
	if b then
		sys.battery = tonumber(b)
		sys.charger = c
	end
	if old.charger ~= "unknown" and old.charger ~= sys.charger then
		if sys.charger == "charging" or sys.charger == "present" then
			add_event("⚡", "Charger connected",
				string.format("Battery at %d%%", sys.battery))
		elseif sys.charger == "full" then
			add_event("⚡", "Battery full", "You can unplug the charger")
		elseif sys.charger == "none" then
			add_event("⚡", "Charger disconnected",
				string.format("Battery at %d%%", sys.battery))
		end
	end
	if old.battery > 15 and sys.battery >= 0 and sys.battery <= 15 then
		add_event("▮", "Battery low",
			string.format("%d%% left", sys.battery))
	end
end

local function clock_str()
	return os.date("%H:%M")
end

local function date_str()
	return os.date("%A, %d %B")
end

------------------------------------------------------------------------
-- status bar

local function draw_status()
	if not valid_vid(sbar.bg) then
		sbar.bg = box(nil, 0, 0, W, SB, {0, 0, 0}, nil, 0.35, O_SB)
		sbar.items = anchor(sbar.bg, 0, 0, W, SB, 1)
	end
	clear_children(sbar.items)
	text(sbar.items, 12, 6, clock_str(), 12, "ffffff", FONT_B)
	if focus and app_by_id[focus] then
		text_c(sbar.items, W / 2, 7, app_by_id[focus].name, 11, "c8d0e0")
	end
	-- battery: outline, fill, tip, and the number
	local bx, by, bw, bh = W - 46, 9, 26, 13
	box(sbar.items, bx, by, bw, bh, {200, 205, 215}, "rrect", 1.0, 1)
	box(sbar.items, bx + 2, by + 2, bw - 4, bh - 4, {20, 22, 30}, nil, 1.0, 2)
	local pct = math.max(0, math.min(100, sys.battery))
	local col = pct <= 15 and {235, 80, 70} or
		(sys.charger == "charging" and {110, 220, 120} or {235, 238, 245})
	if pct > 0 then
		box(sbar.items, bx + 2, by + 2, math.max(1, (bw - 4) * pct / 100),
			bh - 4, col, nil, 1.0, 3)
	end
	box(sbar.items, bx + bw, by + 4, 3, bh - 8, {200, 205, 215}, nil, 1.0, 1)
	local s = sys.battery >= 0 and string.format("%d%%", sys.battery) or "?"
	local v, w = text(sbar.items, 0, 7, s, 11, "ffffff")
	if v then
		move_image(v, bx - w - (sys.charger == "charging" and 18 or 6), 7)
	end
	if sys.charger == "charging" or sys.charger == "present" then
		text(sbar.items, bx - 16, 5, "⚡", 12, "ffd860")
	end
end

------------------------------------------------------------------------
-- banners and events

local banner = {}

local function show_banner(icon, title, body)
	if valid_vid(banner.v) then
		delete_image(banner.v)
	end
	local v = box(nil, 10, -90, W - 20, 70, {28, 34, 52}, "rrect", 0.97,
		O_BANNER)
	text(v, 16, 12, icon, 20, "8fb4ff")
	text(v, 56, 12, title, 13, "ffffff", FONT_B)
	text(v, 56, 38, body, 11, "b8c0d0")
	move_image(v, 10, SB + 6, 8)
	banner.v = v
	banner.hide_at = CLOCK + 100
end

add_event = function(icon, title, body, quiet)
	table.insert(events, 1, {icon = icon, title = title, body = body,
		time = os.date("%H:%M")})
	while #events > 30 do
		table.remove(events)
	end
	if not quiet and not screen_off then
		show_banner(icon, title, body)
	end
	if redraw_pages then
		redraw_pages()
	end
end

------------------------------------------------------------------------
-- the app grid

local ICON = 72

local function draw_icon(parent, x, y, app, with_label)
	local cell = anchor(parent, x, y, ICON, ICON + 26, 1)
	box(cell, 0, 0, ICON, ICON, app.col, "rrect", 1.0, 1)
	-- a lighter top half for some depth
	box(cell, 0, 0, ICON, ICON / 2, {255, 255, 255}, "rrect", 0.08, 2)
	text_c(cell, ICON / 2, (ICON - app.gsize * 1.35) / 2, app.glyph,
		app.gsize, app.gcol or "ffffff", FONT_B, 3)
	if with_label then
		text_c(cell, ICON / 2, ICON + 6, app.name, 10, "ffffff", FONT, 3)
	end
	return cell
end

-- lay apps out in a grid; returns hit rects {x, y, w, h, app}
local function grid(parent, list, y0, cols, labels)
	local rects = {}
	local cw = W / cols
	for i, app in ipairs(list) do
		local c = (i - 1) % cols
		local r = math.floor((i - 1) / cols)
		local x = math.floor(c * cw + (cw - ICON) / 2)
		local y = y0 + r * (ICON + (labels and 44 or 20))
		draw_icon(parent, x, y, app, labels)
		table.insert(rects, {x = x - 10, y = y - 6, w = ICON + 20,
			h = ICON + 34, app = app})
	end
	return rects
end

------------------------------------------------------------------------
-- home views

local function draw_home(p)
	clear_children(p.a)
	p.hits = {}
	local cy = SB + 54
	text_c(p.a, W / 2, cy, clock_str(), 64, "ffffff", FONT_L)
	text_c(p.a, W / 2, cy + 92, date_str(), 13, "d8def0")
	local line = sys.battery >= 0 and string.format("%d%%  ·  %s",
		sys.battery, sys.charger == "charging" and "charging" or
		(sys.charger == "full" and "full" or "on battery")) or ""
	text_c(p.a, W / 2, cy + 118, line, 10, "a8b0c8")

	local favs = {}
	for _, a in ipairs(apps) do
		if not a.dock then
			table.insert(favs, a)
		end
	end
	local rows = math.ceil(#favs / 4)
	p.hits = grid(p.a, favs, H - 128 - 46 - rows * (ICON + 44), 4, true)

	-- dock
	local dy = H - 128
	box(p.a, 14, dy, W - 28, 108, {255, 255, 255}, "rrect", 0.12, 1)
	local dock = {}
	for _, a in ipairs(apps) do
		if a.dock then
			table.insert(dock, a)
		end
	end
	local cw = (W - 28) / #dock
	for i, a in ipairs(dock) do
		local x = math.floor(14 + (i - 1) * cw + (cw - ICON) / 2)
		draw_icon(p.a, x, dy + 18, a, false)
		table.insert(p.hits, {x = x - 8, y = dy + 10, w = ICON + 16,
			h = ICON + 16, app = a})
	end
	text_c(p.a, W / 2, dy - 30, "▲", 10, "a0a8c0")
end

local function draw_events(p)
	clear_children(p.a)
	p.hits = {}
	text(p.a, 22, SB + 24, "Events", 26, "ffffff", FONT_L)
	if #events > 0 then
		local v = text(p.a, W - 80, SB + 34, "Clear", 12, "8fb4ff")
		table.insert(p.hits, {x = W - 100, y = SB + 20, w = 100, h = 44,
			clear = true})
	end
	local y = SB + 86
	if #events == 0 then
		text_c(p.a, W / 2, H / 2 - 20, "No events", 14, "8088a0")
		return
	end
	for i, e in ipairs(events) do
		if y > H - 90 then
			break
		end
		-- (children inherit opacity: the content sits beside the
		-- translucent card, not inside it)
		box(p.a, 14, y, W - 28, 78, {255, 255, 255}, "rrect", 0.10, 1)
		box(p.a, 28, y + 17, 44, 44, {70, 110, 200}, "circle", 1.0, 2)
		text_c(p.a, 50, y + 27, e.icon, 15, "ffffff", FONT, 3)
		text(p.a, 86, y + 14, e.title, 13, "ffffff", FONT_B, 3)
		text(p.a, 86, y + 42, e.body, 11, "b0b8cc", FONT, 3)
		local tv, tw = text(p.a, 0, y + 16, e.time, 10, "8088a0", FONT, 3)
		if tv then
			move_image(tv, W - 30 - tw, y + 16)
		end
		y = y + 88
	end
end

local function draw_switcher(p)
	clear_children(p.a)
	p.hits = {}
	text(p.a, 22, SB + 24, "Open apps", 26, "ffffff", FONT_L)
	if #run_order == 0 then
		text_c(p.a, W / 2, H / 2 - 20, "No open apps", 14, "8088a0")
		return
	end
	local cw = math.floor((W - 3 * 16) / 2)
	local ch = math.floor(cw * 1.45)
	for i, id in ipairs(run_order) do
		if i > 4 then
			break
		end
		local inst = running[id]
		local c = (i - 1) % 2
		local r = math.floor((i - 1) / 2)
		local x = 16 + c * (cw + 16)
		local y = SB + 90 + r * (ch + 48)
		local card = box(p.a, x, y, cw, ch, {18, 20, 28}, "rrect", 1.0, 1)
		if inst.thumb_src and valid_vid(inst.thumb_src) then
			local t = null_surface(cw - 8, ch - 8)
			image_sharestorage(inst.thumb_src, t)
			link_image(t, card)
			image_inherit_order(t, true)
			move_image(t, 4, 4)
			order_image(t, 2)
			show_image(t)
		else
			local a = inst.app
			box(card, (cw - ICON) / 2, ch / 2 - ICON, ICON, ICON, a.col,
				"rrect", 1.0, 2)
			text_c(card, cw / 2, ch / 2 - ICON + (ICON - a.gsize * 1.35) / 2,
				a.glyph, a.gsize, a.gcol or "ffffff", FONT_B, 3)
			if inst.summary then
				text_c(card, cw / 2, ch / 2 + 20, inst.summary(), 10,
					"a0a8c0", FONT, 3)
			end
		end
		text(p.a, x + 4, y + ch + 10, inst.app.name, 12, "ffffff", FONT, 2)
		box(card, cw - 40, 8, 32, 32, {0, 0, 0}, "circle", 0.6, 5)
		text_c(card, cw - 24, 14, "×", 14, "ffffff", FONT_B, 6)
		table.insert(p.hits, {x = x + cw - 48, y = y, w = 48, h = 48,
			close = id})
		table.insert(p.hits, {x = x, y = y, w = cw, h = ch + 36, open = id})
	end
end

redraw_pages = function()
	if pages[1] then
		draw_events(pages[1])
		draw_home(pages[2])
		draw_switcher(pages[3])
	end
end

set_page = function(n, t)
	page = math.max(1, math.min(3, n))
	instant_image_transform(strip)
	move_image(strip, -(page - 1) * W, 0, t or 7, INTERP_EXPOUT)
	-- the dots under the status bar
	for i, d in ipairs(sbar.dots or {}) do
		blend_image(d, i == page and 0.95 or 0.35, 4)
	end
end

------------------------------------------------------------------------
-- app drawer

local function draw_drawer()
	clear_children(drawer.a)
	box(drawer.a, 0, 0, W, H, {12, 14, 26}, nil, 0.975, 1)
	text_c(drawer.a, W / 2, SB + 16, "▼", 10, "a0a8c0")
	text(drawer.a, 22, SB + 40, "All apps", 22, "ffffff", FONT_L)
	drawer.hits = grid(drawer.a, apps, SB + 100, 4, true)
end

local function drawer_show(open, t)
	drawer.open = open
	instant_image_transform(drawer.a)
	move_image(drawer.a, 0, open and 0 or H, t or 7, INTERP_EXPOUT)
end

------------------------------------------------------------------------
-- quick settings

local QS_H = 430

local function bl_to_frac(v)
	return math.max(0, math.min(1, (v - 30) / (4095 - 30)))
end

local function frac_to_bl(f)
	return math.floor(30 + f * (4095 - 30))
end

local function draw_qs()
	clear_children(qs.a)
	qs.hits = {}
	box(qs.a, 0, 0, W, QS_H, {16, 18, 30}, "rrect", 0.98, 1)
	box(qs.a, 0, 0, W, 40, {16, 18, 30}, nil, 0.98, 1)
	text(qs.a, 22, SB + 12, "Quick settings", 15, "ffffff", FONT_B)
	text(qs.a, 22, SB + 36, date_str(), 11, "a0a8c0")

	-- brightness
	local y = SB + 76
	text(qs.a, 22, y, "☀", 16, "ffd860")
	qs.slider = {x = 60, y = y - 6, w = W - 90, h = 36}
	box(qs.a, 60, y + 8, W - 90, 8, {60, 66, 90}, "rrect", 1.0, 2)
	local f = bl_to_frac(sys.backlight)
	qs.fill = box(qs.a, 60, y + 8, math.max(8, (W - 90) * f), 8,
		{120, 160, 255}, "rrect", 1.0, 3)
	qs.knob = box(qs.a, 60 + (W - 90) * f - 12, y, 24, 24, {235, 240, 255},
		"circle", 1.0, 4)

	-- toggles
	local tg = {
		{"●", "Light", function() return sys.led ~= 0 end, function()
			sys.led = sys.led ~= 0 and 0 or 0x00ff40
			sysctl_set("hw.mi5.led", sys.led)
		end},
		{"◆", "Torch", function() return sys.led == 0xffffff end, function()
			sys.led = sys.led == 0xffffff and 0 or 0xffffff
			sysctl_set("hw.mi5.led", sys.led)
		end},
		{"≋", "Buzz", function() return false end, function()
			sysctl_set("hw.mi5.vibrate", 250)
		end},
		{"☾", "Lock", function() return false end, function()
			qs_show(false)
			lock_screen(true)
		end},
	}
	local cw = (W - 32) / #tg
	for i, t in ipairs(tg) do
		local x = math.floor(16 + (i - 1) * cw + (cw - 64) / 2)
		local on = t[3]()
		box(qs.a, x, SB + 128, 64, 64, on and {110, 150, 255} or {48, 54, 78},
			"circle", 1.0, 2)
		text_c(qs.a, x + 32, SB + 144, t[1], 18, "ffffff", FONT, 3)
		text_c(qs.a, x + 32, SB + 200, t[2], 10, "c8d0e0", FONT, 3)
		table.insert(qs.hits, {x = x - 10, y = SB + 120, w = 84, h = 104,
			fn = t[4]})
	end

	-- status lines
	local y2 = SB + 240
	local lines = {
		{"Battery", sys.battery >= 0 and string.format("%d%%, %s",
			sys.battery, sys.charger) or "unknown"},
		{"Graphics", sys.renderer ~= "" and sys.renderer or "software"},
	}
	for _, l in ipairs(lines) do
		text(qs.a, 22, y2, l[1], 11, "8088a0")
		text(qs.a, 130, y2, l[2], 11, "e0e4f0")
		y2 = y2 + 26
	end

	-- power
	local bw = math.floor((W - 56) / 2)
	for i, b in ipairs({{"Restart", "reboot"}, {"Power off", "reboot -p"}}) do
		local x = 22 + (i - 1) * (bw + 12)
		box(qs.a, x, QS_H - 76, bw, 46, {48, 54, 78}, "rrect", 1.0, 2)
		text_c(qs.a, x + bw / 2, QS_H - 64, b[1], 12, "ffffff", FONT, 3)
		table.insert(qs.hits, {x = x, y = QS_H - 76, w = bw, h = 46,
			fn = function()
				add_event("☾", b[1], "Going down now", false)
				run(b[2] .. " &")
			end})
	end
	box(qs.a, W / 2 - 24, QS_H - 14, 48, 5, {120, 128, 150}, "rrect", 1.0, 2)
end

function qs_show(open, t)
	qs.open = open
	if open then
		draw_qs()
	end
	instant_image_transform(qs.a)
	move_image(qs.a, 0, open and 0 or -QS_H - 10, t or 7, INTERP_EXPOUT)
end

local function qs_slider(x, final)
	local s = qs.slider
	local f = math.max(0, math.min(1, (x - s.x) / s.w))
	resize_image(qs.fill, math.max(8, s.w * f), 8)
	move_image(qs.knob, s.x + s.w * f - 12, s.y + 6)
	sys.backlight = frac_to_bl(f)
	if final or not qs.last_set or CLOCK - qs.last_set > 3 then
		sysctl_set("hw.mi5.backlight", sys.backlight)
		qs.last_set = CLOCK
	end
end

------------------------------------------------------------------------
-- lock screen

local function draw_lock()
	clear_children(lock.a)
	local wl = box(lock.a, 0, 0, W, H, {0, 0, 0}, "wall", 1.0, 1)
	box(lock.a, 0, 0, W, H, {0, 0, 0}, nil, 0.25, 2)
	text_c(lock.a, W / 2, 190, clock_str(), 92, "ffffff", FONT_L, 3)
	text_c(lock.a, W / 2, 320, date_str(), 15, "e0e6f8", FONT, 3)
	if sys.battery >= 0 then
		text_c(lock.a, W / 2, 352, string.format("%s%d%%",
			sys.charger == "charging" and "⚡ " or "", sys.battery), 12,
			"b0b8d0", FONT, 3)
	end
	if #events > 0 then
		local e = events[1]
		box(lock.a, 30, 420, W - 60, 64, {255, 255, 255}, "rrect", 0.12, 3)
		text(lock.a, 48, 430, e.title, 12, "ffffff", FONT_B, 4)
		text(lock.a, 48, 454, e.body, 10, "c0c8dc", FONT, 4)
	end
	text_c(lock.a, W / 2, H - 92, "▲", 14, "d0d8f0", FONT, 3)
	text_c(lock.a, W / 2, H - 66, "Swipe up to unlock", 12, "d0d8f0", FONT, 3)
	text_c(lock.a, W / 2, H - 36, "DragonFly BSD", 9, "8088a0", FONT, 3)
end

function lock_screen(on, t)
	locked = on
	instant_image_transform(lock.a)
	if on then
		draw_lock()
		move_image(lock.a, 0, 0, t or 0)
		blend_image(lock.a, 1.0, t or 0)
		hide_image(sbar.bg)
	else
		move_image(lock.a, 0, -H, t or 8, INTERP_EXPOUT)
		blend_image(sbar.bg, 0.35)
		force_image_blend(sbar.bg, BLEND_NORMAL)
	end
end

local function set_screen(on)
	if on == not screen_off then
		return
	end
	screen_off = not on
	if screen_off then
		sysctl_set("hw.mi5.backlight", 0)
		lock_screen(true)
	else
		read_status()
		draw_status()
		lock_screen(true)
		sysctl_set("hw.mi5.backlight", sys.backlight)
	end
end

------------------------------------------------------------------------
-- on-screen keyboard (for the terminal)

local kbd = {layer = "lower", keys = {}, ctrl = false, h = 0, key_h = 46}
local SHIFT, SYM, CTRL = {}, {}, {}
local BKSP, ENTER, TAB, ESC = "\127", "\r", "\t", "\027"

local function krow(chars)
	local r = {}
	for _, c in utf8_chars(chars) do
		table.insert(r, {c, c, 1})
	end
	return r
end

function utf8_chars(s)
	local i, n = 1, #s
	return function()
		if i > n then
			return nil
		end
		local c = string.byte(s, i)
		local len = c < 0x80 and 1 or (c < 0xe0 and 2 or (c < 0xf0 and 3 or 4))
		local ch = string.sub(s, i, i + len - 1)
		i = i + len
		return i, ch
	end
end

local function krow4(mid)
	local r = krow(mid)
	table.insert(r, 1, {"⇧", SHIFT, 1.5})
	table.insert(r, {"⌫", BKSP, 1.5})
	return r
end

local layers = {
	lower = {krow("1234567890"), krow("qwertyuiop"), krow("asdfghjkl"),
		krow4("zxcvbnm")},
	upper = {krow("!@#$%^&*()"), krow("QWERTYUIOP"), krow("ASDFGHJKL"),
		krow4("ZXCVBNM")},
	sym = {krow("1234567890"), krow("~`|\\{}[]<>"), krow("'\";:=+-_/"),
		krow4(",.?!$&")},
}
for _, l in pairs(layers) do
	table.insert(l, {{"ctrl", CTRL, 1.5}, {"?123", SYM, 1.5},
		{"", " ", 4}, {"tab", TAB, 1}, {"esc", ESC, 1}, {"⏎", ENTER, 1.5}})
	-- centre the shorter rows
	for _, r in ipairs(l) do
		local tw = 0
		for _, k in ipairs(r) do
			tw = tw + k[3]
		end
		r.pad = (10 - tw) / 2
	end
end

local function kbd_build(parent, y)
	if valid_vid(kbd.surf) then
		delete_image(kbd.surf)
	end
	kbd.keys = {}
	kbd.y = y
	local kh = kbd.key_h
	kbd.surf = alloc_surface(W, kbd.h)
	local vids = {}
	local bg = color_surface(W, kbd.h, 22, 24, 34)
	show_image(bg)
	table.insert(vids, bg)
	local unit = W / 10
	for ri, r in ipairs(layers[kbd.layer]) do
		local x = r.pad * unit
		for _, k in ipairs(r) do
			local w = k[3] * unit
			local yy = 6 + (ri - 1) * kh
			local special = type(k[2]) == "table" or #k[2] ~= 1 or
				k[2] == BKSP or k[2] == ENTER or k[2] == TAB or k[2] == ESC
			local on = (k[2] == SHIFT and kbd.layer == "upper") or
				(k[2] == SYM and kbd.layer == "sym") or
				(k[2] == CTRL and kbd.ctrl)
			local col = on and {100, 140, 240} or
				(special and {50, 54, 72} or {72, 78, 102})
			local key = color_surface(math.floor(w - 6), kh - 8,
				col[1], col[2], col[3])
			image_shader(key, shaders.rrect)
			force_image_blend(key, BLEND_NORMAL)
			move_image(key, x + 3, yy + 2)
			order_image(key, 2)
			show_image(key)
			table.insert(vids, key)
			if k[1] ~= "" then
				local t = render_text({string.format([[\f%s,%d\#ffffff]], FONT,
					#k[1] > 1 and 10 or 14), k[1]})
				if valid_vid(t) then
					local p = image_surface_properties(t)
					move_image(t, math.floor(x + (w - p.width) / 2),
						math.floor(yy + 2 + (kh - 8 - p.height) / 2))
					order_image(t, 3)
					show_image(t)
					table.insert(vids, t)
				end
			end
			table.insert(kbd.keys, {x = x, y = yy, w = w, h = kh, k = k})
			x = x + w
		end
	end
	define_rendertarget(kbd.surf, vids, RENDERTARGET_DETACH,
		RENDERTARGET_NOSCALE, 0)
	rendertarget_forceupdate(kbd.surf)
	link_image(kbd.surf, parent)
	image_inherit_order(kbd.surf, true)
	move_image(kbd.surf, 0, y)
	order_image(kbd.surf, 8)
	show_image(kbd.surf)
	if not valid_vid(kbd.hl) then
		kbd.hl = color_surface(1, 1, 220, 230, 255)
		image_shader(kbd.hl, shaders.rrect)
		force_image_blend(kbd.hl, BLEND_NORMAL)
		blend_image(kbd.hl, 0.0)
		order_image(kbd.hl, O_KBD)
	end
end

------------------------------------------------------------------------
-- apps

-- the terminal: Arcan's terminal frameserver, sh inside
local function term_type(inst, str)
	if not valid_vid(inst.vid, TYPE_FRAMESERVER) then
		return
	end
	for _, ch in utf8_chars(str) do
		target_input(inst.vid, {kind = "digital", translated = true,
			active = true, number = 0, keysym = 0, modifiers = 0,
			utf8 = ch, devid = 0, subid = 0})
	end
end

local function kbd_press(inst, k)
	local v = k[2]
	if v == SHIFT then
		kbd.layer = kbd.layer == "upper" and "lower" or "upper"
		kbd_build(inst.a, kbd.y)
	elseif v == SYM then
		kbd.layer = kbd.layer == "sym" and "lower" or "sym"
		kbd_build(inst.a, kbd.y)
	elseif v == CTRL then
		kbd.ctrl = not kbd.ctrl
		kbd_build(inst.a, kbd.y)
	else
		if kbd.ctrl and #v == 1 and string.match(v, "%a") then
			v = string.char(string.byte(string.lower(v)) - 96)
		end
		term_type(inst, v)
		local rebuild = kbd.ctrl or kbd.layer == "upper"
		kbd.ctrl = false
		if kbd.layer == "upper" then
			kbd.layer = "lower"
		end
		if rebuild then
			kbd_build(inst.a, kbd.y)
		end
	end
end

local term_app = {id = "term", name = "Terminal", glyph = ">_", gsize = 20,
	col = {28, 32, 40}, gcol = "7cf29a", dock = true}

function term_app.create(inst, ah)
	kbd.h = kbd.key_h * 5 + 10
	box(inst.a, 0, 0, W, ah, {0, 0, 0}, nil, 1.0, 1)
	kbd_build(inst.a, ah - kbd.h)
	inst.th = ah - kbd.h
	inst.vid = launch_avfeed("", "terminal", function(source, status)
		if status.kind == "preroll" or status.kind == "registered" then
			target_fonthint(source, FONT_M, 4.0, 2)
			target_displayhint(source, W, inst.th, 0)
		elseif status.kind == "resized" then
			resize_image(source, status.width, status.height)
			show_image(source)
		elseif status.kind == "terminated" then
			close_app("term")
		end
	end)
	if valid_vid(inst.vid) then
		link_image(inst.vid, inst.a)
		image_inherit_order(inst.vid, true)
		order_image(inst.vid, 3)
		inst.thumb_src = inst.vid
	end
end

function term_app.down(inst, x, y)
	local ky = y - kbd.y
	if ky < 0 or ky >= kbd.h then
		return false
	end
	for _, e in ipairs(kbd.keys) do
		if x >= e.x and x < e.x + e.w and ky >= e.y and ky < e.y + e.h then
			move_image(kbd.hl, e.x + 3, SB + kbd.y + e.y + 2)
			resize_image(kbd.hl, math.floor(e.w - 6), e.h - 8)
			blend_image(kbd.hl, 0.45)
			kbd.hl_until = CLOCK + 3
			kbd_press(inst, e.k)
			return true
		end
	end
	return true
end

function term_app.input(inst, iotbl)
	if valid_vid(inst.vid, TYPE_FRAMESERVER) then
		target_input(inst.vid, iotbl)
	end
end

-- a scrolling list of text rows, for files, system and settings' about
local function list_view(inst, y0, rows, row_h, ontap)
	if valid_vid(inst.view) then
		delete_image(inst.view)
	end
	local vh = inst.ah - y0
	inst.view = anchor(inst.a, 0, y0, W, vh, 5)
	inst.content = anchor(inst.view, 0, 0, W, math.max(vh, #rows * row_h), 1)
	inst.list = {rows = rows, row_h = row_h, y0 = y0, vh = vh, off = 0,
		tap = ontap}
	for i, r in ipairs(rows) do
		local y = (i - 1) * row_h
		if i % 2 == 0 then
			local b = box(inst.content, 0, y, W, row_h, {255, 255, 255}, nil,
				0.035, 1)
			image_clip_on(b, CLIP_SHALLOW, inst.view)
		end
		if r.icon then
			local t = text(inst.content, 18, y + (row_h - 22) / 2, r.icon, 14,
				r.icol or "8fb4ff", FONT, 2)
			image_clip_on(t, CLIP_SHALLOW, inst.view)
		end
		local t = text(inst.content, r.icon and 52 or 18,
			y + (row_h - 18) / 2, r.text, r.size or 12, r.col or "e8ecf8",
			r.font or FONT, 2)
		if t then
			image_clip_on(t, CLIP_SHALLOW, inst.view)
		end
		if r.right then
			local v, w = text(inst.content, 0, y + (row_h - 18) / 2, r.right,
				11, "8088a0", FONT, 2)
			if v then
				move_image(v, W - w - 18, y + (row_h - 18) / 2)
				image_clip_on(v, CLIP_SHALLOW, inst.view)
			end
		end
	end
end

local function list_drag(inst, dy, final)
	local l = inst.list
	if not l then
		return
	end
	local maxoff = math.max(0, #l.rows * l.row_h - l.vh)
	local off = math.max(0, math.min(maxoff, l.off - dy))
	move_image(inst.content, 0, -off)
	if final then
		l.off = off
	end
end

local function list_tap(inst, x, y)
	local l = inst.list
	if not l or y < l.y0 then
		return false
	end
	local i = math.floor((y - l.y0 + l.off) / l.row_h) + 1
	if l.rows[i] and l.tap then
		l.tap(l.rows[i], i)
	end
	return true
end

local function app_header(inst, title, back)
	box(inst.a, 0, 0, W, 64, {20, 22, 34}, nil, 1.0, 20)
	text(inst.a, back and 56 or 20, 16, title, 18, "ffffff", FONT, 21)
	if back then
		text(inst.a, 18, 16, "◀", 16, "8fb4ff", FONT, 21)
	end
	inst.back = back
end

-- files
local files_app = {id = "files", name = "Files", glyph = "▤", gsize = 26,
	col = {232, 160, 48}, dock = true}

local function files_show(inst, path)
	clear_children(inst.a)
	inst.view = nil
	box(inst.a, 0, 0, W, inst.ah, {14, 16, 24}, nil, 1.0, 1)
	inst.path = path
	app_header(inst, path, path ~= "/")
	local rows = {}
	local out = run("ls -pa " .. sh_quote(path))
	for name in string.gmatch(out, "[^\n]+") do
		if name ~= "./" and name ~= "../" then
			local dir = string.sub(name, -1) == "/"
			table.insert(rows, {text = dir and string.sub(name, 1, -2) or name,
				icon = dir and "▤" or "▫", icol = dir and "e8a030" or "a0a8c0",
				dir = dir})
		end
	end
	if #rows == 0 then
		table.insert(rows, {text = "(empty or unreadable)", col = "8088a0"})
	end
	list_view(inst, 64, rows, 48, function(r)
		if r.dir then
			files_show(inst, (path == "/" and "" or path) .. "/" .. r.text)
		elseif r.icon then
			files_view(inst, (path == "/" and "" or path) .. "/" .. r.text)
		end
	end)
end

function files_view(inst, file)
	clear_children(inst.a)
	inst.view = nil
	box(inst.a, 0, 0, W, inst.ah, {14, 16, 24}, nil, 1.0, 1)
	app_header(inst, string.match(file, "[^/]+$") or file, true)
	inst.viewing = file
	local rows = {}
	local f = io and io.open(file, "rb")
	local data = f and f:read(16384) or nil
	if f then
		f:close()
	end
	if not data then
		table.insert(rows, {text = "(cannot read)", col = "8088a0"})
	elseif string.find(data, "%z") then
		table.insert(rows, {text = string.format("binary file, %d+ bytes",
			#data), col = "8088a0"})
	else
		for line in string.gmatch(data .. "\n", "([^\n]*)\n") do
			line = string.gsub(line, "\t", "    ")
			if #line > 62 then
				line = string.sub(line, 1, 61) .. "…"
			end
			table.insert(rows, {text = line, size = 10, font = FONT_M})
			if #rows > 400 then
				break
			end
		end
	end
	list_view(inst, 64, rows, 22, nil)
end

function files_app.create(inst)
	files_show(inst, "/")
end

function files_app.tap(inst, x, y)
	if y < 64 and x < 120 and inst.back then
		if inst.viewing then
			local dir = string.match(inst.viewing, "^(.*)/[^/]*$")
			inst.viewing = nil
			files_show(inst, dir == "" and "/" or dir)
		else
			local up = string.match(inst.path, "^(.*)/[^/]*$")
			files_show(inst, (up == nil or up == "") and "/" or up)
		end
		return
	end
	list_tap(inst, x, y)
end

files_app.drag = list_drag

function files_app.summary(inst)
	return inst.viewing or inst.path
end

-- system monitor
local system_app = {id = "system", name = "System", glyph = "☷", gsize = 28,
	col = {54, 132, 220}, dock = true}

local function system_refresh(inst)
	clear_children(inst.a)
	inst.view = nil
	box(inst.a, 0, 0, W, inst.ah, {14, 16, 24}, nil, 1.0, 1)
	app_header(inst, "System", false)
	local y = 80
	local function stat(label, val)
		text(inst.a, 20, y, label, 11, "8088a0")
		text(inst.a, 150, y, val, 12, "e8ecf8")
		y = y + 26
	end
	local up = ""
	local bt = tonumber(string.match(run("sysctl -n kern.boottime"),
		"sec = (%d+)") or "")
	if bt then
		local s = os.time() - bt
		up = string.format("%dh %02dm %02ds", math.floor(s / 3600),
			math.floor(s / 60) % 60, s % 60)
	end
	local mem = tonumber(run("sysctl -n hw.physmem")) or 0
	local pg = tonumber(run("sysctl -n hw.pagesize")) or 4096
	local free = tonumber(run("sysctl -n vm.stats.vm.v_free_count")) or 0
	stat("Uptime", up)
	stat("Memory", string.format("%d MB free of %d MB",
		math.floor(free * pg / 1048576), math.floor(mem / 1048576)))
	stat("CPUs", (string.gsub(run("sysctl -n hw.ncpu"), "%s", "")) ..
		" × Kryo")
	stat("Battery", sys.battery >= 0 and
		string.format("%d%%, %s", sys.battery, sys.charger) or "unknown")
	stat("Graphics", sys.renderer ~= "" and sys.renderer or "software")
	local rows = {{text = "  PID   RSS  COMMAND", size = 10, font = FONT_M,
		col = "8fb4ff"}}
	local out = run("ps -axo pid,rss,comm")
	local first = true
	for line in string.gmatch(out, "[^\n]+") do
		if not first then
			local pid, rss, cmd = string.match(line,
				"^%s*(%d+)%s+(%d+)%s+(.*)$")
			if pid then
				table.insert(rows, {text = string.format("%5s %5dM  %s", pid,
					math.floor(tonumber(rss) / 1024), cmd), size = 10,
					font = FONT_M})
			end
		end
		first = false
	end
	list_view(inst, y + 10, rows, 24, nil)
	inst.next_refresh = CLOCK + 75
end

function system_app.create(inst)
	system_refresh(inst)
end

function system_app.tick(inst)
	if CLOCK >= (inst.next_refresh or 0) and not inst.dragging then
		local off = inst.list and inst.list.off or 0
		system_refresh(inst)
		list_drag(inst, -off, true)
	end
end

function system_app.drag(inst, dy, final)
	inst.dragging = not final
	list_drag(inst, dy, final)
end

-- settings
local settings_app = {id = "settings", name = "Settings", glyph = "⚙",
	gsize = 30, col = {96, 104, 126}, dock = true}

local function settings_draw(inst)
	clear_children(inst.a)
	inst.view = nil
	inst.hits = {}
	box(inst.a, 0, 0, W, inst.ah, {14, 16, 24}, nil, 1.0, 1)
	app_header(inst, "Settings", false)
	local y = 84
	local function section(t)
		text(inst.a, 20, y, t, 11, "8fb4ff", FONT_B)
		y = y + 28
	end

	section("Display")
	text(inst.a, 20, y, "Brightness", 12, "e8ecf8")
	inst.slider = {x = 150, y = y - 6, w = W - 180, h = 34}
	box(inst.a, 150, y + 6, W - 180, 8, {60, 66, 90}, "rrect", 1.0, 2)
	local f = bl_to_frac(sys.backlight)
	inst.fill = box(inst.a, 150, y + 6, math.max(8, (W - 180) * f), 8,
		{120, 160, 255}, "rrect", 1.0, 3)
	inst.knob = box(inst.a, 150 + (W - 180) * f - 12, y - 2, 24, 24,
		{235, 240, 255}, "circle", 1.0, 4)
	y = y + 50

	section("Notification light")
	local colours = {{0, {60, 64, 80}, "off"}, {0x00ff40, {0, 230, 70}},
		{0x0040ff, {40, 90, 255}}, {0xff2000, {255, 50, 30}},
		{0xffa000, {255, 170, 0}}, {0xffffff, {250, 250, 250}}}
	for i, c in ipairs(colours) do
		local x = 20 + (i - 1) * 62
		box(inst.a, x, y, 44, 44, c[2], "circle", 1.0, 2)
		if sys.led == c[1] then
			box(inst.a, x - 4, y - 4, 52, 52, {255, 255, 255}, "circle", 0.35, 1)
		end
		if c[3] then
			text_c(inst.a, x + 22, y + 12, "off", 10, "ffffff", FONT, 3)
		end
		table.insert(inst.hits, {x = x - 6, y = y - 6, w = 56, h = 56,
			fn = function()
				sys.led = c[1]
				sysctl_set("hw.mi5.led", c[1])
				settings_draw(inst)
			end})
	end
	y = y + 70

	section("Vibration")
	box(inst.a, 20, y, 140, 42, {48, 54, 78}, "rrect", 1.0, 2)
	text_c(inst.a, 90, y + 11, "Test", 12, "ffffff", FONT, 3)
	table.insert(inst.hits, {x = 20, y = y, w = 140, h = 42, fn = function()
		sysctl_set("hw.mi5.vibrate", 300)
	end})
	y = y + 66

	section("About phone")
	local about = {
		{"Model", "Xiaomi Mi 5 (gemini)"},
		{"System", (string.gsub(run("uname -sr"), "\n", ""))},
		{"Machine", (string.gsub(run("uname -m"), "\n", "")) ..
			", " .. (string.gsub(run("sysctl -n hw.ncpu"), "%s", "")) ..
			" CPUs"},
		{"Memory", string.format("%d MB",
			math.floor((tonumber(run("sysctl -n hw.physmem")) or 0) / 1048576))},
		{"Graphics", sys.renderer ~= "" and sys.renderer or "software"},
		{"Shell", "phone, on Arcan 0.7"},
	}
	for _, a in ipairs(about) do
		text(inst.a, 20, y, a[1], 11, "8088a0")
		text(inst.a, 130, y, a[2], 11, "e8ecf8")
		y = y + 26
	end
end

function settings_app.create(inst)
	settings_draw(inst)
end

function settings_app.tap(inst, x, y)
	for _, r in ipairs(inst.hits or {}) do
		if hit(r, x, y) then
			r.fn()
			return
		end
	end
	if hit(inst.slider, x, y) then
		settings_app.slide(inst, x, true)
	end
end

function settings_app.slide(inst, x, final)
	local s = inst.slider
	local f = math.max(0, math.min(1, (x - s.x) / s.w))
	resize_image(inst.fill, math.max(8, s.w * f), 8)
	move_image(inst.knob, s.x + s.w * f - 12, s.y + 4)
	sys.backlight = frac_to_bl(f)
	if final or not inst.last_set or CLOCK - inst.last_set > 3 then
		sysctl_set("hw.mi5.backlight", sys.backlight)
		inst.last_set = CLOCK
	end
end

-- calculator
local calc_app = {id = "calc", name = "Calculator", glyph = "±", gsize = 30,
	col = {236, 98, 62}}

local calc_keys = {
	{"C", "(", ")", "÷"},
	{"7", "8", "9", "×"},
	{"4", "5", "6", "−"},
	{"1", "2", "3", "+"},
	{"0", ".", "⌫", "="},
}

local function calc_draw(inst)
	if valid_vid(inst.disp) then
		delete_image(inst.disp)
	end
	inst.disp = anchor(inst.a, 0, 0, W, 230, 5)
	local e = inst.expr == "" and "0" or inst.expr
	local v, w = text(inst.disp, 0, 110, e, #e > 14 and 22 or 34, "ffffff",
		FONT_L)
	if v then
		move_image(v, math.max(10, W - w - 24), 110)
	end
	if inst.result then
		local r, rw = text(inst.disp, 0, 170, "= " .. inst.result, 16,
			"8fb4ff", FONT)
		if r then
			move_image(r, W - rw - 24, 170)
		end
	end
end

function calc_app.create(inst)
	inst.expr = ""
	box(inst.a, 0, 0, W, inst.ah, {14, 16, 24}, nil, 1.0, 1)
	inst.keys = {}
	local kw = math.floor((W - 20) / 4)
	local kh = math.floor((inst.ah - 250) / 5)
	for r, row in ipairs(calc_keys) do
		for c, k in ipairs(row) do
			local x = 10 + (c - 1) * kw
			local y = 240 + (r - 1) * kh
			local op = c == 4 or k == "C"
			local col = k == "=" and {236, 98, 62} or
				(op and {48, 56, 86} or {34, 38, 54})
			box(inst.a, x + 5, y + 5, kw - 10, kh - 10, col, "rrect", 1.0, 2)
			text_c(inst.a, x + kw / 2, y + kh / 2 - 16, k, 20,
				op and "8fb4ff" or "ffffff", FONT, 3)
			table.insert(inst.keys, {x = x, y = y, w = kw, h = kh, k = k})
		end
	end
	calc_draw(inst)
end

-- a small recursive-descent evaluator: + - * / ( ), unary minus
local function calc_eval(expr)
	local src = string.gsub(string.gsub(string.gsub(expr, "×", "*"), "÷",
		"/"), "−", "-")
	local pos = 1
	local sum
	local function peek()
		return string.sub(src, pos, pos)
	end
	local function factor()
		local c = peek()
		if c == "-" or c == "+" then
			pos = pos + 1
			local v = factor()
			return v and (c == "-" and -v or v)
		elseif c == "(" then
			pos = pos + 1
			local v = sum()
			if peek() ~= ")" then
				return nil
			end
			pos = pos + 1
			return v
		end
		local n = string.match(src, "^%d*%.?%d+", pos) or
			string.match(src, "^%d+%.?", pos)
		if not n then
			return nil
		end
		pos = pos + #n
		return tonumber(n)
	end
	local function product()
		local v = factor()
		while v do
			local c = peek()
			if c ~= "*" and c ~= "/" then
				break
			end
			pos = pos + 1
			local r = factor()
			if not r then
				return nil
			end
			v = c == "*" and v * r or v / r
		end
		return v
	end
	sum = function()
		local v = product()
		while v do
			local c = peek()
			if c ~= "+" and c ~= "-" then
				break
			end
			pos = pos + 1
			local r = product()
			if not r then
				return nil
			end
			v = c == "+" and v + r or v - r
		end
		return v
	end
	local v = sum()
	if not v or pos <= #src then
		return nil
	end
	if v ~= v or v == math.huge or v == -math.huge then
		return "error"
	end
	return string.format("%.10g", v)
end

function calc_app.tap(inst, x, y)
	for _, k in ipairs(inst.keys) do
		if hit(k, x, y) then
			local s = k.k
			if s == "C" then
				inst.expr = ""
				inst.result = nil
			elseif s == "⌫" then
				local i = #inst.expr
				while i > 0 and string.byte(inst.expr, i) >= 0x80 and
					string.byte(inst.expr, i) < 0xc0 do
					i = i - 1
				end
				inst.expr = string.sub(inst.expr, 1, i - 1)
				inst.result = calc_eval(inst.expr)
			elseif s == "=" then
				local r = calc_eval(inst.expr)
				if r then
					inst.expr = r
				end
				inst.result = nil
			else
				inst.expr = inst.expr .. s
				inst.result = calc_eval(inst.expr)
			end
			calc_draw(inst)
			return
		end
	end
end

function calc_app.summary(inst)
	return inst.expr ~= "" and inst.expr or "0"
end

-- clock: the time, and a stopwatch
local clock_app = {id = "clock", name = "Clock", glyph = "◔", gsize = 30,
	col = {40, 46, 70}, gcol = "8fb4ff"}

local function clock_draw(inst)
	if valid_vid(inst.face) then
		delete_image(inst.face)
	end
	inst.face = anchor(inst.a, 0, 0, W, inst.ah, 5)
	text_c(inst.face, W / 2, 70, os.date("%H:%M:%S"), 52, "ffffff", FONT_L)
	text_c(inst.face, W / 2, 150, date_str(), 13, "a0a8c0")
	local t = inst.elapsed + (inst.running and (CLOCK - inst.start) or 0)
	local cs = math.floor(t * 4)	-- CLOCK is 25 Hz: hundredths
	text_c(inst.face, W / 2, 300, string.format("%02d:%02d.%02d",
		math.floor(cs / 6000), math.floor(cs / 100) % 60, cs % 100), 44,
		"8fb4ff", FONT_L)
	text_c(inst.face, W / 2, 370, "stopwatch", 11, "8088a0")
	local bw = 180
	for i, b in ipairs({{inst.running and "Stop" or "Start",
		inst.running and {200, 70, 60} or {60, 150, 90}}, {"Reset",
		{48, 54, 78}}}) do
		local x = W / 2 - bw - 10 + (i - 1) * (bw + 20)
		box(inst.face, x, 440, bw, 64, b[2], "rrect", 1.0, 2)
		text_c(inst.face, x + bw / 2, 458, b[1], 15, "ffffff", FONT, 3)
	end
	for i, lap in ipairs(inst.laps) do
		if i > 8 then
			break
		end
		text_c(inst.face, W / 2, 540 + (i - 1) * 30, lap, 12, "c8d0e0",
			FONT_M)
	end
end

function clock_app.create(inst)
	box(inst.a, 0, 0, W, inst.ah, {14, 16, 24}, nil, 1.0, 1)
	inst.elapsed, inst.running, inst.laps = 0, false, {}
	clock_draw(inst)
	inst.last = ""
end

function clock_app.tap(inst, x, y)
	if y >= 440 and y < 504 then
		if x < W / 2 then
			if inst.running then
				inst.elapsed = inst.elapsed + CLOCK - inst.start
			else
				inst.start = CLOCK
			end
			inst.running = not inst.running
		else
			if inst.running then
				local cs = math.floor((inst.elapsed + CLOCK - inst.start) * 4)
				table.insert(inst.laps, 1, string.format("lap %d  %02d:%02d.%02d",
					#inst.laps + 1, math.floor(cs / 6000),
					math.floor(cs / 100) % 60, cs % 100))
			else
				inst.elapsed, inst.laps = 0, {}
			end
		end
		clock_draw(inst)
	end
end

function clock_app.tick(inst)
	local s = os.date("%S")
	if inst.running or s ~= inst.last then
		inst.last = s
		clock_draw(inst)
	end
end

function clock_app.summary(inst)
	return os.date("%H:%M")
end

apps = {calc_app, clock_app, term_app, files_app, system_app, settings_app}
for _, a in ipairs(apps) do
	app_by_id[a.id] = a
end

------------------------------------------------------------------------
-- running apps

local function touch_order(id)
	for i, v in ipairs(run_order) do
		if v == id then
			table.remove(run_order, i)
			break
		end
	end
	table.insert(run_order, 1, id)
end

open_app = function(id)
	local app = app_by_id[id]
	if not app then
		return
	end
	if drawer.open then
		drawer_show(false, 0)
	end
	local inst = running[id]
	local ah = H - SB
	if not inst then
		inst = {app = app, ah = ah}
		inst.a = anchor(nil, 0, SB, W, ah, O_APP)
		running[id] = inst
		app.create(inst, ah)
	end
	if focus and focus ~= id and running[focus] then
		hide_image(running[focus].a)
	end
	touch_order(id)
	focus = id
	instant_image_transform(inst.a)
	show_image(inst.a)
	move_image(inst.a, 0, SB + 60)
	blend_image(inst.a, 0.0)
	move_image(inst.a, 0, SB, 6, INTERP_EXPOUT)
	blend_image(inst.a, 1.0, 5)
	draw_status()
	redraw_pages()
end

-- leave the app running, back to the home views
leave_app = function(dir)
	if not focus then
		return
	end
	local inst = running[focus]
	focus = nil
	instant_image_transform(inst.a)
	move_image(inst.a, (dir or 1) * W, SB, 6, INTERP_EXPOUT)
	blend_image(inst.a, 0.0, 6)
	draw_status()
	redraw_pages()
end

close_app = function(id)
	local inst = running[id]
	if not inst then
		return
	end
	running[id] = nil
	for i, v in ipairs(run_order) do
		if v == id then
			table.remove(run_order, i)
			break
		end
	end
	if focus == id then
		focus = nil
	end
	if inst.app.destroy then
		inst.app.destroy(inst)
	end
	if valid_vid(inst.a) then
		instant_image_transform(inst.a)
		move_image(inst.a, 0, H, 6, INTERP_EXPIN)
		expire_image(inst.a, 7)
	end
	draw_status()
	redraw_pages()
end

go_home = function()
	if qs.open then
		qs_show(false)
	elseif drawer.open then
		drawer_show(false)
	elseif focus then
		leave_app(1)
		set_page(2)
	else
		set_page(2)
	end
end

------------------------------------------------------------------------
-- gestures

local EDGE = 22

local function on_down(x, y)
	g = {x0 = x, y0 = y, x = x, y = y, t0 = CLOCK, mode = nil}
	if locked or screen_off then
		return
	end
	g.edge = (x < EDGE and "left") or (x > W - EDGE and "right") or
		(y < SB and "top") or nil
	-- keys act on touch-down, for typing speed
	if not qs.open and focus and not g.edge then
		local inst = running[focus]
		if inst.app.down and inst.app.down(inst, x, y - SB) then
			g.consumed = true
		end
	end
	if qs.open and hit(qs.slider, x, y) then
		g.mode = "qs_slider"
		qs_slider(x)
	elseif focus and running[focus].app.slide and running[focus].slider and
		hit(running[focus].slider, x, y - SB) then
		g.mode = "slider"
	end
end

local function on_move(x, y)
	g.x, g.y = x, y
	local dx, dy = x - g.x0, y - g.y0
	if g.consumed then
		return
	end
	if g.mode == "qs_slider" then
		qs_slider(x)
		return
	elseif g.mode == "slider" then
		running[focus].app.slide(running[focus], x, false)
		return
	end
	if not g.mode then
		if math.abs(dx) < 12 and math.abs(dy) < 12 then
			return
		end
		local horiz = math.abs(dx) > math.abs(dy)
		if locked then
			g.mode = (not horiz and not screen_off) and "unlock" or "none"
		elseif qs.open then
			g.mode = "qs"
		elseif focus then
			if g.edge == "left" or g.edge == "right" then
				g.mode = "leave"
			elseif g.edge == "top" and not horiz then
				g.mode = "close"
			elseif not horiz and running[focus].app.drag then
				g.mode = "scroll"
			else
				g.mode = "none"
			end
		elseif drawer.open then
			g.mode = not horiz and "drawer" or "none"
		elseif g.edge == "top" and not horiz then
			g.mode = "qs"
			draw_qs()
		elseif horiz then
			g.mode = "pan"
		elseif dy < 0 and page == 2 then
			g.mode = "drawer"
		else
			g.mode = "none"
		end
	end
	local m = g.mode
	if m == "unlock" then
		instant_image_transform(lock.a)
		move_image(lock.a, 0, math.min(0, dy))
		blend_image(lock.a, 1.0 + math.min(0, dy) / H)
	elseif m == "pan" then
		instant_image_transform(strip)
		local base = -(page - 1) * W
		local lim = dx
		if (page == 1 and dx > 0) or (page == 3 and dx < 0) then
			lim = dx / 3
		end
		move_image(strip, base + lim, 0)
	elseif m == "drawer" then
		instant_image_transform(drawer.a)
		local yy = drawer.open and math.max(0, dy) or math.max(0, H + dy)
		move_image(drawer.a, 0, yy)
	elseif m == "qs" then
		instant_image_transform(qs.a)
		local yy = qs.open and math.min(0, dy) or math.min(0, -QS_H + dy)
		move_image(qs.a, 0, yy)
	elseif m == "leave" then
		instant_image_transform(running[focus].a)
		move_image(running[focus].a, dx, SB)
	elseif m == "close" then
		instant_image_transform(running[focus].a)
		move_image(running[focus].a, 0, SB + math.max(0, dy))
		blend_image(running[focus].a, 1.0 - math.max(0, dy) / H)
	elseif m == "scroll" then
		local inst = running[focus]
		inst.app.drag(inst, dy, false)
	end
end

local function tap_page(x, y)
	local p = pages[page]
	for _, r in ipairs(p.hits or {}) do
		if hit(r, x - 0, y) then
			if r.app then
				open_app(r.app.id)
			elseif r.clear then
				events = {}
				redraw_pages()
			elseif r.close then
				close_app(r.close)
			elseif r.open then
				open_app(r.open)
			end
			return
		end
	end
end

local function on_tap(x, y)
	if locked then
		return
	end
	if qs.open then
		if y > QS_H then
			qs_show(false)
			return
		end
		for _, r in ipairs(qs.hits) do
			if hit(r, x, y) then
				r.fn()
				if qs.open then
					draw_qs()
				end
				return
			end
		end
		return
	end
	if y < SB then
		qs_show(true)
		return
	end
	if focus then
		local inst = running[focus]
		if g.consumed then
			return
		end
		if inst.app.tap then
			inst.app.tap(inst, x, y - SB)
		end
		return
	end
	if drawer.open then
		for _, r in ipairs(drawer.hits) do
			if hit(r, x, y) then
				open_app(r.app.id)
				return
			end
		end
		return
	end
	tap_page(x, y)
end

local function on_up(x, y)
	local dx, dy = x - g.x0, y - g.y0
	local m = g.mode
	if not m then
		on_tap(x, y)
	elseif m == "unlock" then
		if dy < -H / 5 then
			lock_screen(false)
		else
			move_image(lock.a, 0, 0, 5, INTERP_EXPOUT)
			blend_image(lock.a, 1.0, 5)
		end
	elseif m == "pan" then
		if dx < -W / 5 then
			set_page(page + 1)
		elseif dx > W / 5 then
			set_page(page - 1)
		else
			set_page(page)
		end
	elseif m == "drawer" then
		if drawer.open then
			drawer_show(dy < H / 5)
		else
			drawer_show(dy < -H / 6)
		end
	elseif m == "qs" then
		if qs.open then
			qs_show(dy > -QS_H / 4)
		else
			qs_show(dy > QS_H / 4)
		end
	elseif m == "leave" then
		if math.abs(dx) > W / 4 then
			leave_app(dx > 0 and 1 or -1)
		else
			move_image(running[focus].a, 0, SB, 5, INTERP_EXPOUT)
		end
	elseif m == "close" then
		if dy > H / 4 then
			close_app(focus)
		else
			move_image(running[focus].a, 0, SB, 5, INTERP_EXPOUT)
			blend_image(running[focus].a, 1.0, 5)
		end
	elseif m == "scroll" then
		local inst = running[focus]
		inst.app.drag(inst, dy, true)
	elseif m == "qs_slider" then
		qs_slider(x, true)
	elseif m == "slider" then
		running[focus].app.slide(running[focus], x, true)
	end
	g = nil
end

------------------------------------------------------------------------
-- entry points

local autorun, test_view

function phone(args)
	W, H = VRESW, VRESH
	for _, a in ipairs(args or {}) do
		local v = string.match(a, "^autorun=(.*)")
		if v then
			autorun = v
		end
		v = string.match(a, "^test=(.*)")
		if v then
			test_view = v
		end
		v = string.match(a, "^touchw=(%d+)")
		if v then
			touch_w = tonumber(v)
		end
	end
	tscale = W / touch_w
	system_defaultfont(FONT, 12, 1)
	build_shaders()

	wall = box(nil, 0, 0, W, H, {0, 0, 0}, "wall", 1.0, O_WALL)
	strip = anchor(nil, 0, 0, W * 3, H, O_HOME)
	for i = 1, 3 do
		pages[i] = {a = anchor(strip, (i - 1) * W, 0, W, H, 1), hits = {}}
	end
	drawer.a = anchor(nil, 0, H, W, H, O_DRAWER)
	qs.a = anchor(nil, 0, -QS_H - 10, W, QS_H, O_QS)
	lock.a = anchor(nil, 0, 0, W, H, O_LOCK)

	local bl = tonumber(run("sysctl -n hw.mi5.backlight"))
	if bl and bl > 0 then
		sys.backlight = bl
	end
	sys.led = tonumber(run("sysctl -n hw.mi5.led")) or 0
	local rf = io and io.open("/tmp/arcan/renderer", "r")
	if rf then
		local r = rf:read("*l") or ""
		rf:close()
		sys.gpu = string.find(r, "on the GPU") ~= nil
	end
	sys.renderer = sys.gpu and "Adreno 530 (freedreno)" or "software (softpipe)"
	read_status()

	draw_status()
	sbar.dots = {}
	for i = 1, 3 do
		local d = box(nil, W / 2 - 22 + (i - 1) * 18, SB + 10, 8, 8,
			{255, 255, 255}, "circle", 0.35, O_HOME + 5)
		table.insert(sbar.dots, d)
	end
	draw_drawer()
	add_event("◆", "Welcome to DragonFly BSD",
		"Swipe sideways for events and open apps", true)
	add_event("▲", sys.gpu and "Graphics: Adreno 530" or
		"Graphics: software", sys.gpu and "Mesa freedreno on the GPU" or
		"The GPU did not start; drawing in software", true)
	set_page(2, 0)
	lock_screen(true)

	if test_view then
		lock_screen(false, 0)
		if test_view == "lock" then
			lock_screen(true)
		elseif test_view == "events" then
			set_page(1, 0)
		elseif test_view == "drawer" then
			drawer_show(true, 0)
		elseif test_view == "qs" then
			qs_show(true, 0)
		elseif test_view == "switcher" then
			open_app("calc")
			open_app("term")
			leave_app(1)
			set_page(3, 0)
		elseif app_by_id[test_view] then
			open_app(test_view)
		end
	end
	if autorun then
		lock_screen(false, 0)
		open_app("term")
		autorun_at = CLOCK + 75
	end
end

local function button_code(iotbl)
	if iotbl.translated then
		return iotbl.number
	elseif iotbl.digital then
		return iotbl.subid
	end
end

local touching = nil

function phone_input(iotbl)
	if iotbl.kind == "status" then
		return
	end
	if iotbl.touch then
		local x, y = iotbl.x * tscale, iotbl.y * tscale
		if iotbl.active then
			if not touching then
				touching = iotbl.subid
				on_down(x, y)
			elseif touching == iotbl.subid and g then
				on_move(x, y)
			end
		elseif touching == iotbl.subid then
			touching = nil
			if g then
				on_up(g.x, g.y)
			end
		end
		return
	end
	local code = button_code(iotbl)
	if code == KEY_POWER or code == KEY_HOME or code == KEY_VOLUP or
		code == KEY_VOLDOWN then
		if not iotbl.active then
			return
		end
		if code == KEY_POWER then
			set_screen(screen_off)
		elseif screen_off then
			if code == KEY_HOME then
				set_screen(true)
			end
		elseif code == KEY_HOME then
			if not locked then
				go_home()
			end
		else
			local f = bl_to_frac(sys.backlight) +
				(code == KEY_VOLUP and 0.1 or -0.1)
			sys.backlight = frac_to_bl(math.max(0, math.min(1, f)))
			sysctl_set("hw.mi5.backlight", sys.backlight)
			show_banner("☀", "Brightness",
				string.format("%d%%", math.floor(bl_to_frac(sys.backlight) * 100 + 0.5)))
		end
		return
	end
	if focus and running[focus].app.input then
		running[focus].app.input(running[focus], iotbl)
	end
end

local last_min, next_status = "", 0

function phone_clock_pulse()
	if kbd.hl_until and CLOCK >= kbd.hl_until then
		blend_image(kbd.hl, 0.0)
		kbd.hl_until = nil
	end
	if banner.hide_at and CLOCK >= banner.hide_at and valid_vid(banner.v) then
		move_image(banner.v, 10, -90, 8, INTERP_EXPIN)
		expire_image(banner.v, 9)
		banner.v = nil
		banner.hide_at = nil
	end
	if CLOCK >= next_status then
		next_status = CLOCK + 250
		read_status()
		draw_status()
	end
	local m = clock_str()
	if m ~= last_min then
		last_min = m
		draw_status()
		if not focus and not g then
			draw_home(pages[2])
		end
		if locked and not g then
			draw_lock()
		end
	end
	if focus and running[focus].app.tick then
		running[focus].app.tick(running[focus])
	end
	if autorun and autorun_at and CLOCK >= autorun_at and running.term and
		valid_vid(running.term.vid, TYPE_FRAMESERVER) then
		term_type(running.term, autorun .. "\r")
		autorun = nil
	end
end
