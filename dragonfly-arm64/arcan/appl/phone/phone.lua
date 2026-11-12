-- phone: the shell for DragonFly BSD on the Xiaomi Mi 5.
--
-- The look follows ValOS, in black: a photograph of a city at night
-- (London, New York, Warsaw, Sydney) under glossy black glass; a home of
-- widget cards (at a glance, files, the recent app), a
-- floating search pill and a dock of round glass buttons with thin line
-- icons; a quick-settings sheet with tabs, tiles and sliders that light
-- up white.  The glass is real: every glass surface shows a blurred copy
-- of the wallpaper (a small render target, sampled at the surface's own
-- screen position) under a black tint with a glossy sheen.  The
-- wallpaper, the "ambience", is chosen in quick settings or Settings;
-- the photographs and their credits are in wallpapers/.
--
-- Three views side by side, panned with a horizontal swipe, as on the N9
-- and Sailfish:  Events | Home | Open apps.  In an app, swipe in from the
-- left or right edge to leave it running, down from the top edge to close
-- it.  Tap the status bar, the avatar or pull down at home for quick
-- settings; swipe up at home (or the grid button) for all apps.  Power
-- locks, home goes home, volume sets the brightness.
--
-- Built-in apps draw into their own render targets (live thumbnails, the
-- zoom out of their icons); the terminal is Arcan's terminal frameserver.
--
-- Arguments: autorun=CMD (opens the terminal and types CMD), test=VIEW
-- (opens a view at start, for QEMU screenshots), touchw=N (touch panel
-- width in pixels, 1080).

