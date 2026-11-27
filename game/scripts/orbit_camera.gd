extends Camera3D
## Smooth orbit/pan/zoom camera for the M0a look spike.
##
## Right-drag or middle-drag orbits, Shift+drag pans, the mouse wheel zooms and
## WASD walks the focus point across the ground plane. All motion is damped.
## It also exposes a helper that retunes the directional-light shadow cascades
## from camera altitude, so panoramas and close-ups both get crisp shadows.

@export var orbit_sensitivity := 0.0045
@export var pan_sensitivity := 1.0
@export var zoom_step := 0.86
@export var smoothing := 10.0
@export var min_distance := 2.0
@export var max_distance := 600.0
@export var min_pitch := -0.10
@export var max_pitch := 1.45
@export var keyboard_pan_speed := 0.9
## Radians per second for Q/E yaw and R/F pitch keyboard orbit.
@export var keyboard_orbit_speed := 1.5
@export var keyboard_pitch_speed := 1.1
## Exponential zoom rate for the +/- keys (distance e-folds per second).
@export var keyboard_zoom_rate := 1.6
## W/A/S/D and arrow pan speed multiplier while Shift is held.
@export var keyboard_boost := 2.5

const HOME_YAW := 0.65
const HOME_PITCH := 0.34
const HOME_DISTANCE := 90.0
const HOME_FOCUS := Vector3(0.0, 8.0, 0.0)

var _yaw := HOME_YAW
var _pitch := HOME_PITCH
var _distance := HOME_DISTANCE
var _focus := HOME_FOCUS

var _target_yaw := _yaw
var _target_pitch := _pitch
var _target_distance := _distance
var _target_focus := _focus

# Remembered site/castle framing for Home / reset_view().
var _home_focus := HOME_FOCUS
var _home_distance := HOME_DISTANCE

var _drag_mode := 0 # 0 = none, 1 = orbit, 2 = pan
var _drag_button := 0

# Shadow cascades are only retuned when the view changes enough to matter;
# otherwise tiny per-frame altitude changes make the shadow map shimmer.
# A 15% hysteresis plus coarse 25 m span quantisation means orbiting and
# zooming never make the cascade boundaries pop or resample every frame.
const SHADOW_UPDATE_THRESHOLD := 0.15
const SHADOW_SPAN_STEP := 25.0
var _last_shadow_altitude := -1.0
var _last_shadow_distance := -1.0
var _last_shadow_span := -1.0


func _ready() -> void:
	_sync_immediate()


## Current (damped) orbit distance in metres; used by zoom-reactive audio.
func get_view_distance() -> float:
	return _distance


func focus_on(point: Vector3, distance := -1.0) -> void:
	_target_focus = point
	_home_focus = point
	if distance > 0.0:
		_target_distance = clampf(distance, min_distance, max_distance)
		_home_distance = _target_distance
	elif _target_distance <= 0.0:
		_target_distance = HOME_DISTANCE
		_home_distance = _target_distance


## Re-frame the camera to the most recent site/castle framing and the default
## orbit orientation. Bound to Home.
func reset_view() -> void:
	_target_focus = _home_focus
	_target_distance = _home_distance
	_target_yaw = HOME_YAW
	_target_pitch = HOME_PITCH


func _keyboard_allowed() -> bool:
	var viewport := get_viewport()
	if viewport == null:
		return true
	# Never steal movement keys from a focused text field, slider or button.
	return viewport.gui_get_focus_owner() == null


func _sync_immediate() -> void:
	_yaw = _target_yaw
	_pitch = _target_pitch
	_distance = _target_distance
	_focus = _target_focus
	_apply_transform()


func _process(delta: float) -> void:
	if _keyboard_allowed():
		_handle_keyboard_navigation(delta)

	var t := 1.0 - exp(-smoothing * delta)
	_yaw = lerp_angle(_yaw, _target_yaw, t)
	_pitch = lerpf(_pitch, _target_pitch, t)
	_distance = lerpf(_distance, _target_distance, t)
	_focus = _focus.lerp(_target_focus, t)
	_apply_transform()


func _apply_transform() -> void:
	var direction := Vector3(
		cos(_pitch) * sin(_yaw),
		sin(_pitch),
		cos(_pitch) * cos(_yaw)
	)
	global_position = _focus + direction * _distance
	look_at(_focus, Vector3.UP)


func _handle_keyboard_navigation(delta: float) -> void:
	_handle_keyboard_pan(delta)
	_handle_keyboard_orbit(delta)
	_handle_keyboard_zoom(delta)


func _handle_keyboard_pan(delta: float) -> void:
	var x := 0.0
	var z := 0.0
	if Input.is_key_pressed(KEY_A) or Input.is_key_pressed(KEY_LEFT):
		x -= 1.0
	if Input.is_key_pressed(KEY_D) or Input.is_key_pressed(KEY_RIGHT):
		x += 1.0
	if Input.is_key_pressed(KEY_W) or Input.is_key_pressed(KEY_UP):
		z += 1.0
	if Input.is_key_pressed(KEY_S) or Input.is_key_pressed(KEY_DOWN):
		z -= 1.0
	if is_zero_approx(x) and is_zero_approx(z):
		return

	var right := Vector3(cos(_yaw), 0.0, -sin(_yaw))
	var forward := Vector3(-sin(_yaw), 0.0, -cos(_yaw))
	var speed := maxf(_distance, 5.0) * keyboard_pan_speed * delta
	if Input.is_key_pressed(KEY_SHIFT):
		speed *= keyboard_boost
	_target_focus += (right * x + forward * z) * speed


