extends Node3D
## M0b construction theater FX, built entirely in code.
##
## Two procedural effects track the masonry shader's `build_front`:
##   1. Timber scaffolding (a single MultiMeshInstance3D) climbs with the
##      active lift and is struck cleanly once the build is complete.
##   2. Warm limestone dust (CPUParticles3D) rises along the active construction
##      lift only while the build is actually animating.
##
## Public API:
##   configure_bounds(aabb: AABB)          — masonry bounds in world space
##   set_build_front(front, is_playing)    — 0.0..1.0 reveal + play state
##
## No external assets: the scaffold is a unit BoxMesh scaled per instance and
## the dust is a procedural pixel-quad particle.

const MAX_INSTANCES := 128

const SCAFFOLD_OFFSET := 1.15      # metres outside the masonry footprint
const POLE_SIZE := 0.16
const LEDGER_HEIGHT := 0.14
const PLANK_DEPTH := 0.90
const LIFT_SPACING := 2.2
const STRIKE_THRESHOLD := 0.99

const POLE_COLOR := Color(0.34, 0.23, 0.13)
const LEDGER_COLOR := Color(0.52, 0.36, 0.20)
const PLANK_COLOR := Color(0.58, 0.43, 0.26)

var _bounds := AABB(Vector3(-12.0, 0.0, -12.0), Vector3(24.0, 12.0, 24.0))
var _has_bounds := false
var _front := 1.0
var _playing := false
var _last_lift_count := -1

var _mmi: MultiMeshInstance3D
var _multimesh: MultiMesh
var _dust: CPUParticles3D


func _ready() -> void:
	_ensure_built()
	_update_scaffold()
	_update_dust()


func _ensure_built() -> void:
	if _mmi == null:
		_build_scaffold_node()
	if _dust == null:
		_build_dust_node()


## Masonry bounds in world space. Called whenever a build is (re)generated.
func configure_bounds(aabb: AABB) -> void:
	_ensure_built()
	var normalised := aabb.abs()
	_has_bounds = (
		normalised.size.x > 0.01
		and normalised.size.z > 0.01
		and normalised.size.y > 0.01
	)
	_bounds = normalised
	_last_lift_count = -1
	_update_scaffold()
	_update_dust()


## Drive the theatre from the same 0..1 value as the masonry shader.
func set_build_front(front: float, is_playing: bool) -> void:
	_ensure_built()
	_front = clampf(front, 0.0, 1.0)
	_playing = is_playing
	_update_scaffold()
	_update_dust()


func is_scaffold_visible() -> bool:
	return _mmi != null and _mmi.visible and _multimesh.visible_instance_count > 0


func is_dust_emitting() -> bool:
	return _dust != null and _dust.emitting


# ---------------------------------------------------------------------------
# Scaffolding
# ---------------------------------------------------------------------------

func _build_scaffold_node() -> void:
	_multimesh = MultiMesh.new()
	# use_colors must be set before instance_count.
	_multimesh.transform_format = MultiMesh.TRANSFORM_3D
	_multimesh.use_colors = true

	var box := BoxMesh.new()
	box.size = Vector3.ONE
	_multimesh.mesh = box
	_multimesh.instance_count = MAX_INSTANCES
	_multimesh.visible_instance_count = 0

	var material := StandardMaterial3D.new()
	material.albedo_color = Color(1.0, 1.0, 1.0)
	material.vertex_color_use_as_albedo = true
	material.roughness = 0.92
	material.metallic = 0.0

	_mmi = MultiMeshInstance3D.new()
	_mmi.name = "Scaffolding"
	_mmi.multimesh = _multimesh
	_mmi.material_override = material
	_mmi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	_mmi.visible = false
	add_child(_mmi)


