-- phone: the shell for DragonFly BSD on the Xiaomi Mi 5.
--
-- Layout: the N9's and Sailfish's swipe model - three home views side by
-- side, panned with a horizontal swipe -
--
--   Events  |  Home  |  Open apps
--
-- with an Android-style home (clock, app grid, dock, drawer).  The look
-- follows QRT's (GNOME, libadwaita dark): a wallpaper of soft accent
-- light, Inter type, flat line icons on rounded tiles, grouped rows,
-- header bars with minimise and close; from Sailfish come the accent as
-- an "ambience" chosen in the top menu, apps zooming out of their icons
-- and a lit edge under an edge swipe.
--
-- Gestures: in an app, swipe in from the left or right edge to leave it
-- running, swipe down from the top edge to close it.  Tap the status bar
-- (or pull it down at home) for quick settings.  Power locks, home goes
-- home, volume sets the brightness.
--
-- Built-in apps draw into their own render targets, which gives live
-- thumbnails and the zoom; the terminal is Arcan's terminal frameserver.
--
-- Arguments: autorun=CMD (opens the terminal and types CMD), test=VIEW
-- (opens a view at start, for QEMU screenshots), touchw=N (touch panel
-- width in pixels, 1080).

local W, H
local SB = 32			-- status bar
local HB = 58			-- an app's header bar
local touch_w = 1080
local tscale = 1

local KEY_HOME, KEY_VOLDOWN, KEY_VOLUP, KEY_POWER = 102, 114, 115, 116

local F_REG = "Inter-Regular.ttf"
local F_MED = "Inter-Medium.ttf"
local F_SB = "Inter-SemiBold.ttf"
local F_LIGHT = "InterDisplay-Light.ttf"
local F_SYM = "DejaVuSans.ttf"
local F_MONO = "DejaVuSansMono.ttf"

-- QRT's colours
local C = {
	text = "ffffff", text2 = "c6cad2", text3 = "878c97",
	window = {0x22, 0x24, 0x28}, header = {0x2c, 0x2f, 0x34},
	panel = {0x18, 0x1a, 0x20}, card = 0.065, card_hi = 0.12,
	stroke = 0.09,
}
local ACCENTS = {
	{"Blue", {0x35, 0x84, 0xe4}}, {"Teal", {0x21, 0x90, 0xa4}},
	{"Green", {0x3a, 0x94, 0x4a}}, {"Orange", {0xed, 0x5b, 0x00}},
	{"Purple", {0x91, 0x41, 0xac}}, {"Crimson", {0xc0, 0x1c, 0x28}},
}
local accent_i = 1
local function accent()
	return ACCENTS[accent_i][2]
end
local function hex(c)
	return string.format("%02x%02x%02x", c[1], c[2], c[3])
end
local function lighten(c, f)
	return {math.min(255, math.floor(c[1] + (255 - c[1]) * f)),
		math.min(255, math.floor(c[2] + (255 - c[2]) * f)),
		math.min(255, math.floor(c[3] + (255 - c[3]) * f))}
end

-- order bands in the world
local O_WALL, O_HOME, O_DRAWER, O_APP = 1, 10, 60, 100
local O_EDGE, O_KBD, O_LOCK, O_SB, O_QS, O_BANNER = 170, 180, 190, 200, 300, 350

------------------------------------------------------------------------
-- shaders

local shaders = {}

