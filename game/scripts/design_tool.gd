extends Node3D
## M1 interactive design tool overlay, built entirely in code.
##
## Instant-feedback drafting for curtain walls, round/shaped towers,
## gatehouses and ditches. Ground picking and endpoint snapping are delegated
## to KeepBridge when it is available, so the ghost geometry follows the real
## site terrain and latches onto existing tower centres/perimeters and wall
## ends. Without a bridge the tool falls back to a flat ground plane and a 1 m
## grid.
##
## Modes:
##   NONE         — camera only, no interaction
##   WALL         — LMB drag from A to B: translucent ghost prism + readout
##   TOWER        — LMB click places a round tower
##   TOWER_D      — LMB click places a D-shaped tower
##   TOWER_SQUARE — LMB click places a square tower
##   GATEHOUSE    — LMB click places a twin-towered gatehouse
##   DITCH        — LMB drag carves a linear trench preview
##
## RMB and MMB are deliberately untouched so the orbit camera keeps working.

enum Mode { NONE, WALL, TOWER, TOWER_D, TOWER_SQUARE, GATEHOUSE, DITCH }

signal wall_committed(start_m: Vector2, end_m: Vector2, height_m: float, thickness_m: float)
signal tower_committed(center_m: Vector2, radius_m: float, height_m: float)
signal tower_shaped_committed(
	center_m: Vector2, radius_m: float, height_m: float, plan: String, orientation_deg: int
)
signal gatehouse_committed(center_m: Vector2, orientation_deg: int)
signal ditch_committed(start_m: Vector2, end_m: Vector2, width_m: float, depth_m: float)
signal snap_feedback()

const SNAP_STEP := 1.0
const SNAP_RADIUS_M := 2.5
const MIN_WALL_LENGTH := 1.5
const MIN_DITCH_LENGTH := 1.5

const GHOST_WALL_COLOR := Color(0.96, 0.84, 0.58, 0.28)
const GHOST_TOWER_COLOR := Color(0.92, 0.82, 0.62, 0.24)
const GHOST_GATEHOUSE_COLOR := Color(0.90, 0.78, 0.56, 0.26)
const GHOST_DITCH_COLOR := Color(0.62, 0.52, 0.36, 0.34)
const RING_COLOR := Color(1.0, 0.84, 0.38, 0.90)

const GATEHOUSE_HEIGHT := 14.0
const GATEHOUSE_DEPTH := 8.0
const GATEHOUSE_TOWER_RADIUS := 3.4
const GATEHOUSE_PASSAGE_WIDTH := 3.2

var _mode: int = Mode.NONE
var _ground_y := 0.0
var _bridge: Object = null

var _wall_height := 8.0
var _wall_thickness := 2.0
var _tower_radius := 3.5
var _tower_height := 12.0
var _ditch_width := 6.0
var _ditch_depth := 2.5

var _dragging := false
var _drag_mode: int = Mode.NONE
var _has_hover := false
var _snap_active := false
var _last_snap_key := ""
var _wall_start := Vector3.ZERO
var _wall_end := Vector3.ZERO
var _hover := Vector3.ZERO

var _wall_ghost: MeshInstance3D
var _tower_ghost: MeshInstance3D
var _tower_box_ghost: MeshInstance3D
var _gatehouse_ghost: MeshInstance3D
var _ditch_ghost: MeshInstance3D
var _snap_ring: MeshInstance3D
var _readout: Label3D


func _ready() -> void:
	_build_visuals()
	set_process(true)
	set_process_unhandled_input(true)
	_update_preview()


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

func set_mode(mode: int) -> void:
	_mode = mode
	_dragging = false
	_drag_mode = Mode.NONE
	_has_hover = false
	_snap_active = false
	_last_snap_key = ""
	_update_preview()


func get_mode() -> int:
	return _mode


func set_bridge(bridge: Object) -> void:
	_bridge = bridge
	_update_preview()


func set_ground_height(height: float) -> void:
	_ground_y = height
	_update_preview()


func set_defaults(
	wall_height: float,
	wall_thickness: float,
	tower_radius: float,
	tower_height: float,
	ditch_width: float = 6.0,
	ditch_depth: float = 2.5
) -> void:
	_wall_height = maxf(wall_height, 1.0)
	_wall_thickness = maxf(wall_thickness, 0.2)
	_tower_radius = maxf(tower_radius, 0.8)
	_tower_height = maxf(tower_height, 1.0)
	_ditch_width = maxf(ditch_width, 0.5)
	_ditch_depth = maxf(ditch_depth, 0.1)
	_update_preview()


func is_dragging() -> bool:
	return _dragging


# ---------------------------------------------------------------------------
# Input
# ---------------------------------------------------------------------------

