extends Node3D
## Keepwright M0a/M0b — look and feel spike.
##
## Everything is built in code (see AGENTS.md rule 5): a golden-hour
## WorldEnvironment, a warm low sun with tuned cascades, an orbit camera, the
## debug HUD and the KeepBridge GDExtension. Terrain and chunk meshes returned by
## keep-gd are already in world space, so they are wrapped in identity-transform
## MeshInstance3D nodes with visibility ranges for LOD0 (0-90 m), LOD1 (90-200 m)
## and LOD2 (200 m to infinity). Every LOD of a chunk shares one `custom_aabb` and
## zero fade margins so Godot evaluates the exact same camera distance for all
## three LODs; otherwise their slightly different mesh AABBs create pops.
##
## M0b adds the construction theater (climbing scaffolding + rising dust), the
## procedural generator audio and the interactive design-tool overlay.

const MASONRY_SHADER := preload("res://shaders/masonry.gdshader")
const TERRAIN_SHADER := preload("res://shaders/terrain.gdshader")
const ORBIT_CAMERA_SCRIPT := preload("res://scripts/orbit_camera.gd")
const DEBUG_HUD_SCRIPT := preload("res://scripts/debug_hud.gd")
const CONSTRUCTION_FX_SCRIPT := preload("res://scripts/construction_fx.gd")
const BUILD_AUDIO_SCRIPT := preload("res://scripts/build_audio.gd")
const DESIGN_TOOL_SCRIPT := preload("res://scripts/design_tool.gd")

# Chunk visibility ranges in metres. Keep aligned with the M0a budget. LOD0
# extends far enough that the default framing (~50 m) always shows full detail.
const LOD_BEGIN := [0.0, 90.0, 200.0]
## 0.0 means "infinite far distance": distant chunks never disappear.
const LOD_END := [90.0, 200.0, 0.0]

## Seconds for the construction front to sweep 0 → 1 once, then hold.
const BUILD_PLAY_SECONDS := 27.0

## Save slot shared by the HUD Save/Load buttons.
const SAVE_PATH := "user://castle.keep.json"

var _bridge: Object
var _camera: Camera3D
var _hud: CanvasLayer
var _sun: DirectionalLight3D
var _fx: Node3D
var _audio: Node
var _design_tool: Node3D

var _masonry_material: ShaderMaterial
var _terrain_material: ShaderMaterial
var _chunk_instances: Array[MeshInstance3D] = []

var _build_front := 1.0
var _playing := false
var _tick_timer := 0.0
var _mode_busy := false


func _ready() -> void:
	_make_materials()
	_setup_environment()
	_setup_sun()
	_setup_camera()
	_setup_hud()
	_setup_feel()
	_setup_bridge()

	if _bridge != null:
		_rebuild("m0b")


func _process(delta: float) -> void:
	if _camera != null and _sun != null:
		_camera.call("update_shadow_splits", _sun)

	# Zoom-reactive procedural mix + stone-setting ticks while the build plays.
	if _audio != null and _camera != null:
		_audio.call("set_camera_distance", float(_camera.call("get_view_distance")))

	if _playing:
		if _build_front < 1.0:
			_apply_build_front(
				minf(_build_front + delta / BUILD_PLAY_SECONDS, 1.0)
			)
			if _hud != null:
				_hud.call("set_build_front", _build_front)
		# Hold the finished wall at 1.0 instead of wrapping back to bare ground.
		if _build_front >= 1.0:
			_playing = false
			if _hud != null:
				_hud.call("set_playing", false)
		elif _build_front > 0.01 and _audio != null:
			_tick_timer -= delta
			if _tick_timer <= 0.0:
				_audio.call("play_stone_tick", _build_front)
				_tick_timer = randf_range(0.08, 0.18)


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
	_hud.connect("playing_changed", Callable(self, "_on_hud_playing_changed"))
	_hud.connect("mode_requested", Callable(self, "_on_hud_mode_requested"))
	_hud.connect("tool_requested", Callable(self, "_on_hud_tool_requested"))
	_hud.connect("undo_requested", Callable(self, "_on_undo_requested"))
	_hud.connect("save_requested", Callable(self, "_on_save_requested"))
	_hud.connect("load_requested", Callable(self, "_on_load_requested"))


