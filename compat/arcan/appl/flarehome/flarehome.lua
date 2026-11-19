-- flarehome: the Flare phone shell, an Arcan appl.
--
-- A swipe-driven home in the spirit of MeeGo Harmattan's UI: a lock screen,
-- then three views side by side that you swipe between (Events, Applications,
-- Open applications). Apps open full screen with a zoom; swiping in from the
-- left or right edge puts the running app away as a live thumbnail in the
-- Open applications view. Everything is drawn by Arcan on Mesa's software
-- rasterizer, through Flare's KMS device; touch arrives from the virtio
-- tablet through /dev/input.
--
-- Lines starting with "flarehome:" are checked by Flare's boot test.

local W, H
local STATUS_H = 32
local FONT_THIN = "DejaVuSans-ExtraLight.ttf"
local FONT = "DejaVuSans.ttf"
local FONT_BOLD = "DejaVuSans-Bold.ttf"
local PX_PER_PT = 3.4 -- measured at start-up for the panel's density

local ORDER_HOME = 10
local ORDER_APP = 60
local ORDER_LOCK = 80
local ORDER_STATUS = 100

local shaders = {}
local status = {}
local lock = {}
local home = {page = 2, strip = nil, pages = {}, dots = {}}
local apps = {}
local open_apps = {}
local current_app = nil
local touch = {down = false, x = 0, y = 0, sx = 0, sy = 0, t0 = 0, moved = false}
local ticks = 0
local logged_events = 0

local APPS = {
	{"Phone", "\u{260E}", {52, 168, 83}},
	{"Messages", "\u{2709}", {41, 128, 230}},
	{"Browser", "\u{25CE}", {240, 130, 30}},
	{"Camera", "\u{25C9}", {70, 74, 82}},
	{"Gallery", "\u{25A3}", {150, 80, 200}},
	{"Music", "\u{266B}", {226, 52, 120}},
	{"Maps", "\u{2691}", {20, 160, 150}},
	{"Calendar", "19", {214, 48, 49}},
	{"Clock", "\u{25F7}", {35, 52, 110}},
	{"Settings", "\u{2699}", {110, 118, 129}},
	{"Terminal", ">_", {24, 24, 24}},
	{"Store", "\u{2605}", {245, 180, 20}},
	{"Weather", "\u{2600}", {60, 170, 230}},
	{"Notes", "\u{270E}", {230, 160, 40}},
	{"Contacts", "\u{263A}", {90, 70, 190}},
	{"Android", "\u{25B2}", {120, 190, 60}},
}

local EVENTS = {
	{"Flare OS", "Booted to the Arcan shell on the framekernel", "now", {255, 120, 40}},
	{"Display", "480\u{00D7}854 panel, KMS over shared memory", "now", {150, 80, 200}},
	{"Touch", "QEMU Virtio Tablet connected as /dev/input/event0", "now", {41, 128, 230}},
	{"Android", "hello, kotlin and selftest ran without Android", "1 min", {120, 190, 60}},
	{"Linux", "busybox, Rust std, Binder: all checks passed", "1 min", {52, 168, 83}},
	{"Storage", "Driver restarted under load, no reads lost", "2 min", {110, 118, 129}},
}

-- ---------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------

local function hex(c)
	return string.format("%02x%02x%02x", c[1], c[2], c[3])
end

-- Text at roughly `px` pixels line height.
local function text(str, px, color, font)
	local pt = math.max(1, math.floor(px / PX_PER_PT + 0.5))
	local vid = render_text(string.format("\\f%s,%d\\#%s%s", font or FONT, pt, color or "ffffff", str))
	return vid
end

local function size(vid)
	local p = image_surface_properties(vid)
	return p.width, p.height
end

-- Show `vid` at (x, y), linked to `parent`, inheriting its order.
local function place(vid, parent, x, y, order)
	if parent then
		link_image(vid, parent)
		image_inherit_order(vid, true)
	end
	order_image(vid, order or 1)
	move_image(vid, x, y)
	show_image(vid)
	return vid
end

local function centered(vid, parent, cx, y, order)
	local w = size(vid)
	return place(vid, parent, math.floor(cx - w / 2), y, order)
end

local function anchor(parent, x, y, order)
	local a = null_surface(1, 1)
	place(a, parent, x or 0, y or 0, order)
	return a
end

