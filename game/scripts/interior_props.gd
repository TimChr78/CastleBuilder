extends Node3D
## M3.1 procedural dollhouse interior props.
##
## keep-core derives every prop deterministically from the document
## (`keep_core::interior::build_interior_props`) and keep-gd ships them in the
## scene dictionary as `interior_props`. This node turns that plain data into
## crisp, low-poly wooden/iron/stone furniture inside keeps and roofed ranges.
##
## Props are only shown while a cutaway plane is active. In horizontal
## (`Storey`) mode everything above the cut plane is hidden, so the viewer looks
## down into furnished rooms instead of at floating furniture above the slice.

const OAK := Color(0.40, 0.26, 0.14)
const OAK_DARK := Color(0.29, 0.18, 0.10)
const LINEN := Color(0.84, 0.79, 0.66)
const IRON := Color(0.22, 0.23, 0.24)
const STONE := Color(0.55, 0.53, 0.48)
const WAX := Color(0.94, 0.88, 0.70)
const EMBER := Color(0.92, 0.38, 0.12)

var _prop_nodes: Array[Node3D] = []
var _prop_positions: Array[Vector3] = []
var _prop_rooms: Array[String] = []
var _floor_nodes: Array[Node3D] = []
var _floor_positions: Array[Vector3] = []
var _materials: Dictionary = {}
var _cutaway_mode := 0
var _cutaway_pos := 100.0


func _init() -> void:
	_materials = _make_materials()
	visible = false


## Rebuild every prop from the bridge's `interior_props` dictionaries.
func build_props(props: Array) -> void:
	clear_props()
	# Group props by (element, room) so each furnished room gets one timber
	# floor slab under its bounding box. Without it, upper-storey solar props
	# float in mid-air when the keep is cut open (M3.1 M5).
	var groups: Dictionary = {}
	for entry in props:
		if not (entry is Dictionary):
			continue
		var data: Dictionary = entry
		var kind := str(data.get("kind", ""))
		var position: Variant = data.get("position", null)
		if not (position is Vector3):
			continue
		var prop := _build_prop(kind, position as Vector3, int(data.get("yaw_deg", 0)))
		if prop == null:
			continue
		prop.name = "Prop_%s_%d" % [kind, _prop_nodes.size()]
		add_child(prop)
		_prop_nodes.append(prop)
		_prop_positions.append(position as Vector3)
		_prop_rooms.append(str(data.get("room_type", "")))
		var key := "%d:%s" % [
			int(data.get("element_id", -1)), str(data.get("room_type", ""))
		]
		if not groups.has(key):
			groups[key] = {"min": position as Vector3, "max": position as Vector3}
		else:
			var bounds: Dictionary = groups[key]
			bounds["min"] = (bounds["min"] as Vector3).min(position as Vector3)
			bounds["max"] = (bounds["max"] as Vector3).max(position as Vector3)
	_build_floor_slabs(groups)
	_apply_visibility()


## Emit one thin oak slab per furnished room, resting just below the lowest
## prop in that group so furniture never hovers above an empty void.
func _build_floor_slabs(groups: Dictionary) -> void:
	for key in groups:
		var bounds: Dictionary = groups[key]
		var lo: Vector3 = bounds["min"]
		var hi: Vector3 = bounds["max"]
		const PAD := 1.2
		const THICKNESS := 0.12
		const MIN_SPAN := 2.4
		var span_x := maxf(hi.x - lo.x + PAD * 2.0, MIN_SPAN)
		var span_z := maxf(hi.z - lo.z + PAD * 2.0, MIN_SPAN)
		var centre := Vector3((lo.x + hi.x) * 0.5, lo.y - THICKNESS * 0.5, (lo.z + hi.z) * 0.5)
		var slab := MeshInstance3D.new()
		var mesh := BoxMesh.new()
		mesh.size = Vector3(span_x, THICKNESS, span_z)
		slab.mesh = mesh
		slab.material_override = _mat("oak")
		slab.position = centre
		slab.name = "Floor_%d" % _floor_nodes.size()
		slab.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
		slab.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
		add_child(slab)
		_floor_nodes.append(slab)
		_floor_positions.append(centre)


func clear_props() -> void:
	for node in _prop_nodes:
		if is_instance_valid(node):
			node.queue_free()
	_prop_nodes.clear()
	_prop_positions.clear()
	_prop_rooms.clear()
	for node in _floor_nodes:
		if is_instance_valid(node):
			node.queue_free()
	_floor_nodes.clear()
	_floor_positions.clear()