func _setup_feel() -> void:
	_audio = BUILD_AUDIO_SCRIPT.new()
	_audio.name = "BuildAudio"
	add_child(_audio)

	_fx = CONSTRUCTION_FX_SCRIPT.new()
	_fx.name = "ConstructionFx"
	add_child(_fx)

	_design_tool = DESIGN_TOOL_SCRIPT.new()
	_design_tool.name = "DesignTool"
	add_child(_design_tool)
	_design_tool.connect("wall_committed", Callable(self, "_on_wall_committed"))
	_design_tool.connect("tower_committed", Callable(self, "_on_tower_committed"))
	_design_tool.connect("snap_feedback", Callable(self, "_on_snap_feedback"))
	_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.NONE)


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
	if _design_tool != null:
		_design_tool.call("set_bridge", _bridge)


func _rebuild(mode: String) -> void:
	if _bridge == null:
		_hud.call("show_error", "Cannot build: KeepBridge is unavailable.")
		return
	if _mode_busy:
		return

	var method := "build_m0b_showcase"
	var label := "M0b Showcase"
	match mode:
		"showcase":
			method = "build_m0a_showcase"
			label = "M0a Showcase"
		"stress":
			method = "build_stress_200k"
			label = "200k Stress Test"
		_:
			method = "build_m0b_showcase"
			label = "M0b Showcase"

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
	_apply_scene(data, true, true)
	_hud.call("set_mode_label", label)
	_hud.call("clear_error")

	var block_count := int(data.get("block_count", 0))
	var chunk_count := int(data.get("chunk_count", 0))
	var triangle_count := int(data.get("triangle_count", 0))
	var layout_hash := str(data.get("layout_hash", ""))
	var document_hash := str(data.get("document_hash", ""))

	print(
		"Keepwright: %s built in %d ms — %d blocks, %d chunks, %d tris, layout %s, doc %s"
		% [mode, elapsed, block_count, chunk_count, triangle_count, layout_hash, document_hash]
	)
	_mode_busy = false


## Apply a scene dictionary from any KeepBridge command (build or interactive
## edit). `reset_play` restarts the construction front; `reframe` recentres the
## orbit camera, which is desirable for a mode switch but not for every edit.
func _apply_scene(data: Dictionary, reset_play: bool, reframe: bool) -> void:
	_clear_world_meshes()
	_spawn_terrain(data.get("terrain"))
	_spawn_chunks(data.get("chunks"))

	var block_count := int(data.get("block_count", 0))
	var chunk_count := int(data.get("chunk_count", 0))
	var triangle_count := int(data.get("triangle_count", 0))
	var layout_hash := str(data.get("layout_hash", ""))
	var document_hash := str(data.get("document_hash", ""))

	_hud.call(
		"set_scene_stats", block_count, chunk_count, layout_hash, triangle_count, document_hash
	)

	if reset_play:
		_hud.call("clear_error")
		_hud.call("set_playing", false)
		_hud.call("set_build_front", 1.0)
		_playing = false
		_apply_build_front(1.0)
	else:
		_apply_build_front(_build_front)

	var bounds := _compute_masonry_bounds(data)
	if _fx != null:
		var elements: Variant = data.get("elements", [])
		if elements is Array and not (elements as Array).is_empty():
			_fx.call("configure_elements", elements as Array, bounds)
		else:
			_fx.call("configure_bounds", bounds)
	if _design_tool != null:
		_design_tool.call("set_ground_height", bounds.position.y)

	if reframe:
		_frame_camera(data)


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

		# One shared AABB for every LOD of this chunk. Godot measures the
		# camera-to-AABB distance for visibility ranges, so differing per-LOD
		# mesh AABBs would make LOD0/LOD1/LOD2 swap at slightly different
		# distances and flicker. Sharing the AABB removes that mismatch.
		var shared_aabb := _chunk_shared_aabb(chunk)
		var has_shared := (
			shared_aabb.size.x > 0.0
			or shared_aabb.size.y > 0.0
			or shared_aabb.size.z > 0.0
		)

		for lod in range(3):
			var mesh: Variant = chunk.get("lod%d" % lod, null)
			if mesh == null or not (mesh is ArrayMesh):
				continue

			var instance := MeshInstance3D.new()
			instance.name = "Chunk%d_LOD%d" % [chunk_index, lod]
			instance.mesh = mesh as ArrayMesh
			instance.material_override = _masonry_material
			# LOD1/LOD2 shells are single quads per element face whose UV spans
			# the whole face, so the shader recomposes coursed masonry in world
			# space for them. Tell it which pipeline this instance needs.
			instance.set_instance_shader_parameter(&"lod_level", lod)
			# keep-core already emits world-space vertices; leave the node at the
			# identity transform so its AABB matches the real world position.
			instance.position = Vector3.ZERO
			if has_shared:
				instance.custom_aabb = shared_aabb
			instance.visibility_range_begin = LOD_BEGIN[lod]
			# Zero margins with FADE_DISABLED: non-zero margins create hysteresis
			# dead zones where both LOD0 and LOD1 are hidden at once.
			instance.visibility_range_begin_margin = 0.0
			instance.visibility_range_end = LOD_END[lod]
			instance.visibility_range_end_margin = 0.0
			instance.visibility_range_fade_mode = GeometryInstance3D.VISIBILITY_RANGE_FADE_DISABLED
			instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
			instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
			add_child(instance)
			_chunk_instances.append(instance)