local function rounded(w, h, c, parent, x, y, order, shader)
	local v = color_surface(w, h, c[1], c[2], c[3])
	image_shader(v, shader or shaders.tile)
	return place(v, parent, x, y, order)
end

-- Actions to run after `n` logic ticks (when an animation has finished).
local pending = {}
local function later(n, fn)
	table.insert(pending, {at = ticks + n, fn = fn})
end

local function log(msg)
	print("flarehome: " .. msg)
end

-- ---------------------------------------------------------------------
-- Shaders: rounded tiles with a soft vertical sheen, and the wallpaper
-- ---------------------------------------------------------------------

local ROUNDED = [[
precision mediump float;
uniform vec3 obj_col;
uniform float obj_opacity;
uniform vec2 obj_output_sz;
uniform float radius;
uniform float sheen;
varying vec2 texco;
void main()
{
	vec2 p = texco * obj_output_sz;
	vec2 h = obj_output_sz * 0.5;
	vec2 q = abs(p - h) - (h - vec2(radius));
	float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
	float a = clamp(0.5 - d, 0.0, 1.0);
	vec3 c = obj_col * (1.0 + sheen * (0.5 - texco.y));
	gl_FragColor = vec4(c, a * obj_opacity);
}
]]

local WALLPAPER = [[
precision mediump float;
uniform float obj_opacity;
varying vec2 texco;
void main()
{
	vec2 p = texco - vec2(0.3, 0.25);
	float glow = exp(-dot(p, p) * 5.0);
	vec2 q = texco - vec2(0.85, 0.8);
	float glow2 = exp(-dot(q, q) * 7.0);
	vec3 base = mix(vec3(0.02, 0.04, 0.10), vec3(0.01, 0.02, 0.04), texco.y);
	vec3 c = base + glow * vec3(0.10, 0.22, 0.45) + glow2 * vec3(0.28, 0.08, 0.30);
	gl_FragColor = vec4(c, obj_opacity);
}
]]

local function build_shaders()
	shaders.tile = build_shader(nil, ROUNDED, "flare_tile")
	shader_uniform(shaders.tile, "radius", "f", 18.0)
	shader_uniform(shaders.tile, "sheen", "f", 0.35)
	shaders.card = shader_ugroup(shaders.tile)
	shader_uniform(shaders.card, "radius", "f", 10.0)
	shader_uniform(shaders.card, "sheen", "f", 0.0)
	shaders.pill = shader_ugroup(shaders.tile)
	shader_uniform(shaders.pill, "radius", "f", 4.0)
	shader_uniform(shaders.pill, "sheen", "f", 0.0)
	shaders.dot = shader_ugroup(shaders.tile)
	shader_uniform(shaders.dot, "radius", "f", 4.0)
	shader_uniform(shaders.dot, "sheen", "f", 0.0)
	shaders.wallpaper = build_shader(nil, WALLPAPER, "flare_wallpaper")
end

-- ---------------------------------------------------------------------
-- Status bar
-- ---------------------------------------------------------------------

local function clock_strings()
	local t = os.date("*t")
	local days = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"}
	local months = {"January", "February", "March", "April", "May", "June", "July",
		"August", "September", "October", "November", "December"}
	return string.format("%02d:%02d", t.hour, t.min),
		string.format("%s %d %s", days[t.wday], t.day, months[t.month]), t.min
end

local function build_status()
	status.bg = color_surface(W, STATUS_H, 0, 0, 0)
	place(status.bg, nil, 0, 0, ORDER_STATUS)
	blend_image(status.bg, 0.92)
	-- signal bars
	for i = 1, 4 do
		local h = 4 + i * 4
		local b = color_surface(4, h, 255, 255, 255)
		place(b, status.bg, 10 + (i - 1) * 7, 24 - h, 1)
	end
	place(text("Flare", 18, "ffffff", FONT), status.bg, 44, 6, 1)
	-- battery
	local bat = color_surface(26, 13, 255, 255, 255)
	place(bat, status.bg, W - 98, 10, 1)
	local hole = color_surface(22, 9, 0, 0, 0)
	place(hole, bat, 2, 2, 2)
	local level = color_surface(16, 9, 120, 220, 90)
	place(level, bat, 2, 2, 3)
	place(color_surface(3, 5, 255, 255, 255), bat, 26, 4, 1)
	status.clock_min = -1
end

