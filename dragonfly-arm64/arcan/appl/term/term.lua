-- term: a terminal (afsrv_terminal running sh) for a phone with only
-- buttons.  The bottom bar holds a command list:
--
--   volume up / down   choose a command
--   power              type it into the terminal (with Enter)
--   home               type Ctrl-C
--
-- Above the bar sits an on-screen keyboard for the touchscreen; tapping
-- the bar itself chooses (left / right third) or runs (middle) a command.
-- Any other key (a USB keyboard, say) goes to the terminal.

local cmds = {
	"uname -a",
	"ls -l /",
	"ps ax",
	"df -h",
	"sysctl hw.ncpu hw.physmem hw.machine",
	"dmesg | grep touch",
	"dmesg | tail -15",
	"hello | tail -4",
	"mount_ext2fs -o ro /dev/ufs0s11 /mnt && ls /mnt; umount /mnt",
	"sh /etc/h2test.sh 2>/dev/null | tail -3",
	"clear",
}

-- Linux input codes, as the kernel's evdev buttons report them
local KEY_HOME, KEY_VOLDOWN, KEY_VOLUP, KEY_POWER = 102, 114, 115, 116

local term_vid
local bar, bar_text
local sel = 1
local bar_h = 64
local kbd_h = 0

-- Touch coordinates arrive in panel pixels; the canvas is smaller.
local touch_w = 1080
local tscale = 1

