extends Node3D
## Keepwright M0a — look and performance spike.
##
## Everything is built in code (see AGENTS.md rule 5): a golden-hour
## WorldEnvironment, a warm low sun with tuned cascades, an orbit camera, the
## debug HUD and the KeepBridge GDExtension. Terrain and chunk meshes returned by
## keep-gd are already in world space, so they are wrapped in identity-transform
## MeshInstance3D nodes with visibility ranges for LOD0 (0-80 m), LOD1 (80-180 m)
## and LOD2 (180-600 m). Keeping the nodes at the origin preserves the mesh AABB
## at its true world position for frustum, shadow and visibility-range culling.

const MASONRY_SHADER := preload("res://shaders/masonry.gdshader")
const TERRAIN_SHADER := preload("res://shaders/terrain.gdshader")
const ORBIT_CAMERA_SCRIPT := preload("res://scripts/orbit_camera.gd")
const DEBUG_HUD_SCRIPT := preload("res://scripts/debug_hud.gd")

# Chunk visibility ranges in metres. Keep aligned with the M0a budget. LOD0
# extends far enough that the default framing (~50 m) always shows full detail.
const LOD_BEGIN := [0.0, 80.0, 180.0]
const LOD_END := [80.0, 180.0, 600.0]
const LOD_FADE_MARGIN := [3.0, 6.0, 12.0]

var _bridge: Object
var _camera: Camera3D
var _hud: CanvasLayer
var _sun: DirectionalLight3D

var _masonry_material: ShaderMaterial
var _terrain_material: ShaderMaterial
var _chunk_instances: Array[MeshInstance3D] = []

var _build_front := 1.0
var _mode_busy := false


func _ready() -> void:
	_make_materials()
	_setup_environment()
	_setup_sun()
	_setup_camera()
	_setup_hud()
	_setup_bridge()

	if _bridge != null:
		_rebuild("showcase")


func _process(_delta: float) -> void:
	if _camera != null and _sun != null:
		_camera.call("update_shadow_splits", _sun)


func _make_materials() -> void:
	_masonry_material = ShaderMaterial.new()
	_masonry_material.shader = MASONRY_SHADER
	_masonry_material.set_shader_parameter("build_front", 1.0)

	_terrain_material = ShaderMaterial.new()
	_terrain_material.shader = TERRAIN_SHADER


func _setup_environment() -> void:
	var sky_material := PhysicalSkyMaterial.new()
	sky_material.turbidity = 5.0
	sky_material.rayleigh_coefficient = 2.4
	sky_material.rayleigh_color = Color(0.26, 0.40, 0.65)
	sky_material.mie_coefficient = 0.004
	sky_material.mie_eccentricity = 0.82
	sky_material.mie_color = Color(1.0, 0.85, 0.66)
	sky_material.sun_disk_scale = 1.6
	sky_material.ground_color = Color(0.30, 0.27, 0.21)
	sky_material.energy_multiplier = 1.0

	var sky := Sky.new()
	sky.sky_material = sky_material

	var environment := Environment.new()
	environment.background_mode = Environment.BG_SKY
	environment.sky = sky
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_SKY
	environment.ambient_light_sky_contribution = 0.75
	environment.ambient_light_energy = 0.55
	environment.reflected_light_source = Environment.REFLECTION_SOURCE_SKY

	environment.tonemap_mode = Environment.TONE_MAPPER_ACES
	environment.tonemap_exposure = 1.05
	environment.tonemap_white = 6.0

	environment.ssao_enabled = true
	environment.ssao_radius = 1.6
	environment.ssao_intensity = 1.8
	environment.ssao_power = 1.6
	environment.ssao_detail = 0.6
	environment.ssao_horizon = 0.06
	environment.ssao_sharpness = 0.98
	environment.ssao_light_affect = 0.25

	environment.fog_enabled = true
	environment.fog_mode = Environment.FOG_MODE_EXPONENTIAL
	environment.fog_light_color = Color(0.68, 0.62, 0.52)
	environment.fog_light_energy = 0.35
	environment.fog_density = 0.0018
	environment.fog_sky_affect = 0.40

	environment.volumetric_fog_enabled = true
	environment.volumetric_fog_density = 0.008
	environment.volumetric_fog_albedo = Color(0.90, 0.85, 0.78)
	environment.volumetric_fog_anisotropy = 0.35
	environment.volumetric_fog_length = 160.0
	environment.volumetric_fog_detail_spread = 2.0
	environment.volumetric_fog_ambient_inject = 0.35
	environment.volumetric_fog_sky_affect = 0.15

	environment.adjustment_enabled = true
	environment.adjustment_contrast = 1.04
	environment.adjustment_saturation = 1.05

	var world_environment := WorldEnvironment.new()
	world_environment.name = "WorldEnvironment"
	world_environment.environment = environment
	add_child(world_environment)