func _handle_keyboard_orbit(delta: float) -> void:
	var yaw_input := 0.0
	if Input.is_key_pressed(KEY_Q):
		yaw_input += 1.0
	if Input.is_key_pressed(KEY_E):
		yaw_input -= 1.0
	_target_yaw += yaw_input * keyboard_orbit_speed * delta

	var pitch_input := 0.0
	if Input.is_key_pressed(KEY_R):
		pitch_input += 1.0
	if Input.is_key_pressed(KEY_F):
		pitch_input -= 1.0
	if not is_zero_approx(pitch_input):
		_target_pitch = clampf(
			_target_pitch + pitch_input * keyboard_pitch_speed * delta,
			min_pitch,
			max_pitch
		)


func _handle_keyboard_zoom(delta: float) -> void:
	var zoom := 0.0
	if (
		Input.is_key_pressed(KEY_EQUAL)
		or Input.is_key_pressed(KEY_PLUS)
		or Input.is_key_pressed(KEY_KP_ADD)
	):
		zoom -= 1.0
	if Input.is_key_pressed(KEY_MINUS) or Input.is_key_pressed(KEY_KP_SUBTRACT):
		zoom += 1.0
	if is_zero_approx(zoom):
		return
	_target_distance = clampf(
		_target_distance * exp(zoom * keyboard_zoom_rate * delta),
		min_distance,
		max_distance
	)


func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventKey:
		var key := event as InputEventKey
		if (
			key.pressed
			and not key.echo
			and key.keycode == KEY_HOME
			and _keyboard_allowed()
		):
			reset_view()
			get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseButton:
		var mouse_button := event as InputEventMouseButton
		if mouse_button.pressed:
			if mouse_button.button_index == MOUSE_BUTTON_WHEEL_UP:
				_target_distance = clampf(_target_distance * zoom_step, min_distance, max_distance)
				return
			if mouse_button.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				_target_distance = clampf(_target_distance / zoom_step, min_distance, max_distance)
				return

			var supported := (
				mouse_button.button_index == MOUSE_BUTTON_RIGHT
				or mouse_button.button_index == MOUSE_BUTTON_MIDDLE
				or mouse_button.button_index == MOUSE_BUTTON_LEFT
			)
			if supported:
				_drag_mode = 2 if mouse_button.shift_pressed else 1
				if _drag_mode == 1 and mouse_button.button_index == MOUSE_BUTTON_LEFT:
					_drag_mode = 0
				_drag_button = mouse_button.button_index
		elif mouse_button.button_index == _drag_button:
			_drag_mode = 0
			_drag_button = 0
		return

	if event is InputEventMouseMotion and _drag_mode != 0:
		var motion := event as InputEventMouseMotion
		if _drag_mode == 1:
			_target_yaw -= motion.relative.x * orbit_sensitivity
			_target_pitch = clampf(_target_pitch - motion.relative.y * orbit_sensitivity, min_pitch, max_pitch)
		else:
			_pan_screen(motion.relative)


func _pan_screen(relative: Vector2) -> void:
	var pan_scale := maxf(_distance, 5.0) * 0.0018 * pan_sensitivity
	var right := Vector3(cos(_yaw), 0.0, -sin(_yaw))
	var forward := Vector3(-sin(_yaw), 0.0, -cos(_yaw))
	_target_focus += (-right * relative.x - forward * relative.y) * pan_scale


## Camera-altitude-reactive shadow tuning. Wide vistas push the cascades out,
## while low, close inspection keeps them tight and crisp.
##
## Retuning is deliberately rare: we only touch the light when the view moved
## by more than 15% since the last update, and the shadow span itself is
## snapped to 25 m steps (splits derive from that span). This keeps cascade
## boundaries stable through an entire orbit/zoom gesture.
func update_shadow_splits(sun: DirectionalLight3D) -> void:
	if sun == null:
		return
	var altitude := maxf(global_position.y - 1.0, 0.0)

	# Debounce: outside a settled view, bail out unless altitude or distance
	# moved more than 15% since the last retune. Once the zoom settles we allow
	# one retune, but the quantised span below means it only actually writes
	# when a 25 m step was crossed. This eliminates shadow-map shimmer while
	# zooming gently and keeps cascades fixed through an orbit.
	var settled := absf(_distance - _target_distance) <= maxf(0.25, _target_distance * 0.01)
	if _last_shadow_altitude >= 0.0 and not settled:
		var altitude_delta := absf(altitude - _last_shadow_altitude) / maxf(_last_shadow_altitude, 1.0)
		var distance_delta := absf(_distance - _last_shadow_distance) / maxf(_last_shadow_distance, 1.0)
		if altitude_delta < SHADOW_UPDATE_THRESHOLD and distance_delta < SHADOW_UPDATE_THRESHOLD:
			return

	# Coarse quantisation: the shadow distance only ever takes 25 m steps, so
	# slow directional changes cannot resample the shadow map every frame.
	var raw_span := clampf(80.0 + altitude * 7.0 + _distance * 0.8, 80.0, 700.0)
	var span := clampf(roundf(raw_span / SHADOW_SPAN_STEP) * SHADOW_SPAN_STEP, 75.0, 700.0)
	if is_equal_approx(span, _last_shadow_span):
		return

	_last_shadow_altitude = altitude
	_last_shadow_distance = _distance
	_last_shadow_span = span

	sun.directional_shadow_max_distance = span

	# Splits are a pure function of the quantised span, so they only change
	# when the span steps — never per frame.
	var near := clampf(span * 0.12, 10.0, 48.0)
	sun.directional_shadow_split_1 = clampf(near / span, 0.05, 0.30)
	sun.directional_shadow_split_2 = clampf((near * 3.0) / span, 0.15, 0.65)
	sun.directional_shadow_split_3 = clampf((near * 7.0) / span, 0.35, 0.92)
