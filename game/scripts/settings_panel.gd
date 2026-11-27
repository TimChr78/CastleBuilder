extends CanvasLayer
## M4b accessibility, settings and in-game credits modal.
##
## A centred, code-built modal (AGENTS.md rule 5) that owns the player-facing
## accessibility switches and the "Credits & Licences" summary. It persists its
## state to `user://settings.json` and never touches authoritative castle
## state; `main.gd` applies the emitted values to the window, the masonry
## shaders, the proposal ghosts and the audio bus.
##
## Styling mirrors CodexPanel (dark slate with a warm parchment accent) so the
## two modals read as one family.

signal ui_scale_changed(scale: float)
signal ghost_palette_changed(high_contrast: bool)
signal reduced_motion_changed(enabled: bool)
signal mute_changed(muted: bool)

const SETTINGS_PATH := "user://settings.json"
const PANEL_SIZE := Vector2(620.0, 460.0)
const UI_SCALE_PRESETS := [1.0, 1.15, 1.3]
const DEFAULT_UI_SCALE := 1.0
const PANEL_BG := Color(0.06, 0.065, 0.08, 0.96)
const BORDER := Color(0.55, 0.46, 0.32, 0.60)
const TEXT := Color(0.90, 0.88, 0.81)
const MUTED := Color(0.66, 0.64, 0.57)
const PARCHMENT := Color(0.95, 0.84, 0.62)
const ACCENT := Color(0.86, 0.72, 0.46)

## Self-made / CC0 / zero-AI attribution summary shown inside the modal.
## Kept deliberately explicit so the player can audit the licence promise.
const CREDITS_TEXT := """Keepwright ships only procedural, self-made, CC0 or openly licensed assets, and every asset is declared AI-free.

PROCEDURAL SHADERS (self-made)
masonry.gdshader, masonry_cutaway.gdshader, terrain.gdshader and water.gdshader are hand-written GLSL. Coursed ashlar, weathered Baltic brick, bowler-hat mortar joints, wet base courses, winter rime/thatch and the dollhouse cutaway are all computed at runtime — no texture or model files.

PROCEDURAL AUDIO (self-made)
build_audio.gd synthesises stone ticks, placement thuds and zoom-reactive ambience at runtime. No recorded audio files ship with the game.

STYLE PACKS (self-made)
Edwardian Welsh (1277-1300) and Baltic Teutonic Brick Gothic (1280-1410) packs are original JSON with historical source notes for Caernarfon, Conwy, Harlech, Beaumaris, Malbork, Gniew, Kwidzyn and Lidzbark Warminski.

SITES (self-made)
The promontory, coastal headland and lake-island terrain generators are original procedural code. No photogrammetry, scanned textures or third-party heightmaps are used.

LICENCE POLICY
Approved licences: self-made, CC0-1.0, MIT, CC-BY-4.0. Non-commercial and AI-generated assets are excluded from shipped builds. The authoritative manifest is assets-src/LICENCE_MANIFEST.json; docs/CREDITS.md is generated from it by tools/check_licences.py."""

var _panel: PanelContainer
var _credits_label: Label
var _close_button: Button
var _scale_buttons: Dictionary = {}
var _ghost_buttons: Dictionary = {}
var _reduced_motion_check: CheckBox
var _mute_check: CheckBox
var _scale_group: ButtonGroup
var _ghost_group: ButtonGroup

var _ui_scale := DEFAULT_UI_SCALE
var _high_contrast := false
var _reduced_motion := false
var _muted := false


func _init() -> void:
	layer = 39
	visible = false
	_build()


func _ready() -> void:
	if _panel == null:
		_build()