local function build_shaders()
	local head = [[
uniform float obj_opacity;
uniform vec3 obj_col;
uniform vec2 obj_output_sz;
varying vec2 texco;
float rr(vec2 p, vec2 sz, float r)
{
	vec2 q = abs(p - 0.5 * sz) - (0.5 * sz - vec2(r));
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
]]
	local function mk(name, body)
		shaders[name] = build_shader(nil, head .. body, "phone_" .. name)
	end
	-- rounded rectangle; radius a share of the short side, capped
	mk("rrect", [[
void main()
{
	vec2 sz = obj_output_sz;
	float r = min(0.26 * min(sz.x, sz.y), 18.0);
	float d = rr(texco * sz, sz, r);
	gl_FragColor = vec4(obj_col, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]])
	mk("pill", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz, sz, 0.5 * min(sz.x, sz.y));
	gl_FragColor = vec4(obj_col, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]])
	mk("circle", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = length(texco * sz - 0.5 * sz) - 0.5 * min(sz.x, sz.y);
	gl_FragColor = vec4(obj_col, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]])
	-- a ring, a tenth of the diameter thick
	mk("ring", [[
void main()
{
	vec2 sz = obj_output_sz;
	float r = 0.5 * min(sz.x, sz.y);
	float d = abs(length(texco * sz - 0.5 * sz) - r * 0.88) - r * 0.10;
	gl_FragColor = vec4(obj_col, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]])
	-- an app icon's tile: rounded, lighter at the top, a soft lower edge
	mk("tile", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz, sz, 0.27 * min(sz.x, sz.y));
	vec3 c = obj_col * mix(1.16, 0.90, texco.y);
	c = mix(c, vec3(1.0), 0.10 * smoothstep(0.48, 0.44, texco.y));
	gl_FragColor = vec4(min(c, vec3(1.0)),
		obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]])
	-- a soft shadow: a rounded rectangle blurred by 10 px
	mk("shadow", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz - vec2(10.0), sz - vec2(20.0),
		0.22 * min(sz.x, sz.y));
	gl_FragColor = vec4(obj_col, obj_opacity *
		(1.0 - smoothstep(-6.0, 10.0, d)));
}]])
	-- a texture (an app's render target, a thumbnail) with round corners
	shaders.rimg = build_shader(nil, [[
uniform sampler2D map_tu0;
uniform float obj_opacity;
uniform vec2 obj_output_sz;
varying vec2 texco;
void main()
{
	vec2 sz = obj_output_sz;
	vec2 q = abs(texco * sz - 0.5 * sz) - (0.5 * sz - vec2(16.0));
	float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - 16.0;
	vec4 t = texture2D(map_tu0, texco);
	gl_FragColor = vec4(t.rgb, t.a * obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]], "phone_rimg")
	-- QRT's wallpaper: charcoal-blue, a light of the accent at the top
	-- right, cooler lights low and to the left
	mk("wall", [[
uniform vec3 accent;
float glow(vec2 p, vec2 c, float r)
{
	float d2 = dot(p - c, p - c) / (r * r);
	float t = max(1.0 - d2, 0.0);
	return t * t;
}
void main()
{
	vec2 sz = obj_output_sz;
	vec2 p = texco * sz;
	float m = max(sz.x, sz.y);
	vec3 c = mix(vec3(0.102, 0.122, 0.169), vec3(0.047, 0.059, 0.082),
		texco.y);
	c = mix(c, accent, 0.42 * glow(p, vec2(0.86, 0.04) * sz, 0.66 * m));
	c = mix(c, vec3(0.30, 0.17, 0.58), 0.30 * glow(p, vec2(0.05, 0.42) * sz,
		0.48 * m));
	c = mix(c, vec3(0.06, 0.40, 0.46), 0.28 * glow(p, vec2(0.22, 0.98) * sz,
		0.52 * m));
	gl_FragColor = vec4(c, obj_opacity);
}]])
end

local function set_wall_accent()
	local a = accent()
	shader_uniform(shaders.wall, "accent", "fff", a[1] / 255, a[2] / 255,
		a[3] / 255)
end

------------------------------------------------------------------------
-- drawing helpers.  While an app builds its content cur_rt names its
-- render target, and every object made is attached to it.

local cur_rt = nil

local function adopt(v)
	if cur_rt and valid_vid(cur_rt) then
		rendertarget_attach(cur_rt, v, RENDERTARGET_DETACH)
	end
	image_mask_set(v, MASK_UNPICKABLE)
end

local function place(v, parent, x, y, order)
	if parent then
		link_image(v, parent)
		image_inherit_order(v, true)
	end
	move_image(v, math.floor(x), math.floor(y))
	order_image(v, order or 1)
	adopt(v)
end

-- a coloured shape; col is {r, g, b}
local function box(parent, x, y, w, h, col, shape, alpha, order)
	local v = color_surface(math.max(1, math.floor(w)),
		math.max(1, math.floor(h)), col[1], col[2], col[3])
	if not valid_vid(v) then
		return nil
	end
	alpha = alpha or 1.0
	if shape and shaders[shape] then
		image_shader(v, shaders[shape])
		-- opaque objects are drawn unblended: keep just under 1
		alpha = math.min(alpha, 0.996)
	end
	place(v, parent, x, y, order)
	blend_image(v, alpha)
	if alpha < 1.0 then
		force_image_blend(v, BLEND_NORMAL)
	end
	return v
end

-- a translucent white surface (QRT's cards), alpha is the whiteness
local function card(parent, x, y, w, h, alpha, order, shape)
	return box(parent, x, y, w, h, {255, 255, 255}, shape or "rrect",
		alpha or C.card, order)
end

-- text; the string goes in raw (never as a format string)
local function text(parent, x, y, str, size, color, font, order)
	local v = render_text({string.format([[\f%s,%d\#%s]], font or F_REG,
		size, color or C.text), tostring(str)})
	if not valid_vid(v) then
		return nil, 0, 0
	end
	local p = image_surface_properties(v)
	place(v, parent, x, y, order or 2)
	show_image(v)
	return v, p.width, p.height
end

local function text_c(parent, cx, y, str, size, color, font, order)
	local v, w, h = text(parent, 0, y, str, size, color, font, order)
	if v then
		move_image(v, math.floor(cx - w / 2), math.floor(y))
	end
	return v, w, h
end

local function text_r(parent, rx, y, str, size, color, font, order)
	local v, w, h = text(parent, 0, y, str, size, color, font, order)
	if v then
		move_image(v, math.floor(rx - w), math.floor(y))
	end
	return v, w, h
end

local function anchor(parent, x, y, w, h, order)
	local v = null_surface(math.max(1, w or 1), math.max(1, h or 1))
	place(v, parent, x or 0, y or 0, order)
	show_image(v)
	return v
end

-- a bar of length len and thickness th, centred on cx, cy, at angle deg
-- (0 points up), for clock hands and chevrons
local function bar(parent, cx, cy, len, th, deg, col, order, alpha)
	local v = box(parent, 0, 0, th, len, col, "pill", alpha or 1.0, order)
	if v then
		move_image(v, math.floor(cx - th / 2), math.floor(cy - len / 2))
		rotate_image(v, deg)
	end
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
	return r and x >= r.x and x < r.x + r.w and y >= r.y and y < r.y + r.h
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
local page = 2
local strip, pages = nil, {}
local wall
local apps, app_by_id = {}, {}
local running, run_order = {}, {}
local focus = nil
local drawer = {open = false}
local qs = {open = false}
local lock = {}
local sbar = {}
local edge = {}
local events = {}
local g = nil
local redraw_pages, redraw_all
local open_app, leave_app, close_app, go_home, set_page, add_event
local qs_show, lock_screen, draw_qs

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
			add_event("charge", "Charger connected",
				string.format("Battery at %d%%", sys.battery))
		elseif sys.charger == "full" then
			add_event("charge", "Battery full", "You can unplug the charger")
		elseif sys.charger == "none" then
			add_event("charge", "Charger disconnected",
				string.format("Battery at %d%%", sys.battery))
		end
	end
	if old.battery > 15 and sys.battery >= 0 and sys.battery <= 15 then
		add_event("battery", "Battery low",
			string.format("%d%% left", sys.battery))
	end
end

local function clock_str()
	return os.date("%H:%M")
end

local function date_str()
	return (string.gsub(os.date("%A, %d %B"), " 0(%d)", " %1"))
end

local function short_date()
	return (string.gsub(os.date("%a %d %b"), " 0(%d)", " %1"))
end

local function battery_text()
	if sys.battery < 0 then
		return "Battery unknown"
	end
	local st = sys.charger == "charging" and "charging" or
		(sys.charger == "full" and "full" or
		(sys.charger == "present" and "plugged in" or "on battery"))
	return string.format("%d%% · %s", sys.battery, st)
end

------------------------------------------------------------------------
-- icons: flat white line drawings on rounded tiles, as QRT's

local ICON = 64

local glyphs = {}

function glyphs.folder(p, s, col)
	box(p, s * 0.20, s * 0.30, s * 0.26, s * 0.14, {255, 255, 255}, "rrect",
		1.0, 3)
	box(p, s * 0.20, s * 0.36, s * 0.60, s * 0.38, {255, 255, 255}, "rrect",
		1.0, 3)
end

function glyphs.terminal(p, s, col)
	bar(p, s * 0.33, s * 0.42, s * 0.20, s * 0.075, -45, {255, 255, 255}, 3)
	bar(p, s * 0.33, s * 0.55, s * 0.20, s * 0.075, 45, {255, 255, 255}, 3)
	box(p, s * 0.48, s * 0.60, s * 0.22, s * 0.075, {255, 255, 255}, "pill",
		1.0, 3)
end

function glyphs.gear(p, s, col)
	local c = s / 2
	for i = 0, 3 do
		bar(p, c, c, s * 0.62, s * 0.13, i * 45, {255, 255, 255}, 3)
	end
	box(p, c - s * 0.21, c - s * 0.21, s * 0.42, s * 0.42, {255, 255, 255},
		"circle", 1.0, 4)
	box(p, c - s * 0.09, c - s * 0.09, s * 0.18, s * 0.18, col, "circle",
		1.0, 5)
end

function glyphs.chip(p, s, col)
	local c = s / 2
	for i = -1, 1 do
		box(p, c + i * s * 0.13 - s * 0.025, s * 0.18, s * 0.05, s * 0.64,
			{255, 255, 255}, "pill", 1.0, 3)
		box(p, s * 0.18, c + i * s * 0.13 - s * 0.025, s * 0.64, s * 0.05,
			{255, 255, 255}, "pill", 1.0, 3)
	end
	box(p, c - s * 0.24, c - s * 0.24, s * 0.48, s * 0.48, {255, 255, 255},
		"rrect", 1.0, 4)
	box(p, c - s * 0.18, c - s * 0.18, s * 0.36, s * 0.36, col, "rrect",
		1.0, 5)
	box(p, c - s * 0.08, c - s * 0.08, s * 0.16, s * 0.16, {255, 255, 255},
		"rrect", 1.0, 6)
end

function glyphs.clock(p, s, col)
	local c = s / 2
	box(p, c - s * 0.30, c - s * 0.30, s * 0.60, s * 0.60, {255, 255, 255},
		"ring", 1.0, 3)
	bar(p, c, c - s * 0.08, s * 0.20, s * 0.065, 0, {255, 255, 255}, 4)
	bar(p, c + s * 0.06, c, s * 0.15, s * 0.065, 90, {255, 255, 255}, 4)
end

function glyphs.calc(p, s, col)
	local c = s / 2
	local w = s * 0.17
	-- plus
	bar(p, c - w, c - w, w * 1.1, s * 0.06, 0, {255, 255, 255}, 3)
	bar(p, c - w, c - w, w * 1.1, s * 0.06, 90, {255, 255, 255}, 3)
	-- minus
	bar(p, c + w, c - w, w * 1.1, s * 0.06, 90, {255, 255, 255}, 3)
	-- times
	bar(p, c - w, c + w, w * 1.1, s * 0.06, 45, {255, 255, 255}, 3)
	bar(p, c - w, c + w, w * 1.1, s * 0.06, -45, {255, 255, 255}, 3)
	-- equals
	bar(p, c + w, c + w - s * 0.045, w * 1.1, s * 0.06, 90, {255, 255, 255}, 3)
	bar(p, c + w, c + w + s * 0.045, w * 1.1, s * 0.06, 90, {255, 255, 255}, 3)
end

function glyphs.dots(p, s, col)
	for i = 0, 2 do
		for j = 0, 2 do
			box(p, s * (0.30 + i * 0.16) - s * 0.045,
				s * (0.30 + j * 0.16) - s * 0.045, s * 0.09, s * 0.09,
				{255, 255, 255}, "circle", 1.0, 3)
		end
	end
end

local function draw_icon(parent, x, y, app, size, with_label)
	size = size or ICON
	local cell = anchor(parent, x, y, size, size, 1)
	if size >= 40 then
		box(cell, -10, -6, size + 20, size + 22, {0, 0, 0}, "shadow", 0.55, 0)
	end
	box(cell, 0, 0, size, size, app.col, "tile", 1.0, 1)
	glyphs[app.glyph](cell, size, app.col)
	if with_label then
		text_c(cell, size / 2, size + 7, app.name, 10, C.text, F_MED, 3)
	end
	return cell
end

-- a grid of icons; returns hit rects
local function grid(parent, list, y0, cols, labels)
	local rects = {}
	local cw = W / cols
	for i, app in ipairs(list) do
		local c = (i - 1) % cols
		local r = math.floor((i - 1) / cols)
		local x = math.floor(c * cw + (cw - ICON) / 2)
		local y = y0 + r * (ICON + 44)
		draw_icon(parent, x, y, app, ICON, labels)
		table.insert(rects, {x = x - 12, y = y - 8, w = ICON + 24,
			h = ICON + 36, app = app, ix = x, iy = y})
	end
	return rects
end

------------------------------------------------------------------------
-- status bar: QRT's dark top bar, date and time in the middle

local function draw_status()
	if not valid_vid(sbar.bg) then
		sbar.bg = box(nil, 0, 0, W, SB, {0, 0, 0}, nil, 0.55, O_SB)
		sbar.items = anchor(nil, 0, 0, W, SB, O_SB + 1)
	end
	clear_children(sbar.items)
	text_c(sbar.items, W / 2, 8, short_date() .. "   " .. clock_str(), 11,
		C.text, F_SB)
	-- battery
	local bx, by, bw, bh = W - 42, 10, 24, 12
	box(sbar.items, bx, by, bw, bh, {235, 238, 245}, "rrect", 0.95, 1)
	box(sbar.items, bx + 1.5, by + 1.5, bw - 3, bh - 3, {10, 10, 14},
		"rrect", 1.0, 2)
	local pct = math.max(0, math.min(100, sys.battery))
	local col = pct <= 15 and {237, 51, 59} or
		(sys.charger == "charging" and {87, 227, 137} or {235, 238, 245})
	if pct > 0 then
		box(sbar.items, bx + 3, by + 3, math.max(2, (bw - 6) * pct / 100),
			bh - 6, col, "rrect", 1.0, 3)
	end
	box(sbar.items, bx + bw + 1, by + 3.5, 2.5, bh - 7, {235, 238, 245},
		"rrect", 0.95, 1)
	local s = sys.battery >= 0 and string.format("%d%%", sys.battery) or ""
	text_r(sbar.items, bx - (sys.charger == "charging" and 18 or 6), 8, s, 10,
		C.text, F_MED)
	if sys.charger == "charging" or sys.charger == "present" then
		text(sbar.items, bx - 15, 6, "⚡", 11, "f8e45c", F_SYM)
	end
	-- the DragonFly mark at the left
	text(sbar.items, 14, 8, "DragonFly", 10, C.text2, F_MED)
end

------------------------------------------------------------------------
-- banners and events

local banner = {}
local event_icon = {charge = "⚡", battery = "▮", gpu = "◆", info = "●",
	power = "☾"}

local function show_banner(kind, title, body)
	if valid_vid(banner.v) then
		delete_image(banner.v)
	end
	local v = anchor(nil, 10, -96, W - 20, 72, O_BANNER)
	box(v, 0, 0, W - 20, 72, {0x26, 0x29, 0x31}, "rrect", 0.97, 1)
	box(v, 14, 16, 40, 40, accent(), "circle", 1.0, 2)
	text_c(v, 34, 26, event_icon[kind] or "●", 14, C.text, F_SYM, 3)
	text(v, 68, 15, title, 13, C.text, F_SB, 3)
	text(v, 68, 40, body, 11, C.text2, F_REG, 3)
	move_image(v, 10, SB + 8, 8, INTERP_EXPOUT)
	banner.v = v
	banner.hide_at = CLOCK + 100
end

add_event = function(kind, title, body, quiet)
	table.insert(events, 1, {kind = kind, title = title, body = body,
		time = os.date("%H:%M")})
	while #events > 30 do
		table.remove(events)
	end
	if not quiet and not screen_off then
		show_banner(kind, title, body)
	end
	if redraw_pages then
		redraw_pages()
	end
end

------------------------------------------------------------------------
-- home views

local function view_title(p, str)
	text(p, 24, SB + 30, str, 30, C.text, F_LIGHT)
end

local function draw_home(p)
	clear_children(p.a)
	p.hits = {}
	local cy = SB + 66
	text_c(p.a, W / 2, cy, clock_str(), 84, C.text, F_LIGHT)
	text_c(p.a, W / 2, cy + 128, date_str(), 14, C.text2, F_SB)
	text_c(p.a, W / 2, cy + 154, battery_text(), 11, C.text3, F_MED)

	local favs, dock = {}, {}
	for _, a in ipairs(apps) do
		table.insert(a.dock and dock or favs, a)
	end
	-- the dock: a dark rounded panel with the pinned apps and the drawer
	local dh, dy = 96, H - 112
	local items = #dock + 1
	box(p.a, 12, dy, W - 24, dh, C.panel, "rrect", 0.86, 1)
	local cw = (W - 24) / items
	for i = 1, items do
		local x = math.floor(12 + (i - 1) * cw + (cw - ICON) / 2)
		local y = dy + (dh - ICON) / 2
		local a = dock[i]
		if a then
			draw_icon(p.a, x, y, a, ICON, false)
			if running[a.id] then
				box(p.a, x + ICON / 2 - 3, y + ICON + 6, 6, 6,
					focus == a.id and accent() or {255, 255, 255}, "circle",
					1.0, 4)
			end
			table.insert(p.hits, {x = x - 8, y = dy, w = ICON + 16, h = dh,
				app = a, ix = x, iy = y})
		else
			-- QRT's 3x3 dots: the drawer
			local cell = anchor(p.a, x, y, ICON, ICON, 2)
			glyphs.dots(cell, ICON, nil)
			table.insert(p.hits, {x = x - 8, y = dy, w = ICON + 16, h = dh,
				drawer = true})
		end
	end
	local rows = math.ceil(#favs / 4)
	for _, r in ipairs(grid(p.a, favs, dy - 30 - rows * (ICON + 44), 4,
		true)) do
		table.insert(p.hits, r)
	end
end

local function draw_events(p)
	clear_children(p.a)
	p.hits = {}
	view_title(p.a, "Events")
	if #events == 0 then
		text_c(p.a, W / 2, H / 2 - 20, "No events", 14, C.text3, F_MED)
		return
	end
	card(p.a, W - 110, SB + 36, 90, 34, C.card_hi, 2, "pill")
	text_c(p.a, W - 65, SB + 44, "Clear", 11, C.text, F_SB, 3)
	table.insert(p.hits, {x = W - 120, y = SB + 26, w = 110, h = 54,
		clear = true})
	local y = SB + 100
	for _, e in ipairs(events) do
		if y > H - 90 then
			break
		end
		-- the content sits beside the translucent card, not in it:
		-- children inherit their parent's opacity
		card(p.a, 14, y, W - 28, 80, C.card, 1)
		box(p.a, 30, y + 18, 44, 44, accent(), "circle", 1.0, 2)
		text_c(p.a, 52, y + 29, event_icon[e.kind] or "●", 14, C.text,
			F_SYM, 3)
		text(p.a, 90, y + 16, e.title, 13, C.text, F_SB, 3)
		text(p.a, 90, y + 44, e.body, 11, C.text2, F_REG, 3)
		text_r(p.a, W - 30, y + 17, e.time, 10, C.text3, F_MED, 3)
		y = y + 90
	end
end

local function draw_switcher(p)
	clear_children(p.a)
	p.hits = {}
	view_title(p.a, "Open apps")
	if #run_order == 0 then
		text_c(p.a, W / 2, H / 2 - 30, "No open apps", 14, C.text3, F_MED)
		text_c(p.a, W / 2, H / 2, "Apps you leave keep running here", 11,
			C.text3, F_REG)
		return
	end
	local ch = math.floor((H - SB - 92 - 2 * 62 - 20) / 2)
	local cw = math.floor(ch * W / (H - SB))
	for i, id in ipairs(run_order) do
		if i > 4 then
			break
		end
		local inst = running[id]
		local c = (i - 1) % 2
		local r = math.floor((i - 1) / 2)
		local x = math.floor((W - 2 * cw - 20) / 2) + c * (cw + 20)
		local y = SB + 92 + r * (ch + 62)
		-- QRT: the app's icon and name, a close button, then the picture
		draw_icon(p.a, x, y, inst.app, 26, false)
		text(p.a, x + 34, y + 4, inst.app.name, 12, C.text, F_SB, 2)
		box(p.a, x + cw - 30, y - 1, 28, 28, {255, 255, 255}, "circle",
			C.card_hi, 2)
		bar(p.a, x + cw - 16, y + 13, 12, 2.2, 45, {255, 255, 255}, 3)
		bar(p.a, x + cw - 16, y + 13, 12, 2.2, -45, {255, 255, 255}, 3)
		table.insert(p.hits, {x = x + cw - 44, y = y - 12, w = 52, h = 48,
			close = id})
		local src = inst.rt or inst.vid
		local ty = y + 36
		if src and valid_vid(src) then
			local t = null_surface(cw, ch)
			image_sharestorage(src, t)
			image_shader(t, shaders.rimg)
			place(t, p.a, x, ty, 2)
			blend_image(t, 0.996)
			force_image_blend(t, BLEND_NORMAL)
		else
			card(p.a, x, ty, cw, ch, C.card, 2)
		end
		table.insert(p.hits, {x = x, y = ty, w = cw, h = ch, open = id})
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
	for i, d in ipairs(sbar.dots or {}) do
		blend_image(d, i == page and 0.95 or 0.30, 4)
	end
end

------------------------------------------------------------------------
-- the app drawer

local function draw_drawer()
	clear_children(drawer.a)
	box(drawer.a, 0, SB + 6, W, H, C.panel, "rrect", 1.0, 1)
	box(drawer.a, W / 2 - 22, SB + 18, 44, 5, {255, 255, 255}, "pill", 0.3,
		2)
	text(drawer.a, 24, SB + 40, "Apps", 30, C.text, F_LIGHT)
	text_r(drawer.a, W - 24, SB + 54, #apps .. " installed", 11, C.text3,
		F_MED)
	drawer.hits = grid(drawer.a, apps, SB + 118, 4, true)
end

local function drawer_show(open, t)
	drawer.open = open
	instant_image_transform(drawer.a)
	move_image(drawer.a, 0, open and 0 or H, t or 7, INTERP_EXPOUT)
end

------------------------------------------------------------------------
-- widgets: QRT's grouped rows, switches and sliders

local function bl_to_frac(v)
	return math.max(0, math.min(1, (v - 30) / (4095 - 30)))
end

local function frac_to_bl(f)
	return math.floor(30 + f * (4095 - 30))
end

-- a slider; returns its state table (x, y, w, fill, knob)
local function slider(parent, x, y, w, frac, order)
	local s = {x = x, y = y - 14, w = w, h = 36}
	box(parent, x, y, w, 6, {255, 255, 255}, "pill", 0.16, order)
	s.fill = box(parent, x, y, math.max(6, w * frac), 6, accent(), "pill",
		1.0, (order or 1) + 1)
	s.knob = box(parent, x + w * frac - 11, y - 8, 22, 22, {255, 255, 255},
		"circle", 1.0, (order or 1) + 2)
	return s
end

local function slider_set(s, x)
	local f = math.max(0, math.min(1, (x - s.x) / s.w))
	resize_image(s.fill, math.max(6, s.w * f), 6)
	move_image(s.knob, math.floor(s.x + s.w * f - 11), s.y + 6)
	return f
end

local function switch(parent, x, y, on, order)
	box(parent, x, y, 48, 28, on and accent() or {255, 255, 255}, "pill",
		on and 1.0 or 0.18, order)
	box(parent, on and x + 23 or x + 3, y + 3, 22, 22, {255, 255, 255},
		"circle", 1.0, (order or 1) + 1)
end

-- a group of rows in a rounded card; rows are {label, sub, value, right}
local function group(parent, y, rows, rh)
	rh = rh or 56
	card(parent, 16, y, W - 32, rh * #rows, C.card, 1)
	for i, r in ipairs(rows) do
		local ry = y + (i - 1) * rh
		if i > 1 then
			box(parent, 30, ry, W - 60, 1, {255, 255, 255}, nil, C.stroke, 2)
		end
		if r.sub then
			text(parent, 32, ry + 10, r.label, 13, C.text, F_REG, 3)
			text(parent, 32, ry + 31, r.sub, 10, C.text3, F_REG, 3)
		else
			text(parent, 32, ry + (rh - 18) / 2, r.label, 13, C.text, F_REG, 3)
		end
		if r.value then
			text_r(parent, W - 34, ry + (rh - 16) / 2, r.value, 12, C.text2,
				F_REG, 3)
		end
		if r.right then
			r.right(parent, ry, rh)
		end
	end
	return y + rh * #rows
end

local function section(parent, y, title)
	text(parent, 22, y, title, 12, C.text, F_SB, 3)
	return y + 26
end

------------------------------------------------------------------------
-- quick settings: Sailfish's top menu, QRT's drawer

local QS_H = 480

local function apply_accent(i)
	accent_i = i
	set_wall_accent()
	if store_key then
		pcall(store_key, "phone_accent", tostring(i))
	end
	redraw_all()
end

draw_qs = function()
	clear_children(qs.a)
	qs.hits = {}
	box(qs.a, 0, QS_H + 10, W, 30, {0, 0, 0}, "shadow", 0.6, 0)
	box(qs.a, 0, -40, W, QS_H + 40, C.panel, "rrect", 1.0, 1)
	text(qs.a, 24, SB + 16, date_str(), 15, C.text, F_SB)
	text(qs.a, 24, SB + 40, battery_text(), 11, C.text3, F_MED)

	-- brightness
	local y = SB + 90
	text(qs.a, 24, y - 10, "☼", 14, C.text2, F_SYM, 3)
	text_r(qs.a, W - 22, y - 12, "☀", 17, C.text, F_SYM, 3)
	qs.slider = slider(qs.a, 58, y, W - 116, bl_to_frac(sys.backlight), 3)

	-- toggles: QRT tiles
	local tg = {
		{"◆", "Torch", function() return sys.led == 0xffffff end, function()
			sys.led = sys.led == 0xffffff and 0 or 0xffffff
			sysctl_set("hw.mi5.led", sys.led)
		end},
		{"●", "Light", function() return sys.led ~= 0 and
			sys.led ~= 0xffffff end, function()
			sys.led = (sys.led ~= 0 and sys.led ~= 0xffffff) and 0 or 0x00ff40
			sysctl_set("hw.mi5.led", sys.led)
		end},
		{"≋", "Vibrate", function() return false end, function()
			sysctl_set("hw.mi5.vibrate", 250)
		end},
		{"☾", "Lock", function() return false end, function()
			qs_show(false)
			lock_screen(true)
		end},
	}
	local tw = math.floor((W - 48 - 12) / 2)
	for i, t in ipairs(tg) do
		local c = (i - 1) % 2
		local r = math.floor((i - 1) / 2)
		local x = 24 + c * (tw + 12)
		local ty = SB + 126 + r * 68
		local on = t[3]()
		if on then
			box(qs.a, x, ty, tw, 58, accent(), "rrect", 1.0, 2)
		else
			card(qs.a, x, ty, tw, 58, C.card_hi, 2)
		end
		text(qs.a, x + 18, ty + 17, t[1], 16, C.text, F_SYM, 3)
		text(qs.a, x + 50, ty + 19, t[2], 13, C.text, F_SB, 3)
		table.insert(qs.hits, {x = x, y = ty, w = tw, h = 58, fn = t[4]})
	end

	-- the ambience: an accent and its wallpaper light
	local ay = SB + 276
	text(qs.a, 24, ay, "Ambience", 12, C.text2, F_SB, 3)
	for i, a in ipairs(ACCENTS) do
		local x = 24 + (i - 1) * 52
		if i == accent_i then
			box(qs.a, x - 4, ay + 26, 44, 44, {255, 255, 255}, "ring", 1.0, 2)
		end
		box(qs.a, x + 2, ay + 32, 32, 32, a[2], "circle", 1.0, 3)
		table.insert(qs.hits, {x = x - 6, y = ay + 22, w = 50, h = 52,
			fn = function()
				apply_accent(i)
			end})
	end

	-- power
	local bw = math.floor((W - 60) / 2)
	for i, b in ipairs({{"Restart", "reboot"}, {"Power off", "reboot -p"}}) do
		local x = 24 + (i - 1) * (bw + 12)
		card(qs.a, x, QS_H - 82, bw, 46, C.card_hi, 2, "pill")
		text_c(qs.a, x + bw / 2, QS_H - 69, b[1], 12, C.text, F_SB, 3)
		table.insert(qs.hits, {x = x, y = QS_H - 82, w = bw, h = 46,
			fn = function()
				add_event("power", b[1], "Going down now")
				run(b[2] .. " &")
			end})
	end
	box(qs.a, W / 2 - 22, QS_H - 18, 44, 5, {255, 255, 255}, "pill", 0.35, 2)
end

qs_show = function(open, t)
	qs.open = open
	if open then
		draw_qs()
	end
	instant_image_transform(qs.a)
	move_image(qs.a, 0, open and 0 or -QS_H - 10, t or 7, INTERP_EXPOUT)
end

local function qs_slide(x, final)
	sys.backlight = frac_to_bl(slider_set(qs.slider, x))
	if final or not qs.last_set or CLOCK - qs.last_set > 3 then
		sysctl_set("hw.mi5.backlight", sys.backlight)
		qs.last_set = CLOCK
	end
end

------------------------------------------------------------------------
-- lock screen: QRT's

local function draw_lock()
	clear_children(lock.a)
	box(lock.a, 0, 0, W, H, {0, 0, 0}, "wall", 1.0, 1)
	text_c(lock.a, W / 2, 210, clock_str(), 96, C.text, F_LIGHT, 3)
	text_c(lock.a, W / 2, 340, date_str(), 16, C.text, F_SB, 3)
	text_c(lock.a, W / 2, 368, battery_text(), 11, C.text2, F_MED, 3)
	if #events > 0 then
		local e = events[1]
		card(lock.a, 30, 430, W - 60, 66, C.card, 3)
		box(lock.a, 46, 444, 38, 38, accent(), "circle", 1.0, 4)
		text_c(lock.a, 65, 453, event_icon[e.kind] or "●", 13, C.text, F_SYM,
			5)
		text(lock.a, 98, 442, e.title, 12, C.text, F_SB, 4)
		text(lock.a, 98, 466, e.body, 10, C.text2, F_REG, 4)
	end
	bar(lock.a, W / 2 - 8, H - 104, 22, 3, 55, {200, 205, 215}, 3)
	bar(lock.a, W / 2 + 8, H - 104, 22, 3, -55, {200, 205, 215}, 3)
	text_c(lock.a, W / 2, H - 82, "Swipe up to unlock", 12, C.text2, F_SB, 3)
end

lock_screen = function(on, t)
	locked = on
	instant_image_transform(lock.a)
	if on then
		draw_lock()
		move_image(lock.a, 0, 0, t or 0)
		blend_image(lock.a, 1.0, t or 0)
		if qs.open then
			qs_show(false, 0)
		end
	else
		move_image(lock.a, 0, -H, t or 8, INTERP_EXPOUT)
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
-- the lit edge under an edge swipe (Sailfish)

local EDGE_W = {3, 10, 26}

local function edge_show(side, amount)
	if not valid_vid(edge.a) then
		edge.a = anchor(nil, 0, 0, W, H, O_EDGE)
		edge.bars = {}
		for i, al in ipairs({0.9, 0.30, 0.10}) do
			edge.bars[i] = box(edge.a, 0, 0, EDGE_W[i], H, accent(), nil, al, i)
		end
	end
	local a = accent()
	for i, b in ipairs(edge.bars) do
		local w = EDGE_W[i]
		image_color(b, a[1], a[2], a[3])
		if side == "left" then
			move_image(b, 0, 0)
			resize_image(b, w, H)
		elseif side == "right" then
			move_image(b, W - w, 0)
			resize_image(b, w, H)
		else
			move_image(b, 0, SB)
			resize_image(b, W, w)
		end
	end
	blend_image(edge.a, math.max(0, math.min(1, amount)))
end

local function edge_hide()
	if valid_vid(edge.a) then
		blend_image(edge.a, 0.0, 5)
	end
end

------------------------------------------------------------------------
-- on-screen keyboard, for the terminal

local kbd = {layer = "lower", keys = {}, ctrl = false, h = 0, key_h = 48}
local SHIFT, SYM, CTRL = {}, {}, {}
local BKSP, ENTER, TAB, ESC = "\127", "\r", "\t", "\027"

local function utf8_iter(s)
	local i, n = 1, #s
	return function()
		if i > n then
			return nil
		end
		local c = string.byte(s, i)
		local len = c < 0x80 and 1 or (c < 0xe0 and 2 or (c < 0xf0 and 3 or 4))
		local ch = string.sub(s, i, i + len - 1)
		i = i + len
		return ch
	end
end

local function krow(chars)
	local r = {}
	for c in utf8_iter(chars) do
		table.insert(r, {c, c, 1})
	end
	return r
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
	local function add(v)
		show_image(v)
		table.insert(vids, v)
		return v
	end
	add(color_surface(W, kbd.h, 0x1d, 0x1f, 0x24))
	local unit = W / 10
	local a = accent()
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
			local col = on and a or (k[2] == ENTER and a or
				(special and {0x36, 0x39, 0x40} or {0x4a, 0x4d, 0x55}))
			local key = add(color_surface(math.floor(w - 6), kh - 8,
				col[1], col[2], col[3]))
			image_shader(key, shaders.rrect)
			blend_image(key, 0.996)
			force_image_blend(key, BLEND_NORMAL)
			move_image(key, math.floor(x + 3), yy + 2)
			order_image(key, 2)
			if k[1] ~= "" then
				local sym = string.byte(k[1]) >= 0x80
				local t = render_text({string.format([[\f%s,%d\#ffffff]],
					sym and F_SYM or F_MED, #k[1] > 1 and not sym and 10 or 14),
					k[1]})
				if valid_vid(t) then
					local p = image_surface_properties(t)
					move_image(t, math.floor(x + (w - p.width) / 2),
						math.floor(yy + 2 + (kh - 8 - p.height) / 2))
					order_image(t, 3)
					add(t)
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
		kbd.hl = color_surface(1, 1, 255, 255, 255)
		image_shader(kbd.hl, shaders.rrect)
		force_image_blend(kbd.hl, BLEND_NORMAL)
		blend_image(kbd.hl, 0.0)
		order_image(kbd.hl, O_KBD)
	end
end

------------------------------------------------------------------------
-- app chrome: QRT's header bar, with minimise and close

local function header(inst, title, back)
	box(inst.a, 0, 0, W, HB, C.header, nil, 1.0, 20)
	box(inst.a, 0, HB - 1, W, 1, {0, 0, 0}, nil, 0.5, 21)
	text_c(inst.a, W / 2, (HB - 18) / 2, title, 14, C.text, F_SB, 22)
	-- minimise, close
	for i = 1, 2 do
		local cx = W - 30 - (2 - i) * 44
		box(inst.a, cx - 15, HB / 2 - 15, 30, 30, {255, 255, 255}, "circle",
			C.card_hi, 21)
		if i == 1 then
			bar(inst.a, cx, HB / 2, 11, 2.2, 90, {255, 255, 255}, 23)
		else
			bar(inst.a, cx, HB / 2, 13, 2.2, 45, {255, 255, 255}, 23)
			bar(inst.a, cx, HB / 2, 13, 2.2, -45, {255, 255, 255}, 23)
		end
	end
	if back then
		box(inst.a, 12, HB / 2 - 15, 30, 30, {255, 255, 255}, "circle",
			C.card_hi, 21)
		bar(inst.a, 25, HB / 2 - 4, 10, 2.4, 45, {255, 255, 255}, 23)
		bar(inst.a, 25, HB / 2 + 4, 10, 2.4, -45, {255, 255, 255}, 23)
	end
	inst.back = back
end

local function app_bg(inst)
	box(inst.a, 0, 0, W, inst.ah, C.window, nil, 1.0, 1)
end

------------------------------------------------------------------------
-- a scrolling list, for files, processes and file contents

local function list_view(inst, y0, rows, row_h, ontap)
	if valid_vid(inst.view) then
		delete_image(inst.view)
	end
	local vh = inst.ah - y0
	inst.view = anchor(inst.a, 0, y0, W, vh, 5)
	inst.content = anchor(inst.view, 0, 0, W, math.max(vh, #rows * row_h), 1)
	inst.list = {rows = rows, row_h = row_h, y0 = y0, vh = vh, off = 0,
		tap = ontap}
	local function clip(v)
		if v then
			image_clip_on(v, CLIP_SHALLOW, inst.view)
		end
		return v
	end
	for i, r in ipairs(rows) do
		local y = (i - 1) * row_h
		if i > 1 and not r.nosep then
			clip(box(inst.content, r.icon and 62 or 18, y, W, 1, {255, 255, 255},
				nil, C.stroke * 0.7, 1))
		end
		if r.icon then
			local iy = y + (row_h - 32) / 2
			clip(box(inst.content, 18, iy, 32, 32, r.icol, "tile", 1.0, 2))
			if r.dir then
				clip(box(inst.content, 25, iy + 10, 9, 5, {255, 255, 255},
					"rrect", 1.0, 3))
				clip(box(inst.content, 25, iy + 12, 18, 12, {255, 255, 255},
					"rrect", 1.0, 3))
			else
				clip(box(inst.content, 27, iy + 7, 14, 18, {255, 255, 255},
					"rrect", 1.0, 3))
				for k = 0, 2 do
					clip(box(inst.content, 30, iy + 11 + k * 4, 8, 1.6, r.icol,
						nil, 1.0, 4))
				end
			end
		end
		clip((text(inst.content, r.icon and 62 or 20, y + (row_h - 18) / 2,
			r.text, r.size or 13, r.col or C.text, r.font or F_REG, 3)))
		if r.right then
			clip((text_r(inst.content, W - 20, y + (row_h - 16) / 2, r.right, 11,
				C.text3, F_REG, 3)))
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

------------------------------------------------------------------------
-- apps

-- Terminal: Arcan's terminal frameserver
local term_app = {id = "term", name = "Terminal", glyph = "terminal",
	col = {0x3d, 0x40, 0x47}, dock = true, direct = true}

local function term_type(inst, str)
	if not valid_vid(inst.vid, TYPE_FRAMESERVER) then
		return
	end
	for ch in utf8_iter(str) do
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

function term_app.create(inst)
	kbd.h = kbd.key_h * 5 + 10
	box(inst.a, 0, 0, W, inst.ah, {0x16, 0x17, 0x1a}, nil, 1.0, 1)
	header(inst, "Terminal", false)
	kbd_build(inst.a, inst.ah - kbd.h)
	inst.th = inst.ah - kbd.h - HB - 6
	inst.vid = launch_avfeed("", "terminal", function(source, status)
		if status.kind == "preroll" or status.kind == "registered" then
			target_fonthint(source, F_MONO, 4.0, 2)
			target_displayhint(source, W - 8, inst.th, 0)
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
		move_image(inst.vid, 4, HB + 4)
		order_image(inst.vid, 3)
	end
end

function term_app.down(inst, x, y)
	local ky = y - kbd.y
	if ky < 0 or ky >= kbd.h then
		return false
	end
	for _, e in ipairs(kbd.keys) do
		if x >= e.x and x < e.x + e.w and ky >= e.y and ky < e.y + e.h then
			move_image(kbd.hl, math.floor(e.x + 3), SB + kbd.y + e.y + 2)
			resize_image(kbd.hl, math.floor(e.w - 6), e.h - 8)
			blend_image(kbd.hl, 0.35)
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

-- Files
local files_app = {id = "files", name = "Files", glyph = "folder",
	col = {0x35, 0x84, 0xe4}, dock = true}
local files_view

local function files_show(inst, path)
	clear_children(inst.a)
	inst.view = nil
	app_bg(inst)
	inst.path = path
	inst.viewing = nil
	header(inst, "Files", path ~= "/")
	card(inst.a, 16, HB + 12, W - 32, 34, C.card_hi, 2, "pill")
	text(inst.a, 32, HB + 20, path, 11, C.text2, F_MED, 3)
	local rows = {}
	local out = run("ls -pa " .. sh_quote(path))
	for name in string.gmatch(out, "[^\n]+") do
		if name ~= "./" and name ~= "../" then
			local dir = string.sub(name, -1) == "/"
			table.insert(rows, {text = dir and string.sub(name, 1, -2) or name,
				icon = true, icol = dir and {0x35, 0x84, 0xe4} or
				{0x6f, 0x71, 0x7a}, dir = dir, right = dir and "›" or nil})
		end
	end
	if #rows == 0 then
		table.insert(rows, {text = "Empty, or no permission", col = C.text3})
	end
	list_view(inst, HB + 56, rows, 52, function(r)
		if r.dir then
			files_show(inst, (path == "/" and "" or path) .. "/" .. r.text)
		elseif r.icon then
			files_view(inst, (path == "/" and "" or path) .. "/" .. r.text)
		end
	end)
end

files_view = function(inst, file)
	clear_children(inst.a)
	inst.view = nil
	app_bg(inst)
	header(inst, string.match(file, "[^/]+$") or file, true)
	inst.viewing = file
	local rows = {}
	local f = io and io.open(file, "rb")
	local data = f and f:read(16384) or nil
	if f then
		f:close()
	end
	if not data then
		table.insert(rows, {text = "Cannot read this file", col = C.text3})
	elseif string.find(data, "%z") then
		table.insert(rows, {text = string.format("Binary file, %s%d bytes",
			#data == 16384 and "over " or "", #data), col = C.text3})
	else
		for line in string.gmatch(data .. "\n", "([^\n]*)\n") do
			line = string.gsub(line, "\t", "    ")
			if #line > 62 then
				line = string.sub(line, 1, 61) .. "…"
			end
			table.insert(rows, {text = line, size = 10, font = F_MONO,
				nosep = true})
			if #rows > 400 then
				break
			end
		end
	end
	list_view(inst, HB + 8, rows, 22, nil)
end

function files_app.create(inst)
	files_show(inst, "/")
end

function files_app.tap(inst, x, y)
	if y < HB and x < 56 and inst.back then
		if inst.viewing then
			local dir = string.match(inst.viewing, "^(.*)/[^/]*$")
			files_show(inst, (dir == nil or dir == "") and "/" or dir)
		else
			local up = string.match(inst.path, "^(.*)/[^/]*$")
			files_show(inst, (up == nil or up == "") and "/" or up)
		end
		return
	end
	list_tap(inst, x, y)
end

files_app.drag = list_drag

-- System monitor
local system_app = {id = "system", name = "System", glyph = "chip",
	col = {0x3a, 0x94, 0x4a}, dock = true}

local function uptime_str()
	local bt = tonumber(string.match(run("sysctl -n kern.boottime"),
		"sec = (%d+)") or "")
	if not bt then
		return "?"
	end
	local s = os.time() - bt
	return string.format("%d:%02d:%02d", math.floor(s / 3600),
		math.floor(s / 60) % 60, s % 60)
end

local function system_refresh(inst)
	local off = inst.list and inst.list.off or 0
	clear_children(inst.a)
	inst.view = nil
	app_bg(inst)
	header(inst, "System", false)
	local mem = tonumber(run("sysctl -n hw.physmem")) or 0
	local pg = tonumber(run("sysctl -n hw.pagesize")) or 4096
	local free = tonumber(run("sysctl -n vm.stats.vm.v_free_count")) or 0
	local used = mem > 0 and 1 - (free * pg) / mem or 0
	-- four tiles
	local tw = math.floor((W - 44) / 2)
	local tiles = {
		{"Memory", string.format("%d%%", math.floor(used * 100 + 0.5)),
			string.format("%d of %d MB", math.floor((mem - free * pg) / 1048576),
			math.floor(mem / 1048576)), used},
		{"Battery", sys.battery >= 0 and string.format("%d%%", sys.battery) or
			"—", sys.charger, sys.battery >= 0 and sys.battery / 100 or nil},
		{"Uptime", uptime_str(), "since boot"},
		{"Processors", (string.gsub(run("sysctl -n hw.ncpu"), "%s", "")),
			"Kryo, arm64"},
	}
	for i, t in ipairs(tiles) do
		local x = 16 + ((i - 1) % 2) * (tw + 12)
		local y = HB + 14 + math.floor((i - 1) / 2) * 100
		card(inst.a, x, y, tw, 88, C.card, 2)
		text(inst.a, x + 16, y + 12, t[1], 11, C.text2, F_MED, 3)
		text(inst.a, x + 16, y + 30, t[2], 22, C.text, F_LIGHT, 3)
		text(inst.a, x + 16, y + 62, t[3], 10, C.text3, F_REG, 3)
		if t[4] then
			box(inst.a, x + tw - 64, y + 40, 48, 6, {255, 255, 255}, "pill",
				0.15, 3)
			box(inst.a, x + tw - 64, y + 40, math.max(6, 48 * t[4]), 6,
				accent(), "pill", 1.0, 4)
		end
	end
	local y = HB + 222
	text(inst.a, 22, y, "Processes", 12, C.text, F_SB, 3)
	local rows = {{text = "  PID    RSS  COMMAND", size = 10, font = F_MONO,
		col = hex(lighten(accent(), 0.35)), nosep = true}}
	local out = run("ps -axo pid,rss,comm")
	local first = true
	for line in string.gmatch(out, "[^\n]+") do
		if not first then
			local pid, rss, cmd = string.match(line,
				"^%s*(%d+)%s+(%d+)%s+(.*)$")
			if pid then
				table.insert(rows, {text = string.format("%5s %5dM  %s", pid,
					math.floor(tonumber(rss) / 1024), cmd), size = 10,
					font = F_MONO, nosep = true})
			end
		end
		first = false
	end
	list_view(inst, y + 26, rows, 24, nil)
	list_drag(inst, -off, true)
	inst.next_refresh = CLOCK + 75
end

function system_app.create(inst)
	system_refresh(inst)
end

function system_app.tick(inst)
	if CLOCK >= (inst.next_refresh or 0) and not inst.dragging then
		system_refresh(inst)
	end
end

function system_app.drag(inst, dy, final)
	inst.dragging = not final
	list_drag(inst, dy, final)
end

-- Settings: QRT's grouped rows
local settings_app = {id = "settings", name = "Settings", glyph = "gear",
	col = {0x6f, 0x71, 0x7a}, dock = true}

local function settings_draw(inst)
	clear_children(inst.a)
	inst.hits = {}
	app_bg(inst)
	header(inst, "Settings", false)
	local y = HB + 18

	y = section(inst.a, y, "Appearance")
	y = group(inst.a, y, {{label = "Accent colour", right = function(p, ry,
		rh)
		for i = #ACCENTS, 1, -1 do
			local x = W - 34 - (#ACCENTS - i + 1) * 36 + 4
			if i == accent_i then
				box(p, x - 4, ry + rh / 2 - 17, 34, 34, {255, 255, 255}, "ring",
					1.0, 3)
			end
			box(p, x + 1, ry + rh / 2 - 12, 24, 24, ACCENTS[i][2], "circle",
				1.0, 4)
			table.insert(inst.hits, {x = x - 6, y = ry, w = 36, h = rh,
				fn = function()
					apply_accent(i)
				end})
		end
	end}})
	y = y + 18

	y = section(inst.a, y, "Display")
	y = group(inst.a, y, {{label = "Brightness", right = function(p, ry, rh)
		inst.slider = slider(p, 150, ry + rh / 2 - 3, W - 196,
			bl_to_frac(sys.backlight), 3)
	end}})
	y = y + 18

	y = section(inst.a, y, "Notification light")
	local colours = {{0, {0x5e, 0x5c, 0x64}}, {0x00ff40, {0x33, 0xd1, 0x7a}},
		{0x0040ff, {0x35, 0x84, 0xe4}}, {0xff2000, {0xe0, 0x1b, 0x24}},
		{0xffa000, {0xff, 0xa3, 0x48}}, {0xffffff, {0xf6, 0xf5, 0xf4}}}
	y = group(inst.a, y, {
		{label = "Colour", right = function(p, ry, rh)
			for i = #colours, 1, -1 do
				local c = colours[i]
				local x = W - 34 - (#colours - i + 1) * 36 + 4
				if sys.led == c[1] then
					box(p, x - 4, ry + rh / 2 - 17, 34, 34, {255, 255, 255},
						"ring", 1.0, 3)
				end
				box(p, x + 1, ry + rh / 2 - 12, 24, 24, c[2], "circle", 1.0, 4)
				table.insert(inst.hits, {x = x - 6, y = ry, w = 36, h = rh,
					fn = function()
						sys.led = c[1]
						sysctl_set("hw.mi5.led", c[1])
						settings_draw(inst)
					end})
			end
		end},
		{label = "Torch", sub = "The light at full white",
			right = function(p, ry, rh)
				switch(p, W - 82, ry + rh / 2 - 14, sys.led == 0xffffff, 3)
				table.insert(inst.hits, {x = W - 100, y = ry, w = 84, h = rh,
					fn = function()
						sys.led = sys.led == 0xffffff and 0 or 0xffffff
						sysctl_set("hw.mi5.led", sys.led)
						settings_draw(inst)
					end})
			end},
	})
	y = y + 18

	y = section(inst.a, y, "Vibration")
	y = group(inst.a, y, {{label = "Test the motor",
		right = function(p, ry, rh)
			card(p, W - 120, ry + rh / 2 - 17, 86, 34, C.card_hi, 3, "pill")
			text_c(p, W - 77, ry + rh / 2 - 9, "Buzz", 12, C.text, F_SB, 4)
			table.insert(inst.hits, {x = W - 130, y = ry, w = 110, h = rh,
				fn = function()
					sysctl_set("hw.mi5.vibrate", 300)
				end})
		end}})
	y = y + 18

	y = section(inst.a, y, "About")
	inst.about = inst.about or {
		{label = "Model", value = "Xiaomi Mi 5"},
		{label = "System", value = (string.gsub(run("uname -sr"), "\n", ""))},
		{label = "Processor", value = (string.gsub(run("uname -m"), "\n", "")) ..
			" · " .. (string.gsub(run("sysctl -n hw.ncpu"), "%s", "")) ..
			" cores"},
		{label = "Memory", value = string.format("%d MB",
			math.floor((tonumber(run("sysctl -n hw.physmem")) or 0) / 1048576))},
		{label = "Graphics", value = sys.renderer},
	}
	group(inst.a, y, inst.about, 46)
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
	sys.backlight = frac_to_bl(slider_set(inst.slider, x))
	if final or not inst.last_set or CLOCK - inst.last_set > 3 then
		sysctl_set("hw.mi5.backlight", sys.backlight)
		inst.last_set = CLOCK
	end
end

-- Calculator
local calc_app = {id = "calc", name = "Calculator", glyph = "calc",
	col = {0xe6, 0x61, 0x00}}

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
	inst.disp = anchor(inst.a, 0, HB, W, 200, 5)
	local e = inst.expr == "" and "0" or inst.expr
	text_r(inst.disp, W - 24, 70, e, #e > 12 and 26 or 44, C.text, F_LIGHT)
	if inst.result then
		text_r(inst.disp, W - 24, 140, "= " .. inst.result, 16, C.text2, F_REG)
	end
end

function calc_app.create(inst)
	inst.expr = ""
	app_bg(inst)
	header(inst, "Calculator", false)
	inst.keys = {}
	local top = HB + 210
	local kw = math.floor((W - 24) / 4)
	local kh = math.floor((inst.ah - top - 12) / 5)
	for r, row in ipairs(calc_keys) do
		for c, k in ipairs(row) do
			local x = 12 + (c - 1) * kw
			local y = top + (r - 1) * kh
			if k == "=" then
				box(inst.a, x + 5, y + 5, kw - 10, kh - 10, accent(), "rrect",
					1.0, 2)
			else
				card(inst.a, x + 5, y + 5, kw - 10, kh - 10,
					(c == 4 or k == "C") and C.card_hi * 1.3 or C.card, 2)
			end
			local sym = string.byte(k) >= 0x80
			text_c(inst.a, x + kw / 2, y + kh / 2 - 15, k, 20,
				(c == 4 and k ~= "=") and hex(lighten(accent(), 0.45)) or C.text,
				sym and F_SYM or F_REG, 3)
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

-- Clock: an analogue face, the time, a stopwatch
local clock_app = {id = "clock", name = "Clock", glyph = "clock",
	col = {0xff, 0xa3, 0x48}}

local function clock_draw(inst)
	if valid_vid(inst.face) then
		delete_image(inst.face)
	end
	local f = anchor(inst.a, 0, HB, W, inst.ah - HB, 5)
	inst.face = f
	local cx, cy, r = W / 2, 170, 128
	box(f, cx - r, cy - r, 2 * r, 2 * r, {0x2c, 0x2f, 0x34}, "circle", 1.0, 1)
	for i = 0, 11 do
		local a = math.rad(i * 30)
		local len = i % 3 == 0 and 16 or 9
		bar(f, cx + math.sin(a) * (r - 18), cy - math.cos(a) * (r - 18), len,
			i % 3 == 0 and 3 or 2, i * 30, {255, 255, 255}, 2,
			i % 3 == 0 and 1.0 or 0.6)
	end
	local t = os.date("*t")
	local function hand(deg, len, th, col)
		local a = math.rad(deg)
		bar(f, cx + math.sin(a) * len / 2, cy - math.cos(a) * len / 2, len, th,
			deg, col, 3)
	end
	hand((t.hour % 12 + t.min / 60) * 30, r * 0.52, 6, {255, 255, 255})
	hand((t.min + t.sec / 60) * 6, r * 0.78, 4, {255, 255, 255})
	hand(t.sec * 6, r * 0.86, 2, accent())
	box(f, cx - 6, cy - 6, 12, 12, accent(), "circle", 1.0, 4)

	text_c(f, W / 2, 322, os.date("%H:%M:%S"), 30, C.text, F_LIGHT)
	text_c(f, W / 2, 366, date_str(), 12, C.text2, F_MED)

	card(f, 16, 410, W - 32, 210, C.card, 1)
	text(f, 34, 424, "Stopwatch", 12, C.text2, F_SB, 2)
	local el = inst.elapsed + (inst.running and (CLOCK - inst.start) or 0)
	local cs = math.floor(el * 4)	-- CLOCK is 25 Hz: hundredths
	text_c(f, W / 2, 452, string.format("%02d:%02d.%02d",
		math.floor(cs / 6000), math.floor(cs / 100) % 60, cs % 100), 40,
		C.text, F_LIGHT, 2)
	local bw = math.floor((W - 92) / 2)
	local labels = {inst.running and "Stop" or "Start",
		inst.running and "Lap" or "Reset"}
	for i = 1, 2 do
		local x = 34 + (i - 1) * (bw + 24)
		if i == 1 then
			box(f, x, 540, bw, 52, inst.running and {0xc0, 0x1c, 0x28} or
				accent(), "pill", 1.0, 2)
		else
			card(f, x, 540, bw, 52, C.card_hi, 2, "pill")
		end
		text_c(f, x + bw / 2, 555, labels[i], 14, C.text, F_SB, 3)
	end
	for i, lap in ipairs(inst.laps) do
		if i > 6 then
			break
		end
		text_c(f, W / 2, 640 + (i - 1) * 26, lap, 11, C.text2, F_MONO)
	end
end

function clock_app.create(inst)
	app_bg(inst)
	header(inst, "Clock", false)
	inst.elapsed, inst.running, inst.laps = 0, false, {}
	inst.last = ""
	clock_draw(inst)
end

function clock_app.tap(inst, x, y)
	if y >= HB + 540 and y < HB + 592 then
		if x < W / 2 then
			if inst.running then
				inst.elapsed = inst.elapsed + CLOCK - inst.start
			else
				inst.start = CLOCK
			end
			inst.running = not inst.running
		elseif inst.running then
			local cs = math.floor((inst.elapsed + CLOCK - inst.start) * 4)
			table.insert(inst.laps, 1, string.format("Lap %d   %02d:%02d.%02d",
				#inst.laps + 1, math.floor(cs / 6000),
				math.floor(cs / 100) % 60, cs % 100))
		else
			inst.elapsed, inst.laps = 0, {}
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

apps = {calc_app, clock_app, files_app, term_app, system_app, settings_app}
for _, a in ipairs(apps) do
	app_by_id[a.id] = a
end

------------------------------------------------------------------------
-- running apps.  A built-in app draws into its own render target: live
-- while in front, a still picture once left (QRT: "taken when you last
-- left it").  The terminal draws straight into the world.

-- run an app callback with its render target current
local function in_app(inst, fn, ...)
	local saved = cur_rt
	cur_rt = inst.rt
	local ok, err = pcall(fn, inst, ...)
	cur_rt = saved
	if not ok then
		warning("phone: " .. inst.app.id .. ": " .. tostring(err))
	end
end

local function surface(inst)
	return inst.rt or inst.a
end

local function touch_order(id)
	for i, v in ipairs(run_order) do
		if v == id then
			table.remove(run_order, i)
			break
		end
	end
	table.insert(run_order, 1, id)
end

-- from: an icon rect {ix, iy} to zoom out of
open_app = function(id, from)
	local app = app_by_id[id]
	if not app then
		return
	end
	if drawer.open then
		drawer_show(false, 4)
	end
	local inst = running[id]
	local ah = H - SB
	if not inst then
		inst = {app = app, ah = ah}
		running[id] = inst
		if app.direct then
			inst.a = anchor(nil, 0, SB, W, ah, O_APP)
		else
			inst.rt = alloc_surface(W, ah)
			image_shader(inst.rt, shaders.rimg)
			inst.a = null_surface(W, ah)
			show_image(inst.a)
			define_rendertarget(inst.rt, {inst.a}, RENDERTARGET_DETACH,
				RENDERTARGET_NOSCALE, -1)
			order_image(inst.rt, O_APP)
			move_image(inst.rt, 0, SB)
		end
		in_app(inst, app.create)
	elseif inst.rt then
		rendertarget_forceupdate(inst.rt, -1)	-- live again
		inst.freeze_at = nil
	end
	if focus and focus ~= id and running[focus] then
		local prev = running[focus]
		if prev.rt then
			prev.freeze_at = CLOCK + 3
		end
		hide_image(surface(prev))
	end
	touch_order(id)
	focus = id
	local s = surface(inst)
	instant_image_transform(s)
	if inst.rt then
		-- Sailfish and QRT: zoom out of the icon
		if from then
			move_image(s, from.ix, from.iy)
			resize_image(s, ICON, ICON * ah / W)
		else
			move_image(s, W * 0.08, SB + ah * 0.08)
			resize_image(s, W * 0.84, ah * 0.84)
		end
		blend_image(s, 0.3)
		force_image_blend(s, BLEND_NORMAL)
		move_image(s, 0, SB, 7, INTERP_EXPOUT)
		resize_image(s, W, ah, 7, INTERP_EXPOUT)
		blend_image(s, 0.996, 5)
	else
		move_image(s, 0, SB + 50)
		blend_image(s, 0.0)
		move_image(s, 0, SB, 6, INTERP_EXPOUT)
		blend_image(s, 1.0, 5)
	end
	draw_status()
	redraw_pages()
end

-- minimise: the app keeps running; dir 0 shrinks it into the middle
leave_app = function(dir)
	if not focus then
		return
	end
	local inst = running[focus]
	focus = nil
	local s = surface(inst)
	instant_image_transform(s)
	if inst.rt then
		inst.freeze_at = CLOCK + 3	-- then keep the last picture
		if (dir or 0) == 0 then
			move_image(s, W * 0.2, SB + inst.ah * 0.2, 6, INTERP_EXPOUT)
			resize_image(s, W * 0.6, inst.ah * 0.6, 6, INTERP_EXPOUT)
		else
			move_image(s, dir * W, SB, 6, INTERP_EXPOUT)
		end
	else
		move_image(s, (dir or 0) * W, SB + ((dir or 0) == 0 and 60 or 0),
			6, INTERP_EXPOUT)
	end
	blend_image(s, 0.0, 6)
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
	local s = surface(inst)
	if valid_vid(s) then
		instant_image_transform(s)
		if inst.rt then
			move_image(s, W * 0.15, SB + inst.ah * 0.15, 6, INTERP_EXPIN)
			resize_image(s, W * 0.7, inst.ah * 0.7, 6, INTERP_EXPIN)
		else
			move_image(s, 0, H, 6, INTERP_EXPIN)
		end
		blend_image(s, 0.0, 6)
		expire_image(s, 7)
	end
	if inst.rt and valid_vid(inst.a) then
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
		leave_app(0)
		set_page(2)
	else
		set_page(2)
	end
end

redraw_all = function()
	draw_status()
	redraw_pages()
	draw_drawer()
	if qs.open then
		draw_qs()
	end
	if locked then
		draw_lock()
	end
	if running.term then
		kbd_build(running.term.a, kbd.y)
	end
	for _, inst in pairs(running) do
		if not inst.app.direct then
			in_app(inst, function(i)
				clear_children(i.a)
				i.app.create(i)
			end)
			if focus ~= inst.app.id then
				rendertarget_forceupdate(inst.rt)
			end
		end
	end
end

------------------------------------------------------------------------
-- gestures

local EDGE = 22

local function on_down(x, y)
	g = {x0 = x, y0 = y, x = x, y = y, t0 = CLOCK, mode = nil}
	if screen_off then
		return
	end
	g.edge = (x < EDGE and "left") or (x > W - EDGE and "right") or
		(y < SB and "top") or nil
	if locked then
		return
	end
	if not qs.open and focus and not g.edge then
		local inst = running[focus]
		if inst.app.down and inst.app.down(inst, x, y - SB) then
			g.consumed = true
		end
	end
	if qs.open and hit(qs.slider, x, y) then
		g.mode = "qs_slider"
		qs_slide(x)
	elseif focus and running[focus].app.slide and running[focus].slider and
		hit(running[focus].slider, x, y - SB) then
		g.mode = "slider"
	end
end

local function on_move(x, y)
	g.x, g.y = x, y
	local dx, dy = x - g.x0, y - g.y0
	if g.consumed or screen_off then
		return
	end
	if g.mode == "qs_slider" then
		qs_slide(x)
		return
	elseif g.mode == "slider" then
		in_app(running[focus], running[focus].app.slide, x, false)
		return
	end
	if not g.mode then
		if math.abs(dx) < 12 and math.abs(dy) < 12 then
			return
		end
		local horiz = math.abs(dx) > math.abs(dy)
		if qs.open then
			g.mode = "qs"
		elseif g.edge == "top" and not horiz and not focus then
			g.mode = "qs"
			draw_qs()
		elseif locked then
			g.mode = not horiz and "unlock" or "none"
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
		local lim = dx
		if (page == 1 and dx > 0) or (page == 3 and dx < 0) then
			lim = dx / 3
		end
		move_image(strip, -(page - 1) * W + lim, 0)
	elseif m == "drawer" then
		instant_image_transform(drawer.a)
		move_image(drawer.a, 0, drawer.open and math.max(0, dy) or
			math.max(0, H + dy))
	elseif m == "qs" then
		instant_image_transform(qs.a)
		move_image(qs.a, 0, qs.open and math.min(0, dy) or
			math.min(0, -QS_H + dy))
	elseif m == "leave" then
		local s = surface(running[focus])
		instant_image_transform(s)
		move_image(s, dx, SB)
		edge_show(g.edge, math.abs(dx) / (W / 3))
	elseif m == "close" then
		local s = surface(running[focus])
		local d = math.max(0, dy)
		instant_image_transform(s)
		if running[focus].rt then
			local k = 1 - 0.25 * math.min(1, d / H)
			resize_image(s, W * k, running[focus].ah * k)
			move_image(s, W * (1 - k) / 2, SB + d)
		else
			move_image(s, 0, SB + d)
		end
		blend_image(s, math.max(0.2, 1.0 - d / H))
		edge_show("top", d / (H / 4))
	elseif m == "scroll" then
		in_app(running[focus], running[focus].app.drag, dy, false)
	end
end

local function tap_page(x, y)
	local p = pages[page]
	for _, r in ipairs(p.hits or {}) do
		if hit(r, x, y) then
			if r.app then
				open_app(r.app.id, r)
			elseif r.drawer then
				drawer_show(true)
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
	if locked then
		return
	end
	if focus then
		local inst = running[focus]
		local ay = y - SB
		if ay < HB and x > W - 52 then
			close_app(focus)
			return
		elseif ay < HB and x > W - 96 and x <= W - 52 then
			leave_app(0)
			return
		end
		if g.consumed then
			return
		end
		if inst.app.tap then
			in_app(inst, inst.app.tap, x, ay)
		end
		return
	end
	if drawer.open then
		for _, r in ipairs(drawer.hits) do
			if hit(r, x, y) then
				open_app(r.app.id, r)
				return
			end
		end
		return
	end
	tap_page(x, y)
end

local function on_up()
	local dx, dy = g.x - g.x0, g.y - g.y0
	local m = g.mode
	edge_hide()
	if screen_off then
		g = nil
		return
	end
	if not m then
		on_tap(g.x, g.y)
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
			move_image(surface(running[focus]), 0, SB, 5, INTERP_EXPOUT)
		end
	elseif m == "close" then
		if dy > H / 4 then
			close_app(focus)
		else
			local inst = running[focus]
			local s = surface(inst)
			move_image(s, 0, SB, 5, INTERP_EXPOUT)
			if inst.rt then
				resize_image(s, W, inst.ah, 5, INTERP_EXPOUT)
			end
			blend_image(s, inst.rt and 0.996 or 1.0, 5)
		end
	elseif m == "scroll" then
		in_app(running[focus], running[focus].app.drag, dy, true)
	elseif m == "qs_slider" then
		qs_slide(g.x, true)
	elseif m == "slider" then
		in_app(running[focus], running[focus].app.slide, g.x, true)
	end
	g = nil
end

------------------------------------------------------------------------
-- entry points

local autorun, autorun_at, test_view, test_steps

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
	system_defaultfont(F_REG, 12, 1)
	build_shaders()
	if get_key then
		local ok, v = pcall(get_key, "phone_accent")
		if ok and tonumber(v) and ACCENTS[tonumber(v)] then
			accent_i = tonumber(v)
		end
	end
	set_wall_accent()

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
	sys.renderer = sys.gpu and "Adreno 530 · freedreno" or "Software · softpipe"
	read_status()

	draw_status()
	sbar.dots = {}
	for i = 1, 3 do
		table.insert(sbar.dots, box(nil, W / 2 - 22 + (i - 1) * 18, SB + 10, 7,
			7, {255, 255, 255}, "circle", 0.30, O_HOME + 5))
	end
	draw_drawer()
	add_event("info", "Welcome to DragonFly BSD",
		"Swipe sideways for events and open apps", true)
	add_event("gpu", sys.gpu and "Graphics: Adreno 530" or "Graphics: software",
		sys.gpu and "Mesa's freedreno, on the GPU" or
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
			-- one app a second, as a person would
			test_steps = {
				{25, function() open_app("clock") end},
				{50, function() open_app("settings") end},
				{75, function() open_app("files") end},
				{100, function() leave_app(1) set_page(3, 0) end},
			}
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
				on_up()
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
			show_banner("info", "Brightness", string.format("%d%%",
				math.floor(bl_to_frac(sys.backlight) * 100 + 0.5)))
		end
		return
	end
	if focus and running[focus].app.input then
		running[focus].app.input(running[focus], iotbl)
	end
end

local last_min, next_status = "", 0

function phone_clock_pulse()
	if test_steps and test_steps[1] and CLOCK >= test_steps[1][1] then
		table.remove(test_steps, 1)[2]()
	end
	for id, inst in pairs(running) do
		if inst.freeze_at and CLOCK >= inst.freeze_at then
			inst.freeze_at = nil
			if focus ~= id and valid_vid(inst.rt) then
				rendertarget_forceupdate(inst.rt, 0)
			end
		end
	end
	if kbd.hl_until and CLOCK >= kbd.hl_until then
		blend_image(kbd.hl, 0.0)
		kbd.hl_until = nil
	end
	if banner.hide_at and CLOCK >= banner.hide_at and valid_vid(banner.v) then
		move_image(banner.v, 10, -96, 8, INTERP_EXPIN)
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
		if not g then
			draw_home(pages[2])
			if locked then
				draw_lock()
			end
		end
	end
	if focus and running[focus].app.tick then
		in_app(running[focus], running[focus].app.tick)
	end
	if autorun and autorun_at and CLOCK >= autorun_at and running.term and
		valid_vid(running.term.vid, TYPE_FRAMESERVER) then
		term_type(running.term, autorun .. "\r")
		autorun = nil
	end
end
