extends CanvasLayer
## Minimal, code-built M0a debug HUD.
##
## Shows renderer counters and build statistics, drives the construction-front
## uniform, and switches between the showcase and the 200k stress build.

signal build_front_changed(value: float)
signal playing_changed(playing: bool)
signal mode_requested(mode: String)

const REFRESH_INTERVAL := 0.25
const PLAY_RATE := 0.12

var _fps_label: Label
var _render_label: Label
var _scene_label: Label
var _hash_label: Label
var _front_label: Label
var _mode_label: Label
var _warning_label: Label
var _error_label: Label
var _slider: HSlider
var _play_button: Button
var _showcase_button: Button
var _stress_button: Button

var _playing := false
var _refresh_timer := 0.0


func _ready() -> void:
	layer = 100
	_build_ui()


func _process(delta: float) -> void:
	if _playing:
		var next_value := _slider.value + delta * PLAY_RATE
		if next_value >= 1.0:
			next_value = fposmod(next_value, 1.0)
		_slider.value = next_value

	_refresh_timer += delta
	if _refresh_timer >= REFRESH_INTERVAL:
		_refresh_timer = 0.0
		_refresh_render_stats()


func _build_ui() -> void:
	var root := Control.new()
	root.name = "HudRoot"
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	var panel := PanelContainer.new()
	panel.name = "StatsPanel"
	panel.position = Vector2(14.0, 14.0)
	panel.mouse_filter = Control.MOUSE_FILTER_STOP
	root.add_child(panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 14)
	margin.add_theme_constant_override("margin_right", 14)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 12)
	panel.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 4)
	margin.add_child(column)

	var title := Label.new()
	title.text = "KEEPWRIGHT  ·  M0a"
	title.add_theme_font_size_override("font_size", 17)
	title.add_theme_color_override("font_color", Color(0.96, 0.89, 0.72))
	column.add_child(title)

	_mode_label = Label.new()
	_mode_label.text = "M0a Showcase"
	_mode_label.add_theme_font_size_override("font_size", 12)
	_mode_label.add_theme_color_override("font_color", Color(0.72, 0.74, 0.66))
	column.add_child(_mode_label)

	column.add_child(HSeparator.new())

	_fps_label = _make_stat_label("FPS —")
	_render_label = _make_stat_label("Draw calls —   Primitives —   Triangles —")
	_scene_label = _make_stat_label("Blocks —   Chunks —   Tris —")
	_hash_label = _make_stat_label("Layout hash —")
	column.add_child(_fps_label)
	column.add_child(_render_label)
	column.add_child(_scene_label)
	column.add_child(_hash_label)

	column.add_child(HSeparator.new())

	var mode_row := HBoxContainer.new()
	mode_row.add_theme_constant_override("separation", 6)
	column.add_child(mode_row)

	_showcase_button = Button.new()
	_showcase_button.text = "M0a Showcase"
	_showcase_button.focus_mode = Control.FOCUS_NONE
	_showcase_button.pressed.connect(_on_showcase_pressed)
	mode_row.add_child(_showcase_button)

	_stress_button = Button.new()
	_stress_button.text = "200k Stress Test"
	_stress_button.focus_mode = Control.FOCUS_NONE
	_stress_button.pressed.connect(_on_stress_pressed)
	mode_row.add_child(_stress_button)

	var front_row := HBoxContainer.new()
	front_row.add_theme_constant_override("separation", 8)
	column.add_child(front_row)

	_play_button = Button.new()
	_play_button.text = "Play"
	_play_button.toggle_mode = true
	_play_button.focus_mode = Control.FOCUS_NONE
	_play_button.custom_minimum_size = Vector2(64.0, 0.0)
	_play_button.toggled.connect(_on_play_toggled)
	front_row.add_child(_play_button)

	_slider = HSlider.new()
	_slider.min_value = 0.0
	_slider.max_value = 1.0
	_slider.step = 0.001
	_slider.value = 1.0
	_slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_slider.custom_minimum_size = Vector2(190.0, 0.0)
	_slider.focus_mode = Control.FOCUS_NONE
	_slider.value_changed.connect(_on_slider_value_changed)
	front_row.add_child(_slider)

	_front_label = _make_stat_label("Build front 100%")
	column.add_child(_front_label)

	var hint := Label.new()
	hint.text = "RMB / MMB orbit · Shift+drag pan · Wheel zoom · WASD pan"
	hint.add_theme_font_size_override("font_size", 11)
	hint.add_theme_color_override("font_color", Color(0.62, 0.64, 0.58))
	column.add_child(hint)

	_warning_label = Label.new()
	_warning_label.add_theme_color_override("font_color", Color(0.96, 0.78, 0.36))
	_warning_label.add_theme_font_size_override("font_size", 12)
	_warning_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_warning_label.custom_minimum_size = Vector2(330.0, 0.0)
	_warning_label.visible = false
	column.add_child(_warning_label)

	_error_label = Label.new()
	_error_label.add_theme_color_override("font_color", Color(1.0, 0.46, 0.36))
	_error_label.add_theme_font_size_override("font_size", 12)
	_error_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_error_label.custom_minimum_size = Vector2(330.0, 0.0)
	_error_label.visible = false
	column.add_child(_error_label)