local function update_status()
	local hm, _, min = clock_strings()
	if min == status.clock_min then
		return
	end
	status.clock_min = min
	if status.clock then
		delete_image(status.clock)
	end
	status.clock = text(hm, 18, "ffffff", FONT_BOLD)
	local w = size(status.clock)
	place(status.clock, status.bg, W - w - 12, 6, 1)
	if lock.root then
		local hm2, date = clock_strings()
		if lock.time then
			delete_image(lock.time)
			delete_image(lock.date)
		end
		lock.time = centered(text(hm2, 112, "ffffff", FONT_THIN), lock.root, W / 2, 120, 2)
		lock.date = centered(text(date, 26, "dce6ff", FONT), lock.root, W / 2, 248, 2)
	end
end

-- ---------------------------------------------------------------------
-- Lock screen
-- ---------------------------------------------------------------------

local function build_lock()
	lock.root = null_surface(W, H)
	place(lock.root, nil, 0, 0, ORDER_LOCK)
	-- The wallpaper shader is costly for a software rasterizer: run it once
	-- into a texture and show that.
	local src = color_surface(W, H, 0, 0, 0)
	image_shader(src, shaders.wallpaper)
	show_image(src)
	local wp = alloc_surface(W, H)
	define_rendertarget(wp, {src}, RENDERTARGET_DETACH, RENDERTARGET_NOSCALE, 0)
	rendertarget_forceupdate(wp)
	force_image_blend(wp, BLEND_NONE)
	place(wp, lock.root, 0, 0, 1)
	-- notification summary pills
	local y = 330
	for i, ev in ipairs({{"\u{2709}", "2 messages"}, {"\u{2605}", "3 apps installed"}}) do
		local pill = rounded(240, 44, {255, 255, 255}, lock.root, (W - 240) / 2, y, 2, shaders.card)
		blend_image(pill, 0.10)
		local t = text(ev[1] .. "  " .. ev[2], 20, "ffffff", FONT)
		local tw, th = size(t)
		place(t, lock.root, math.floor((W - tw) / 2), y + math.floor((44 - th) / 2), 3)
		y = y + 56
	end
	local hint = text("\u{2190}  Swipe to unlock  \u{2192}", 22, "ffffff", FONT)
	centered(hint, lock.root, W / 2, H - 110, 2)
	blend_image(hint, 0.75)
	local bar = rounded(120, 5, {255, 255, 255}, lock.root, (W - 120) / 2, H - 60, 2, shaders.pill)
	blend_image(bar, 0.6)
	lock.locked = true
end

local function unlock(direction)
	if not lock.locked then
		return
	end
	lock.locked = false
	show_image(home.strip)
	move_image(lock.root, direction * W, 0, 12)
	blend_image(lock.root, 0.0, 12)
	later(13, function() hide_image(lock.root) end)
	log("unlocked")
end

-- ---------------------------------------------------------------------
-- Home views
-- ---------------------------------------------------------------------

local function page_header(page, title)
	place(text(title, 46, "ffffff", FONT_THIN), page, 24, STATUS_H + 22, 2)
end

local function build_events(page)
	page_header(page, "Events")
	local y = STATUS_H + 100
	for _, ev in ipairs(EVENTS) do
		local card = rounded(W - 32, 88, {22, 22, 26}, page, 16, y, 2, shaders.card)
		rounded(52, 52, ev[4], card, 14, 18, 1, shaders.card)
		place(text(string.sub(ev[1], 1, 1), 26, "ffffff", FONT_BOLD), card, 30, 28, 2)
		place(text(ev[1], 20, "ffffff", FONT_BOLD), card, 80, 14, 1)
		local when = text(ev[3], 16, "9aa0a6", FONT)
		local ww = size(when)
		place(when, card, W - 32 - ww - 14, 16, 1)
		place(text(ev[2], 16, "c8ccd2", FONT), card, 80, 48, 1)
		y = y + 100
	end
end

