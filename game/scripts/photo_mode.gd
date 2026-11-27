extends CanvasLayer
## M3.1 photo mode overlay.
##
## A minimal, translucent bottom-centre bar that turns the live scene into a
## screenshot studio: time of day drives the sun and sky, FOV frames the castle,
## and Capture saves a clean PNG with the bar (plus HUD/agent panels, which the
## host hides) hidden for one rendered frame.
##
## This script never touches the sun or camera directly; it emits signals so
## `main.gd` stays the single owner of the lighting rig.

signal sun_time_changed(hour: float)
signal fov_changed(fov: float)
signal capture_requested
signal exit_requested

const DEFAULT_SUN_TIME := 16.5
const MIN_SUN_TIME := 6.0
const MAX_SUN_TIME := 20.5
const DEFAULT_FOV := 50.0
const MIN_FOV := 28.0
const MAX_FOV := 80.0
const PHOTO_DIR := "user://photos"

const BAR_BG := Color(0.05, 0.06, 0.07, 0.62)
const BAR_BORDER := Color(0.85, 0.78, 0.62, 0.30)
const TEXT := Color(0.92, 0.89, 0.80)
const ACCENT := Color(0.98, 0.82, 0.50)

var _bar: PanelContainer
var _sun_slider: HSlider
var _sun_label: Label
var _fov_slider: HSlider
var _fov_label: Label
var _capture_button: Button
var _capture_status: Label
var _last_capture_path := ""
var _sun_time := DEFAULT_SUN_TIME
var _fov := DEFAULT_FOV


func _init() -> void:
	layer = 30
	visible = false
	_build()


func _ready() -> void:
	if _bar == null:
		_build()


func _build() -> void:
	if _bar != null:
		return
	var root := Control.new()
	root.name = "PhotoModeRoot"
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	_bar = PanelContainer.new()
	_bar.name = "PhotoBar"
	_bar.set_anchors_preset(Control.PRESET_CENTER_BOTTOM)
	_bar.offset_left = -318.0
	_bar.offset_right = 318.0
	_bar.offset_top = -78.0
	_bar.offset_bottom = -18.0
	_bar.grow_horizontal = Control.GROW_DIRECTION_BOTH
	_bar.grow_vertical = Control.GROW_DIRECTION_BEGIN
	_bar.mouse_filter = Control.MOUSE_FILTER_STOP
	_bar.add_theme_stylebox_override("panel", _bar_style())
	root.add_child(_bar)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 14)
	margin.add_theme_constant_override("margin_right", 14)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)
	_bar.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 4)
	margin.add_child(column)

	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 10)
	column.add_child(row)

	_sun_label = _make_label("Sun 16:30")
	row.add_child(_sun_label)
	_sun_slider = _make_slider(MIN_SUN_TIME, MAX_SUN_TIME, 0.1, DEFAULT_SUN_TIME, 132.0)
	_sun_slider.value_changed.connect(_on_sun_slider_changed)
	row.add_child(_sun_slider)

	_fov_label = _make_label("FOV 50°")
	row.add_child(_fov_label)
	_fov_slider = _make_slider(MIN_FOV, MAX_FOV, 1.0, DEFAULT_FOV, 112.0)
	_fov_slider.value_changed.connect(_on_fov_slider_changed)
	row.add_child(_fov_slider)

	_capture_button = Button.new()
	_capture_button.text = "Capture PNG"
	_capture_button.focus_mode = Control.FOCUS_NONE
	_capture_button.custom_minimum_size = Vector2(106.0, 0.0)
	_capture_button.pressed.connect(_on_capture_pressed)
	row.add_child(_capture_button)

	var exit_button := Button.new()
	exit_button.text = "Exit (P)"
	exit_button.focus_mode = Control.FOCUS_NONE
	exit_button.pressed.connect(func() -> void: exit_requested.emit())
	row.add_child(exit_button)

	_capture_status = Label.new()
	_capture_status.text = "P hides the HUD · Esc exits"
	_capture_status.add_theme_font_size_override("font_size", 11)
	_capture_status.add_theme_color_override("font_color", Color(0.72, 0.70, 0.62))
	column.add_child(_capture_status)