-- Layout is in a space of 540 x 960 logical pixels; S maps it onto the
-- canvas: 2 at the panel's full 1080 x 1920 (with the GPU), 1 at half
-- resolution (software rendering).  Everything that takes a position or
-- a size goes through these wrappers (they replace Arcan's own in this
-- appl's environment); text is rasterised at the final size, so it
-- stays sharp.
local S = 1
local A = {move = move_image, resize = resize_image, color = color_surface,
	null = null_surface, alloc = alloc_surface,
	props = image_surface_properties, hint = target_displayhint}
local function px(n)
	return math.max(1, math.floor(n * S + 0.5))
end
function move_image(v, x, y, t, i)
	return A.move(v, x * S, y * S, t, i)
end
function resize_image(v, w, h, t, i)
	return A.resize(v, w * S, h * S, t, i)
end
function color_surface(w, h, r, g, b)
	return A.color(px(w), px(h), r, g, b)
end
function null_surface(w, h)
	return A.null(px(w), px(h))
end
function alloc_surface(w, h)
	return A.alloc(px(w), px(h))
end
function image_surface_properties(v)
	local p = A.props(v)
	return {width = p.width / S, height = p.height / S, x = p.x, y = p.y}
end
function target_displayhint(v, w, h, ...)
	return A.hint(v, px(w), px(h), ...)
end

local W, H
local SB = 34			-- status bar
local HB = 100			-- an app's header
local touch_w = 1080
local tscale = 1

local KEY_HOME, KEY_VOLDOWN, KEY_VOLUP, KEY_POWER = 102, 114, 115, 116

local F_REG = "Inter-Regular.ttf"
local F_MED = "Inter-Medium.ttf"
local F_SB = "Inter-SemiBold.ttf"
local F_LIGHT = "InterDisplay-Light.ttf"
local F_SYM = "DejaVuSans.ttf"
local F_MONO = "DejaVuSansMono.ttf"

local WHITE = {255, 255, 255}
local ICE = {0xf2, 0xf4, 0xf7}		-- what lights up
local INK = "0b0d11"			-- text on what lights up
local C = {
	text = "ffffff", text2 = "c9ced6", text3 = "868d98",
	inset = {4, 5, 7},		-- wells inside glass
	tile = {0x26, 0x2a, 0x31},
	gloss = {0x0b, 0x0d, 0x11},
	disc = {0x3a, 0x3e, 0x46},	-- icon discs: graphite, not black
}
-- the ambiences: cities at night (wallpapers/CREDITS)
local SCENES = {
	{"London", file = "london.jpg"},
	{"New York", file = "newyork.jpg"},
	{"Warsaw", file = "warsaw.jpg"},
	{"Sydney", file = "sydney.jpg"},
	{"Warsaw, snow", file = "warsaw-snow.jpg"},
}
local scene_i = 1
local function accent()
	return ICE
end
local function hex(c)
	return string.format("%02x%02x%02x", c[1], c[2], c[3])
end
local function peach()
	return ICE
end

-- order bands in the world
local O_WALL, O_HOME, O_DRAWER, O_APP = 1, 10, 60, 100
local O_EDGE, O_KBD, O_LOCK, O_SB, O_QS, O_BANNER = 170, 180, 190, 200, 300, 350

------------------------------------------------------------------------
-- shaders

local shaders = {}

local SH_HEAD = [[
uniform float PX;
uniform float obj_opacity;
uniform vec3 obj_col;
uniform vec2 obj_output_sz;
varying vec2 texco;
float rr(vec2 p, vec2 sz, float r)
{
	vec2 q = abs(p - 0.5 * sz) - (0.5 * sz - vec2(r));
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
float cover(float d)
{
	return clamp(0.5 - d, 0.0, 1.0);
}
]]

-- the glass surfaces' vertex stage also hands on the screen position
local GLASS_V = [[
uniform mat4 modelview;
uniform mat4 projection;
attribute vec2 texcoord;
attribute vec4 vertex;
varying vec2 texco;
varying vec2 wpos;
void main()
{
	vec4 p = modelview * vertex;
	wpos = p.xy;
	gl_Position = projection * p;
	texco = texcoord;
}]]

-- warm frosted glass: the blurred wallpaper under the surface, tinted
local function glass_f(amount, radius, tint)
	return [[
uniform sampler2D map_tu0;
uniform float PX;
uniform float obj_opacity;
uniform vec2 obj_output_sz;
uniform vec2 screen;
varying vec2 texco;
varying vec2 wpos;
float rr(vec2 p, vec2 sz, float r)
{
	vec2 q = abs(p - 0.5 * sz) - (0.5 * sz - vec2(r));
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz, sz, ]] .. radius .. [[);
	vec2 uv = clamp(wpos / screen, vec2(0.0), vec2(1.0));
	vec2 ox = vec2(0.016, 0.0);
	vec2 oy = vec2(0.0, 0.009);
	vec3 b = texture2D(map_tu0, uv).rgb * 0.2 +
		(texture2D(map_tu0, uv + ox).rgb + texture2D(map_tu0, uv - ox).rgb +
		texture2D(map_tu0, uv + oy).rgb + texture2D(map_tu0, uv - oy).rgb) *
		0.2;
	vec3 tint = ]] .. (tint or "vec3(0.016, 0.019, 0.025)") .. [[;
	vec3 c = mix(b, tint, ]] .. amount .. [[);
	// the gloss: a sheen over the upper part, a bright rim at the top
	c += vec3(0.075) * (1.0 - smoothstep(0.0, 0.42, texco.y));
	float rim = 1.0 - smoothstep(0.0, 1.5 * PX, abs(d + PX));
	c += vec3(0.30) * rim * (1.0 - smoothstep(0.0, 0.5, texco.y)) +
		vec3(0.06) * rim;
	gl_FragColor = vec4(c, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]]
end

local function build_shaders()
	local function mk(name, body)
		shaders[name] = build_shader(nil, SH_HEAD .. body, "phone_" .. name)
	end
	mk("rrect", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz, sz, min(0.5 * min(sz.x, sz.y), 18.0 * PX));
	gl_FragColor = vec4(obj_col, obj_opacity * cover(d));
}]])
	mk("pill", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz, sz, 0.5 * min(sz.x, sz.y));
	gl_FragColor = vec4(obj_col, obj_opacity * cover(d));
}]])
	mk("circle", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = length(texco * sz - 0.5 * sz) - 0.5 * min(sz.x, sz.y);
	gl_FragColor = vec4(obj_col, obj_opacity * cover(d));
}]])
	-- thin line drawing: a 1.8 px ring and a 1.8 px rounded rectangle
	mk("lring", [[
void main()
{
	vec2 sz = obj_output_sz;
	float r = 0.5 * min(sz.x, sz.y) - 1.2 * PX;
	float d = abs(length(texco * sz - 0.5 * sz) - r) - 1.1 * PX;
	gl_FragColor = vec4(obj_col, obj_opacity * cover(d));
}]])
	mk("lrect", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = abs(rr(texco * sz - vec2(1.2 * PX), sz - vec2(2.4 * PX),
		min(0.28 * min(sz.x, sz.y), 6.0 * PX))) - 1.1 * PX;
	gl_FragColor = vec4(obj_col, obj_opacity * cover(d));
}]])
	-- the avatar: a glossy black sphere
	mk("orb", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = length(texco * sz - 0.5 * sz) - 0.5 * min(sz.x, sz.y);
	vec3 c = mix(vec3(0.16, 0.17, 0.20), vec3(0.01), smoothstep(0.0, 0.75,
		length(texco - vec2(0.42, 0.36))));
	float hl = max(0.0, 1.0 - length((texco - vec2(0.36, 0.26)) *
		vec2(3.2, 4.6)));
	c += vec3(0.9) * pow(hl, 2.2);
	c += vec3(0.10) * smoothstep(0.55, 0.95, texco.y);
	gl_FragColor = vec4(c, obj_opacity * cover(d));
}]])
	-- a glossy black disc, for icons that sit on a dot
	mk("gloss", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = length(texco * sz - 0.5 * sz) - 0.5 * min(sz.x, sz.y);
	vec3 c = obj_col + vec3(0.16) * (1.0 - smoothstep(0.0, 0.5, texco.y));
	c += vec3(0.28) * (1.0 - smoothstep(0.0, 1.4 * PX, abs(d + PX)));
	gl_FragColor = vec4(c, obj_opacity * cover(d));
}]])
	-- a soft shadow, 12 px of blur
	mk("shadow", [[
void main()
{
	vec2 sz = obj_output_sz;
	float d = rr(texco * sz - vec2(12.0 * PX), sz - vec2(24.0 * PX),
		min(0.3 * min(sz.x, sz.y), 26.0 * PX));
	gl_FragColor = vec4(obj_col, obj_opacity * (1.0 - smoothstep(-8.0 * PX,
		12.0 * PX, d)));
}]])
	-- a texture with round corners (an app, a thumbnail); crop is the
	-- share of the texture's height shown (texture coordinates are
	-- scaled to it)
	local rimg = [[
uniform sampler2D map_tu0;
uniform float PX;
uniform float obj_opacity;
uniform vec2 obj_output_sz;
uniform float crop;
varying vec2 texco;
void main()
{
	vec2 sz = obj_output_sz;
	vec2 tc = vec2(texco.x, texco.y / max(crop, 0.001));
	vec2 q = abs(tc * sz - 0.5 * sz) - (0.5 * sz - vec2(RAD));
	float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - RAD;
	vec4 t = texture2D(map_tu0, texco);
	gl_FragColor = vec4(t.rgb, t.a * obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]]
	shaders.rimg = build_shader(nil, (string.gsub(rimg, "RAD",
		"(26.0 * PX)")), "phone_rimg")
	shaders.rthumb = build_shader(nil, (string.gsub(rimg, "RAD",
		"(16.0 * PX)")), "phone_rthumb")
	shaders.rcrop = build_shader(nil, (string.gsub(rimg, "RAD",
		"(16.0 * PX)")), "phone_rcrop")
	for _, s in ipairs({shaders.rimg, shaders.rthumb, shaders.rcrop}) do
		shader_uniform(s, "crop", "f", 1.0)
	end

	-- the glass family
	local function mkg(name, amount, radius, tint)
		shaders[name] = build_shader(GLASS_V, glass_f(amount, radius, tint),
			"phone_" .. name)
		shader_uniform(shaders[name], "screen", "ff", W * S, H * S)
	end
	mkg("glass", "0.64", "min(0.5 * min(sz.x, sz.y), 28.0 * PX)")
	mkg("glass_dark", "0.80", "min(0.5 * min(sz.x, sz.y), 22.0 * PX)")
	mkg("glass_pill", "0.68", "0.5 * min(sz.x, sz.y)")
	mkg("glass_sheet", "0.74", "30.0 * PX")
	mkg("glass_flat", "0.84", "0.0")
	-- buttons and app faces: a lighter, graphite glass
	mkg("glass_grey", "0.70", "min(0.5 * min(sz.x, sz.y), 22.0 * PX)",
		"vec3(0.19, 0.20, 0.225)")

	-- a scene's photograph in a circle: the swatches.  The texture
	-- coordinates take a square from the middle of the portrait picture
	-- (0.22 to 0.78 of its height), which the mask undoes
	shaders.swatch = build_shader(nil, [[
uniform sampler2D map_tu0;
uniform float obj_opacity;
uniform vec2 obj_output_sz;
varying vec2 texco;
void main()
{
	vec2 sz = obj_output_sz;
	vec2 tc = vec2(texco.x, (texco.y - 0.22) / 0.5625);
	float d = length(tc * sz - 0.5 * sz) - 0.5 * min(sz.x, sz.y);
	vec4 t = texture2D(map_tu0, texco);
	gl_FragColor = vec4(t.rgb, obj_opacity * clamp(0.5 - d, 0.0, 1.0));
}]], "phone_swatch")
	-- the pixel constants scale with the canvas
	for k, sh in pairs(shaders) do
		if k ~= "swatch" then
			shader_uniform(sh, "PX", "f", S)
		end
	end
end

------------------------------------------------------------------------
-- drawing helpers.  While an app builds its content cur_rt names its
-- render target, and every object made is attached to it.

local cur_rt = nil
local wall_small		-- the blurred wallpaper the glass shows
local wall_big			-- the wallpaper, drawn once

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

-- frosted glass; kind is glass, glass_dark, glass_pill, glass_sheet
local function glass(parent, x, y, w, h, kind, order, alpha)
	local v = null_surface(math.max(1, math.floor(w)),
		math.max(1, math.floor(h)))
	image_sharestorage(wall_small, v)
	image_shader(v, shaders[kind or "glass"])
	place(v, parent, x, y, order)
	blend_image(v, math.min(alpha or 1.0, 0.996))
	force_image_blend(v, BLEND_NORMAL)
	return v
end

-- a well inside glass (fields, wells, rows)
local function inset(parent, x, y, w, h, order, shape, alpha)
	return box(parent, x, y, w, h, C.inset, shape or "rrect", alpha or 0.50,
		order)
end

-- text; the string goes in raw (never as a format string)
local function text(parent, x, y, str, size, color, font, order)
	local v = render_text({string.format([[\f%s,%d\#%s]], font or F_REG,
		px(size), color or C.text), tostring(str)})
	if not valid_vid(v) then
		return nil, 0, 0
	end
	local p = image_surface_properties(v)
	place(v, parent, x, y, order or 4)
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

-- the width a string would take
local function measure(str, size, font)
	local v = render_text({string.format([[\f%s,%d]], font or F_REG,
		px(size)), tostring(str)})
	if not valid_vid(v) then
		return 0
	end
	local w = image_surface_properties(v).width
	delete_image(v)
	return w
end

-- small capitals, as ValOS's labels ("AT A GLANCE")
local function eyebrow(parent, x, y, str, order)
	local spaced = string.gsub(string.upper(str),
		"([%z\1-\127\194-\244][\128-\191]*)", "%1 ")
	return text(parent, x, y, string.sub(spaced, 1, -2), 8, C.text3, F_SB,
		order)
end

-- the wallpaper again, as an object (the lock screen's)
local function wallpaper(parent, order)
	local v = null_surface(W, H)
	image_sharestorage(wall_big, v)
	place(v, parent, 0, 0, order)
	show_image(v)
	return v
end

-- the photographs, loaded once (see build_wallpaper)
local scene_img = {}

-- a round thumbnail of scene i
local function swatch(parent, x, y, d, i, order)
	local v = null_surface(d, d)
	if valid_vid(scene_img[i]) then
		image_sharestorage(scene_img[i], v)
		image_set_txcos(v, {0, 0.22, 1, 0.22, 1, 0.7825, 0, 0.7825})
		image_shader(v, shaders.swatch)
	end
	place(v, parent, x, y, order)
	blend_image(v, 0.996)
	force_image_blend(v, BLEND_NORMAL)
	return v
end

local function anchor(parent, x, y, w, h, order)
	local v = null_surface(math.max(1, w or 1), math.max(1, h or 1))
	place(v, parent, x or 0, y or 0, order)
	show_image(v)
	return v
end

-- a bar of length len and thickness th, centred on cx, cy, at angle deg
local function bar(parent, cx, cy, len, th, deg, col, order, alpha)
	local v = box(parent, 0, 0, th, len, col or WHITE, "pill", alpha or 1.0,
		order)
	if v then
		move_image(v, math.floor(cx - th / 2 + 0.5),
			math.floor(cy - len / 2 + 0.5))
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
-- line icons: thin white drawings, centred on cx, cy, s across

local L = 2.2		-- line weight
local IC = WHITE		-- line colour
local icons = {}

local function lrect(p, cx, cy, w, h, o, col)
	return box(p, cx - w / 2, cy - h / 2, w, h, col or IC, "lrect", 1.0, o)
end

local function lring(p, cx, cy, d, o, col)
	return box(p, cx - d / 2, cy - d / 2, d, d, col or IC, "lring", 1.0, o)
end

function icons.terminal(p, cx, cy, s, o)
	bar(p, cx - s * 0.20, cy - s * 0.09, s * 0.30, L, -45, IC, o)
	bar(p, cx - s * 0.20, cy + s * 0.09, s * 0.30, L, 45, IC, o)
	bar(p, cx + s * 0.18, cy + s * 0.22, s * 0.32, L, 90, IC, o)
end

function icons.folder(p, cx, cy, s, o)
	lrect(p, cx, cy + s * 0.06, s * 0.86, s * 0.62, o)
	bar(p, cx - s * 0.22, cy - s * 0.28, s * 0.34, L, 90, IC, o)
end

function icons.file(p, cx, cy, s, o)
	lrect(p, cx, cy, s * 0.62, s * 0.82, o)
	for k = -1, 1 do
		bar(p, cx, cy + k * s * 0.17, s * 0.30, L, 90, IC, o)
	end
end

function icons.calc(p, cx, cy, s, o)
	-- plus, minus, times, equals in a square of four
	local d = s * 0.22
	bar(p, cx - d, cy - d, s * 0.30, L, 0, IC, o)
	bar(p, cx - d, cy - d, s * 0.30, L, 90, IC, o)
	bar(p, cx + d, cy - d, s * 0.30, L, 90, IC, o)
	bar(p, cx - d, cy + d, s * 0.30, L, 45, IC, o)
	bar(p, cx - d, cy + d, s * 0.30, L, -45, IC, o)
	bar(p, cx + d, cy + d - s * 0.07, s * 0.30, L, 90, IC, o)
	bar(p, cx + d, cy + d + s * 0.07, s * 0.30, L, 90, IC, o)
end

function icons.gear(p, cx, cy, s, o)
	lring(p, cx, cy, s * 0.56, o)
	lring(p, cx, cy, s * 0.22, o)
	for i = 0, 7 do
		local a = math.rad(i * 45)
		bar(p, cx + math.sin(a) * s * 0.38, cy - math.cos(a) * s * 0.38,
			s * 0.14, L * 1.4, i * 45, IC, o)
	end
end

function icons.chip(p, cx, cy, s, o)
	lrect(p, cx, cy, s * 0.58, s * 0.58, o)
	lrect(p, cx, cy, s * 0.22, s * 0.22, o)
	for k = -1, 1 do
		local d = k * s * 0.16
		bar(p, cx + d, cy - s * 0.38, s * 0.14, L, 0, IC, o)
		bar(p, cx + d, cy + s * 0.38, s * 0.14, L, 0, IC, o)
		bar(p, cx - s * 0.38, cy + d, s * 0.14, L, 90, IC, o)
		bar(p, cx + s * 0.38, cy + d, s * 0.14, L, 90, IC, o)
	end
end

function icons.clock(p, cx, cy, s, o)
	lring(p, cx, cy, s * 0.86, o)
	bar(p, cx, cy - s * 0.12, s * 0.26, L, 0, IC, o)
	bar(p, cx + s * 0.09, cy, s * 0.20, L, 90, IC, o)
end

function icons.grid(p, cx, cy, s, o)
	for i = -1, 1, 2 do
		for j = -1, 1, 2 do
			lrect(p, cx + i * s * 0.2, cy + j * s * 0.2, s * 0.32, s * 0.32, o)
		end
	end
end

function icons.recents(p, cx, cy, s, o)
	lrect(p, cx - s * 0.08, cy, s * 0.52, s * 0.72, o)
	bar(p, cx + s * 0.30, cy, s * 0.50, L, 0, IC, o)
end

function icons.search(p, cx, cy, s, o)
	lring(p, cx - s * 0.08, cy - s * 0.08, s * 0.60, o)
	bar(p, cx + s * 0.24, cy + s * 0.24, s * 0.26, L * 1.1, -45, IC, o)
end

function icons.arrow(p, cx, cy, s, o)
	bar(p, cx, cy, s * 0.76, L, 90, IC, o)
	bar(p, cx + s * 0.25, cy - s * 0.13, s * 0.38, L, -45, IC, o)
	bar(p, cx + s * 0.25, cy + s * 0.13, s * 0.38, L, 45, IC, o)
end

function icons.back(p, cx, cy, s, o)
	bar(p, cx, cy, s * 0.76, L, 90, IC, o)
	bar(p, cx - s * 0.25, cy - s * 0.13, s * 0.38, L, 45, IC, o)
	bar(p, cx - s * 0.25, cy + s * 0.13, s * 0.38, L, -45, IC, o)
end

function icons.pencil(p, cx, cy, s, o)
	bar(p, cx, cy, s * 0.80, L * 1.2, 45, IC, o)
	bar(p, cx - s * 0.24, cy + s * 0.34, s * 0.16, L * 1.2, 90, IC, o)
end

function icons.close(p, cx, cy, s, o)
	bar(p, cx, cy, s * 0.70, L, 45, IC, o)
	bar(p, cx, cy, s * 0.70, L, -45, IC, o)
end

function icons.minus(p, cx, cy, s, o)
	bar(p, cx, cy, s * 0.62, L, 90, IC, o)
end

function icons.sun(p, cx, cy, s, o)
	lring(p, cx, cy, s * 0.42, o)
	for i = 0, 7 do
		local a = math.rad(i * 45)
		bar(p, cx + math.sin(a) * s * 0.38, cy - math.cos(a) * s * 0.38,
			s * 0.12, L, i * 45, IC, o)
	end
end

function icons.torch(p, cx, cy, s, o)
	lrect(p, cx, cy + s * 0.16, s * 0.30, s * 0.50, o)
	bar(p, cx - s * 0.17, cy - s * 0.22, s * 0.26, L, 30, IC, o)
	bar(p, cx + s * 0.17, cy - s * 0.22, s * 0.26, L, -30, IC, o)
	bar(p, cx, cy - s * 0.33, s * 0.32, L, 90, IC, o)
end

function icons.bulb(p, cx, cy, s, o)
	lring(p, cx, cy - s * 0.08, s * 0.58, o)
	bar(p, cx, cy + s * 0.34, s * 0.24, L, 90, IC, o)
end

function icons.vibrate(p, cx, cy, s, o)
	lrect(p, cx, cy, s * 0.40, s * 0.74, o)
	bar(p, cx - s * 0.36, cy, s * 0.36, L, 0, IC, o)
	bar(p, cx + s * 0.36, cy, s * 0.36, L, 0, IC, o)
end

function icons.lock(p, cx, cy, s, o)
	lrect(p, cx, cy + s * 0.14, s * 0.62, s * 0.48, o)
	lring(p, cx, cy - s * 0.16, s * 0.38, o)
end

function icons.power(p, cx, cy, s, o)
	lring(p, cx, cy + s * 0.04, s * 0.70, o)
	bar(p, cx, cy - s * 0.20, s * 0.36, L * 1.2, 0, IC, o)
end

function icons.restart(p, cx, cy, s, o)
	lring(p, cx, cy, s * 0.70, o)
	bar(p, cx + s * 0.30, cy - s * 0.30, s * 0.20, L * 1.3, 0, IC, o)
	bar(p, cx + s * 0.22, cy - s * 0.38, s * 0.20, L * 1.3, 90, IC, o)
end

function icons.bell(p, cx, cy, s, o)
	lrect(p, cx, cy, s * 0.56, s * 0.56, o)
	bar(p, cx, cy + s * 0.36, s * 0.62, L, 90, IC, o)
	box(p, cx - 2, cy - s * 0.40, 4, 4, IC, "circle", 1.0, o)
end

function icons.pin(p, cx, cy, s, o)
	lring(p, cx, cy - s * 0.12, s * 0.48, o)
	bar(p, cx, cy + s * 0.24, s * 0.30, L, 0, IC, o)
end

function icons.play(p, cx, cy, s, o)
	text_c(p, cx + s * 0.06, cy - s * 0.42, "▶", math.floor(s * 0.55), C.text,
		F_SYM, o)
end

function icons.charge(p, cx, cy, s, o)
	text_c(p, cx, cy - s * 0.42, "⚡", math.floor(s * 0.55), C.text, F_SYM, o)
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
local qs = {open = false, tab = 3}
local lock = {}
local sbar = {}
local edge = {}
local events = {}
local g = nil
local redraw_pages, redraw_all
local open_app, leave_app, close_app, go_home, set_page, add_event
local qs_show, lock_screen, draw_qs, drawer_show

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
		add_event("bell", "Battery low",
			string.format("%d%% left", sys.battery))
	end
end

local function clock_str()
	return os.date("%H:%M")
end

local function nozero(s)
	return (string.gsub(s, " 0(%d)", " %1"))
end

local function date_str()
	return nozero(os.date("%A, %d %B"))
end

local function short_date()
	return nozero(os.date("%a, %b %d"))
end

local function charge_state()
	return sys.charger == "charging" and "Charging" or
		(sys.charger == "full" and "Full" or
		(sys.charger == "present" and "Plugged in" or
		(sys.battery < 0 and "Unknown" or "On battery")))
end

local function greeting()
	local h = tonumber(os.date("%H"))
	if h < 5 then
		return "Good", "night."
	elseif h < 12 then
		return "Good", "morning."
	elseif h < 18 then
		return "Good", "afternoon."
	end
	return "Good", "evening."
end

------------------------------------------------------------------------
-- status bar: transparent over the wallpaper, as ValOS's

local function draw_status()
	if not valid_vid(sbar.a) then
		sbar.bg = box(nil, 0, 0, W, SB, {0, 0, 0}, nil, 0.0, O_SB)
		sbar.a = anchor(nil, 0, 0, W, SB, O_SB + 1)
	end
	local key = clock_str() .. sys.battery .. sys.charger .. tostring(focus)
	if key == sbar.key then
		return
	end
	sbar.key = key
	clear_children(sbar.a)
	-- a light shade under the bar while an app is in front
	blend_image(sbar.bg, focus and 0.30 or 0.0, 4)
	text(sbar.a, 22, 9, clock_str(), 12, C.text, F_SB)
	-- battery
	local bx, by, bw, bh = W - 46, 11, 24, 12
	box(sbar.a, bx, by, bw, bh, WHITE, "lrect", 1.0, 1)
	local pct = math.max(0, math.min(100, sys.battery))
	if pct > 0 then
		box(sbar.a, bx + 3, by + 3, math.max(2, (bw - 6) * pct / 100), bh - 6,
			pct <= 15 and {255, 110, 90} or WHITE, "rrect", 1.0, 2)
	end
	box(sbar.a, bx + bw + 1, by + 4, 2, bh - 8, WHITE, "rrect", 0.9, 1)
	if sys.battery >= 0 then
		text_r(sbar.a, bx - 7, 10, string.format("%d", sys.battery), 10, C.text,
			F_SB)
	end
	if sys.charger == "charging" then
		text_r(sbar.a, bx - 28, 8, "⚡", 10, C.text, F_SYM)
	end
end

------------------------------------------------------------------------
-- banners and events

local banner = {}

local function show_banner(kind, title, body)
	if valid_vid(banner.v) then
		delete_image(banner.v)
	end
	local v = anchor(nil, 14, -100, W - 28, 76, O_BANNER)
	box(v, -6, 0, W - 16, 88, {0, 0, 0}, "shadow", 0.45, 1)
	glass(v, 0, 0, W - 28, 76, "glass_dark", 2)
	box(v, 14, 16, 44, 44, C.disc, "gloss", 1.0, 3)
	icons[kind] (v, 36, 38, 22, 4)
	text(v, 72, 16, title, 13, C.text, F_SB, 4)
	text(v, 72, 42, body, 11, C.text2, F_REG, 4)
	move_image(v, 14, SB + 6, 8, INTERP_EXPOUT)
	banner.v = v
	banner.hide_at = CLOCK + 100
end

add_event = function(kind, title, body, quiet)
	if not icons[kind] then
		kind = "bell"
	end
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
-- the apps' faces: a round glass button with a line icon

local ICON = 62

local function draw_icon(parent, x, y, app, size, with_label, kind)
	size = size or ICON
	local cell = anchor(parent, x, y, size, size, 1)
	glass(cell, 0, 0, size, size, kind or "glass_grey", 1)
	icons[app.glyph](cell, size / 2, size / 2, size * 0.42, 3)
	if with_label then
		text_c(cell, size / 2, size + 8, app.name, 10, C.text, F_MED, 3)
	end
	return cell
end

-- a grid of app buttons; returns hit rects
local function grid(parent, list, y0, cols, labels)
	local rects = {}
	local cw = (W - 32) / cols
	for i, app in ipairs(list) do
		local c = (i - 1) % cols
		local r = math.floor((i - 1) / cols)
		local x = math.floor(16 + c * cw + (cw - ICON) / 2)
		local y = y0 + r * (ICON + 46)
		draw_icon(parent, x, y, app, ICON, labels)
		table.insert(rects, {x = x - 12, y = y - 8, w = ICON + 24,
			h = ICON + 36, app = app, ix = x, iy = y})
	end
	return rects
end

------------------------------------------------------------------------
-- home: ValOS's widget cards

local function card_header(p, x, y, w, icon, title)
	icons[icon](p, x + 28, y + 26, 22, 5)
	text(p, x + 50, y + 17, title, 13, C.text, F_SB, 5)
	icons.arrow(p, x + w - 30, y + 26, 20, 5)
end


local function draw_home(p)
	clear_children(p.a)
	p.hits = {}
	local x0, cw = 22, W - 44
	local gap = 24

	-- at a glance
	local y = SB + 18
	local gh = 168
	box(p.a, x0 - 8, y + 4, cw + 16, gh + 14, {0, 0, 0}, "shadow", 0.35, 1)
	glass(p.a, x0, y, cw, gh, "glass", 2)
	eyebrow(p.a, x0 + 24, y + 24, "At a glance")
	local g1, g2 = greeting()
	text(p.a, x0 + 24, y + 50, g1, 25, C.text, F_SB)
	text(p.a, x0 + 24, y + 86, g2, 25, "9aa6b8", F_SB)
	text_c(p.a, x0 + cw * 0.52, y + 58, clock_str(), 24, C.text, F_SB)
	text_c(p.a, x0 + cw * 0.52, y + 94, short_date(), 10, C.text2, F_MED)
	local bx = x0 + cw * 0.75
	text_c(p.a, bx, y + 56, sys.battery >= 0 and sys.battery .. "%" or "—",
		22, C.text, F_SB)
	text_c(p.a, bx, y + 90, "Battery", 10, C.text2, F_MED)
	text_c(p.a, bx, y + 107, charge_state(), 10, C.text3, F_REG)
	glass(p.a, x0 + cw - 62, y + 58, 44, 44, "glass_grey", 3)
	icons.pencil(p.a, x0 + cw - 40, y + 80, 22, 5)
	table.insert(p.hits, {x = x0 + cw - 70, y = y + 46, w = 66, h = 68,
		app = app_by_id.settings, ix = x0 + cw - 62, iy = y + 58})
	table.insert(p.hits, {x = x0, y = y, w = cw, h = gh, qs = true})

	-- Files: a search row, folder chips, the root
	y = y + gh + gap
	local fh = 228
	box(p.a, x0 - 8, y + 4, cw + 16, fh + 14, {0, 0, 0}, "shadow", 0.35, 1)
	glass(p.a, x0, y, cw, fh, "glass", 2)
	card_header(p.a, x0, y + 4, cw, "folder", "Files")
	table.insert(p.hits, {x = x0, y = y, w = cw, h = 56,
		app = app_by_id.files, ix = x0, iy = y})
	inset(p.a, x0 + 16, y + 62, cw - 32, 56, 3, "rrect", 0.6)
	icons.search(p.a, x0 + 44, y + 90, 24, 5)
	text(p.a, x0 + 68, y + 81, "Search folders and files", 12, C.text2, F_REG)
	icons.arrow(p.a, x0 + cw - 44, y + 90, 20, 5)
	table.insert(p.hits, {x = x0 + 16, y = y + 62, w = cw - 32, h = 56,
		app = app_by_id.files, ix = x0 + 16, iy = y + 62})
	local chips = {"/", "etc", "tmp", "usr"}
	local cxp = x0 + 16
	for _, c in ipairs(chips) do
		local w = measure(c, 11, F_MED) + 56
		inset(p.a, cxp, y + 132, w, 42, 3, "pill", 0.55)
		icons.folder(p.a, cxp + 23, y + 153, 17, 5)
		text(p.a, cxp + 37, y + 144, c, 11, C.text, F_MED)
		table.insert(p.hits, {x = cxp, y = y + 132, w = w, h = 42,
			path = c == "/" and "/" or "/" .. c})
		cxp = cxp + w + 10
	end
	icons.pin(p.a, x0 + 30, y + 202, 20, 5)
	text(p.a, x0 + 50, y + 193, "/  ·  the root file system", 11, C.text2,
		F_MED)
	icons.arrow(p.a, x0 + cw - 32, y + 202, 20, 5)
	table.insert(p.hits, {x = x0, y = y + 180, w = cw, h = 48, path = "/"})

	-- the recent app, or the terminal
	y = y + fh + gap
	local rh = math.max(150, H - 196 - gap - y)
	box(p.a, x0 - 8, y + 4, cw + 16, rh + 14, {0, 0, 0}, "shadow", 0.35, 1)
	glass(p.a, x0, y, cw, rh, "glass", 2)
	local recent = run_order[1] and running[run_order[1]]
	card_header(p.a, x0, y + 4, cw, recent and "recents" or "terminal",
		recent and recent.app.name or "Terminal")
	local px, py, pw, ph = x0 + 16, y + 60, cw - 32, rh - 76
	local src = recent and (recent.rt or recent.vid)
	if src and valid_vid(src) then
		local t = null_surface(pw, ph)
		image_sharestorage(src, t)
		-- the top of the app, at its own aspect
		local crop = math.min(1, ph / pw * W / (H - SB))
		image_set_txcos(t, {0, 0, 1, 0, 1, crop, 0, crop})
		shader_uniform(shaders.rcrop, "crop", "f", crop)
		image_shader(t, shaders.rcrop)
		place(t, p.a, px, py, 3)
		blend_image(t, 0.996)
		force_image_blend(t, BLEND_NORMAL)
		table.insert(p.hits, {x = px, y = py, w = pw, h = ph,
			open = recent.app.id})
	else
		inset(p.a, px, py, pw, ph, 3, "rrect", 0.6)
		text(p.a, px + 22, py + 20, "A quiet start.", 13, C.text, F_SB)
		text(p.a, px + 22, py + 46, "sh on DragonFly BSD", 10, C.text3,
			F_MONO)
		table.insert(p.hits, {x = px, y = py, w = pw, h = ph,
			app = app_by_id.term, ix = px, iy = py})
	end
	glass(p.a, px + pw - 74, py + ph - 74, 56, 56, "glass_grey", 5)
	icons.play(p.a, px + pw - 46, py + ph - 46, 30, 6)

	-- the search pill and the avatar
	local sy = H - 178
	local sw = 236
	local sx = math.floor((W - sw - 70) / 2)
	box(p.a, sx - 6, sy + 2, sw + 12, 66, {0, 0, 0}, "shadow", 0.3, 1)
	glass(p.a, sx, sy, sw, 56, "glass_pill", 2)
	icons.search(p.a, sx + 32, sy + 28, 24, 5)
	text(p.a, sx + 54, sy + 18, "Search DragonFly", 13, C.text, F_SB)
	table.insert(p.hits, {x = sx, y = sy, w = sw, h = 56, drawer = true})
	local ax = sx + sw + 14
	box(p.a, ax, sy, 56, 56, accent(), "orb", 1.0, 3)
	table.insert(p.hits, {x = ax, y = sy, w = 56, h = 56, qs = true})

	-- the dock
	local dy = H - 108
	box(p.a, 12, dy - 2, W - 24, 92, {0, 0, 0}, "shadow", 0.35, 1)
	glass(p.a, 18, dy, W - 36, 80, "glass_pill", 2)
	local items = {
		{app = app_by_id.term}, {app = app_by_id.files},
		{app = app_by_id.settings}, {glyph = "grid", drawer = true},
		{glyph = "recents", recents = true},
	}
	local step = (W - 36 - 32) / #items
	for i, it in ipairs(items) do
		local cx = math.floor(34 + (i - 0.5) * step)
		local by = dy + 12
		glass(p.a, cx - 28, by, 56, 56, "glass_grey", 3)
		icons[it.app and it.app.glyph or it.glyph](p.a, cx, by + 28, 27, 5)
		if it.app and running[it.app.id] then
			box(p.a, cx - 2, by + 60, 4, 4, peach(), "circle", 1.0, 5)
		end
		table.insert(p.hits, {x = cx - 32, y = dy, w = 64, h = 80,
			app = it.app, drawer = it.drawer, recents = it.recents,
			ix = cx - 28, iy = by})
	end
	-- the home indicator
	box(p.a, W / 2 - 60, H - 14, 120, 5, WHITE, "pill", 0.85, 5)
end

local function view_title(p, eb, str)
	eyebrow(p, 26, SB + 26, eb)
	text(p, 24, SB + 42, str, 26, C.text, F_SB)
end

local function draw_events(p)
	clear_children(p.a)
	p.hits = {}
	box(p.a, 0, 0, W, H, {0x14, 0x0d, 0x0a}, nil, 0.35, 1)
	view_title(p.a, "DragonFly / your day", "Events")
	if #events == 0 then
		text_c(p.a, W / 2, H / 2 - 20, "Nothing new", 14, C.text2, F_MED)
		return
	end
	glass(p.a, W - 112, SB + 42, 88, 38, "glass_pill", 2)
	text_c(p.a, W - 68, SB + 52, "Clear", 11, C.text, F_SB)
	table.insert(p.hits, {x = W - 122, y = SB + 30, w = 108, h = 60,
		clear = true})
	local y = SB + 104
	for _, e in ipairs(events) do
		if y > H - 100 then
			break
		end
		glass(p.a, 16, y, W - 32, 82, "glass", 2)
		box(p.a, 32, y + 19, 44, 44, C.disc, "gloss", 1.0, 3)
		icons[e.kind](p.a, 54, y + 41, 22, 5)
		text(p.a, 92, y + 18, e.title, 13, C.text, F_SB)
		text(p.a, 92, y + 45, e.body, 11, C.text2, F_REG)
		text_r(p.a, W - 34, y + 19, e.time, 10, C.text3, F_MED)
		y = y + 92
	end
end

local function draw_switcher(p)
	clear_children(p.a)
	p.hits = {}
	box(p.a, 0, 0, W, H, {0x14, 0x0d, 0x0a}, nil, 0.35, 1)
	view_title(p.a, "Running now", "Open apps")
	if #run_order == 0 then
		text_c(p.a, W / 2, H / 2 - 30, "No open apps", 14, C.text2, F_MED)
		text_c(p.a, W / 2, H / 2, "Apps you leave keep running here", 11,
			C.text3, F_REG)
		return
	end
	local ch = math.floor((H - SB - 104 - 2 * 66 - 20) / 2)
	local cw = math.floor(ch * W / (H - SB))
	for i, id in ipairs(run_order) do
		if i > 4 then
			break
		end
		local inst = running[id]
		local c = (i - 1) % 2
		local r = math.floor((i - 1) / 2)
		local x = math.floor((W - 2 * cw - 20) / 2) + c * (cw + 20)
		local y = SB + 104 + r * (ch + 66)
		glass(p.a, x, y, 30, 30, "glass_grey", 2)
		icons[inst.app.glyph](p.a, x + 15, y + 15, 14, 4)
		text(p.a, x + 38, y + 6, inst.app.name, 12, C.text, F_SB)
		glass(p.a, x + cw - 30, y, 30, 30, "glass_grey", 2)
		icons.close(p.a, x + cw - 15, y + 15, 12, 4)
		table.insert(p.hits, {x = x + cw - 46, y = y - 12, w = 54, h = 50,
			close = id})
		local src = inst.rt or inst.vid
		local ty = y + 40
		box(p.a, x - 8, ty - 4, cw + 16, ch + 18, {0, 0, 0}, "shadow", 0.4, 1)
		if src and valid_vid(src) then
			local t = null_surface(cw, ch)
			image_sharestorage(src, t)
			image_shader(t, shaders.rthumb)
			place(t, p.a, x, ty, 3)
			blend_image(t, 0.996)
			force_image_blend(t, BLEND_NORMAL)
		else
			glass(p.a, x, ty, cw, ch, "glass", 3)
		end
		table.insert(p.hits, {x = x, y = ty, w = cw, h = ch, open = id,
			ix = x, iy = ty})
	end
end

local pages_dirty = false

local function redraw_pages_now()
	pages_dirty = false
	if pages[1] then
		draw_events(pages[1])
		draw_home(pages[2])
		draw_switcher(pages[3])
	end
end

redraw_pages = function()
	pages_dirty = true
end

set_page = function(n, t)
	page = math.max(1, math.min(3, n))
	instant_image_transform(strip)
	move_image(strip, -(page - 1) * W, 0, t or 10, INTERP_EXPOUT)
end

------------------------------------------------------------------------
-- all apps: a glass sheet from below

local function draw_drawer()
	clear_children(drawer.a)
	glass(drawer.a, 10, SB + 10, W - 20, H - SB - 10, "glass_sheet", 1)
	box(drawer.a, W / 2 - 22, SB + 22, 44, 5, WHITE, "pill", 0.4, 2)
	eyebrow(drawer.a, 34, SB + 46, "DragonFly / everything")
	text(drawer.a, 32, SB + 62, "Apps", 26, C.text, F_SB)
	text_r(drawer.a, W - 34, SB + 74, #apps .. " installed", 11, C.text3,
		F_MED)
	drawer.hits = grid(drawer.a, apps, SB + 132, 4, true)
end

drawer_show = function(open, t)
	drawer.open = open
	instant_image_transform(drawer.a)
	move_image(drawer.a, 0, open and 0 or H, t or 10, INTERP_EXPOUT)
end

------------------------------------------------------------------------
-- widgets: tiles, sliders, rows

local function bl_to_frac(v)
	return math.max(0, math.min(1, (v - 30) / (4095 - 30)))
end

local function frac_to_bl(f)
	return math.floor(30 + f * (4095 - 30))
end

-- a ValOS slider: thin track, peach fill, a round knob
local function slider(parent, x, y, w, frac, order)
	local s = {x = x, y = y - 16, w = w, h = 40}
	box(parent, x, y, w, 6, WHITE, "pill", 0.18, order)
	s.fill = box(parent, x, y, math.max(6, w * frac), 6, peach(), "pill",
		1.0, (order or 1) + 1)
	s.knob = box(parent, x + w * frac - 10, y - 7, 20, 20, peach(),
		"circle", 1.0, (order or 1) + 2)
	return s
end

local function slider_set(s, x)
	local f = math.max(0, math.min(1, (x - s.x) / s.w))
	resize_image(s.fill, math.max(6, s.w * f), 6)
	move_image(s.knob, math.floor(s.x + s.w * f - 10), s.y + 9)
	return f
end

local function switch(parent, x, y, on, order)
	box(parent, x, y, 50, 30, on and accent() or WHITE, "pill",
		on and 1.0 or 0.18, order)
	box(parent, on and x + 23 or x + 3, y + 3, 24, 24, on and C.gloss or
		WHITE, on and "gloss" or "circle", 1.0, (order or 1) + 1)
end

-- a ValOS tile: an icon, a title, a state
local function tile(parent, x, y, w, h, icon, title, state, on, order)
	if on then
		box(parent, x, y, w, h, accent(), "rrect", 1.0, order)
	else
		inset(parent, x, y, w, h, order, "rrect", 0.5)
	end
	IC = on and {0x0b, 0x0d, 0x11} or WHITE
	icons[icon](parent, x + 28, y + h / 2, 22, (order or 1) + 2)
	IC = WHITE
	text(parent, x + 54, y + h / 2 - 18, title, 13, on and INK or C.text, F_SB,
		(order or 1) + 2)
	text(parent, x + 54, y + h / 2 + 3, state, 10, on and "3a3f48" or C.text2,
		F_REG, (order or 1) + 2)
end

-- an action button: icon and label, centred
local function action(parent, x, y, w, h, icon, label, order)
	inset(parent, x, y, w, h, order, "rrect", 0.5)
	local tw = measure(label, 11, F_SB)
	local sx = x + (w - tw - 28) / 2
	icons[icon](parent, sx + 9, y + h / 2, 17, (order or 1) + 2)
	text(parent, sx + 28, y + h / 2 - 8, label, 11, C.text, F_SB,
		(order or 1) + 2)
end

-- grouped rows inside glass
local function group(parent, y, rows, rh)
	rh = rh or 64
	inset(parent, 16, y, W - 32, rh * #rows, 2, "rrect", 0.42)
	for i, r in ipairs(rows) do
		local ry = y + (i - 1) * rh
		if i > 1 then
			box(parent, 32, ry, W - 64, 1, WHITE, nil, 0.08, 3)
		end
		if r.sub then
			text(parent, 34, ry + 11, r.label, 13, C.text, F_MED)
			text(parent, 34, ry + 32, r.sub, 10, C.text3, F_REG)
		else
			text(parent, 34, ry + (rh - 18) / 2, r.label, 13, C.text, F_MED)
		end
		if r.value then
			text_r(parent, W - 36, ry + (rh - 16) / 2, r.value, 12, C.text2,
				F_REG)
		end
		if r.right then
			r.right(parent, ry, rh)
		end
	end
	return y + rh * #rows
end

local function section(parent, y, title)
	eyebrow(parent, 24, y, title)
	return y + 26
end

------------------------------------------------------------------------
-- quick settings: the ValOS sheet, with tabs

local QS_Y, QS_H = SB + 6, 0

local rebuild_wallpaper

local function apply_scene(i)
	scene_i = i
	if store_key then
		pcall(store_key, "phone_scene", tostring(i))
	end
	rebuild_wallpaper()
	redraw_all()
end

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

local function mem_info()
	local mem = tonumber(run("sysctl -n hw.physmem")) or 0
	local pg = tonumber(run("sysctl -n hw.pagesize")) or 4096
	local free = tonumber(run("sysctl -n vm.stats.vm.v_free_count")) or 0
	return mem, free * pg
end

draw_qs = function()
	clear_children(qs.a)
	qs.hits = {}
	local x0, w = 14, W - 28
	box(qs.a, x0 - 8, -6, w + 16, QS_H + 26, {0, 0, 0}, "shadow", 0.5, 1)
	glass(qs.a, x0, 0, w, QS_H, "glass_sheet", 2)
	eyebrow(qs.a, x0 + 24, 24, "DragonFly / your day")
	text(qs.a, x0 + 22, 40, "Quick settings", 25, C.text, F_SB)
	inset(qs.a, x0 + w - 62, 26, 42, 42, 3, "circle", 0.6)
	icons.close(qs.a, x0 + w - 41, 47, 16, 5)
	table.insert(qs.hits, {x = x0 + w - 74, y = 14, w = 66, h = 66,
		fn = function()
			qs_show(false)
		end})

	-- tabs
	local ty = 92
	inset(qs.a, x0 + 14, ty, w - 28, 48, 3, "rrect", 0.5)
	local tabs = {"Overview", "Notifications", "Controls"}
	local tw = (w - 36) / 3
	for i, t in ipairs(tabs) do
		local tx = x0 + 18 + (i - 1) * tw
		if qs.tab == i then
			box(qs.a, tx, ty + 4, tw, 40, {0x1a, 0x11, 0x0e}, "rrect", 0.8, 4)
		end
		local lw = measure(t, 11, F_MED)
		local n = i == 2 and #events or 0
		local lx = tx + (tw - lw - (n > 0 and 14 or 0)) / 2
		text(qs.a, lx, ty + 16, t, 11, C.text, F_MED, 6)
		if n > 0 then
			text(qs.a, lx + lw + 6, ty + 14, tostring(n), 8, C.text2, F_SB, 6)
		end
		table.insert(qs.hits, {x = tx, y = ty, w = tw, h = 48, fn = function()
			qs.tab = i
		end})
	end

	local y = ty + 66
	if qs.tab == 3 then
		-- tiles
		local cw2 = math.floor((w - 40) / 2)
		local tiles = {
			{"torch", "Torch", sys.led == 0xffffff, function()
				sys.led = sys.led == 0xffffff and 0 or 0xffffff
				sysctl_set("hw.mi5.led", sys.led)
			end},
			{"bulb", "Light", sys.led ~= 0 and sys.led ~= 0xffffff, function()
				sys.led = (sys.led ~= 0 and sys.led ~= 0xffffff) and 0 or 0x00ff40
				sysctl_set("hw.mi5.led", sys.led)
			end},
			{"vibrate", "Vibrate", false, function()
				sysctl_set("hw.mi5.vibrate", 250)
			end, "Tap to test"},
			{"lock", "Lock", false, function()
				qs_show(false)
				lock_screen(true)
			end, "Screen"},
		}
		for i, t in ipairs(tiles) do
			local tx = x0 + 14 + ((i - 1) % 2) * (cw2 + 12)
			local tyy = y + math.floor((i - 1) / 2) * 86
			tile(qs.a, tx, tyy, cw2, 72, t[1], t[2], t[5] or (t[3] and "On" or
				"Off"), t[3], 3)
			table.insert(qs.hits, {x = tx, y = tyy, w = cw2, h = 72, fn = t[4]})
		end
		y = y + 190
		-- brightness
		icons.sun(qs.a, x0 + 26, y + 8, 16, 5)
		text(qs.a, x0 + 44, y, "Brightness", 12, C.text, F_MED)
		qs.pct = text_r(qs.a, x0 + w - 18, y, string.format("%d%%",
			math.floor(bl_to_frac(sys.backlight) * 100 + 0.5)), 11, C.text2,
			F_MED)
		qs.slider = slider(qs.a, x0 + 22, y + 36, w - 44,
			bl_to_frac(sys.backlight), 4)
		y = y + 76
		-- actions
		local acts = {
			{"lock", "Lock now", function()
				qs_show(false)
				lock_screen(true)
			end},
			{"gear", "Settings", function()
				qs_show(false)
				lock_screen(false)
				open_app("settings")
			end},
			{"restart", "Restart", function()
				add_event("restart", "Restarting", "Going down now")
				run("reboot &")
			end},
			{"power", "Power off", function()
				add_event("power", "Powering off", "Going down now")
				run("reboot -p &")
			end},
		}
		for i, a in ipairs(acts) do
			local ax = x0 + 14 + ((i - 1) % 2) * (cw2 + 12)
			local ay = y + math.floor((i - 1) / 2) * 64
			action(qs.a, ax, ay, cw2, 52, a[1], a[2], 3)
			table.insert(qs.hits, {x = ax, y = ay, w = cw2, h = 52, fn = a[3]})
		end
		y = y + 140
		box(qs.a, x0 + 22, y, w - 44, 1, WHITE, nil, 0.10, 3)
		y = y + 18
		text(qs.a, x0 + 22, y, "Ambience", 13, C.text, F_SB)
		text_r(qs.a, x0 + w - 22, y + 2, SCENES[scene_i][1], 11, C.text2,
			F_MED)
		y = y + 30
		for i, a in ipairs(SCENES) do
			local ax = x0 + 22 + (i - 1) * 58
			if i == scene_i then
				box(qs.a, ax - 5, y - 5, 50, 50, WHITE, "lring", 1.0, 3)
			end
			swatch(qs.a, ax, y, 40, i, 4)
			table.insert(qs.hits, {x = ax - 8, y = y - 8, w = 56, h = 56,
				fn = function()
					apply_scene(i)
				end})
		end
	elseif qs.tab == 2 then
		if #events == 0 then
			text_c(qs.a, W / 2, y + 60, "No notifications", 13, C.text2, F_MED)
		end
		for i, e in ipairs(events) do
			if y > QS_H - 90 then
				break
			end
			inset(qs.a, x0 + 14, y, w - 28, 72, 3, "rrect", 0.45)
			box(qs.a, x0 + 28, y + 16, 40, 40, C.disc, "gloss", 1.0, 4)
			icons[e.kind](qs.a, x0 + 48, y + 36, 20, 6)
			text(qs.a, x0 + 82, y + 15, e.title, 12, C.text, F_SB)
			text(qs.a, x0 + 82, y + 39, e.body, 10, C.text2, F_REG)
			text_r(qs.a, x0 + w - 28, y + 16, e.time, 9, C.text3, F_MED)
			y = y + 82
		end
	else
		local mem, free = mem_info()
		local cw2 = math.floor((w - 40) / 2)
		local cards = {
			{"Battery", sys.battery >= 0 and sys.battery .. "%" or "—",
				charge_state()},
			{"Memory", string.format("%d%%", mem > 0 and
				math.floor((1 - free / mem) * 100 + 0.5) or 0),
				string.format("%d MB free", math.floor(free / 1048576))},
			{"Uptime", uptime_str(), "since boot"},
			{"Graphics", sys.gpu and "Adreno" or "Software",
				sys.gpu and "freedreno, on the GPU" or "softpipe"},
		}
		for i, c in ipairs(cards) do
			local cx = x0 + 14 + ((i - 1) % 2) * (cw2 + 12)
			local cy = y + math.floor((i - 1) / 2) * 118
			inset(qs.a, cx, cy, cw2, 106, 3, "rrect", 0.45)
			eyebrow(qs.a, cx + 18, cy + 16, c[1])
			text(qs.a, cx + 16, cy + 34, c[2], 26, C.text, F_SB)
			text(qs.a, cx + 18, cy + 76, c[3], 10, C.text2, F_REG)
		end
		y = y + 248
		text(qs.a, x0 + 22, y, "DragonFly BSD on the Xiaomi Mi 5", 11, C.text3,
			F_MED)
	end
	box(qs.a, W / 2 - 22, QS_H - 16, 44, 5, WHITE, "pill", 0.4, 4)
end

qs_show = function(open, t)
	qs.open = open
	if open then
		draw_qs()
	end
	instant_image_transform(qs.a)
	move_image(qs.a, 0, open and QS_Y or -QS_H - 40, t or 10, INTERP_EXPOUT)
end

local function qs_slide(x, final)
	local f = slider_set(qs.slider, x)
	sys.backlight = frac_to_bl(f)
	if final or not qs.last_set or CLOCK - qs.last_set > 3 then
		sysctl_set("hw.mi5.backlight", sys.backlight)
		qs.last_set = CLOCK
	end
	if final then
		draw_qs()
	end
end

------------------------------------------------------------------------
-- lock screen

local function draw_lock()
	clear_children(lock.a)
	wallpaper(lock.a, 1)
	box(lock.a, 0, 0, W, H, {0x14, 0x0c, 0x08}, nil, 0.18, 2)
	text_c(lock.a, W / 2, 140, clock_str(), 92, C.text, F_LIGHT, 3)
	text_c(lock.a, W / 2, 284, date_str(), 15, C.text, F_MED, 3)
	local y = 330
	glass(lock.a, 22, y, W - 44, 96, "glass", 3)
	eyebrow(lock.a, 44, y + 18, "At a glance", 4)
	text(lock.a, 44, y + 38, sys.battery >= 0 and sys.battery .. "%" or "—",
		22, C.text, F_SB, 4)
	text(lock.a, 44, y + 68, "Battery · " .. charge_state(), 10, C.text2,
		F_REG, 4)
	text_r(lock.a, W - 44, y + 40, sys.gpu and "Adreno 530" or "Software",
		13, C.text, F_SB, 4)
	text_r(lock.a, W - 44, y + 64, "Graphics", 10, C.text2, F_REG, 4)
	if #events > 0 then
		local e = events[1]
		y = y + 110
		glass(lock.a, 22, y, W - 44, 76, "glass_dark", 3)
		box(lock.a, 38, y + 16, 44, 44, C.disc, "gloss", 1.0, 4)
		icons[e.kind](lock.a, 60, y + 38, 22, 6)
		text(lock.a, 96, y + 16, e.title, 12, C.text, F_SB, 5)
		text(lock.a, 96, y + 41, e.body, 10, C.text2, F_REG, 5)
	end
	bar(lock.a, W / 2 - 8, H - 100, 20, 2.4, 55, WHITE, 4)
	bar(lock.a, W / 2 + 8, H - 100, 20, 2.4, -55, WHITE, 4)
	text_c(lock.a, W / 2, H - 80, "Swipe up to unlock", 12, C.text, F_SB, 4)
	box(lock.a, W / 2 - 60, H - 14, 120, 5, WHITE, "pill", 0.85, 4)
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
		move_image(lock.a, 0, -H, t or 11, INTERP_EXPOUT)
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
			edge.bars[i] = box(edge.a, 0, 0, EDGE_W[i], H, peach(), nil, al, i)
		end
	end
	local a = peach()
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

local kbd = {layer = "lower", keys = {}, ctrl = false, h = 0, key_h = 62}
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
	add(color_surface(W, kbd.h, 0x0c, 0x0e, 0x12))
	local unit = (W - 8) / 10
	local a = {0x5c, 0x63, 0x70}
	for ri, r in ipairs(layers[kbd.layer]) do
		local x = 4 + r.pad * unit
		for _, k in ipairs(r) do
			local w = k[3] * unit
			local yy = 8 + (ri - 1) * kh
			local special = type(k[2]) == "table" or #k[2] ~= 1 or
				k[2] == BKSP or k[2] == ENTER or k[2] == TAB or k[2] == ESC
			local on = (k[2] == SHIFT and kbd.layer == "upper") or
				(k[2] == SYM and kbd.layer == "sym") or
				(k[2] == CTRL and kbd.ctrl)
			local col = (on or k[2] == ENTER) and a or
				(special and {0x1e, 0x21, 0x27} or {0x34, 0x38, 0x40})
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
					sym and F_SYM or F_MED, px(#k[1] > 1 and not sym and 12 or 18)),
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
-- app chrome: a ValOS header - small capitals, a big title, round glass
-- buttons for minimise and close

local function header(inst, eb, title, back)
	local tx = back and 72 or 24
	if back then
		glass(inst.a, 18, 38, 44, 44, "glass_grey", 20)
		icons.back(inst.a, 40, 60, 18, 22)
	end
	eyebrow(inst.a, tx + 2, 26, eb, 22)
	text(inst.a, tx, 44, title, 26, C.text, F_SB, 22)
	for i = 1, 2 do
		local cx = W - 42 - (2 - i) * 56
		glass(inst.a, cx - 22, 38, 44, 44, "glass_grey", 20)
		icons[i == 1 and "minus" or "close"](inst.a, cx, 60, 16, 22)
	end
	inst.back = back
end

local function app_bg(inst)
	glass(inst.a, 0, 0, W, inst.ah, "glass_flat", 1)
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
			clip(box(inst.content, r.icon and 74 or 22, y, W - 96, 1, WHITE,
				nil, 0.07, 1))
		end
		if r.icon then
			local iy = y + row_h / 2
			clip(box(inst.content, 22, iy - 19, 38, 38, r.dir and
				{0x4a, 0x55, 0x66} or C.disc, "gloss", 1.0, 2))
			local before = image_children(inst.content) or {}
			local seen = {}
			for _, v in ipairs(before) do
				seen[v] = true
			end
			icons[r.dir and "folder" or "file"](inst.content, 41, iy, 18, 3)
			for _, v in ipairs(image_children(inst.content) or {}) do
				if not seen[v] then
					clip(v)
				end
			end
		end
		clip((text(inst.content, r.icon and 74 or 24, y + (row_h - 18) / 2,
			r.text, r.size or 13, r.col or C.text, r.font or F_MED, 3)))
		if r.right then
			clip((text_r(inst.content, W - 26, y + (row_h - 16) / 2, r.right, 12,
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
	direct = true}

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
	kbd.h = kbd.key_h * 5 + 14
	app_bg(inst)
	header(inst, "sh · DragonFly BSD", "Terminal", false)
	inst.th = inst.ah - kbd.h - HB - 16
	box(inst.a, 12, HB, W - 24, inst.th + 8, {0x0e, 0x09, 0x07}, "rrect", 0.85,
		2)
	kbd_build(inst.a, inst.ah - kbd.h)
	inst.vid = launch_avfeed("", "terminal", function(source, status)
		if status.kind == "preroll" or status.kind == "registered" then
			target_fonthint(source, F_MONO, 3.6 * S, 2)
			target_displayhint(source, W - 40, inst.th - 8, 0)
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
		move_image(inst.vid, 20, HB + 8)
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
local files_app = {id = "files", name = "Files", glyph = "folder"}
local files_view, files_show

files_show = function(inst, path)
	clear_children(inst.a)
	inst.view = nil
	app_bg(inst)
	inst.path = path
	inst.viewing = nil
	header(inst, "Files", path == "/" and "Root" or
		(string.match(path, "[^/]+$") or path), path ~= "/")
	inset(inst.a, 16, HB + 4, W - 32, 46, 3, "pill", 0.55)
	icons.pin(inst.a, 40, HB + 27, 16, 5)
	text(inst.a, 58, HB + 17, path, 11, C.text2, F_MED)
	local rows = {}
	local out = run("ls -pa " .. sh_quote(path))
	for name in string.gmatch(out, "[^\n]+") do
		if name ~= "./" and name ~= "../" then
			local dir = string.sub(name, -1) == "/"
			table.insert(rows, {text = dir and string.sub(name, 1, -2) or name,
				icon = true, dir = dir, right = dir and "›" or nil})
		end
	end
	if #rows == 0 then
		table.insert(rows, {text = "Empty, or no permission", col = C.text3})
	end
	list_view(inst, HB + 60, rows, 58, function(r)
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
	header(inst, "Files · " .. (string.match(file, "^(.*)/") or "/"),
		string.match(file, "[^/]+$") or file, true)
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
			if #line > 60 then
				line = string.sub(line, 1, 59) .. "…"
			end
			table.insert(rows, {text = line, size = 10, font = F_MONO,
				nosep = true, col = C.text2})
			if #rows > 400 then
				break
			end
		end
	end
	inset(inst.a, 12, HB, W - 24, inst.ah - HB - 12, 2, "rrect", 0.5)
	list_view(inst, HB + 10, rows, 22, nil)
end

function files_app.create(inst)
	files_show(inst, inst.start_path or "/")
end

function files_app.tap(inst, x, y)
	if y < HB and x < 70 and inst.back then
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

-- System
local system_app = {id = "system", name = "System", glyph = "chip"}

local function system_refresh(inst)
	local off = inst.list and inst.list.off or 0
	clear_children(inst.a)
	inst.view = nil
	app_bg(inst)
	header(inst, "Monitor", "System", false)
	local mem, free = mem_info()
	local used = mem > 0 and 1 - free / mem or 0
	local tw = math.floor((W - 44) / 2)
	local tiles = {
		{"Memory", string.format("%d%%", math.floor(used * 100 + 0.5)),
			string.format("%d of %d MB", math.floor((mem - free) / 1048576),
			math.floor(mem / 1048576)), used},
		{"Battery", sys.battery >= 0 and sys.battery .. "%" or "—",
			charge_state(), sys.battery >= 0 and sys.battery / 100 or nil},
		{"Uptime", uptime_str(), "since boot"},
		{"Processors", (string.gsub(run("sysctl -n hw.ncpu"), "%s", "")),
			"Kryo · arm64"},
	}
	for i, t in ipairs(tiles) do
		local x = 16 + ((i - 1) % 2) * (tw + 12)
		local y = HB + 4 + math.floor((i - 1) / 2) * 112
		inset(inst.a, x, y, tw, 100, 2, "rrect", 0.5)
		eyebrow(inst.a, x + 18, y + 16, t[1])
		text(inst.a, x + 16, y + 34, t[2], 24, C.text, F_SB)
		text(inst.a, x + 18, y + 72, t[3], 10, C.text2, F_REG)
		if t[4] then
			box(inst.a, x + tw - 66, y + 46, 48, 6, WHITE, "pill", 0.18, 3)
			box(inst.a, x + tw - 66, y + 46, math.max(6, 48 * t[4]), 6,
				peach(), "pill", 1.0, 4)
		end
	end
	local y = HB + 236
	eyebrow(inst.a, 22, y, "Processes")
	inset(inst.a, 12, y + 20, W - 24, inst.ah - y - 32, 2, "rrect", 0.5)
	local rows = {{text = "  PID    RSS  COMMAND", size = 10, font = F_MONO,
		col = C.text3, nosep = true}}
	local out = run("ps -axo pid,rss,comm")
	local first = true
	for line in string.gmatch(out, "[^\n]+") do
		if not first then
			local pid, rss, cmd = string.match(line,
				"^%s*(%d+)%s+(%d+)%s+(.*)$")
			if pid then
				table.insert(rows, {text = string.format("%5s %5dM  %s", pid,
					math.floor(tonumber(rss) / 1024), cmd), size = 10,
					font = F_MONO, nosep = true, col = C.text2})
			end
		end
		first = false
	end
	list_view(inst, y + 30, rows, 24, nil)
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

-- Settings
local settings_app = {id = "settings", name = "Settings", glyph = "gear"}

-- hw.mi5.wlan: the athp driver's progress, one line
function wifi_driver_state()
	local st = string.gsub(run("sysctl -n hw.mi5.wlan"), "\n", "")
	if st == "" then
		st = "Not in this kernel"
	end
	if #st > 44 then
		st = string.sub(st, 1, 43) .. "…"
	end
	return st
end

local function settings_draw(inst)
	clear_children(inst.a)
	inst.hits = {}
	app_bg(inst)
	header(inst, "DragonFly / this phone", "Settings", false)
	local y = HB + 6

	y = section(inst.a, y, "Ambience")
	y = group(inst.a, y, {{label = "Wallpaper", sub = SCENES[scene_i][1],
		right = function(p, ry, rh)
		for i = #SCENES, 1, -1 do
			local x = W - 36 - (#SCENES - i + 1) * 38 + 6
			if i == scene_i then
				box(p, x - 5, ry + rh / 2 - 18, 36, 36, WHITE, "lring", 1.0, 3)
			end
			swatch(p, x, ry + rh / 2 - 13, 26, i, 4)
			table.insert(inst.hits, {x = x - 6, y = ry, w = 38, h = rh,
				fn = function()
					apply_scene(i)
				end})
		end
	end}})
	y = y + 12

	y = section(inst.a, y, "Display")
	y = group(inst.a, y, {{label = "Brightness", right = function(p, ry, rh)
		inst.slider = slider(p, 160, ry + rh / 2 - 3, W - 206,
			bl_to_frac(sys.backlight), 4)
	end}})
	y = y + 12

	y = section(inst.a, y, "Notification light")
	local colours = {{0, {0x6a, 0x5c, 0x56}}, {0x00ff40, {0x5a, 0xc8, 0x7a}},
		{0x0040ff, {0x4a, 0x8a, 0xe0}}, {0xff2000, {0xe0, 0x50, 0x40}},
		{0xffa000, {0xf0, 0xa8, 0x50}}, {0xffffff, {0xf4, 0xee, 0xea}}}
	y = group(inst.a, y, {
		{label = "Colour", right = function(p, ry, rh)
			for i = #colours, 1, -1 do
				local c = colours[i]
				local x = W - 36 - (#colours - i + 1) * 38 + 6
				if sys.led == c[1] then
					box(p, x - 5, ry + rh / 2 - 18, 36, 36, WHITE, "lring", 1.0, 3)
				end
				box(p, x, ry + rh / 2 - 13, 26, 26, c[2], "circle", 1.0, 4)
				table.insert(inst.hits, {x = x - 6, y = ry, w = 38, h = rh,
					fn = function()
						sys.led = c[1]
						sysctl_set("hw.mi5.led", c[1])
						settings_draw(inst)
					end})
			end
		end},
		{label = "Torch", sub = "The light at full white",
			right = function(p, ry, rh)
				switch(p, W - 86, ry + rh / 2 - 15, sys.led == 0xffffff, 4)
				table.insert(inst.hits, {x = W - 104, y = ry, w = 88, h = rh,
					fn = function()
						sys.led = sys.led == 0xffffff and 0 or 0xffffff
						sysctl_set("hw.mi5.led", sys.led)
						settings_draw(inst)
					end})
			end},
	})
	y = y + 12

	y = section(inst.a, y, "Wi-Fi")
	local wid = tonumber(run("sysctl -n hw.mi5.wlan_id")) or 0
	local probed = (tonumber(run("sysctl -n hw.mi5.pcie_up")) or 0) ~= 0
	local last = ""
	for l in string.gmatch(run("sysctl -n hw.mi5.pcie"), "[^\n]+") do
		last = l
	end
	local state = wid == 0x168c003e and "QCA6174 found on PCIe" or
		(wid ~= 0 and string.format("PCIe device %08x", wid) or
		(probed and (last ~= "" and last or "not found") or "Not probed yet"))
	if #state > 44 then
		state = string.sub(state, 1, 43) .. "…"
	end
	y = group(inst.a, y, {{label = "Wi-Fi chip", sub = state,
		right = function(p, ry, rh)
			box(p, W - 122, ry + rh / 2 - 18, 86, 36, accent(), "pill", 1.0, 4)
			text_c(p, W - 79, ry + rh / 2 - 9, "Probe", 12, INK, F_SB, 5)
			table.insert(inst.hits, {x = W - 132, y = ry, w = 110, h = rh,
				fn = function()
					run("sysctl -q hw.mi5.pcie_up=1")
					settings_draw(inst)
				end})
		end},
		{label = "Driver", sub = wifi_driver_state(),
		right = function(p, ry, rh)
			box(p, W - 122, ry + rh / 2 - 18, 86, 36, accent(), "pill", 1.0, 4)
			text_c(p, W - 79, ry + rh / 2 - 9, "Start", 12, INK, F_SB, 5)
			table.insert(inst.hits, {x = W - 132, y = ry, w = 110, h = rh,
				fn = function()
					-- attaching takes seconds; let it run on its own
					run("sysctl -q hw.mi5.wlan_attach=1 >/dev/null 2>&1 &")
					inst.wifi_until = CLOCK + 25 * 90
					settings_draw(inst)
				end})
		end}})
	-- the driver's kernel messages, while bringing Wi-Fi up
	local log = {}
	if not string.find(wifi_driver_state(), "Not started", 1, true) then
		-- the bring-up diagnostics first, then the latest messages
		local cmd = "dmesg | grep -E 'athp diag|pcie0: (pm8994|link|chip)' | " ..
			"tail -6; dmesg | grep -E 'athp|pcie' | grep -v regio_s_ | " ..
			"grep -v 'athp diag' | tail -8"
		for l in string.gmatch(run(cmd), "[^\n]+") do
			l = string.gsub(l, "^athp diag: ", "")
			if #l > 84 then
				l = string.sub(l, 1, 83) .. "…"
			end
			table.insert(log, l)
		end
	end
	if #log > 0 then
		y = y + 8
		inset(inst.a, 16, y, W - 32, 16 + 15 * #log, 2, "rrect", 0.42)
		for i, l in ipairs(log) do
			text(inst.a, 28, y + 8 + (i - 1) * 15, l, 9, C.text3, F_MONO)
		end
		y = y + 16 + 15 * #log
	end
	y = y + 12

	y = section(inst.a, y, "Vibration")
	y = group(inst.a, y, {{label = "Test the motor",
		right = function(p, ry, rh)
			box(p, W - 122, ry + rh / 2 - 18, 86, 36, accent(), "pill", 1.0, 4)
			text_c(p, W - 79, ry + rh / 2 - 9, "Buzz", 12, INK, F_SB, 5)
			table.insert(inst.hits, {x = W - 132, y = ry, w = 110, h = rh,
				fn = function()
					sysctl_set("hw.mi5.vibrate", 300)
				end})
		end}})
	y = y + 12

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
	group(inst.a, y, inst.about, 40)
end

-- while the Wi-Fi driver starts, refresh once a second
function settings_app.tick(inst)
	if inst.wifi_until and CLOCK < inst.wifi_until and
		CLOCK >= (inst.wifi_next or 0) then
		inst.wifi_next = CLOCK + 25
		settings_draw(inst)
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
	sys.backlight = frac_to_bl(slider_set(inst.slider, x))
	if final or not inst.last_set or CLOCK - inst.last_set > 3 then
		sysctl_set("hw.mi5.backlight", sys.backlight)
		inst.last_set = CLOCK
	end
end

-- Calculator
local calc_app = {id = "calc", name = "Calculator", glyph = "calc"}

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
	inst.disp = anchor(inst.a, 0, HB, W, 180, 5)
	local e = inst.expr == "" and "0" or inst.expr
	text_r(inst.disp, W - 30, 50, e, #e > 12 and 26 or 46, C.text, F_LIGHT)
	if inst.result then
		text_r(inst.disp, W - 30, 120, "= " .. inst.result, 16, C.text2,
			F_MED)
	end
end

function calc_app.create(inst)
	inst.expr = ""
	app_bg(inst)
	header(inst, "Arithmetic", "Calculator", false)
	inset(inst.a, 12, HB, W - 24, 170, 2, "rrect", 0.45)
	inst.keys = {}
	local top = HB + 184
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
				inset(inst.a, x + 5, y + 5, kw - 10, kh - 10, 2, "rrect",
					(c == 4 or k == "C") and 0.65 or 0.42)
			end
			local sym = string.byte(k) >= 0x80
			text_c(inst.a, x + kw / 2, y + kh / 2 - 15, k, 20,
				k == "=" and INK or C.text,
				sym and F_SYM or F_REG, 4)
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

-- Clock
local clock_app = {id = "clock", name = "Clock", glyph = "clock"}

local function clock_draw(inst)
	if valid_vid(inst.face) then
		delete_image(inst.face)
	end
	local f = anchor(inst.a, 0, HB, W, inst.ah - HB, 5)
	inst.face = f
	local cx, cy, r = W / 2, 150, 126
	box(f, cx - r - 10, cy - r - 4, 2 * r + 20, 2 * r + 24, {0, 0, 0},
		"shadow", 0.4, 1)
	box(f, cx - r, cy - r, 2 * r, 2 * r, C.inset, "circle", 0.75, 2)
	for i = 0, 11 do
		local a = math.rad(i * 30)
		local len = i % 3 == 0 and 14 or 8
		bar(f, cx + math.sin(a) * (r - 18), cy - math.cos(a) * (r - 18), len,
			i % 3 == 0 and 3 or 2, i * 30, WHITE, 3, i % 3 == 0 and 1.0 or 0.55)
	end
	local t = os.date("*t")
	local function hand(deg, len, th, col)
		local a = math.rad(deg)
		bar(f, cx + math.sin(a) * len / 2, cy - math.cos(a) * len / 2, len, th,
			deg, col, 4)
	end
	hand((t.hour % 12 + t.min / 60) * 30, r * 0.52, 6, WHITE)
	hand((t.min + t.sec / 60) * 6, r * 0.78, 4, WHITE)
	hand(t.sec * 6, r * 0.86, 2, peach())
	box(f, cx - 6, cy - 6, 12, 12, peach(), "circle", 1.0, 5)

	text_c(f, W / 2, 296, os.date("%H:%M:%S"), 28, C.text, F_SB)
	text_c(f, W / 2, 338, date_str(), 12, C.text2, F_MED)

	inset(f, 12, 380, W - 24, 230, 1, "rrect", 0.45)
	eyebrow(f, 34, 398, "Stopwatch")
	local el = inst.elapsed + (inst.running and (CLOCK - inst.start) or 0)
	local cs = math.floor(el * 4)	-- CLOCK is 25 Hz: hundredths
	text_c(f, W / 2, 424, string.format("%02d:%02d.%02d",
		math.floor(cs / 6000), math.floor(cs / 100) % 60, cs % 100), 42,
		C.text, F_LIGHT)
	local bw = math.floor((W - 92) / 2)
	local labels = {inst.running and "Stop" or "Start",
		inst.running and "Lap" or "Reset"}
	for i = 1, 2 do
		local x = 34 + (i - 1) * (bw + 24)
		if i == 1 then
			box(f, x, 520, bw, 54, accent(), "pill", 1.0, 3)
		else
			inset(f, x, 520, bw, 54, 3, "pill", 0.6)
		end
		text_c(f, x + bw / 2, 536, labels[i], 14, i == 1 and INK or C.text,
			F_SB)
	end
	for i, lap in ipairs(inst.laps) do
		if i > 6 then
			break
		end
		text_c(f, W / 2, 630 + (i - 1) * 26, lap, 11, C.text2, F_MONO)
	end
end

function clock_app.create(inst)
	app_bg(inst)
	header(inst, "Time", "Clock", false)
	inst.elapsed, inst.running, inst.laps = 0, false, {}
	inst.last = ""
	clock_draw(inst)
end

function clock_app.tap(inst, x, y)
	if y >= HB + 520 and y < HB + 574 then
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

apps = {term_app, files_app, calc_app, clock_app, system_app, settings_app}
for _, a in ipairs(apps) do
	app_by_id[a.id] = a
end

------------------------------------------------------------------------
-- running apps.  A built-in app draws into its own render target: live
-- while in front, a still picture once left.  The terminal draws
-- straight into the world.

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

-- from: a rect {ix, iy} to zoom out of; opts.path for Files
open_app = function(id, from, opts)
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
		inst = {app = app, ah = ah, start_path = opts and opts.path}
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
	else
		if inst.rt then
			rendertarget_forceupdate(inst.rt, -1)	-- live again
			inst.freeze_at = nil
		end
		if opts and opts.path and id == "files" then
			in_app(inst, files_show, opts.path)
		end
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
	show_image(s)
	if inst.rt then
		if from then
			move_image(s, from.ix, from.iy)
			resize_image(s, ICON, ICON * ah / W)
		else
			move_image(s, W * 0.08, SB + ah * 0.08)
			resize_image(s, W * 0.84, ah * 0.84)
		end
		blend_image(s, 0.3)
		force_image_blend(s, BLEND_NORMAL)
		move_image(s, 0, SB, 10, INTERP_EXPOUT)
		resize_image(s, W, ah, 10, INTERP_EXPOUT)
		blend_image(s, 0.996, 7)
	else
		move_image(s, 0, SB + 50)
		blend_image(s, 0.0)
		move_image(s, 0, SB, 9, INTERP_EXPOUT)
		blend_image(s, 1.0, 7)
	end
	draw_status()
	redraw_pages()
end

leave_app = function(dir)
	if not focus then
		return
	end
	local inst = running[focus]
	focus = nil
	local s = surface(inst)
	instant_image_transform(s)
	if inst.rt then
		inst.freeze_at = CLOCK + 3
		if (dir or 0) == 0 then
			move_image(s, W * 0.2, SB + inst.ah * 0.2, 9, INTERP_EXPOUT)
			resize_image(s, W * 0.6, inst.ah * 0.6, 9, INTERP_EXPOUT)
		else
			move_image(s, dir * W, SB, 9, INTERP_EXPOUT)
		end
	else
		move_image(s, (dir or 0) * W, SB + ((dir or 0) == 0 and 60 or 0),
			6, INTERP_EXPOUT)
	end
	blend_image(s, 0.0, 8)
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
			move_image(s, W * 0.15, SB + inst.ah * 0.15, 8, INTERP_EXPIN)
			resize_image(s, W * 0.7, inst.ah * 0.7, 8, INTERP_EXPIN)
		else
			move_image(s, 0, H, 8, INTERP_EXPIN)
		end
		blend_image(s, 0.0, 8)
		expire_image(s, 9)
	end
	if inst.rt and valid_vid(inst.a) then
		expire_image(inst.a, 9)
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
				i.view = nil
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
	if qs.open and hit(qs.slider, x, y - QS_Y) and qs.tab == 3 then
		g.mode = "qs_slider"
		qs_slide(x)
		return
	end
	if locked then
		return
	end
	if not qs.open and focus and not g.edge then
		local inst = running[focus]
		if inst.app.down and inst.app.down(inst, x, y - SB) then
			g.consumed = true
		end
	end
	if not qs.open and focus and running[focus].app.slide and
		running[focus].slider and hit(running[focus].slider, x, y - SB) then
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
			g.mode = (not horiz) and "qs" or "none"
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
		move_image(qs.a, 0, qs.open and QS_Y + math.min(0, dy) or
			math.min(QS_Y, -QS_H + dy))
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
			elseif r.path then
				open_app("files", {ix = x - 30, iy = y - 30}, {path = r.path})
			elseif r.drawer then
				drawer_show(true)
			elseif r.recents then
				set_page(3)
			elseif r.qs then
				qs_show(true)
			elseif r.clear then
				events = {}
				redraw_pages()
			elseif r.close then
				close_app(r.close)
			elseif r.open then
				open_app(r.open, r.ix and r or nil)
			end
			return
		end
	end
end

local function on_tap(x, y)
	if qs.open then
		local qy = y - QS_Y
		if qy > QS_H then
			qs_show(false)
			return
		end
		for _, r in ipairs(qs.hits) do
			if hit(r, x, qy) then
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
		if ay < HB and ay > 24 and x > W - 70 then
			close_app(focus)
			return
		elseif ay < HB and ay > 24 and x > W - 126 and x <= W - 70 then
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
			move_image(lock.a, 0, 0, 8, INTERP_EXPOUT)
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
			qs_show(dy > QS_H / 5)
		end
	elseif m == "leave" then
		if math.abs(dx) > W / 4 then
			leave_app(dx > 0 and 1 or -1)
		else
			move_image(surface(running[focus]), 0, SB, 8, INTERP_EXPOUT)
		end
	elseif m == "close" then
		if dy > H / 4 then
			close_app(focus)
		else
			local inst = running[focus]
			local s = surface(inst)
			move_image(s, 0, SB, 8, INTERP_EXPOUT)
			if inst.rt then
				resize_image(s, W, inst.ah, 8, INTERP_EXPOUT)
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

-- the wallpaper shows scene_i's photograph; the glass's source is the
-- same picture at an eighth of the size, which the texture filtering and
-- the glass shader's taps blur
local wall_src

rebuild_wallpaper = function()
	local img = scene_img[scene_i]
	if not valid_vid(img) then
		return
	end
	wall_big = img
	image_sharestorage(img, wall)
	image_sharestorage(img, wall_src)
	rendertarget_forceupdate(wall_small)
end

local function build_wallpaper()
	for i, sc in ipairs(SCENES) do
		local v = load_image("wallpapers/" .. sc.file)
		if valid_vid(v) then
			hide_image(v)
			image_mask_set(v, MASK_UNPICKABLE)
			scene_img[i] = v
		end
	end
	if not valid_vid(scene_img[scene_i]) then
		scene_i = 1
	end
	wall = null_surface(W, H)
	order_image(wall, O_WALL)
	show_image(wall)
	image_mask_set(wall, MASK_UNPICKABLE)
	local sw, shh = math.floor(W / 8), math.floor(H / 8)
	wall_small = alloc_surface(sw, shh)
	wall_src = null_surface(sw, shh)
	show_image(wall_src)
	define_rendertarget(wall_small, {wall_src}, RENDERTARGET_DETACH,
		RENDERTARGET_NOSCALE, 0)
	rebuild_wallpaper()
end

function phone(args)
	S = math.max(1, math.floor(VRESW / 540 + 0.5))
	W, H = VRESW / S, VRESH / S
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
	QS_H = H - QS_Y - 40
	system_defaultfont(F_REG, 12, 1)
	build_shaders()
	if get_key then
		local ok, v = pcall(get_key, "phone_scene")
		if ok and tonumber(v) and SCENES[tonumber(v)] then
			scene_i = tonumber(v)
		end
	end
	build_wallpaper()

	strip = anchor(nil, 0, 0, W * 3, H, O_HOME)
	for i = 1, 3 do
		pages[i] = {a = anchor(strip, (i - 1) * W, 0, W, H, 1), hits = {}}
	end
	drawer.a = anchor(nil, 0, H, W, H, O_DRAWER)
	qs.a = anchor(nil, 0, -QS_H - 40, W, QS_H, O_QS)
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
	draw_drawer()
	pages_dirty = true
	add_event("bell", "Welcome to DragonFly BSD",
		"Swipe sideways for events and open apps", true)
	add_event("chip", sys.gpu and "Graphics: Adreno 530" or
		"Graphics: software", sys.gpu and "Mesa's freedreno, on the GPU" or
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
		elseif string.match(test_view, "^qs") then
			qs.tab = tonumber(string.match(test_view, "%d")) or 3
			qs_show(true, 0)
		elseif test_view == "switcher" or test_view == "recent" then
			test_steps = {
				{25, function() open_app("clock") end},
				{50, function() open_app("settings") end},
				{75, function() open_app("files") end},
				{100, function()
					leave_app(1)
					set_page(test_view == "recent" and 2 or 3, 0)
				end},
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
			show_banner("sun", "Brightness", string.format("%d%%",
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
	if pages_dirty and not g then
		redraw_pages_now()
	end
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
		move_image(banner.v, 14, -100, 8, INTERP_EXPIN)
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
