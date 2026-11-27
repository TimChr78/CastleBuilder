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
##   configure_elements(elements, fallback_bounds) — per-wall/tower footprints
##   configure_bounds(aabb: AABB)                  — legacy rectangular fallback
##   set_build_front(front, is_playing)            — 0.0..1.0 reveal + play state
##
## No external assets: the scaffold is a unit BoxMesh scaled per instance and
## the dust is a procedural pixel-quad particle.

const MAX_INSTANCES := 2048

const WALL_SCAFFOLD_OFFSET := 0.75   # metres beyond each wall face
const TOWER_SCAFFOLD_OFFSET := 0.85  # metres beyond the tower perimeter
const SCAFFOLD_OFFSET := 1.15        # legacy rectangular fallback offset
const POLE_SIZE := 0.16
const LEDGER_HEIGHT := 0.14
const PLANK_DEPTH := 0.90
const LIFT_SPACING := 2.2
const STRIKE_THRESHOLD := 0.99

## Above this element count the stress-test scene subsamples towers/walls and
## caps lift detail so the MultiMesh stays fast.
const STRESS_ELEMENT_LIMIT := 16

const POLE_COLOR := Color(0.34, 0.23, 0.13)
const LEDGER_COLOR := Color(0.52, 0.36, 0.20)
const PLANK_COLOR := Color(0.58, 0.43, 0.26)

var _bounds := AABB(Vector3(-12.0, 0.0, -12.0), Vector3(24.0, 12.0, 24.0))
var _has_bounds := false
var _elements: Array = []
var _has_elements := false
var _front := 1.0
var _playing := false
var _reduced_motion := false
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


## Masonry bounds in world space. Legacy fallback for callers without element
## footprints; builds one rectangular scaffold around the whole AABB.
func configure_bounds(aabb: AABB) -> void:
	_ensure_built()
	var normalised := aabb.abs()
	_elements = []
	_has_elements = false
	_has_bounds = (
		normalised.size.x > 0.01
		and normalised.size.z > 0.01
		and normalised.size.y > 0.01
	)
	_bounds = normalised
	_last_lift_count = -1
	_update_scaffold()
	_update_dust()