func _process(_delta: float) -> void:
	if _mode == Mode.NONE:
		return
	var point: Variant = _point_on_ground(get_viewport().get_mouse_position())
	if point == null:
		_has_hover = false
		_snap_active = false
		_last_snap_key = ""
		_update_preview()
		return
	_hover = _snap_point(point)
	_has_hover = true
	if _dragging:
		_wall_end = _hover
	_update_preview()


func _unhandled_input(event: InputEvent) -> void:
	if _mode == Mode.NONE:
		return
	if not (event is InputEventMouseButton):
		return

	var mouse := event as InputEventMouseButton
	if mouse.button_index != MOUSE_BUTTON_LEFT:
		return

	if mouse.pressed:
		var point: Variant = _point_on_ground(mouse.position)
		if point == null:
			return
		var snapped := _snap_point(point)
		_hover = snapped
		_has_hover = true

		if _mode == Mode.WALL or _mode == Mode.DITCH:
			_wall_start = snapped
			_wall_end = snapped
			_dragging = true
			_drag_mode = _mode
		elif _mode == Mode.GATEHOUSE:
			_commit_gatehouse(snapped)
		else:
			_commit_tower(snapped)

		_update_preview()
		get_viewport().set_input_as_handled()
	elif _dragging:
		_dragging = false
		match _drag_mode:
			Mode.WALL:
				_commit_wall()
			Mode.DITCH:
				_commit_ditch()
			_:
				pass
		_drag_mode = Mode.NONE
		_update_preview()
		get_viewport().set_input_as_handled()


func _commit_wall() -> void:
	var delta := Vector2(_wall_end.x - _wall_start.x, _wall_end.z - _wall_start.z)
	if delta.length() < MIN_WALL_LENGTH:
		return
	wall_committed.emit(
		Vector2(_wall_start.x, _wall_start.z),
		Vector2(_wall_end.x, _wall_end.z),
		_wall_height,
		_wall_thickness
	)


func _commit_ditch() -> void:
	var delta := Vector2(_wall_end.x - _wall_start.x, _wall_end.z - _wall_start.z)
	if delta.length() < MIN_DITCH_LENGTH:
		return
	ditch_committed.emit(
		Vector2(_wall_start.x, _wall_start.z),
		Vector2(_wall_end.x, _wall_end.z),
		_ditch_width,
		_ditch_depth
	)


func _commit_tower(center: Vector3) -> void:
	var plan := "round"
	if _mode == Mode.TOWER_D:
		plan = "d_shaped"
	elif _mode == Mode.TOWER_SQUARE:
		plan = "square"
	if plan == "round":
		tower_committed.emit(Vector2(center.x, center.z), _tower_radius, _tower_height)
	else:
		tower_shaped_committed.emit(
			Vector2(center.x, center.z),
			_tower_radius,
			_tower_height,
			plan,
			_plan_orientation()
		)


func _commit_gatehouse(center: Vector3) -> void:
	gatehouse_committed.emit(Vector2(center.x, center.z), _plan_orientation())


## Gatehouse/tower plan rotation, aligned with the camera's view direction so a
## placed building faces the player.
func _plan_orientation() -> int:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return 0
	var forward := -camera.global_transform.basis.z
	return int(round(rad_to_deg(atan2(forward.z, forward.x))))


# ---------------------------------------------------------------------------
# Ground picking
# ---------------------------------------------------------------------------

func _point_on_ground(screen_position: Vector2) -> Variant:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return null
	var from := camera.project_ray_origin(screen_position)
	var direction := camera.project_ray_normal(screen_position)

	# Prefer the core's terrain ray-march so the ghost sits on the real
	# promontory, including the falling cliff and sloping shoulders.
	if _bridge != null and _bridge.has_method("pick_terrain"):
		var hit: Variant = _bridge.call("pick_terrain", from, direction)
		if hit is Dictionary:
			var data: Dictionary = hit
			if bool(data.get("hit", false)):
				var picked: Variant = data.get("point", null)
				if picked is Vector3:
					return picked

	if absf(direction.y) < 0.00001:
		return null
	var distance := (_ground_y - from.y) / direction.y
	if distance <= 0.0 or distance > 5000.0:
		return null
	return from + direction * distance


func _snap_point(point: Vector3) -> Vector3:
	if _bridge != null and _bridge.has_method("snap_point"):
		var result: Variant = _bridge.call(
			"snap_point", Vector2(point.x, point.z), SNAP_RADIUS_M
		)
		if result is Dictionary:
			var data: Dictionary = result
			var snapped := bool(data.get("snapped", false))
			var kind := str(data.get("kind", ""))
			var element_id := int(data.get("element_id", -1))
			var plan: Variant = data.get("point", Vector2(point.x, point.z))
			var snapped_xz: Vector2 = (
				plan if plan is Vector2 else Vector2(point.x, point.z)
			)
			_snap_active = snapped
			# Key on the snapped *target*, not the cursor position, so the thud
			# fires once when latching onto a new element and stays silent while
			# the cursor slides across it.
			var key := ""
			if snapped:
				key = "%s:%d" % [kind, element_id]
			if snapped and key != _last_snap_key:
				snap_feedback.emit()
			_last_snap_key = key
			return Vector3(snapped_xz.x, point.y, snapped_xz.y)

	_snap_active = false
	_last_snap_key = ""
	return Vector3(
		roundf(point.x / SNAP_STEP) * SNAP_STEP,
		_ground_y + 0.02,
		roundf(point.z / SNAP_STEP) * SNAP_STEP
	)