## Toggle the dollhouse view. `mode` matches the masonry shader: 0 = off,
## 1 = horizontal, 2 = vertical X, 3 = vertical Z.
func set_cutaway(mode: int, position: float) -> void:
	_cutaway_mode = mode
	_cutaway_pos = position
	_apply_visibility()


func get_cutaway_mode() -> int:
	return _cutaway_mode


func get_cutaway_position() -> float:
	return _cutaway_pos


func get_prop_count() -> int:
	return _prop_nodes.size()


func get_visible_prop_count() -> int:
	var count := 0
	for node in _prop_nodes:
		if is_instance_valid(node) and node.visible:
			count += 1
	return count


func get_room_types() -> Array[String]:
	var rooms: Array[String] = []
	for room in _prop_rooms:
		if not rooms.has(room):
			rooms.append(room)
	return rooms


func is_props_visible() -> bool:
	return visible


func _apply_visibility() -> void:
	var show_props := _cutaway_mode > 0
	visible = show_props
	if not show_props:
		return
	for index in _prop_nodes.size():
		var point := _prop_positions[index]
		var inside := true
		match _cutaway_mode:
			1:
				inside = point.y <= _cutaway_pos
			2:
				inside = point.x <= _cutaway_pos
			3:
				inside = point.z <= _cutaway_pos
			_:
				inside = true
		_prop_nodes[index].visible = inside
	# Floors obey the same cut plane so a slab never hangs above its props.
	for index in _floor_nodes.size():
		var point := _floor_positions[index]
		var inside := true
		match _cutaway_mode:
			1:
				inside = point.y <= _cutaway_pos
			2:
				inside = point.x <= _cutaway_pos
			3:
				inside = point.z <= _cutaway_pos
			_:
				inside = true
		_floor_nodes[index].visible = inside


# ---------------------------------------------------------------------------
# Procedural furniture
# ---------------------------------------------------------------------------

func _build_prop(kind: String, position: Vector3, yaw_deg: int) -> Node3D:
	var prop := Node3D.new()
	prop.position = position
	# keep-core's yaw is clockwise from +X in plan; Godot's Y rotation is
	# counter-clockwise, so negate it to keep the furniture aligned to the room.
	prop.rotation.y = -deg_to_rad(float(yaw_deg))

	match kind:
		"table":
			_build_table(prop)
		"bench":
			_build_bench(prop)
		"bed":
			_build_bed(prop)
		"altar":
			_build_altar(prop)
		"cauldron":
			_build_cauldron(prop)
		"barrel":
			_build_barrel(prop)
		"chest":
			_build_chest(prop)
		_:
			prop.queue_free()
			return null
	return prop


func _build_table(prop: Node3D) -> void:
	_add_box(prop, Vector3(1.8, 0.09, 0.85), Vector3(0.0, 0.74, 0.0), _mat("oak"))
	_add_box(prop, Vector3(0.14, 0.70, 0.62), Vector3(-0.62, 0.35, 0.0), _mat("oak_dark"))
	_add_box(prop, Vector3(0.14, 0.70, 0.62), Vector3(0.62, 0.35, 0.0), _mat("oak_dark"))
	_add_box(prop, Vector3(0.10, 0.06, 0.30), Vector3(0.0, 0.20, 0.0), _mat("oak_dark"))


func _build_bench(prop: Node3D) -> void:
	_add_box(prop, Vector3(1.7, 0.08, 0.36), Vector3(0.0, 0.44, 0.0), _mat("oak"))
	_add_box(prop, Vector3(0.10, 0.40, 0.30), Vector3(-0.60, 0.20, 0.0), _mat("oak_dark"))
	_add_box(prop, Vector3(0.10, 0.40, 0.30), Vector3(0.60, 0.20, 0.0), _mat("oak_dark"))


func _build_bed(prop: Node3D) -> void:
	_add_box(prop, Vector3(1.90, 0.32, 1.05), Vector3(0.0, 0.16, 0.0), _mat("oak_dark"))
	_add_box(prop, Vector3(1.78, 0.18, 0.94), Vector3(0.0, 0.41, 0.0), _mat("linen"))
	_add_box(prop, Vector3(0.44, 0.14, 0.66), Vector3(0.62, 0.56, 0.0), _mat("wax"))
	_add_box(prop, Vector3(1.26, 0.08, 0.98), Vector3(-0.20, 0.48, 0.0), _mat("oak"))