func _update_scaffold() -> void:
	if _multimesh == null or _mmi == null:
		return

	# Completed build: strike the scaffolding (also covers "raise instantly").
	if not _has_bounds or _front >= STRIKE_THRESHOLD:
		_mmi.visible = false
		_multimesh.visible_instance_count = 0
		_last_lift_count = -1
		return

	var climb := maxf(_bounds.size.y * _front, 0.8)
	var lifts := maxi(1, int(ceil(climb / LIFT_SPACING)))
	if lifts == _last_lift_count and _mmi.visible:
		return
	_last_lift_count = lifts

	var base_y := _bounds.position.y - 0.20
	var top_y := base_y + climb + 1.10
	var x0 := _bounds.position.x - SCAFFOLD_OFFSET
	var x1 := _bounds.end.x + SCAFFOLD_OFFSET
	var z0 := _bounds.position.z - SCAFFOLD_OFFSET
	var z1 := _bounds.end.z + SCAFFOLD_OFFSET

	var xs := _edge_positions(x0, x1)
	var zs := _edge_positions(z0, z1)

	var index := 0

	# Vertical standards at both long edges and the two short edges (corner
	# positions are shared with the long edges, so skip the end entries).
	for x in xs:
		index = _add_pole(x, z0, base_y, top_y, index)
		index = _add_pole(x, z1, base_y, top_y, index)
	for i in range(1, zs.size() - 1):
		var z := zs[i]
		index = _add_pole(x0, z, base_y, top_y, index)
		index = _add_pole(x1, z, base_y, top_y, index)

	# Ledgers and plank decks follow each lift the masons have reached.
	for lift in range(1, lifts + 1):
		var y: float = base_y + minf(float(lift) * LIFT_SPACING, climb + 0.35)
		var mid_x := (x0 + x1) * 0.5
		var mid_z := (z0 + z1) * 0.5
		index = _add_box(
			Vector3(mid_x, y, z0), Vector3(x1 - x0, LEDGER_HEIGHT, PLANK_DEPTH),
			PLANK_COLOR, index
		)
		index = _add_box(
			Vector3(mid_x, y, z1), Vector3(x1 - x0, LEDGER_HEIGHT, PLANK_DEPTH),
			PLANK_COLOR, index
		)
		index = _add_box(
			Vector3(x0, y, mid_z), Vector3(PLANK_DEPTH, LEDGER_HEIGHT, z1 - z0),
			PLANK_COLOR, index
		)
		index = _add_box(
			Vector3(x1, y, mid_z), Vector3(PLANK_DEPTH, LEDGER_HEIGHT, z1 - z0),
			PLANK_COLOR, index
		)

	_multimesh.visible_instance_count = index
	_mmi.visible = index > 0


func _edge_positions(a: float, b: float) -> Array[float]:
	var span := b - a
	var segments := clampi(int(ceil(span / 4.0)), 2, 7)
	var out: Array[float] = []
	for i in range(segments + 1):
		out.append(a + span * float(i) / float(segments))
	return out


func _add_pole(x: float, z: float, base_y: float, top_y: float, index: int) -> int:
	var height := top_y - base_y
	if height <= 0.05:
		return index
	var center := Vector3(x, base_y + height * 0.5, z)
	return _add_box(center, Vector3(POLE_SIZE, height, POLE_SIZE), POLE_COLOR, index)


func _add_box(center: Vector3, size: Vector3, color: Color, index: int) -> int:
	if index >= MAX_INSTANCES:
		return index
	_multimesh.set_instance_transform(index, Transform3D(Basis.from_scale(size), center))
	_multimesh.set_instance_color(index, color)
	return index + 1


# ---------------------------------------------------------------------------
# Rising limestone dust
# ---------------------------------------------------------------------------

func _build_dust_node() -> void:
	var quad := QuadMesh.new()
	quad.size = Vector2(0.75, 0.75)

	var material := StandardMaterial3D.new()
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.billboard_mode = BaseMaterial3D.BILLBOARD_ENABLED
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.vertex_color_use_as_albedo = true
	material.albedo_color = Color(1.0, 1.0, 1.0)
	material.no_depth_test = false
	quad.material = material

	_dust = CPUParticles3D.new()
	_dust.name = "MasonryDust"
	_dust.mesh = quad
	_dust.amount = 28
	_dust.lifetime = 2.6
	_dust.local_coords = true
	_dust.emission_shape = CPUParticles3D.EMISSION_SHAPE_BOX
	_dust.emission_box_extents = Vector3(8.0, 0.30, 8.0)
	_dust.direction = Vector3(0.0, 1.0, 0.0)
	_dust.spread = 38.0
	_dust.flatness = 1.0
	_dust.gravity = Vector3(0.0, 0.22, 0.0)
	_dust.initial_velocity_min = 0.25
	_dust.initial_velocity_max = 0.85
	_dust.angular_velocity_min = -20.0
	_dust.angular_velocity_max = 20.0
	_dust.scale_amount_min = 0.30
	_dust.scale_amount_max = 0.85
	_dust.color = Color(0.83, 0.77, 0.64, 0.38)
	_dust.emitting = false
	add_child(_dust)


func _update_dust() -> void:
	if _dust == null:
		return
	var active := (
		_has_bounds
		and _playing
		and _front > 0.01
		and _front < STRIKE_THRESHOLD
	)
	_dust.emitting = active
	if not _has_bounds:
		return

	var lift_y := _bounds.position.y + clampf(_bounds.size.y * _front, 0.4, _bounds.size.y)
	var center := _bounds.get_center()
	_dust.position = Vector3(center.x, lift_y, center.z)
	_dust.emission_box_extents = Vector3(
		maxf(_bounds.size.x * 0.45, 0.8),
		0.35,
		maxf(_bounds.size.z * 0.45, 0.8)
	)