local function build_launcher(page)
	local cols, cell_w, cell_h, icon = 4, W / 4, 132, 76
	local top = STATUS_H + 30
	for i, a in ipairs(APPS) do
		local col, row = (i - 1) % cols, math.floor((i - 1) / cols)
		local cx = col * cell_w + cell_w / 2
		local x, y = math.floor(cx - icon / 2), top + row * cell_h
		-- Icon art from icons/make_icons.py: an 88 px image holding the
		-- 76 px tile plus a 6 px margin for its shadow. Shown 1:1.
		local img = load_image("icons/" .. string.lower(a[1]) .. ".png")
		if valid_vid(img) then
			place(img, page, x - 6, y - 6, 2)
		else
			local tile = rounded(icon, icon, a[3], page, x, y, 2, shaders.tile)
			local glyph = text(a[2], 40, "ffffff", FONT)
			local gw, gh = size(glyph)
			place(glyph, tile, math.floor((icon - gw) / 2), math.floor((icon - gh) / 2), 1)
		end
		centered(text(a[1], 17, "ffffff", FONT), page, cx, y + icon + 8, 2)
		table.insert(apps, {name = a[1], glyph = a[2], color = a[3], x = x, y = y, w = icon, h = icon})
	end
end

local function build_open(page)
	page_header(page, "Open applications")
	home.open_page = page
	home.open_empty = text("No open applications", 22, "6b7178", FONT)
	centered(home.open_empty, page, W / 2, H / 2 - 20, 2)
end

local function build_home()
	-- No background surface: the display is cleared to black anyway, and
	-- every full-screen layer costs a software rasterizer dearly.
	home.strip = null_surface(1, 1)
	place(home.strip, nil, 0, 0, ORDER_HOME)
	for i = 1, 3 do
		home.pages[i] = anchor(home.strip, (i - 1) * W, 0, 1)
	end
	build_events(home.pages[1])
	build_launcher(home.pages[2])
	build_open(home.pages[3])
	for i = 1, 3 do
		home.dots[i] = rounded(8, 8, {255, 255, 255}, nil, W / 2 - 22 + (i - 1) * 18, H - 22, ORDER_HOME + 5, shaders.dot)
	end
	move_image(home.strip, -(home.page - 1) * W, 0)
end

local PAGE_NAMES = {"events", "applications", "open applications"}

local function update_dots()
	for i = 1, 3 do
		blend_image(home.dots[i], i == home.page and 1.0 or 0.3, 6)
	end
end

local function go_page(p, quiet)
	p = math.max(1, math.min(3, p))
	local changed = p ~= home.page
	home.page = p
	instant_image_transform(home.strip)
	move_image(home.strip, -(p - 1) * W, 0, 10)
	update_dots()
	if changed and not quiet then
		log("page " .. PAGE_NAMES[p])
	end
end

-- ---------------------------------------------------------------------
-- Applications: rendered into an offscreen target, so the same image is
-- the full-screen app, the zoom animation and the open-apps thumbnail.
-- ---------------------------------------------------------------------