local function draw_bar()
	if valid_vid(bar_text) then
		delete_image(bar_text)
	end
	bar_text = render_text(string.format(
		[[\f,14\#ffffff%d/%d  %s]], sel, #cmds, cmds[sel]))
	link_image(bar_text, bar)
	image_inherit_order(bar_text, true)
	order_image(bar_text, 1)
	move_image(bar_text, 12, 10)
	show_image(bar_text)
	local hint = render_text(
		[[\f,9\#a0a0a0vol: choose    power: run    home: ctrl-c]])
	link_image(hint, bar_text)
	image_inherit_order(hint, true)
	move_image(hint, 0, 30)
	show_image(hint)
end

local function type_into(str)
	if not valid_vid(term_vid, TYPE_FRAMESERVER) then
		return
	end
	for i = 1, #str do
		target_input(term_vid, {kind = "digital", translated = true,
			active = true, number = 0, keysym = 0, modifiers = 0,
			utf8 = string.sub(str, i, i), devid = 0, subid = 0})
	end
end

local function term_handler(source, status)
	if status.kind == "preroll" or status.kind == "registered" then
		target_fonthint(source, "default.ttf", 4.2, 2)
		target_displayhint(source, VRESW, VRESH - bar_h - kbd_h, 0)
	elseif status.kind == "resized" then
		resize_image(source, status.width, status.height)
		order_image(source, 5)
		show_image(source)
	elseif status.kind == "terminated" then
		delete_image(source)
		term_vid = nil
		spawn()
	end
end

function spawn()
	term_vid = launch_avfeed("", "terminal", term_handler)
end

-- On-screen keyboard: rows of ten units; a key is {label, sends, width}.
-- sends is a string, or one of the actions below.
local SHIFT, SYM, CTRL = {}, {}, {}
local BKSP, ENTER, TAB = "\127", "\r", "\t"

local function row(chars)
	local r = {}
	for i = 1, #chars do
		local c = string.sub(chars, i, i)
		table.insert(r, {c, c, 1})
	end
	return r
end

local function row4(mid)
	local r = row(mid)
	table.insert(r, 1, {"shift", SHIFT, 1})
	table.insert(r, {"del", BKSP, 1})
	return r
end

local layers = {
	lower = {
		row("1234567890"), row("qwertyuiop"), row("asdfghjkl-"),
		row4("zxcvbnm/"),
	},
	upper = {
		row("!@#$%^&*()"), row("QWERTYUIOP"), row("ASDFGHJKL_"),
		row4("ZXCVBNM?"),
	},
	sym = {
		row("1234567890"), row("~`|\\{}[]<>"), row("'\";:=+*&$#"),
		row4(",!?%^@()"),
	},
}
for _, l in pairs(layers) do
	table.insert(l, {{"ctrl", CTRL, 1.5}, {"sym", SYM, 1.5},
		{"space", " ", 4}, {"tab", TAB, 1}, {"enter", ENTER, 2}})
end

local kbd = {layer = "lower", keys = {}, ctrl = false, anchor = nil,
	flash = {}}
local key_h = 44

local function kbd_build()
	if valid_vid(kbd.anchor) then
		delete_image(kbd.anchor)	-- takes the linked keys with it
	end
	kbd.keys = {}
	kbd.anchor = null_surface(VRESW, kbd_h)
	move_image(kbd.anchor, 0, VRESH - bar_h - kbd_h)
	order_image(kbd.anchor, 8)
	show_image(kbd.anchor)

	local unit = VRESW / 10
	for ri, r in ipairs(layers[kbd.layer]) do
		local x = 0
		for _, k in ipairs(r) do
			local w = k[3] * unit
			local on = (k[2] == SHIFT and kbd.layer == "upper") or
				(k[2] == SYM and kbd.layer == "sym") or
				(k[2] == CTRL and kbd.ctrl)
			local bg = color_surface(w - 4, key_h - 4,
				on and 90 or 48, on and 110 or 52, on and 160 or 64)
			link_image(bg, kbd.anchor)
			image_inherit_order(bg, true)
			order_image(bg, 1)
			move_image(bg, x + 2, (ri - 1) * key_h + 2)
			show_image(bg)
			local label = k[1] == "\\" and "\\\\" or k[1]
			local txt = render_text([[\f,13\#ffffff]] .. label)
			if valid_vid(txt) then
				link_image(txt, bg)
				image_inherit_order(txt, true)
				order_image(txt, 1)
				local tw, th = image_surface_properties(txt).width,
					image_surface_properties(txt).height
				move_image(txt, (w - 4 - tw) / 2, (key_h - 4 - th) / 2)
				show_image(txt)
			end
			table.insert(kbd.keys, {x = x, y = (ri - 1) * key_h, w = w,
				k = k, vid = bg})
			x = x + w
		end
	end
end

local function kbd_press(k)
	local v = k[2]
	if v == SHIFT then
		kbd.layer = kbd.layer == "upper" and "lower" or "upper"
		kbd_build()
	elseif v == SYM then
		kbd.layer = kbd.layer == "sym" and "lower" or "sym"
		kbd_build()
	elseif v == CTRL then
		kbd.ctrl = not kbd.ctrl
		kbd_build()
	else
		if kbd.ctrl and #v == 1 and string.match(v, "%a") then
			v = string.char(string.byte(string.lower(v)) - 96)
		end
		type_into(v)
		local rebuild = kbd.ctrl or kbd.layer == "upper"
		kbd.ctrl = false
		if kbd.layer == "upper" then	-- shift is one-shot
			kbd.layer = "lower"
		end
		if rebuild then
			kbd_build()
		end
	end
end

-- A tap at canvas x, y: the keyboard, the bar, or nothing.
local function tap(x, y)
	local ky = y - (VRESH - bar_h - kbd_h)
	if ky >= 0 and ky < kbd_h then
		for _, e in ipairs(kbd.keys) do
			if x >= e.x and x < e.x + e.w and ky >= e.y and ky < e.y + key_h then
				image_color(e.vid, 140, 160, 220)
				table.insert(kbd.flash, {vid = e.vid, at = CLOCK + 4})
				kbd_press(e.k)
				return
			end
		end
	elseif y >= VRESH - bar_h then
		if x < VRESW / 3 then
			sel = sel == 1 and #cmds or sel - 1
			draw_bar()
		elseif x > VRESW * 2 / 3 then
			sel = sel == #cmds and 1 or sel + 1
			draw_bar()
		else
			type_into(cmds[sel] .. "\r")
		end
	end
end

local autorun, autorun_at

-- term [autorun=COMMAND]: COMMAND is typed into the terminal shortly
-- after it starts (a demo, and a test without buttons).
function term(args)
	local want_kbd = true
	for _, a in ipairs(args or {}) do
		local v = string.match(a, "^autorun=(.*)")
		if v then
			autorun = v
			autorun_at = CLOCK + 100	-- about four seconds
		end
		v = string.match(a, "^touchw=(%d+)")
		if v then
			touch_w = tonumber(v)
		end
		if a == "nokbd" then
			want_kbd = false
		end
	end
	tscale = VRESW / touch_w
	if want_kbd then
		kbd_h = key_h * 5
	end
	local back = color_surface(VRESW, VRESH, 0, 0, 0)
	order_image(back, 1)
	show_image(back)
	bar = color_surface(VRESW, bar_h, 32, 48, 80)
	move_image(bar, 0, VRESH - bar_h)
	order_image(bar, 10)
	show_image(bar)
	draw_bar()
	if kbd_h > 0 then
		kbd_build()
	end
	spawn()
end

-- The phone's buttons: Arcan's evdev platform files a device with a few
-- keys as a game device, so they arrive as digital events with the Linux
-- key code in subid; a keyboard's would arrive translated, code in number.
local function button_code(iotbl)
	if iotbl.translated then
		return iotbl.number
	elseif iotbl.digital then
		return iotbl.subid
	end
end

local touching = {}

function term_input(iotbl)
	if iotbl.kind == "status" then	-- devices coming and going
		return
	end
	if iotbl.touch then
		-- act on the first contact of each finger, not on its motion
		local id = iotbl.subid
		if iotbl.active and not touching[id] then
			touching[id] = true
			tap(iotbl.x * tscale, iotbl.y * tscale)
		elseif not iotbl.active then
			touching[id] = nil
		end
		return
	end
	local code = button_code(iotbl)
	if code then
		if code == KEY_VOLUP or code == KEY_VOLDOWN or
			code == KEY_POWER or code == KEY_HOME then
			if not iotbl.active then
				return
			end
			if code == KEY_VOLUP then
				sel = sel == 1 and #cmds or sel - 1
				draw_bar()
			elseif code == KEY_VOLDOWN then
				sel = sel == #cmds and 1 or sel + 1
				draw_bar()
			elseif code == KEY_POWER then
				type_into(cmds[sel] .. "\r")
			else
				type_into("\003")
			end
			return
		end
	end
	if valid_vid(term_vid, TYPE_FRAMESERVER) then
		target_input(term_vid, iotbl)
	end
end

function term_clock_pulse()
	for i = #kbd.flash, 1, -1 do
		local f = kbd.flash[i]
		if CLOCK >= f.at then
			if valid_vid(f.vid) then
				image_color(f.vid, 48, 52, 64)
			end
			table.remove(kbd.flash, i)
		end
	end
	if autorun and CLOCK >= autorun_at and
		valid_vid(term_vid, TYPE_FRAMESERVER) then
		type_into(autorun .. "\r")
		autorun = nil
	end
end