func _setup_sun() -> void:
	_sun = DirectionalLight3D.new()
	_sun.name = "Sun"
	# Low warm key at golden hour: just above the horizon, raking across the wall.
	_sun.rotation_degrees = Vector3(-11.0, 42.0, 0.0)
	_sun.light_color = Color(1.0, 0.74, 0.46)
	_sun.light_energy = 2.4
	_sun.light_angular_distance = 0.6
	_sun.shadow_enabled = true
	_sun.directional_shadow_mode = DirectionalLight3D.SHADOW_PARALLEL_4_SPLITS
	_sun.directional_shadow_split_1 = 0.05
	_sun.directional_shadow_split_2 = 0.15
	_sun.directional_shadow_split_3 = 0.42
	_sun.directional_shadow_blend_splits = true
	_sun.directional_shadow_max_distance = 180.0
	_sun.shadow_bias = 0.04
	_sun.shadow_normal_bias = 1.2
	_sun.shadow_blur = 1.2
	_sun.sky_mode = DirectionalLight3D.SKY_MODE_LIGHT_AND_SKY
	add_child(_sun)

	# Align the shader's stylised bounce fill with the real sun. A directional
	# light shines along its local -Z, so +Z points from the scene toward the sun.
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("sun_bounce_dir", _sun.global_transform.basis.z.normalized())


func _setup_camera() -> void:
	_camera = ORBIT_CAMERA_SCRIPT.new()
	_camera.name = "OrbitCamera"
	add_child(_camera)
	_camera.current = true
	_camera.call("focus_on", Vector3(0.0, 8.0, 0.0), 90.0)


func _setup_hud() -> void:
	_hud = DEBUG_HUD_SCRIPT.new()
	_hud.name = "DebugHud"
	add_child(_hud)
	_hud.connect("build_front_changed", Callable(self, "_on_hud_build_front_changed"))
	_hud.connect("mode_requested", Callable(self, "_on_hud_mode_requested"))


func _setup_bridge() -> void:
	if not ClassDB.class_exists("KeepBridge"):
		var message := (
			"KeepBridge GDExtension is not registered.\n"
			+ "Build it with: cargo build --manifest-path core/Cargo.toml -p keep-gd\n"
			+ "and check that game/keep_gd.gdextension resolves."
		)
		_hud.call("show_error", message)
		push_warning("Keepwright: KeepBridge class not found; scene is environment-only.")
		return

	var instance: Object = ClassDB.instantiate("KeepBridge")
	if instance == null:
		_hud.call("show_error", "ClassDB.instantiate(\"KeepBridge\") returned null.")
		return
	if not instance.has_method("build_m0a_showcase"):
		_hud.call("show_error", "KeepBridge is registered but has no build_m0a_showcase() method.")
		if instance is Node:
			(instance as Node).free()
		return

	_bridge = instance
	if instance is Node:
		add_child(instance as Node)


func _rebuild(mode: String) -> void:
	if _bridge == null:
		_hud.call("show_error", "Cannot build: KeepBridge is unavailable.")
		return
	if _mode_busy:
		return

	var method := "build_m0a_showcase" if mode == "showcase" else "build_stress_200k"
	if not _bridge.has_method(method):
		_hud.call("show_error", "KeepBridge has no %s() method." % method)
		return

	_mode_busy = true
	var started := Time.get_ticks_msec()
	var result: Variant = _bridge.call(method)
	var elapsed := Time.get_ticks_msec() - started

	if not (result is Dictionary):
		_hud.call("show_error", "%s() did not return a Dictionary." % method)
		_mode_busy = false
		return

	var data: Dictionary = result
	_clear_world_meshes()
	_spawn_terrain(data.get("terrain"))
	_spawn_chunks(data.get("chunks"))

	var block_count := int(data.get("block_count", 0))
	var chunk_count := int(data.get("chunk_count", 0))
	var triangle_count := int(data.get("triangle_count", 0))
	var layout_hash := str(data.get("layout_hash", ""))

	_hud.call("set_scene_stats", block_count, chunk_count, layout_hash, triangle_count)
	_hud.call("set_mode_label", "M0a Showcase" if mode == "showcase" else "200k Stress Test")
	_hud.call("clear_error")
	_hud.call("set_playing", false)
	_hud.call("set_build_front", 1.0)
	_apply_build_front(1.0)
	_frame_camera(data)

	print("Keepwright: %s built in %d ms — %d blocks, %d chunks, %d tris, hash %s" % [
		mode, elapsed, block_count, chunk_count, triangle_count, layout_hash
	])
	_mode_busy = false


