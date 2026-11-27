extends CanvasLayer
## Minimal, code-built M0b debug HUD.
##
## Shows renderer counters and build statistics, drives the construction-front
## uniform, switches between the M0a/M0b showcase and the 200k stress build, and
## exposes the interactive undo command.

signal build_front_changed(value: float)
signal playing_changed(playing: bool)
signal speed_requested(multiplier: float)
signal timelapse_requested()
signal mute_toggled(muted: bool)
signal mode_requested(mode: String)
signal style_requested(style_id: String)
signal site_requested(site_id: String)
signal tool_requested(tool: String)
signal fix_requested(fix_json: String)
signal undo_requested()
signal save_requested()
signal load_requested()
signal cutaway_mode_requested(mode: int)
signal cutaway_position_changed(position: float)
signal terrain_tool_requested(kind: String)
signal photo_requested()
signal codex_requested()

const REFRESH_INTERVAL := 0.25
const MAX_ASSESSMENT_FINDINGS := 3

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
var _speed_buttons: Dictionary = {}
var _timelapse_button: Button
var _mute_button: Button
var _m1_button: Button
var _m2_button: Button
var _showcase_button: Button
var _m0b_button: Button
var _stress_button: Button
var _undo_button: Button
var _save_button: Button
var _load_button: Button
var _tool_status: Label
var _tool_buttons: Dictionary = {}
var _cutaway_buttons: Dictionary = {}
var _cutaway_slider: HSlider
var _cutaway_value_label: Label
var _cutaway_mode := 0
var _assessment_label: Label
var _historian_label: Label
var _assessment_findings: VBoxContainer
var _style_buttons: Dictionary = {}
var _stats_panel: Panel
var _stats_scroll: ScrollContainer
var _stats_column: VBoxContainer

var _playing := false
var _speed_multiplier := 1.0
var _timelapse_active := false
var _muted := false
var _refresh_timer := 0.0