# ---------------------------------------------------------------------------
# Preview visuals
# ---------------------------------------------------------------------------

func _build_visuals() -> void:
	var box := BoxMesh.new()
	box.size = Vector3.ONE
	_wall_ghost = MeshInstance3D.new()
	_wall_ghost.name = "WallGhost"
	_wall_ghost.mesh = box
	_wall_ghost.material_override = _ghost_material(GHOST_WALL_COLOR)
	_wall_ghost.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_wall_ghost.visible = false
	add_child(_wall_ghost)

	var cylinder := CylinderMesh.new()
	cylinder.top_radius = 1.0
	cylinder.bottom_radius = 1.0
	cylinder.height = 1.0
	cylinder.radial_segments = 32
	_tower_ghost = MeshInstance3D.new()
	_tower_ghost.name = "TowerGhost"
	_tower_ghost.mesh = cylinder
	_tower_ghost.material_override = _ghost_material(GHOST_TOWER_COLOR)
	_tower_ghost.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_tower_ghost.visible = false
	add_child(_tower_ghost)

	var box_tower := BoxMesh.new()
	box_tower.size = Vector3.ONE
	_tower_box_ghost = MeshInstance3D.new()
	_tower_box_ghost.name = "ShapedTowerGhost"
	_tower_box_ghost.mesh = box_tower
	_tower_box_ghost.material_override = _ghost_material(GHOST_TOWER_COLOR)
	_tower_box_ghost.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_tower_box_ghost.visible = false
	add_child(_tower_box_ghost)

	var gatehouse_box := BoxMesh.new()
	gatehouse_box.size = Vector3.ONE
	_gatehouse_ghost = MeshInstance3D.new()
	_gatehouse_ghost.name = "GatehouseGhost"
	_gatehouse_ghost.mesh = gatehouse_box
	_gatehouse_ghost.material_override = _ghost_material(GHOST_GATEHOUSE_COLOR)
	_gatehouse_ghost.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_gatehouse_ghost.visible = false
	add_child(_gatehouse_ghost)

	var ditch_box := BoxMesh.new()
	ditch_box.size = Vector3.ONE
	_ditch_ghost = MeshInstance3D.new()
	_ditch_ghost.name = "DitchGhost"
	_ditch_ghost.mesh = ditch_box
	_ditch_ghost.material_override = _ghost_material(GHOST_DITCH_COLOR)
	_ditch_ghost.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_ditch_ghost.visible = false
	add_child(_ditch_ghost)

	var torus := TorusMesh.new()
	torus.inner_radius = 0.90
	torus.outer_radius = 1.0
	torus.rings = 48
	torus.ring_segments = 8
	_snap_ring = MeshInstance3D.new()
	_snap_ring.name = "SnapRing"
	_snap_ring.mesh = torus
	_snap_ring.material_override = _ghost_material(RING_COLOR)
	_snap_ring.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	# Godot 4's TorusMesh already lies flat in the XZ plane; rotating it 90° on
	# X would stand the ring upright.
	_snap_ring.visible = false
	add_child(_snap_ring)

	_readout = Label3D.new()
	_readout.name = "Readout"
	_readout.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	_readout.no_depth_test = true
	_readout.font_size = 48
	_readout.pixel_size = 0.0022
	_readout.outline_size = 10
	_readout.modulate = Color(1.0, 0.93, 0.72)
	_readout.visible = false
	add_child(_readout)


func _ghost_material(color: Color) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	material.no_depth_test = false
	return material