func _build_altar(prop: Node3D) -> void:
	_add_box(prop, Vector3(1.60, 0.95, 0.65), Vector3(0.0, 0.475, 0.0), _mat("stone"))
	_add_box(prop, Vector3(1.84, 0.10, 0.80), Vector3(0.0, 1.00, 0.0), _mat("linen"))
	_add_cylinder(prop, 0.05, 0.05, 0.26, Vector3(-0.50, 1.18, 0.0), _mat("wax"))
	_add_cylinder(prop, 0.05, 0.05, 0.26, Vector3(0.50, 1.18, 0.0), _mat("wax"))
	_add_cylinder(prop, 0.02, 0.02, 0.06, Vector3(-0.50, 1.34, 0.0), _mat("ember"))
	_add_cylinder(prop, 0.02, 0.02, 0.06, Vector3(0.50, 1.34, 0.0), _mat("ember"))


func _build_cauldron(prop: Node3D) -> void:
	_add_cylinder(prop, 0.46, 0.36, 0.52, Vector3(0.0, 0.40, 0.0), _mat("iron"), 14)
	_add_cylinder(prop, 0.48, 0.48, 0.07, Vector3(0.0, 0.67, 0.0), _mat("iron"), 14)
	_add_cylinder(prop, 0.31, 0.31, 0.03, Vector3(0.0, 0.14, 0.0), _mat("ember"), 12)
	for offset in [Vector3(-0.28, 0.14, -0.10), Vector3(0.28, 0.14, -0.10), Vector3(0.0, 0.14, 0.30)]:
		_add_cylinder(prop, 0.04, 0.04, 0.28, offset, _mat("iron"), 6)


func _build_barrel(prop: Node3D) -> void:
	_add_cylinder(prop, 0.32, 0.32, 0.86, Vector3(0.0, 0.43, 0.0), _mat("oak"), 12)
	_add_cylinder(prop, 0.335, 0.335, 0.05, Vector3(0.0, 0.20, 0.0), _mat("iron"), 12)
	_add_cylinder(prop, 0.335, 0.335, 0.05, Vector3(0.0, 0.66, 0.0), _mat("iron"), 12)
	_add_cylinder(prop, 0.28, 0.28, 0.02, Vector3(0.0, 0.87, 0.0), _mat("oak_dark"), 12)


func _build_chest(prop: Node3D) -> void:
	_add_box(prop, Vector3(0.95, 0.48, 0.55), Vector3(0.0, 0.24, 0.0), _mat("oak"))
	_add_box(prop, Vector3(1.00, 0.14, 0.60), Vector3(0.0, 0.55, 0.0), _mat("oak_dark"))
	_add_box(prop, Vector3(0.07, 0.62, 0.58), Vector3(-0.28, 0.31, 0.0), _mat("iron"))
	_add_box(prop, Vector3(0.07, 0.62, 0.58), Vector3(0.28, 0.31, 0.0), _mat("iron"))
	_add_box(prop, Vector3(0.12, 0.10, 0.06), Vector3(0.0, 0.38, 0.28), _mat("iron"))


func _add_box(
	parent: Node3D, size: Vector3, offset: Vector3, material: Material
) -> MeshInstance3D:
	var mesh := BoxMesh.new()
	mesh.size = size
	var instance := MeshInstance3D.new()
	instance.mesh = mesh
	instance.material_override = material
	instance.position = offset
	instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
	parent.add_child(instance)
	return instance


func _add_cylinder(
	parent: Node3D,
	top_radius: float,
	bottom_radius: float,
	height: float,
	offset: Vector3,
	material: Material,
	sides: int = 12
) -> MeshInstance3D:
	var mesh := CylinderMesh.new()
	mesh.top_radius = top_radius
	mesh.bottom_radius = bottom_radius
	mesh.height = height
	mesh.radial_segments = sides
	mesh.rings = 1
	var instance := MeshInstance3D.new()
	instance.mesh = mesh
	instance.material_override = material
	instance.position = offset
	instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
	parent.add_child(instance)
	return instance


# ---------------------------------------------------------------------------
# Materials
# ---------------------------------------------------------------------------

func _make_materials() -> Dictionary:
	return {
		"oak": _make_material(OAK, 0.82),
		"oak_dark": _make_material(OAK_DARK, 0.88),
		"linen": _make_material(LINEN, 0.95),
		"iron": _make_material(IRON, 0.42, 0.65),
		"stone": _make_material(STONE, 0.90),
		"wax": _make_material(WAX, 0.70),
		"ember": _make_material(EMBER, 0.90, 0.0, EMBER, 2.2),
	}


func _make_material(
	color: Color,
	roughness: float,
	metallic: float = 0.0,
	emission: Color = Color.BLACK,
	emission_energy: float = 0.0
) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.roughness = roughness
	material.metallic = metallic
	if emission_energy > 0.0:
		material.emission_enabled = true
		material.emission = emission
		material.emission_energy_multiplier = emission_energy
	return material


func _mat(key: String) -> Material:
	return _materials.get(key, _materials["oak"])