local function app_content(a)
	local vids = {}
	local function add(v, x, y)
		order_image(v, #vids + 1)
		move_image(v, x, y)
		show_image(v)
		table.insert(vids, v)
		return v
	end
	add(color_surface(W, H, 246, 246, 248), 0, 0)
	add(color_surface(W, 104, a.color[1], a.color[2], a.color[3]), 0, 0)
	local title = text(a.name, 34, "ffffff", FONT)
	add(title, 24, STATUS_H + 26)
	local glyph = text(a.glyph, 34, "ffffff", FONT)
	local gw = size(glyph)
	add(glyph, W - gw - 24, STATUS_H + 26)
	local rows = {
		{"Running on Flare", "Arcan 0.6 on Mesa's software rasterizer"},
		{"Display", W .. "\u{00D7}" .. H .. " via /dev/dri/card0"},
		{"Input", "virtio tablet via /dev/input"},
		{"Gesture", "Swipe in from an edge to minimize"},
	}
	local y = 128
	for _, r in ipairs(rows) do
		add(text(r[1], 22, "1f2328", FONT), 24, y)
		add(text(r[2], 17, "6b7178", FONT), 24, y + 30)
		add(color_surface(W - 48, 1, 220, 222, 226), 24, y + 62)
		y = y + 78
	end
	return vids
end

local function open_app(a)
	if current_app then
		return
	end
	for _, o in ipairs(open_apps) do
		if o.app == a then
			return restore_app(o)
		end
	end
	local rt = alloc_surface(W, H)
	local vids = app_content(a)
	define_rendertarget(rt, vids, RENDERTARGET_DETACH, RENDERTARGET_NOSCALE, 0)
	rendertarget_forceupdate(rt)
	force_image_blend(rt, BLEND_NONE)
	order_image(rt, ORDER_APP)
	-- zoom out of the icon
	move_image(rt, a.x, a.y)
	resize_image(rt, a.w, a.h)
	show_image(rt)
	move_image(rt, 0, 0, 12)
	resize_image(rt, W, H, 12)
	current_app = {app = a, rt = rt}
	-- the home underneath is fully covered once the zoom ends
	later(13, function()
		if current_app and current_app.app == a then
			hide_image(home.strip)
		end
	end)
	log("launched " .. a.name)
end

local function layout_open()
	local tw, th = 200, math.floor(200 * H / W)
	for i, o in ipairs(open_apps) do
		local col, row = (i - 1) % 2, math.floor((i - 1) / 2)
		o.tx = 26 + col * (tw + 28)
		o.ty = STATUS_H + 96 + row * (th + 48)
		o.tw, o.th = tw, th
	end
	if #open_apps > 0 then
		hide_image(home.open_empty)
	else
		show_image(home.open_empty)
	end
end

local function minimize_app(direction)
	local o = current_app
	if not o then
		return
	end
	current_app = nil
	show_image(home.strip)
	table.insert(open_apps, o)
	layout_open()
	-- the thumbnail lives on the open-apps page
	link_image(o.rt, home.open_page)
	image_inherit_order(o.rt, true)
	order_image(o.rt, 3)
	image_texfilter(o.rt, FILTER_LINEAR) -- scaled down: filter it
	go_page(3, true)
	instant_image_transform(home.strip)
	move_image(home.strip, -2 * W, 0)
	move_image(o.rt, o.tx, o.ty, 12)
	resize_image(o.rt, o.tw, o.th, 12)
	if o.label then
		delete_image(o.label)
	end
	o.label = text(o.app.name, 18, "ffffff", FONT)
	local lw = size(o.label)
	place(o.label, home.open_page, o.tx + math.floor((o.tw - lw) / 2), o.ty + o.th + 10, 3)
	log("minimized " .. o.app.name .. " (" .. #open_apps .. " open)")
end

function restore_app(o)
	for i, x in ipairs(open_apps) do
		if x == o then
			table.remove(open_apps, i)
			break
		end
	end
	if o.label then
		delete_image(o.label)
		o.label = nil
	end
	local x, y = image_surface_resolve(o.rt).x, image_surface_resolve(o.rt).y
	link_image(o.rt, o.rt)
	order_image(o.rt, ORDER_APP)
	image_texfilter(o.rt, FILTER_NONE)
	move_image(o.rt, x, y)
	move_image(o.rt, 0, 0, 10)
	resize_image(o.rt, W, H, 10)
	current_app = o
	later(11, function()
		if current_app == o then
			hide_image(home.strip)
		end
	end)
	layout_open()
	for _, p in ipairs(open_apps) do
		move_image(p.rt, p.tx, p.ty, 8)
		if p.label then
			local lw = size(p.label)
			move_image(p.label, p.tx + math.floor((p.tw - lw) / 2), p.ty + p.th + 10, 8)
		end
	end
	log("restored " .. o.app.name)
end

-- ---------------------------------------------------------------------
-- Touch
-- ---------------------------------------------------------------------

local function on_tap(x, y)
	if lock.locked then
		return
	end
	if current_app then
		return
	end
	if home.page == 2 then
		for _, a in ipairs(apps) do
			if x >= a.x and x < a.x + a.w and y >= a.y and y < a.y + a.h + 30 then
				open_app(a)
				return
			end
		end
	elseif home.page == 3 then
		for _, o in ipairs(open_apps) do
			if x >= o.tx and x < o.tx + o.tw and y >= o.ty and y < o.ty + o.th then
				restore_app(o)
				return
			end
		end
	end
end

local function on_drag(dx, dy)
	if lock.locked then
		instant_image_transform(lock.root)
		move_image(lock.root, dx, 0)
		blend_image(lock.root, math.max(0.2, 1.0 - math.abs(dx) / W))
	elseif current_app then
		if touch.edge then
			instant_image_transform(current_app.rt)
			local s = math.max(0.5, 1.0 - math.abs(dx) / (W * 1.5))
			resize_image(current_app.rt, W * s, H * s)
			move_image(current_app.rt, dx + W * (1 - s) / 2, H * (1 - s) / 2)
		end
	else
		instant_image_transform(home.strip)
		move_image(home.strip, -(home.page - 1) * W + dx, 0)
	end
end

local function on_release(dx, dy)
	if lock.locked then
		if math.abs(dx) > W / 4 then
			unlock(dx > 0 and 1 or -1)
		else
			move_image(lock.root, 0, 0, 8)
			blend_image(lock.root, 1.0, 8)
		end
	elseif current_app then
		if touch.edge then
			if math.abs(dx) > W / 5 then
				minimize_app(dx > 0 and 1 or -1)
			else
				instant_image_transform(current_app.rt)
				move_image(current_app.rt, 0, 0, 8)
				resize_image(current_app.rt, W, H, 8)
			end
		end
	else
		if dx < -W / 5 then
			go_page(home.page + 1)
		elseif dx > W / 5 then
			go_page(home.page - 1)
		else
			go_page(home.page)
		end
	end
end

local function touch_point(iotbl)
	-- Absolute pointer axes arrive in device units (0..32767 for the virtio
	-- tablet); map them to the panel.
	local v = iotbl.samples and iotbl.samples[1] or 0
	if iotbl.subid == 0 then
		touch.x = math.floor(v * W / 32768)
	elseif iotbl.subid == 1 then
		touch.y = math.floor(v * H / 32768)
	end
end

function flarehome_input(iotbl)
	if logged_events < 3 then
		logged_events = logged_events + 1
		print(string.format("flarehome: input kind=%s digital=%s analog=%s touch=%s subid=%s",
			tostring(iotbl.kind), tostring(iotbl.digital), tostring(iotbl.analog),
			tostring(iotbl.touch), tostring(iotbl.subid)))
	end
	if iotbl.touch then
		touch.x = math.floor(iotbl.x * W / 32768)
		touch.y = math.floor(iotbl.y * H / 32768)
	elseif iotbl.analog then
		touch_point(iotbl)
	end
	if iotbl.digital and iotbl.translated then
		-- hardware keys: arrows switch views, Escape minimizes
		if iotbl.active then
			local sym = iotbl.keysym
			if sym == 276 then
				go_page(home.page - 1)
			elseif sym == 275 then
				go_page(home.page + 1)
			elseif sym == 27 then
				minimize_app(1)
			elseif lock.locked then
				unlock(1)
			end
		end
		return
	end
	if iotbl.digital then
		if iotbl.active and not touch.down then
			touch.down = true
			touch.sx, touch.sy = touch.x, touch.y
			touch.moved = false
			touch.edge = touch.sx < 28 or touch.sx > W - 28
		elseif not iotbl.active and touch.down then
			touch.down = false
			local dx, dy = touch.x - touch.sx, touch.y - touch.sy
			if math.abs(dx) < 12 and math.abs(dy) < 12 then
				on_tap(touch.x, touch.y)
			else
				on_release(dx, dy)
			end
		end
		return
	end
	if touch.down then
		local dx, dy = touch.x - touch.sx, touch.y - touch.sy
		if math.abs(dx) > 6 or math.abs(dy) > 6 then
			touch.moved = true
			on_drag(dx, dy)
		end
	end
end

-- ---------------------------------------------------------------------
-- Entry points
-- ---------------------------------------------------------------------

function flarehome()
	W, H = VRESW, VRESH
	-- Software rendering budget: almost everything is drawn 1:1, so
	-- nearest-neighbour sampling looks the same as bilinear and is far
	-- cheaper; opaque full-screen layers skip blending.
	switch_default_texfilter(FILTER_NONE)
	-- calibrate text size: the panel is dense (about 244 dpi)
	local probe = render_text(string.format("\\f%s,10\\#ffffffHg", FONT))
	local _, ph = size(probe)
	delete_image(probe)
	PX_PER_PT = ph / 10
	build_shaders()
	build_home()
	build_lock()
	build_status()
	update_status()
	update_dots()
	hide_image(home.strip) -- under the lock screen
	log(string.format("ready on a %dx%d panel, %d apps, %d events", W, H, #APPS, #EVENTS))
end

function flarehome_clock_pulse()
	ticks = ticks + 1
	for i = #pending, 1, -1 do
		if pending[i].at <= ticks then
			local p = table.remove(pending, i)
			p.fn()
		end
	end
	if ticks % 25 == 0 then
		update_status()
	end
end