func _build() -> void:
	if _panel != null:
		return
	var root := Control.new()
	root.name = "SettingsRoot"
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	var backdrop := ColorRect.new()
	backdrop.name = "SettingsBackdrop"
	backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	backdrop.color = Color(0.02, 0.03, 0.04, 0.55)
	backdrop.mouse_filter = Control.MOUSE_FILTER_STOP
	root.add_child(backdrop)

	_panel = PanelContainer.new()
	_panel.name = "SettingsFrame"
	_panel.set_anchors_preset(Control.PRESET_CENTER)
	_panel.offset_left = -PANEL_SIZE.x * 0.5
	_panel.offset_right = PANEL_SIZE.x * 0.5
	_panel.offset_top = -PANEL_SIZE.y * 0.5
	_panel.offset_bottom = PANEL_SIZE.y * 0.5
	_panel.custom_minimum_size = PANEL_SIZE
	_panel.add_theme_stylebox_override("panel", _panel_style())
	root.add_child(_panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 18)
	margin.add_theme_constant_override("margin_right", 18)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 14)
	_panel.add_child(margin)

	var column := VBoxContainer.new()
	column.name = "SettingsColumn"
	column.add_theme_constant_override("separation", 8)
	margin.add_child(column)

	# --- Header -------------------------------------------------------
	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	column.add_child(header)

	var title := Label.new()
	title.text = "SETTINGS & CREDITS"
	title.add_theme_font_size_override("font_size", 16)
	title.add_theme_color_override("font_color", PARCHMENT)
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)

	var hint := Label.new()
	hint.text = "F10 / Esc to close"
	hint.add_theme_font_size_override("font_size", 11)
	hint.add_theme_color_override("font_color", MUTED)
	header.add_child(hint)

	_close_button = Button.new()
	_close_button.name = "SettingsClose"
	_close_button.text = "×"
	_close_button.focus_mode = Control.FOCUS_NONE
	_close_button.custom_minimum_size = Vector2(30.0, 24.0)
	_close_button.add_theme_font_size_override("font_size", 16)
	_close_button.pressed.connect(close)
	header.add_child(_close_button)

	column.add_child(HSeparator.new())

	# --- UI scale -----------------------------------------------------
	var scale_row := _make_row(column, "UI Scale")
	_scale_group = ButtonGroup.new()
	_scale_group.allow_unpress = false
	for scale_value in UI_SCALE_PRESETS:
		var percent := roundi(float(scale_value) * 100.0)
		var scale_button := Button.new()
		scale_button.text = "%d%%" % percent
		scale_button.toggle_mode = true
		scale_button.button_group = _scale_group
		scale_button.focus_mode = Control.FOCUS_NONE
		scale_button.custom_minimum_size = Vector2(64.0, 0.0)
		scale_button.pressed.connect(_on_scale_pressed.bind(float(scale_value)))
		scale_row.add_child(scale_button)
		_scale_buttons[float(scale_value)] = scale_button

	# --- Proposal ghost colour ---------------------------------------
	var ghost_row := _make_row(column, "Proposal Ghost Colour")
	_ghost_group = ButtonGroup.new()
	_ghost_group.allow_unpress = false
	for entry in [
		[false, "Cyan (Default)"],
		[true, "High-Contrast Amber"],
	]:
		var ghost_button := Button.new()
		ghost_button.text = entry[1]
		ghost_button.toggle_mode = true
		ghost_button.button_group = _ghost_group
		ghost_button.focus_mode = Control.FOCUS_NONE
		ghost_button.pressed.connect(_on_ghost_pressed.bind(bool(entry[0])))
		ghost_row.add_child(ghost_button)
		_ghost_buttons[bool(entry[0])] = ghost_button

	# --- Motion / audio booleans -------------------------------------
	_reduced_motion_check = CheckBox.new()
	_reduced_motion_check.name = "ReducedMotionCheck"
	_reduced_motion_check.text = "Reduced motion (no masonry drop-sway or dust bursts)"
	_reduced_motion_check.focus_mode = Control.FOCUS_NONE
	_reduced_motion_check.add_theme_font_size_override("font_size", 12)
	_reduced_motion_check.add_theme_color_override("font_color", TEXT)
	_reduced_motion_check.toggled.connect(_on_reduced_motion_toggled)
	column.add_child(_reduced_motion_check)

	_mute_check = CheckBox.new()
	_mute_check.name = "MuteCheck"
	_mute_check.text = "Mute audio"
	_mute_check.focus_mode = Control.FOCUS_NONE
	_mute_check.add_theme_font_size_override("font_size", 12)
	_mute_check.add_theme_color_override("font_color", TEXT)
	_mute_check.toggled.connect(_on_mute_toggled)
	column.add_child(_mute_check)

	column.add_child(HSeparator.new())

	# --- Credits & licences ------------------------------------------
	var credits_heading := Label.new()
	credits_heading.text = "Credits & Licences"
	credits_heading.add_theme_font_size_override("font_size", 13)
	credits_heading.add_theme_color_override("font_color", ACCENT)
	column.add_child(credits_heading)

	var credits_scroll := ScrollContainer.new()
	credits_scroll.name = "CreditsScroll"
	credits_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	credits_scroll.vertical_scroll_mode = ScrollContainer.SCROLL_MODE_AUTO
	credits_scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	credits_scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	credits_scroll.custom_minimum_size = Vector2(0.0, 150.0)
	column.add_child(credits_scroll)

	_credits_label = Label.new()
	_credits_label.name = "CreditsText"
	_credits_label.text = CREDITS_TEXT
	_credits_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_credits_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_credits_label.add_theme_font_size_override("font_size", 11)
	_credits_label.add_theme_color_override("font_color", MUTED)
	credits_scroll.add_child(_credits_label)

	_sync_widgets()


func _panel_style() -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = PANEL_BG
	style.border_color = BORDER
	style.set_border_width_all(1)
	style.set_corner_radius_all(8)
	style.content_margin_left = 4.0
	style.content_margin_right = 4.0
	style.content_margin_top = 4.0
	style.content_margin_bottom = 4.0
	return style