func _update_preview() -> void:
	if _wall_ghost == null:
		return

	var wall_mode := _mode == Mode.WALL
	var ditch_mode := _mode == Mode.DITCH
	var tower_mode := (
		_mode == Mode.TOWER or _mode == Mode.TOWER_D or _mode == Mode.TOWER_SQUARE
	)
	var gatehouse_mode := _mode == Mode.GATEHOUSE

	var wall_active := wall_mode and _has_hover and _dragging
	var ditch_active := ditch_mode and _has_hover and _dragging
	var tower_active := tower_mode and _has_hover
	var gatehouse_active := gatehouse_mode and _has_hover

	_wall_ghost.visible = false
	_tower_ghost.visible = false
	_tower_box_ghost.visible = false
	_gatehouse_ghost.visible = false
	_ditch_ghost.visible = false
	_snap_ring.visible = false
	_readout.visible = false

	if wall_active:
		var delta := Vector2(_wall_end.x - _wall_start.x, _wall_end.z - _wall_start.z)
		var length := delta.length()
		if length >= 0.2:
			var yaw := atan2(delta.x, delta.y)
			var center := (_wall_start + _wall_end) * 0.5
			# Stand the ghost on the picked ground surface, not the masonry
			# foundation level, so it does not float above or sink into terrain.
			center.y = (_wall_start.y + _wall_end.y) * 0.5 + _wall_height * 0.5
			var basis := Basis(Vector3.UP, yaw) * Basis.from_scale(
				Vector3(_wall_thickness, _wall_height, length)
			)
			_wall_ghost.transform = Transform3D(basis, center)
			_wall_ghost.visible = true

			_readout.text = "Wall  %.1f m   ·   H %.1f m   ·   T %.1f m" % [
				length, _wall_height, _wall_thickness
			]
			_readout.position = center + Vector3(0.0, _wall_height * 0.5 + 1.1, 0.0)
			_readout.visible = true
	elif ditch_active:
		var delta := Vector2(_wall_end.x - _wall_start.x, _wall_end.z - _wall_start.z)
		var length := delta.length()
		if length >= 0.2:
			var yaw := atan2(delta.x, delta.y)
			var center := (_wall_start + _wall_end) * 0.5
			center.y = (_wall_start.y + _wall_end.y) * 0.5 - _ditch_depth * 0.5 + 0.05
			var basis := Basis(Vector3.UP, yaw) * Basis.from_scale(
				Vector3(_ditch_width, _ditch_depth, length)
			)
			_ditch_ghost.transform = Transform3D(basis, center)
			_ditch_ghost.visible = true

			_readout.text = "Ditch  %.1f m   ·   W %.1f m   ·   D %.1f m" % [
				length, _ditch_width, _ditch_depth
			]
			_readout.position = center + Vector3(0.0, 1.2, 0.0)
			_readout.visible = true
	elif tower_active:
		var shaped := _mode != Mode.TOWER
		if shaped:
			_tower_box_ghost.position = _hover + Vector3(0.0, _tower_height * 0.5, 0.0)
			_tower_box_ghost.scale = Vector3(
				_tower_radius * 2.0, _tower_height, _tower_radius * 2.0
			)
			_tower_box_ghost.rotation = Vector3(0.0, -deg_to_rad(float(_plan_orientation())), 0.0)
			_tower_box_ghost.visible = true
		else:
			_tower_ghost.position = _hover + Vector3(0.0, _tower_height * 0.5, 0.0)
			_tower_ghost.scale = Vector3(_tower_radius, _tower_height, _tower_radius)
			_tower_ghost.visible = true

		var plan_name := "Round tower"
		if _mode == Mode.TOWER_D:
			plan_name = "D-tower"
		elif _mode == Mode.TOWER_SQUARE:
			plan_name = "Square tower"
		_readout.text = "%s   r %.1f m   ·   H %.1f m" % [
			plan_name, _tower_radius, _tower_height
		]
		_readout.position = _hover + Vector3(0.0, _tower_height + 1.4, 0.0)
		_readout.visible = true
	elif gatehouse_active:
		var yaw := -deg_to_rad(float(_plan_orientation()))
		var half_width := GATEHOUSE_PASSAGE_WIDTH * 0.5 + GATEHOUSE_TOWER_RADIUS + 0.1
		var total_width := half_width * 2.0 + GATEHOUSE_TOWER_RADIUS * 2.0
		var center := _hover + Vector3(0.0, GATEHOUSE_HEIGHT * 0.5, 0.0)
		var basis := Basis(Vector3.UP, yaw) * Basis.from_scale(
			Vector3(GATEHOUSE_DEPTH, GATEHOUSE_HEIGHT, total_width)
		)
		_gatehouse_ghost.transform = Transform3D(basis, center)
		_gatehouse_ghost.visible = true

		_readout.text = "Gatehouse  %.1f m wide   ·   H %.1f m" % [
			total_width, GATEHOUSE_HEIGHT
		]
		_readout.position = _hover + Vector3(0.0, GATEHOUSE_HEIGHT + 1.4, 0.0)
		_readout.visible = true

	# Show the snap ring on hover as well as during a drag, so the user sees the
	# latch target before pressing LMB.
	if _snap_active and _has_hover and _mode != Mode.NONE:
		_snap_ring.position = _hover + Vector3(0.0, 0.06, 0.0)
		_snap_ring.scale = Vector3.ONE * (_tower_radius if tower_mode else 1.1)
		_snap_ring.visible = true