## Merge the LOD meshes' world-space bounds for one chunk. LOD0 anchors the
## result; LOD1/LOD2 contribute only when they actually contain geometry.
func _chunk_shared_aabb(chunk: Dictionary) -> AABB:
	var bounds := AABB()
	var have_bounds := false
	for lod in range(3):
		var mesh: Variant = chunk.get("lod%d" % lod, null)
		if not (mesh is ArrayMesh):
			continue
		var array_mesh: ArrayMesh = mesh as ArrayMesh
		if array_mesh.get_surface_count() == 0:
			continue
		var mesh_bounds: AABB = array_mesh.get_aabb()
		if not have_bounds:
			bounds = mesh_bounds
			have_bounds = true
		else:
			bounds = bounds.merge(mesh_bounds)
	return bounds


func _clear_world_meshes() -> void:
	for instance in _chunk_instances:
		if is_instance_valid(instance):
			instance.queue_free()
	_chunk_instances.clear()


## World-space bounds of the generated masonry (all LOD0 chunk meshes).
func _compute_masonry_bounds(data: Dictionary) -> AABB:
	var bounds := AABB()
	var have_bounds := false
	var chunks: Variant = data.get("chunks", null)
	if chunks is Array:
		for entry in (chunks as Array):
			if not (entry is Dictionary):
				continue
			var mesh: Variant = (entry as Dictionary).get("lod0", null)
			if not (mesh is ArrayMesh):
				continue
			var mesh_bounds: AABB = (mesh as ArrayMesh).get_aabb()
			if not have_bounds:
				bounds = mesh_bounds
				have_bounds = true
			else:
				bounds = bounds.merge(mesh_bounds)
	if not have_bounds:
		bounds = AABB(Vector3(-12.0, 0.0, -12.0), Vector3(24.0, 12.0, 24.0))
	return bounds


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


func _on_hud_playing_changed(playing: bool) -> void:
	_playing = playing
	_tick_timer = 0.0
	# Replaying a finished build starts cleanly from bare ground.
	if playing and _build_front >= 0.99:
		_apply_build_front(0.0)
		if _hud != null:
			_hud.call("set_build_front", 0.0)
	if _fx != null:
		_fx.call("set_build_front", _build_front, _playing)


func _apply_build_front(value: float) -> void:
	_build_front = clampf(value, 0.0, 1.0)
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("build_front", _build_front)
	if _fx != null:
		_fx.call("set_build_front", _build_front, _playing)


func _on_hud_mode_requested(mode: String) -> void:
	_rebuild(mode)


func _on_hud_tool_requested(tool: String) -> void:
	if _design_tool == null:
		return
	match tool:
		"wall":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.WALL)
			_hud.call("set_tool_status", "Tool: Draw Wall — drag LMB across the promontory")
		"tower":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.TOWER)
			_hud.call("set_tool_status", "Tool: Place Tower — click LMB to place")
		_:
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.NONE)
			_hud.call("set_tool_status", "Tool: View — orbit around the site")


func _on_snap_feedback() -> void:
	if _audio != null:
		_audio.call("play_place_thud", 1.18)