func _make_row(parent: VBoxContainer, label_text: String) -> HBoxContainer:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)
	parent.add_child(row)
	var label := Label.new()
	label.text = label_text
	label.add_theme_font_size_override("font_size", 12)
	label.add_theme_color_override("font_color", TEXT)
	label.custom_minimum_size = Vector2(168.0, 0.0)
	row.add_child(label)
	return row


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

func is_open() -> bool:
	return visible


func open() -> void:
	visible = true


func close() -> void:
	visible = false


func toggle() -> void:
	visible = not visible


func get_ui_scale() -> float:
	return _ui_scale


func is_high_contrast() -> bool:
	return _high_contrast


func is_reduced_motion() -> bool:
	return _reduced_motion


func is_muted() -> bool:
	return _muted


func get_credits_text() -> String:
	return CREDITS_TEXT


## Programmatic updates (used by tests and by main.gd on load). Widgets are
## synced without emitting so callers stay in control of signal ordering.
func set_ui_scale(scale: float) -> void:
	_ui_scale = clampf(scale, 0.85, 1.5)
	_sync_widgets()


func set_high_contrast(enabled: bool) -> void:
	_high_contrast = enabled
	_sync_widgets()


func set_reduced_motion(enabled: bool) -> void:
	_reduced_motion = enabled
	_sync_widgets()


func set_muted(muted: bool) -> void:
	_muted = muted
	_sync_widgets()


## Persist the current accessibility settings to `path` (default `user://settings.json`).
func save_settings(path: String = SETTINGS_PATH) -> bool:
	var data := {
		"ui_scale": _ui_scale,
		"high_contrast": _high_contrast,
		"reduced_motion": _reduced_motion,
		"muted": _muted,
	}
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		push_warning(
			"Keepwright: could not open %s for writing (error %d)."
			% [path, FileAccess.get_open_error()]
		)
		return false
	file.store_string(JSON.stringify(data, "\t"))
	file.close()
	return true


## Load persisted settings, refresh the widgets and emit every applied value so
## main.gd can re-apply them. Returns false when no valid file exists.
func load_settings(path: String = SETTINGS_PATH) -> bool:
	if not FileAccess.file_exists(path):
		return false
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return false
	var text := file.get_as_text()
	file.close()
	var parsed: Variant = JSON.parse_string(text)
	if not (parsed is Dictionary):
		push_warning("Keepwright: %s is not valid JSON." % path)
		return false
	var data: Dictionary = parsed
	var scale_value: Variant = data.get("ui_scale", DEFAULT_UI_SCALE)
	if scale_value is float or scale_value is int:
		_ui_scale = clampf(float(scale_value), 0.85, 1.5)
	var hc_value: Variant = data.get("high_contrast", false)
	if hc_value is bool:
		_high_contrast = hc_value
	var rm_value: Variant = data.get("reduced_motion", false)
	if rm_value is bool:
		_reduced_motion = rm_value
	var mute_value: Variant = data.get("muted", false)
	if mute_value is bool:
		_muted = mute_value
	_sync_widgets()
	ui_scale_changed.emit(_ui_scale)
	ghost_palette_changed.emit(_high_contrast)
	reduced_motion_changed.emit(_reduced_motion)
	mute_changed.emit(_muted)
	return true


# ---------------------------------------------------------------------------
# Internals
# ---------------------------------------------------------------------------

## Mirror the stored state onto the buttons/checks without emitting signals.
func _sync_widgets() -> void:
	for scale_value in _scale_buttons:
		var button: Button = _scale_buttons[scale_value]
		button.set_pressed_no_signal(is_equal_approx(float(scale_value), _ui_scale))
	for high_contrast in _ghost_buttons:
		var ghost_button: Button = _ghost_buttons[high_contrast]
		var ghost_match := bool(high_contrast) == _high_contrast
		ghost_button.set_pressed_no_signal(ghost_match)
	if _reduced_motion_check != null:
		_reduced_motion_check.set_pressed_no_signal(_reduced_motion)
	if _mute_check != null:
		_mute_check.set_pressed_no_signal(_muted)


func _on_scale_pressed(scale: float) -> void:
	_ui_scale = scale
	_sync_widgets()
	save_settings()
	ui_scale_changed.emit(_ui_scale)


func _on_ghost_pressed(high_contrast: bool) -> void:
	_high_contrast = high_contrast
	_sync_widgets()
	save_settings()
	ghost_palette_changed.emit(_high_contrast)


func _on_reduced_motion_toggled(enabled: bool) -> void:
	_reduced_motion = enabled
	save_settings()
	reduced_motion_changed.emit(_reduced_motion)


func _on_mute_toggled(muted: bool) -> void:
	_muted = muted
	save_settings()
	mute_changed.emit(_muted)