func _make_label(text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.add_theme_font_size_override("font_size", 12)
	label.add_theme_color_override("font_color", TEXT)
	return label


func _make_slider(
	min_value: float, max_value: float, step: float, value: float, width: float
) -> HSlider:
	var slider := HSlider.new()
	slider.min_value = min_value
	slider.max_value = max_value
	slider.step = step
	slider.value = value
	slider.custom_minimum_size = Vector2(width, 0.0)
	slider.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	slider.focus_mode = Control.FOCUS_NONE
	return slider


func _bar_style() -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = BAR_BG
	style.border_color = BAR_BORDER
	style.set_border_width_all(1)
	style.set_corner_radius_all(8)
	style.content_margin_left = 6.0
	style.content_margin_right = 6.0
	return style


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

## Apply a time of day without emitting (used when entering photo mode and by
## tests). The slider and readout stay in sync.
func set_sun_time(hour: float) -> void:
	_sun_time = clampf(hour, MIN_SUN_TIME, MAX_SUN_TIME)
	if _sun_slider != null:
		_sun_slider.set_value_no_signal(_sun_time)
	_update_sun_label()
	_apply_sun_to_host()


func get_sun_time() -> float:
	return _sun_time


func set_fov(value: float) -> void:
	_fov = clampf(value, MIN_FOV, MAX_FOV)
	if _fov_slider != null:
		_fov_slider.set_value_no_signal(_fov)
	_update_fov_label()
	fov_changed.emit(_fov)


func get_fov() -> float:
	return _fov


func get_last_capture_path() -> String:
	return _last_capture_path


func is_visible_mode() -> bool:
	return visible


## Save one clean PNG of the current viewport. The photo bar is hidden for the
## frame that is read, and restored afterwards. Returns the path, or "" when the
## viewport cannot be captured (e.g. headless).
func capture_photo() -> String:
	if _bar != null:
		_bar.visible = false
	var path := _save_viewport_png()
	if _bar != null:
		_bar.visible = true
	_last_capture_path = path
	if _capture_status != null:
		if path.is_empty():
			_capture_status.text = "Capture unavailable in this display mode"
		else:
			_capture_status.text = "Saved %s" % path
	return path


# ---------------------------------------------------------------------------
# Internals
# ---------------------------------------------------------------------------

func _on_sun_slider_changed(value: float) -> void:
	_sun_time = value
	_update_sun_label()
	_apply_sun_to_host()


func _on_fov_slider_changed(value: float) -> void:
	_fov = value
	_update_fov_label()
	fov_changed.emit(_fov)


func _on_capture_pressed() -> void:
	# The host owns capture so it can hide the whole photo overlay for a full
	# frame before reading the viewport (M3.1 M8).
	capture_requested.emit()


func _apply_sun_to_host() -> void:
	sun_time_changed.emit(_sun_time)


func _update_sun_label() -> void:
	if _sun_label == null:
		return
	var hours := int(floor(_sun_time))
	var minutes := int(round((_sun_time - float(hours)) * 60.0))
	# Rounding can produce 60 minutes at e.g. 16.999; normalise it.
	if minutes >= 60:
		hours += 1
		minutes = 0
	_sun_label.text = "Sun %02d:%02d" % [hours, minutes]


func _update_fov_label() -> void:
	if _fov_label != null:
		_fov_label.text = "FOV %d°" % roundi(_fov)


func _save_viewport_png() -> String:
	var viewport := get_viewport()
	if viewport == null or viewport.get_texture() == null:
		return ""
	var image: Image = viewport.get_texture().get_image()
	if image == null or image.is_empty():
		return ""
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(PHOTO_DIR))
	var path := "%s/keepwright_%d_%d.png" % [
		PHOTO_DIR,
		int(Time.get_unix_time_from_system()),
		Time.get_ticks_msec(),
	]
	if image.save_png(path) != OK:
		return ""
	return path
