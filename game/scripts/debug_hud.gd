extends CanvasLayer
## Minimal, code-built M0b debug HUD.
##
## Shows renderer counters and build statistics, drives the construction-front
## uniform, switches between the M0a/M0b showcase and the 200k stress build, and
## exposes the interactive undo command.

signal build_front_changed(value: float)
signal playing_changed(playing: bool)
signal mode_requested(mode: String)
signal tool_requested(tool: String)
signal undo_requested()
signal save_requested()
signal load_requested()

const REFRESH_INTERVAL := 0.25

var _fps_label: Label
var _render_label: Label
var _scene_label: Label
var _hash_label: Label
var _doc_hash_label: Label
var _front_label: Label
var _mode_label: Label
var _warning_label: Label
var _error_label: Label
var _slider: HSlider
var _play_button: Button
var _showcase_button: Button
var _m0b_button: Button
var _stress_button: Button
var _undo_button: Button
var _save_button: Button
var _load_button: Button
var _tool_status: Label
var _tool_buttons: Dictionary = {}

var _playing := false
var _refresh_timer := 0.0


func _ready() -> void:
	layer = 100
	_build_ui()


func _process(delta: float) -> void:
	# Playback is advanced by main.gd so it can hold at 1.0 instead of wrapping.
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
	title.text = "KEEPWRIGHT  ·  M0b"
	title.add_theme_font_size_override("font_size", 17)
	title.add_theme_color_override("font_color", Color(0.96, 0.89, 0.72))
	column.add_child(title)

	_mode_label = Label.new()
	_mode_label.text = "M0b Showcase"
	_mode_label.add_theme_font_size_override("font_size", 12)
	_mode_label.add_theme_color_override("font_color", Color(0.72, 0.74, 0.66))
	column.add_child(_mode_label)

	column.add_child(HSeparator.new())

	_fps_label = _make_stat_label("FPS —")
	_render_label = _make_stat_label("Draw calls —   Primitives —   Triangles —")
	_scene_label = _make_stat_label("Blocks —   Chunks —   Tris —")
	_hash_label = _make_stat_label("Layout hash —")
	_doc_hash_label = _make_stat_label("Doc hash —")
	column.add_child(_fps_label)
	column.add_child(_render_label)
	column.add_child(_scene_label)
	column.add_child(_hash_label)
	column.add_child(_doc_hash_label)

	column.add_child(HSeparator.new())

	var mode_row := HBoxContainer.new()
	mode_row.add_theme_constant_override("separation", 6)
	column.add_child(mode_row)

	_showcase_button = Button.new()
	_showcase_button.text = "M0a Showcase"
	_showcase_button.focus_mode = Control.FOCUS_NONE
	_showcase_button.pressed.connect(_on_showcase_pressed)
	mode_row.add_child(_showcase_button)

	_m0b_button = Button.new()
	_m0b_button.text = "M0b Showcase"
	_m0b_button.focus_mode = Control.FOCUS_NONE
	_m0b_button.pressed.connect(_on_m0b_pressed)
	mode_row.add_child(_m0b_button)

	_stress_button = Button.new()
	_stress_button.text = "200k Stress Test"
	_stress_button.focus_mode = Control.FOCUS_NONE
	_stress_button.pressed.connect(_on_stress_pressed)
	mode_row.add_child(_stress_button)

	_undo_button = Button.new()
	_undo_button.text = "Undo"
	_undo_button.focus_mode = Control.FOCUS_NONE
	_undo_button.pressed.connect(_on_undo_pressed)
	mode_row.add_child(_undo_button)

	var file_row := HBoxContainer.new()
	file_row.add_theme_constant_override("separation", 6)
	column.add_child(file_row)

	_save_button = Button.new()
	_save_button.text = "Save"
	_save_button.focus_mode = Control.FOCUS_NONE
	_save_button.pressed.connect(_on_save_pressed)
	file_row.add_child(_save_button)

	_load_button = Button.new()
	_load_button.text = "Load"
	_load_button.focus_mode = Control.FOCUS_NONE
	_load_button.pressed.connect(_on_load_pressed)
	file_row.add_child(_load_button)

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

	column.add_child(HSeparator.new())

	var tool_row := HBoxContainer.new()
	tool_row.add_theme_constant_override("separation", 6)
	column.add_child(tool_row)

	var tool_group := ButtonGroup.new()
	tool_group.allow_unpress = false
	var tool_entries := [
		["view", "View"],
		["wall", "Draw Wall"],
		["tower", "Place Tower"],
	]
	for entry in tool_entries:
		var tool_id: String = entry[0]
		var button := Button.new()
		button.text = entry[1]
		button.toggle_mode = true
		button.button_group = tool_group
		button.focus_mode = Control.FOCUS_NONE
		button.pressed.connect(_on_tool_pressed.bind(tool_id))
		tool_row.add_child(button)
		_tool_buttons[tool_id] = button
	_tool_buttons["view"].set_pressed_no_signal(true)

	_tool_status = _make_stat_label("Tool: View — orbit around the site")
	_tool_status.add_theme_color_override("font_color", Color(0.90, 0.82, 0.60))
	column.add_child(_tool_status)

	var hint := Label.new()
	hint.text = "RMB / MMB orbit · Shift+drag pan · Wheel zoom · WASD pan\nLMB drag to draw a wall · LMB to place a tower · Ctrl+Z undo"
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


func set_scene_stats(
	block_count: int,
	chunk_count: int,
	layout_hash: String,
	triangle_count: int,
	document_hash: String = ""
) -> void:
	_scene_label.text = "Blocks %s   Chunks %s   Tris %s" % [
		_format_int(block_count),
		_format_int(chunk_count),
		_format_int(triangle_count),
	]
	_hash_label.text = "Layout hash %s" % (layout_hash if not layout_hash.is_empty() else "—")
	_doc_hash_label.text = "Doc hash %s" % (document_hash if not document_hash.is_empty() else "—")


func set_mode_label(mode_name: String) -> void:
	_mode_label.text = mode_name


func set_build_front(value: float) -> void:
	var clamped := clampf(value, 0.0, 1.0)
	_slider.set_value_no_signal(clamped)
	_front_label.text = "Build front %d%%" % roundi(clamped * 100.0)


func set_tool_status(text: String) -> void:
	if _tool_status != null:
		_tool_status.text = text


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
	if pressed and _slider.value >= 0.99:
		_slider.value = 0.0
	playing_changed.emit(pressed)


func _on_showcase_pressed() -> void:
	mode_requested.emit("showcase")


func _on_m0b_pressed() -> void:
	mode_requested.emit("m0b")


func _on_stress_pressed() -> void:
	mode_requested.emit("stress")


func _on_undo_pressed() -> void:
	undo_requested.emit()


func _on_save_pressed() -> void:
	save_requested.emit()


func _on_load_pressed() -> void:
	load_requested.emit()


func _on_tool_pressed(tool: String) -> void:
	tool_requested.emit(tool)


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