func _spawn_terrain(mesh: Variant) -> void:
	if mesh == null or not (mesh is ArrayMesh):
		push_warning("Keepwright: build result has no usable terrain ArrayMesh.")
		return
	var instance := MeshInstance3D.new()
	instance.name = "Terrain"
	instance.mesh = mesh as ArrayMesh
	instance.material_override = _terrain_material
	instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	add_child(instance)
	_chunk_instances.append(instance)


func _spawn_chunks(chunks: Variant) -> void:
	if chunks == null or not (chunks is Array):
		push_warning("Keepwright: build result has no chunks Array.")
		return

	var chunk_array: Array = chunks
	for chunk_index in chunk_array.size():
		var entry: Variant = chunk_array[chunk_index]
		if not (entry is Dictionary):
			continue
		var chunk: Dictionary = entry

		for lod in range(3):
			var mesh: Variant = chunk.get("lod%d" % lod, null)
			if mesh == null or not (mesh is ArrayMesh):
				continue

			var instance := MeshInstance3D.new()
			instance.name = "Chunk%d_LOD%d" % [chunk_index, lod]
			instance.mesh = mesh as ArrayMesh
			instance.material_override = _masonry_material
			# keep-core already emits world-space vertices; leave the node at the
			# identity transform so its AABB matches the real world position.
			instance.position = Vector3.ZERO
			instance.visibility_range_begin = LOD_BEGIN[lod]
			instance.visibility_range_begin_margin = LOD_FADE_MARGIN[lod]
			instance.visibility_range_end = LOD_END[lod]
			instance.visibility_range_end_margin = LOD_FADE_MARGIN[lod]
			instance.visibility_range_fade_mode = GeometryInstance3D.VISIBILITY_RANGE_FADE_DISABLED
			instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
			instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
			add_child(instance)
			_chunk_instances.append(instance)


func _clear_world_meshes() -> void:
	for instance in _chunk_instances:
		if is_instance_valid(instance):
			instance.queue_free()
	_chunk_instances.clear()


func _frame_camera(data: Dictionary) -> void:
	var focus := Vector3(0.0, 8.0, 0.0)
	var span := 60.0
	var have_bounds := false

	var low := Vector3(INF, INF, INF)
	var high := Vector3(-INF, -INF, -INF)

	var chunks: Variant = data.get("chunks", null)
	if chunks is Array and not (chunks as Array).is_empty():
		for entry in (chunks as Array):
			if not (entry is Dictionary):
				continue
			var chunk: Dictionary = entry
			var center: Vector3 = chunk.get("center", Vector3.ZERO)
			low = low.min(center)
			high = high.max(center)
			have_bounds = true

	if have_bounds:
		focus = (low + high) * 0.5
		span = maxf(high.x - low.x, high.z - low.z) + 30.0
	else:
		var terrain: Variant = data.get("terrain", null)
		if terrain is ArrayMesh:
			var aabb: AABB = (terrain as ArrayMesh).get_aabb()
			focus = aabb.get_center()
			span = maxf(aabb.size.x, aabb.size.z)

	focus.y = maxf(focus.y + 4.0, 6.0)
	var distance := clampf(span * 0.85, 35.0, 220.0)
	_camera.call("focus_on", focus, distance)


func _on_hud_build_front_changed(value: float) -> void:
	_apply_build_front(value)


func _apply_build_front(value: float) -> void:
	_build_front = clampf(value, 0.0, 1.0)
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("build_front", _build_front)


func _on_hud_mode_requested(mode: String) -> void:
	_rebuild(mode)
