-- term: a terminal (afsrv_terminal running sh) for a phone with only
-- buttons.  The bottom bar holds a command list:
--
--   volume up / down   choose a command
--   power              type it into the terminal (with Enter)
--   home               type Ctrl-C
--
-- Any other key (a keyboard, later an on-screen one) goes to the terminal.

local cmds = {
	"uname -a",
	"ls -l /",
	"ps ax",
	"df -h",
	"sysctl hw.ncpu hw.physmem hw.machine",
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
		target_displayhint(source, VRESW, VRESH - bar_h, 0)
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

local autorun, autorun_at

-- term [autorun=COMMAND]: COMMAND is typed into the terminal shortly
-- after it starts (a demo, and a test without buttons).
function term(args)
	for _, a in ipairs(args or {}) do
		local v = string.match(a, "^autorun=(.*)")
		if v then
			autorun = v
			autorun_at = CLOCK + 100	-- about four seconds
		end
	end
	local back = color_surface(VRESW, VRESH, 0, 0, 0)
	order_image(back, 1)
	show_image(back)
	bar = color_surface(VRESW, bar_h, 32, 48, 80)
	move_image(bar, 0, VRESH - bar_h)
	order_image(bar, 10)
	show_image(bar)
	draw_bar()
	spawn()
end

function term_input(iotbl)
	if iotbl.translated and iotbl.number then
		local code = iotbl.number
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
	if autorun and CLOCK >= autorun_at and
		valid_vid(term_vid, TYPE_FRAMESERVER) then
		type_into(autorun .. "\r")
		autorun = nil
	end
end
