-- Smallest useful Arcan appl: a coloured box and a line of text, to check
-- that the engine renders through the fbmem platform.
function hello()
	local box = color_surface(VRESW - 40, 120, 40, 90, 160)
	move_image(box, 20, 20)
	show_image(box)
	local txt = render_text([[\ffonts/default.ttf,18\#ffffffArcan on DragonFly/arm64]])
	move_image(txt, 30, 60)
	order_image(txt, 2)
	show_image(txt)
	local info = render_text(string.format(
		[[\ffonts/default.ttf,12\#c0c0c0canvas %d x %d]], VRESW, VRESH))
	move_image(info, 30, 160)
	show_image(info)
end