func _ready() -> void:
	# Keep the HUD below AgentPanel (20), PhotoMode (30) and CodexPanel (40) so
	# the modal codex always draws in front of every HUD (M3.1 H4).
	layer = 10
	_build_ui()
	if get_viewport() != null:
		get_viewport().size_changed.connect(_update_hud_size)
	_update_hud_size.call_deferred()


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

	# The stats panel is a plain Control (not a PanelContainer) so an explicit
	# size can clamp it. Its content lives in a vertical ScrollContainer, which
	# keeps 3+ findings and error lines usable on a 900 px screen instead of
	# running off the bottom (M3.1 H4).
	var panel := Panel.new()
	panel.name = "StatsPanel"
	panel.position = Vector2(14.0, 14.0)
	panel.size = Vector2(360.0, 420.0)
	panel.mouse_filter = Control.MOUSE_FILTER_STOP
	root.add_child(panel)
	_stats_panel = panel

	var margin := MarginContainer.new()
	margin.name = "StatsMargin"
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", 14)
	margin.add_theme_constant_override("margin_right", 14)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 12)
	panel.add_child(margin)

	var scroll := ScrollContainer.new()
	scroll.name = "StatsScroll"
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	scroll.vertical_scroll_mode = ScrollContainer.SCROLL_MODE_AUTO
	scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	margin.add_child(scroll)
	_stats_scroll = scroll

	var column := VBoxContainer.new()
	column.name = "StatsColumn"
	column.add_theme_constant_override("separation", 4)
	column.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(column)
	_stats_column = column

	var title := Label.new()
	title.text = "KEEPWRIGHT  ·  M1"
	title.add_theme_font_size_override("font_size", 17)
	title.add_theme_color_override("font_color", Color(0.96, 0.89, 0.72))
	column.add_child(title)

	_mode_label = Label.new()
	_mode_label.text = "M1 Edwardian"
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
	_showcase_button.text = "M0a"
	_showcase_button.focus_mode = Control.FOCUS_NONE
	_showcase_button.pressed.connect(_on_showcase_pressed)
	mode_row.add_child(_showcase_button)

	_m1_button = Button.new()
	_m1_button.text = "M1 Edwardian"
	_m1_button.focus_mode = Control.FOCUS_NONE
	_m1_button.pressed.connect(_on_m1_pressed)
	mode_row.add_child(_m1_button)

	_m2_button = Button.new()
	_m2_button.text = "M2 Baltic"
	_m2_button.focus_mode = Control.FOCUS_NONE
	_m2_button.pressed.connect(_on_m2_pressed)
	mode_row.add_child(_m2_button)

	_m0b_button = Button.new()
	_m0b_button.text = "M0b"
	_m0b_button.focus_mode = Control.FOCUS_NONE
	_m0b_button.pressed.connect(_on_m0b_pressed)
	mode_row.add_child(_m0b_button)

	_stress_button = Button.new()
	_stress_button.text = "200k"
	_stress_button.focus_mode = Control.FOCUS_NONE
	_stress_button.pressed.connect(_on_stress_pressed)
	mode_row.add_child(_stress_button)

	_undo_button = Button.new()
	_undo_button.text = "Undo"
	_undo_button.focus_mode = Control.FOCUS_NONE
	_undo_button.pressed.connect(_on_undo_pressed)
	mode_row.add_child(_undo_button)

	var site_row := HBoxContainer.new()
	site_row.add_theme_constant_override("separation", 6)
	column.add_child(site_row)

	var style_label := Label.new()
	style_label.text = "Style"
	style_label.add_theme_font_size_override("font_size", 11)
	style_label.add_theme_color_override("font_color", Color(0.62, 0.64, 0.58))
	site_row.add_child(style_label)

	var style_group := ButtonGroup.new()
	style_group.allow_unpress = false
	for style_entry in [
		["edwardian_welsh", "Welsh Stone"],
		["baltic_brick", "Baltic Brick"],
	]:
		var style_button := Button.new()
		style_button.text = style_entry[1]
		style_button.toggle_mode = true
		style_button.button_group = style_group
		style_button.focus_mode = Control.FOCUS_NONE
		style_button.pressed.connect(_on_style_pressed.bind(style_entry[0]))
		site_row.add_child(style_button)
		_style_buttons[style_entry[0]] = style_button
	_style_buttons["edwardian_welsh"].set_pressed_no_signal(true)

	var site_row_2 := HBoxContainer.new()
	site_row_2.add_theme_constant_override("separation", 6)
	column.add_child(site_row_2)

	var site_label := Label.new()
	site_label.text = "Site"
	site_label.add_theme_font_size_override("font_size", 11)
	site_label.add_theme_color_override("font_color", Color(0.62, 0.64, 0.58))
	site_row_2.add_child(site_label)

	for site_entry in [
		["m0a_promontory", "Promontory"],
		["coastal_headland", "Headland"],
		["lake_island", "Lake Island"],
	]:
		var site_button := Button.new()
		site_button.text = site_entry[1]
		site_button.focus_mode = Control.FOCUS_NONE
		site_button.pressed.connect(_on_site_pressed.bind(site_entry[0]))
		site_row_2.add_child(site_button)

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

	var photo_button := Button.new()
	photo_button.text = "Photo"
	photo_button.focus_mode = Control.FOCUS_NONE
	photo_button.pressed.connect(func() -> void: photo_requested.emit())
	file_row.add_child(photo_button)

	var codex_button := Button.new()
	codex_button.text = "Codex"
	codex_button.focus_mode = Control.FOCUS_NONE
	codex_button.pressed.connect(func() -> void: codex_requested.emit())
	file_row.add_child(codex_button)

	# Dollhouse cutaway: a plane sweep with a floor-height slider (M3.1).
	var cutaway_row := HBoxContainer.new()
	cutaway_row.add_theme_constant_override("separation", 6)
	column.add_child(cutaway_row)

	var cutaway_label := Label.new()
	cutaway_label.text = "Cutaway"
	cutaway_label.add_theme_font_size_override("font_size", 11)
	cutaway_label.add_theme_color_override("font_color", Color(0.62, 0.64, 0.58))
	cutaway_row.add_child(cutaway_label)

	var cutaway_group := ButtonGroup.new()
	cutaway_group.allow_unpress = false
	for entry in [[0, "Off"], [1, "Storey"], [2, "Sec X"], [3, "Sec Z"]]:
		var cutaway_button := Button.new()
		cutaway_button.text = entry[1]
		cutaway_button.toggle_mode = true
		cutaway_button.button_group = cutaway_group
		cutaway_button.focus_mode = Control.FOCUS_NONE
		cutaway_button.pressed.connect(_on_cutaway_mode_pressed.bind(entry[0]))
		cutaway_row.add_child(cutaway_button)
		_cutaway_buttons[int(entry[0])] = cutaway_button
	_cutaway_buttons[0].set_pressed_no_signal(true)

	var cutaway_slider_row := HBoxContainer.new()
	cutaway_slider_row.add_theme_constant_override("separation", 8)
	column.add_child(cutaway_slider_row)

	_cutaway_slider = HSlider.new()
	_cutaway_slider.min_value = -2.0
	_cutaway_slider.max_value = 24.0
	_cutaway_slider.step = 0.05
	_cutaway_slider.value = 6.0
	_cutaway_slider.editable = false
	_cutaway_slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_cutaway_slider.custom_minimum_size = Vector2(190.0, 0.0)
	_cutaway_slider.focus_mode = Control.FOCUS_NONE
	_cutaway_slider.value_changed.connect(_on_cutaway_slider_changed)
	cutaway_slider_row.add_child(_cutaway_slider)

	_cutaway_value_label = _make_stat_label("Cut —")
	cutaway_slider_row.add_child(_cutaway_value_label)

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

	# Construction speed: 30 s / 7.5 s / 1.5 s rises, plus an instant snap.
	var speed_row := HBoxContainer.new()
	speed_row.add_theme_constant_override("separation", 6)
	column.add_child(speed_row)

	var speed_label := Label.new()
	speed_label.text = "Speed"
	speed_label.add_theme_font_size_override("font_size", 11)
	speed_label.add_theme_color_override("font_color", Color(0.62, 0.64, 0.58))
	speed_row.add_child(speed_label)

	var speed_group := ButtonGroup.new()
	speed_group.allow_unpress = false
	for entry in [[1.0, "1×"], [4.0, "4×"], [20.0, "20×"], [0.0, "Instant"]]:
		var speed_button := Button.new()
		speed_button.text = entry[1]
		speed_button.toggle_mode = true
		speed_button.button_group = speed_group
		speed_button.focus_mode = Control.FOCUS_NONE
		speed_button.custom_minimum_size = Vector2(52.0, 0.0)
		speed_button.pressed.connect(_on_speed_pressed.bind(entry[0]))
		speed_row.add_child(speed_button)
		_speed_buttons[float(entry[0])] = speed_button
	_speed_buttons[1.0].set_pressed_no_signal(true)

	_timelapse_button = Button.new()
	_timelapse_button.text = "Timelapse"
	_timelapse_button.toggle_mode = true
	_timelapse_button.focus_mode = Control.FOCUS_NONE
	_timelapse_button.pressed.connect(_on_timelapse_pressed)
	speed_row.add_child(_timelapse_button)

	_mute_button = Button.new()
	_mute_button.text = "Mute"
	_mute_button.toggle_mode = true
	_mute_button.focus_mode = Control.FOCUS_NONE
	_mute_button.toggled.connect(_on_mute_toggled)
	speed_row.add_child(_mute_button)

	column.add_child(HSeparator.new())

	var tool_row := GridContainer.new()
	tool_row.columns = 4
	tool_row.add_theme_constant_override("h_separation", 6)
	tool_row.add_theme_constant_override("v_separation", 4)
	column.add_child(tool_row)

	var tool_group := ButtonGroup.new()
	tool_group.allow_unpress = false
	var tool_entries := [
		["view", "View"],
		["wall", "Wall"],
		["tower", "Round Tower"],
		["tower_d", "D-Tower"],
		["tower_square", "Square Tower"],
		["gatehouse", "Gatehouse"],
		["ditch", "Ditch"],
		["keep", "Keep"],
		["hall", "Hall"],
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

	# Terrain brushes share the tool group so only one edit tool is armed.
	for entry in [
		["raise", "Raise"],
		["lower", "Lower"],
		["flatten", "Flatten"],
		["smooth", "Smooth"],
	]:
		var brush_id: String = entry[0]
		var brush_button := Button.new()
		brush_button.text = entry[1]
		brush_button.toggle_mode = true
		brush_button.button_group = tool_group
		brush_button.focus_mode = Control.FOCUS_NONE
		brush_button.pressed.connect(_on_terrain_tool_pressed.bind(brush_id))
		tool_row.add_child(brush_button)
		_tool_buttons[brush_id] = brush_button

	_tool_status = _make_stat_label("Tool: View — orbit around the site")
	_tool_status.add_theme_color_override("font_color", Color(0.90, 0.82, 0.60))
	column.add_child(_tool_status)

	column.add_child(HSeparator.new())

	_assessment_label = Label.new()
	_assessment_label.text = "Structure: —"
	_assessment_label.add_theme_font_size_override("font_size", 13)
	_assessment_label.add_theme_color_override("font_color", Color(0.86, 0.87, 0.80))
	column.add_child(_assessment_label)

	_historian_label = Label.new()
	_historian_label.text = "Historian: —"
	_historian_label.add_theme_font_size_override("font_size", 12)
	_historian_label.add_theme_color_override("font_color", Color(0.74, 0.80, 0.88))
	column.add_child(_historian_label)

	_assessment_findings = VBoxContainer.new()
	_assessment_findings.add_theme_constant_override("separation", 3)
	column.add_child(_assessment_findings)

	var hint := Label.new()
	hint.text = (
		"RMB / MMB orbit · Shift+drag pan · Wheel zoom\n"
		+ "WASD/Arrows pan · Q/E orbit · R/F tilt · +/- zoom · Home reset\n"
		+ "LMB drag: wall/ditch · LMB click: tower/gatehouse · Ctrl+Z undo\n"
		+ "Raise/Lower/Flatten/Smooth: LMB sculpts ground · P photo · C codex"
	)
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


## Recompute the stats panel height so it always fits the current window but
## never exceeds 840 px. The vertical ScrollContainer handles any overflow from
## assessment findings or error lines (M3.1 H4).
func _update_hud_size() -> void:
	if _stats_panel == null or _stats_column == null:
		return
	var viewport_height := 900.0
	if get_viewport() != null:
		viewport_height = get_viewport().get_visible_rect().size.y
	var max_height := minf(viewport_height - 28.0, 840.0)
	var column_min := _stats_column.get_combined_minimum_size()
	var desired_width := maxf(column_min.x + 28.0, 360.0)
	var desired_height := minf(maxf(column_min.y + 24.0, 120.0), max_height)
	_stats_panel.size = Vector2(desired_width, desired_height)


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


## Compact structural assessment readout. Passing shows the score line (plus a
## warning count when informational findings remain); findings are listed up to
## three, each fixable one with a one-click Fix button.
func set_assessment(data: Dictionary) -> void:
	if _assessment_label == null or _assessment_findings == null:
		return
	for child in _assessment_findings.get_children():
		child.queue_free()

	var score := int(data.get("score", 100))
	var passed := bool(data.get("passed", true))
	var findings: Variant = data.get("findings", [])
	var has_findings := findings is Array and not (findings as Array).is_empty()

	if passed:
		if has_findings:
			_assessment_label.text = "Structure: %d/100 ✓ (%d warnings)" % [
				score, (findings as Array).size(),
			]
		else:
			_assessment_label.text = "Structure: %d/100 ✓" % score
		_assessment_label.add_theme_color_override("font_color", Color(0.62, 0.88, 0.54))
		if not has_findings:
			_update_hud_size.call_deferred()
			return
	else:
		_assessment_label.text = "Structure: %d/100 ✗" % score
		_assessment_label.add_theme_color_override("font_color", Color(1.0, 0.55, 0.42))

	if not (findings is Array):
		_update_hud_size.call_deferred()
		return
	var shown := 0
	for entry in (findings as Array):
		if shown >= MAX_ASSESSMENT_FINDINGS:
			break
		if not (entry is Dictionary):
			continue
		var finding: Dictionary = entry
		shown += 1

		var row := HBoxContainer.new()
		row.add_theme_constant_override("separation", 6)

		var message := Label.new()
		var severity_name := str(finding.get("severity", "info"))
		message.text = "%s  %s" % [
			severity_name.to_upper(),
			str(finding.get("message", finding.get("code", ""))),
		]
		message.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		message.custom_minimum_size = Vector2(240.0, 0.0)
		message.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		message.add_theme_font_size_override("font_size", 11)
		match severity_name:
			"error":
				message.add_theme_color_override("font_color", Color(1.0, 0.55, 0.42))
			"warning":
				message.add_theme_color_override("font_color", Color(0.96, 0.78, 0.36))
			_:
				message.add_theme_color_override("font_color", Color(0.78, 0.80, 0.72))
		row.add_child(message)

		if bool(finding.get("has_fix", false)):
			var fix_button := Button.new()
			fix_button.text = "Fix"
			fix_button.focus_mode = Control.FOCUS_NONE
			fix_button.custom_minimum_size = Vector2(42.0, 0.0)
			fix_button.pressed.connect(
				_on_fix_pressed.bind(str(finding.get("fix_json", "")))
			)
			row.add_child(fix_button)

		_assessment_findings.add_child(row)
	_update_hud_size.call_deferred()


## Compact historian readout shown directly under the structural score.
func set_historian(data: Dictionary) -> void:
	if _historian_label == null:
		return
	if data.is_empty():
		_historian_label.text = "Historian: —"
		return
	_historian_label.text = "Historian: %d/100 · Auth %d · Def %d · Dom %d" % [
		int(data.get("overall_score", 0)),
		int(data.get("authenticity_score", 0)),
		int(data.get("defensive_score", 0)),
		int(data.get("domestic_score", 0)),
	]


## Keep the style toggle in sync with the bridge's active pack.
func set_style_selection(style_id: String) -> void:
	if _style_buttons.has(style_id):
		var button: Button = _style_buttons[style_id]
		button.set_pressed_no_signal(true)


## Reflect the active dollhouse cutaway mode and enable the plane slider only
## while a cut is active.
func set_cutaway_mode(mode: int) -> void:
	_cutaway_mode = mode
	if _cutaway_buttons.has(mode):
		var button: Button = _cutaway_buttons[mode]
		button.set_pressed_no_signal(true)
	if _cutaway_slider != null:
		_cutaway_slider.editable = mode > 0
	if _cutaway_value_label != null:
		_cutaway_value_label.text = "Cut —" if mode == 0 else _format_cutaway_value()


## Set the slider's usable range and current value without re-emitting, so the
## vertical and horizontal planes can use their own sensible bounds.
func set_cutaway_range(min_value: float, max_value: float, value: float) -> void:
	if _cutaway_slider == null:
		return
	_cutaway_slider.min_value = min_value
	_cutaway_slider.max_value = max_value
	_cutaway_slider.set_value_no_signal(clampf(value, min_value, max_value))
	if _cutaway_value_label != null:
		_cutaway_value_label.text = _format_cutaway_value()


func get_cutaway_mode() -> int:
	return _cutaway_mode


func get_cutaway_position() -> float:
	if _cutaway_slider == null:
		return 0.0
	return _cutaway_slider.value


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


## Select a construction-speed preset. 0.0 means "Instant" (snap to 1.0).
func set_speed(multiplier: float) -> void:
	_speed_multiplier = multiplier
	var key := float(multiplier)
	if _speed_buttons.has(key):
		var button: Button = _speed_buttons[key]
		button.set_pressed_no_signal(true)


func get_speed_multiplier() -> float:
	return _speed_multiplier


func set_timelapse(active: bool) -> void:
	_timelapse_active = active
	if _timelapse_button != null:
		_timelapse_button.set_pressed_no_signal(active)
		_timelapse_button.text = "Stop Replay" if active else "Timelapse"


func is_timelapse_active() -> bool:
	return _timelapse_active


func set_muted(muted: bool) -> void:
	_muted = muted
	if _mute_button != null:
		_mute_button.set_pressed_no_signal(muted)
		_mute_button.text = "Unmute" if muted else "Mute"


func is_muted() -> bool:
	return _muted


func show_error(message: String) -> void:
	_error_label.text = message
	_error_label.visible = true
	_update_hud_size.call_deferred()


func clear_error() -> void:
	_error_label.text = ""
	_error_label.visible = false
	_update_hud_size.call_deferred()


func show_warning(message: String) -> void:
	_warning_label.text = message
	_warning_label.visible = true
	_update_hud_size.call_deferred()


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


func _on_speed_pressed(multiplier: float) -> void:
	_speed_multiplier = multiplier
	speed_requested.emit(multiplier)


func _on_timelapse_pressed() -> void:
	timelapse_requested.emit()


func _on_mute_toggled(pressed: bool) -> void:
	_muted = pressed
	_mute_button.text = "Unmute" if pressed else "Mute"
	mute_toggled.emit(pressed)


func _on_showcase_pressed() -> void:
	mode_requested.emit("showcase")


func _on_m1_pressed() -> void:
	mode_requested.emit("m1")


func _on_m2_pressed() -> void:
	mode_requested.emit("m2")


func _on_style_pressed(style_id: String) -> void:
	style_requested.emit(style_id)


func _on_site_pressed(site_id: String) -> void:
	site_requested.emit(site_id)


func _on_fix_pressed(fix_json: String) -> void:
	if not fix_json.is_empty():
		fix_requested.emit(fix_json)


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


func _on_terrain_tool_pressed(kind: String) -> void:
	terrain_tool_requested.emit(kind)


func _on_cutaway_mode_pressed(mode: int) -> void:
	cutaway_mode_requested.emit(mode)


func _on_cutaway_slider_changed(value: float) -> void:
	if _cutaway_value_label != null:
		_cutaway_value_label.text = _format_cutaway_value()
	cutaway_position_changed.emit(value)


func _format_cutaway_value() -> String:
	if _cutaway_slider == null:
		return "Cut —"
	return "Cut %.1f m" % _cutaway_slider.value


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