## Per-element wall/tower footprints (metres, world space). `fallback_bounds`
## is the merged masonry AABB and is only used if `elements` is empty.
func configure_elements(elements: Array, fallback_bounds: AABB) -> void:
	_ensure_built()
	_elements = elements.duplicate()
	_has_elements = not _elements.is_empty()
	var normalised := fallback_bounds.abs()
	_has_bounds = _has_elements or (
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


## M4b accessibility: reduced motion suppresses the rising dust bursts while
## keeping the scaffold readable. Scaffolding itself is static, so only the
## particle burst needs disabling.
func set_reduced_motion(enabled: bool) -> void:
	_reduced_motion = enabled
	_update_dust()


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
	if (not _has_bounds and not _has_elements) or _front >= STRIKE_THRESHOLD:
		_mmi.visible = false
		_multimesh.visible_instance_count = 0
		_last_lift_count = -1
		return

	var reference := _bounds
	if _has_elements:
		reference = _elements_bounds()
	var climb := maxf(reference.size.y * _front, 0.8)
	var lifts := maxi(1, int(ceil(climb / LIFT_SPACING)))
	if lifts == _last_lift_count and _mmi.visible:
		return
	_last_lift_count = lifts

	var index := 0
	if _has_elements:
		index = _build_element_scaffold(index)
	else:
		index = _build_bounds_scaffold(index)

	_multimesh.visible_instance_count = index
	_mmi.visible = index > 0


# ---------------------------------------------------------------------------
# Per-element scaffold: timber standards hug each wall face and tower ring.
# ---------------------------------------------------------------------------

func _build_element_scaffold(index: int) -> int:
	var source: Array = _elements
	var stress := _elements.size() > STRESS_ELEMENT_LIMIT
	var wall_max_bays := 7
	var tower_min_bays := 10
	var tower_max_bays := 14
	var max_lifts := 1_000
	if stress:
		# Subsample elements so the 200k stress test keeps a light MultiMesh.
		var stride := maxi(1, int(ceil(float(_elements.size()) / float(STRESS_ELEMENT_LIMIT))))
		source = []
		for i in range(0, _elements.size(), stride):
			source.append(_elements[i])
		wall_max_bays = 4
		tower_min_bays = 10
		tower_max_bays = 10
		max_lifts = 6

	for entry in source:
		if not (entry is Dictionary):
			continue
		var element: Dictionary = entry
		var kind := str(element.get("kind", ""))
		match kind:
			"wall":
				index = _build_wall_scaffold(element, index, wall_max_bays, max_lifts)
			"tower":
				index = _build_tower_scaffold(
					element, index, tower_min_bays, tower_max_bays, max_lifts
				)
	return index


func _build_wall_scaffold(
	element: Dictionary, index: int, max_bays: int, max_lifts: int
) -> int:
	var start: Vector2 = element.get("start", Vector2.ZERO)
	var end: Vector2 = element.get("end", Vector2.ZERO)
	var span := start.distance_to(end)
	if span < 0.5:
		return index

	var dir := (end - start) / span
	var normal := Vector2(-dir.y, dir.x)
	var thickness := float(element.get("thickness", 1.0))
	var offset := thickness * 0.5 + WALL_SCAFFOLD_OFFSET
	var height := float(element.get("height", 8.0))
	var base_y := float(element.get("base_y", 0.0))
	var start_y := float(element.get("start_y", base_y))
	var end_y := float(element.get("end_y", base_y))
	var climb := height * clampf(_front, 0.0, 1.0)
	if climb <= 0.05:
		return index

	var lifts := clampi(int(ceil(climb / LIFT_SPACING)), 1, max_lifts)
	var bays := clampi(int(ceil(span / 3.0)), 2, max_bays)
	var yaw := atan2(-dir.y, dir.x)

	# Vertical standards on both sides, each grounded at the interpolated
	# terrain height so slopes are followed rather than buried.
	for i in range(bays + 1):
		var t := float(i) / float(bays)
		var ground := lerpf(start_y, end_y, t)
		var point := start + dir * (span * t)
		for side in [-1.0, 1.0]:
			var base := point + normal * (offset * float(side))
			index = _add_pole(base.x, base.y, ground, ground + climb, index)

	# Horizontal plank ledgers/dec beds along both faces, one per lift.
	var midpoint := start + dir * (span * 0.5)
	for lift in range(1, lifts + 1):
		var y := base_y + minf(float(lift) * LIFT_SPACING, climb)
		for side in [-1.0, 1.0]:
			var pos := midpoint + normal * (offset * float(side))
			index = _add_oriented_box(
				Vector3(pos.x, y, pos.y),
				Vector3(span, LEDGER_HEIGHT, PLANK_DEPTH),
				yaw,
				PLANK_COLOR,
				index
			)
	return index


func _build_tower_scaffold(
	element: Dictionary, index: int, min_bays: int, max_bays: int, max_lifts: int
) -> int:
	var center: Vector2 = element.get("center", Vector2.ZERO)
	var radius := float(element.get("radius", 2.0))
	var height := float(element.get("height", 8.0))
	var base_y := float(element.get("base_y", 0.0))
	var climb := height * clampf(_front, 0.0, 1.0)
	if climb <= 0.05:
		return index

	var ring := radius + TOWER_SCAFFOLD_OFFSET
	var bays := clampi(int(ceil(TAU * ring / 3.0)), min_bays, max_bays)
	var lifts := clampi(int(ceil(climb / LIFT_SPACING)), 1, max_lifts)

	# Radially placed standards around the tower perimeter.
	for i in range(bays):
		var angle := TAU * float(i) / float(bays)
		var x := center.x + cos(angle) * ring
		var z := center.y + sin(angle) * ring
		index = _add_pole(x, z, base_y, base_y + climb, index)

	# Tangential plank ledgers, chained bay to bay around the ring.
	var chord := 2.0 * ring * sin(PI / float(bays))
	for lift in range(1, lifts + 1):
		var y := base_y + minf(float(lift) * LIFT_SPACING, climb)
		for i in range(bays):
			var angle := TAU * (float(i) + 0.5) / float(bays)
			var cx := center.x + cos(angle) * ring
			var cz := center.y + sin(angle) * ring
			# Local X runs along the ring tangent (-sin a, cos a).
			var yaw := atan2(-cos(angle), -sin(angle))
			index = _add_oriented_box(
				Vector3(cx, y, cz),
				Vector3(chord * 1.06, LEDGER_HEIGHT, PLANK_DEPTH),
				yaw,
				PLANK_COLOR,
				index
			)
	return index


# ---------------------------------------------------------------------------
# Legacy rectangular scaffold around a whole AABB (used when no elements).
# ---------------------------------------------------------------------------

func _build_bounds_scaffold(index: int) -> int:
	var climb := maxf(_bounds.size.y * _front, 0.8)
	var lifts := maxi(1, int(ceil(climb / LIFT_SPACING)))
	var base_y := _bounds.position.y - 0.20
	var top_y := base_y + climb + 1.10
	var x0 := _bounds.position.x - SCAFFOLD_OFFSET
	var x1 := _bounds.end.x + SCAFFOLD_OFFSET
	var z0 := _bounds.position.z - SCAFFOLD_OFFSET
	var z1 := _bounds.end.z + SCAFFOLD_OFFSET

	var xs := _edge_positions(x0, x1)
	var zs := _edge_positions(z0, z1)

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
	return index


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


func _add_oriented_box(
	center: Vector3, size: Vector3, yaw: float, color: Color, index: int
) -> int:
	if index >= MAX_INSTANCES:
		return index
	var basis := Basis(Vector3.UP, yaw) * Basis.from_scale(size)
	_multimesh.set_instance_transform(index, Transform3D(basis, center))
	_multimesh.set_instance_color(index, color)
	return index + 1


## Merge the footprints of all known elements into one world-space AABB.
func _elements_bounds() -> AABB:
	var bounds := AABB()
	var have := false
	for entry in _elements:
		if not (entry is Dictionary):
			continue
		var element: Dictionary = entry
		var kind := str(element.get("kind", ""))
		var min_x: float
		var max_x: float
		var min_z: float
		var max_z: float
		var base_y: float
		var top_y: float
		if kind == "tower":
			var center: Vector2 = element.get("center", Vector2.ZERO)
			var radius := float(element.get("radius", 2.0)) + TOWER_SCAFFOLD_OFFSET
			min_x = center.x - radius
			max_x = center.x + radius
			min_z = center.y - radius
			max_z = center.y + radius
			base_y = float(element.get("base_y", 0.0))
			top_y = base_y + float(element.get("height", 8.0))
		else:
			var start: Vector2 = element.get("start", Vector2.ZERO)
			var end: Vector2 = element.get("end", Vector2.ZERO)
			var half := float(element.get("thickness", 1.0)) * 0.5 + WALL_SCAFFOLD_OFFSET
			min_x = minf(start.x, end.x) - half
			max_x = maxf(start.x, end.x) + half
			min_z = minf(start.y, end.y) - half
			max_z = maxf(start.y, end.y) + half
			var base := float(element.get("base_y", 0.0))
			base_y = minf(float(element.get("start_y", base)), float(element.get("end_y", base)))
			var top_base := maxf(
				float(element.get("start_y", base)), float(element.get("end_y", base))
			)
			top_y = top_base + float(element.get("height", 8.0))
		var element_bounds := AABB(
			Vector3(min_x, base_y, min_z),
			Vector3(max_x - min_x, maxf(top_y - base_y, 0.01), max_z - min_z)
		)
		if not have:
			bounds = element_bounds
			have = true
		else:
			bounds = bounds.merge(element_bounds)
	return bounds if have else _bounds


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
		(_has_bounds or _has_elements)
		and _playing
		and not _reduced_motion
		and _front > 0.01
		and _front < STRIKE_THRESHOLD
	)
	_dust.emitting = active
	if not (_has_bounds or _has_elements):
		return

	# Ride the active lift along the real element footprint, not a castle-wide box.
	var bounds := _elements_bounds() if _has_elements else _bounds
	var lift_y := bounds.position.y + clampf(bounds.size.y * _front, 0.4, bounds.size.y)
	var center := bounds.get_center()
	_dust.position = Vector3(center.x, lift_y, center.z)
	_dust.emission_box_extents = Vector3(
		maxf(bounds.size.x * 0.45, 0.8),
		0.35,
		maxf(bounds.size.z * 0.45, 0.8)
	)