func _on_wall_committed(
	start_m: Vector2,
	end_m: Vector2,
	height_m: float,
	thickness_m: float
) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 1.0)
	var length := start_m.distance_to(end_m)
	print(
		"Keepwright: wall committed — %.1f m long, %.1f m high, %.1f m thick (start %s → end %s)" %
		[length, height_m, thickness_m, str(start_m), str(end_m)]
	)
	if _bridge == null or not _bridge.has_method("add_wall"):
		_hud.call(
			"set_tool_status",
			"Wall committed: %.1f m · H %.1f m · T %.1f m" % [length, height_m, thickness_m]
		)
		return
	var result: Variant = _bridge.call("add_wall", start_m, end_m, height_m, thickness_m)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_wall() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Wall rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call(
		"set_tool_status",
		"Wall added: %.1f m · H %.1f m · T %.1f m — Ctrl+Z to undo"
		% [length, height_m, thickness_m]
	)


func _on_tower_committed(center_m: Vector2, radius_m: float, height_m: float) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 0.82)
	print(
		"Keepwright: tower committed — centre %s, radius %.1f m, height %.1f m" %
		[str(center_m), radius_m, height_m]
	)
	if _bridge == null or not _bridge.has_method("add_tower"):
		_hud.call(
			"set_tool_status",
			"Tower committed: r %.1f m · H %.1f m" % [radius_m, height_m]
		)
		return
	var result: Variant = _bridge.call("add_tower", center_m, radius_m, height_m)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_tower() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Tower rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call(
		"set_tool_status",
		"Tower added: r %.1f m · H %.1f m — Ctrl+Z to undo" % [radius_m, height_m]
	)


func _on_undo_requested() -> void:
	if _bridge == null or not _bridge.has_method("undo_last"):
		_hud.call("show_error", "Cannot undo: KeepBridge.undo_last() is unavailable.")
		return
	var result: Variant = _bridge.call("undo_last")
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.undo_last() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if bool(data.get("undone", false)):
		_apply_scene(data, false, false)
		_hud.call("set_tool_status", str(data.get("summary", "Undid last edit")))
	else:
		_hud.call(
			"set_tool_status", "Undo refused: %s" % str(data.get("reason", "nothing to undo"))
		)


func _on_save_requested() -> void:
	if _bridge == null or not _bridge.has_method("save_castle_json"):
		_hud.call("show_error", "Cannot save: KeepBridge.save_castle_json() is unavailable.")
		return
	var result: Variant = _bridge.call("save_castle_json")
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.save_castle_json() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Save failed: %s" % str(data.get("error", "unknown error")))
		return

	var tmp_path := SAVE_PATH + ".tmp"
	var file := FileAccess.open(tmp_path, FileAccess.WRITE)
	if file == null:
		_hud.call(
			"show_error",
			"Could not open %s for writing (error %d)."
			% [tmp_path, FileAccess.get_open_error()]
		)
		return
	var wrote := file.store_string(str(data.get("json", "")))
	var write_error := file.get_error()
	file.close()
	if not wrote or write_error != OK:
		_hud.call(
			"show_error",
			"Could not write %s (error %d)." % [tmp_path, write_error]
		)
		return

	# Replace the live save only after the temporary file is fully written, so a
	# crash or full disk cannot leave a truncated castle.keep.json behind.
	var rename_error := DirAccess.rename_absolute(
		ProjectSettings.globalize_path(tmp_path),
		ProjectSettings.globalize_path(SAVE_PATH)
	)
	if rename_error != OK:
		_hud.call(
			"show_error",
			"Could not replace %s (error %d)." % [SAVE_PATH, rename_error]
		)
		return
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Saved castle.keep.json")


func _on_load_requested() -> void:
	if _bridge == null or not _bridge.has_method("load_castle_json"):
		_hud.call("show_error", "Cannot load: KeepBridge.load_castle_json() is unavailable.")
		return
	if not FileAccess.file_exists(SAVE_PATH):
		_hud.call("set_tool_status", "No save file yet")
		return

	var file := FileAccess.open(SAVE_PATH, FileAccess.READ)
	if file == null:
		_hud.call(
			"show_error",
			"Could not open %s for reading (error %d)."
			% [SAVE_PATH, FileAccess.get_open_error()]
		)
		return
	var json := file.get_as_text()
	file.close()

	var result: Variant = _bridge.call("load_castle_json", json)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.load_castle_json() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Load failed: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, true, true)
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Loaded castle.keep.json")


func _unhandled_input(event: InputEvent) -> void:
	if not (event is InputEventKey):
		return
	var key := event as InputEventKey
	if key.pressed and not key.echo and key.ctrl_pressed and key.keycode == KEY_Z:
		_on_undo_requested()
		get_viewport().set_input_as_handled()