func _make_stat_label(initial_text: String) -> Label:
	var label := Label.new()
	label.text = initial_text
	label.add_theme_font_size_override("font_size", 12)
	label.add_theme_color_override("font_color", Color(0.86, 0.87, 0.80))
	return label


func _refresh_render_stats() -> void:
	var draw_calls := RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_DRAW_CALLS_IN_FRAME)
	# In Godot 4 this counter already reports triangles (not index primitives).
	var primitives := RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_PRIMITIVES_IN_FRAME)
	_fps_label.text = "FPS %d" % Engine.get_frames_per_second()
	_render_label.text = "Draw calls %s   Primitives %s   Triangles %s" % [
		_format_int(draw_calls),
		_format_int(primitives),
		_format_int(primitives),
	]


func set_scene_stats(block_count: int, chunk_count: int, layout_hash: String, triangle_count: int) -> void:
	_scene_label.text = "Blocks %s   Chunks %s   Tris %s" % [
		_format_int(block_count),
		_format_int(chunk_count),
		_format_int(triangle_count),
	]
	_hash_label.text = "Layout hash %s" % (layout_hash if not layout_hash.is_empty() else "—")


func set_mode_label(mode_name: String) -> void:
	_mode_label.text = mode_name


func set_build_front(value: float) -> void:
	var clamped := clampf(value, 0.0, 1.0)
	_slider.set_value_no_signal(clamped)
	_front_label.text = "Build front %d%%" % roundi(clamped * 100.0)


func set_playing(playing: bool) -> void:
	_playing = playing
	_play_button.set_pressed_no_signal(playing)
	_play_button.text = "Pause" if playing else "Play"


func show_error(message: String) -> void:
	_error_label.text = message
	_error_label.visible = true


func clear_error() -> void:
	_error_label.text = ""
	_error_label.visible = false


func show_warning(message: String) -> void:
	_warning_label.text = message
	_warning_label.visible = true


func clear_warning() -> void:
	_warning_label.text = ""
	_warning_label.visible = false


func _on_slider_value_changed(value: float) -> void:
	_front_label.text = "Build front %d%%" % roundi(value * 100.0)
	build_front_changed.emit(value)


func _on_play_toggled(pressed: bool) -> void:
	_playing = pressed
	_play_button.text = "Pause" if pressed else "Play"
	if pressed and _slider.value >= 0.999:
		_slider.value = 0.0
	playing_changed.emit(pressed)


func _on_showcase_pressed() -> void:
	mode_requested.emit("showcase")


func _on_stress_pressed() -> void:
	mode_requested.emit("stress")


func _format_int(value: int) -> String:
	var text := str(value)
	var out := ""
	var count := 0
	for i in range(text.length() - 1, -1, -1):
		if count > 0 and count % 3 == 0:
			out = "," + out
		out = text[i] + out
		count += 1
	return out
