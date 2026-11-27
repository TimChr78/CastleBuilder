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
const WATER_SHADER := preload("res://shaders/water.gdshader")
const ORBIT_CAMERA_SCRIPT := preload("res://scripts/orbit_camera.gd")
const DEBUG_HUD_SCRIPT := preload("res://scripts/debug_hud.gd")
const CONSTRUCTION_FX_SCRIPT := preload("res://scripts/construction_fx.gd")
const BUILD_AUDIO_SCRIPT := preload("res://scripts/build_audio.gd")
const DESIGN_TOOL_SCRIPT := preload("res://scripts/design_tool.gd")
const AGENT_PANEL_SCRIPT := preload("res://scripts/agent_panel.gd")
const INTERIOR_PROPS_SCRIPT := preload("res://scripts/interior_props.gd")
const PHOTO_MODE_SCRIPT := preload("res://scripts/photo_mode.gd")
const CODEX_PANEL_SCRIPT := preload("res://scripts/codex_panel.gd")
const COMMISSION_PANEL_SCRIPT := preload("res://scripts/commission_panel.gd")

## Preferred loopback port for the live MCP/JSON-RPC control server.
const CONTROL_PORT := 17731
## Captured render_view size requested by agents (architecture section 7.3).
const RENDER_VIEW_SIZE := Vector2i(768, 512)

# Chunk visibility ranges in metres. Keep aligned with the M0a budget. LOD0
# extends far enough that the default framing (~50 m) always shows full detail.
const LOD_BEGIN := [0.0, 90.0, 200.0]
## 0.0 means "infinite far distance": distant chunks never disappear.
const LOD_END := [90.0, 200.0, 0.0]

## Seconds for the construction front to sweep 0 → 1 at 1× speed; the HUD's
## 4× and 20× presets divide this by the speed multiplier.
const BASE_BUILD_SECONDS := 30.0
## Time-lapse replay duration and camera yaw rate (radians/second).
const TIMELAPSE_SECONDS := 9.0
const TIMELAPSE_YAW_RATE := 0.28

## Save slot shared by the HUD Save/Load buttons.
const SAVE_PATH := "user://castle.keep.json"

var _bridge: Object
var _camera: Camera3D
var _hud: CanvasLayer
var _sun: DirectionalLight3D
var _fx: Node3D
var _audio: Node
var _design_tool: Node3D
var _agent_panel: CanvasLayer
var _ghost_root: Node3D
var _ghost_material: StandardMaterial3D
var _ghost_nodes: Array[MeshInstance3D] = []
var _ghost_visibility: Dictionary = {}
var _control_token := ""
var _control_url := ""

var _interior_props: Node3D
var _photo_mode: CanvasLayer
var _codex_panel: CanvasLayer
var _commission_panel: CanvasLayer
var _sky_material: PhysicalSkyMaterial
var _environment: Environment
var _cutaway_mode := 0
var _cutaway_pos := 100.0
var _masonry_bounds := AABB(Vector3(-12.0, 0.0, -12.0), Vector3(24.0, 12.0, 24.0))
var _ground_min_y := 0.0
var _ground_max_y := 0.0
var _ground_median_y := 0.0
var _terrain_tool := ""
var _default_fov := 75.0

var _masonry_material: ShaderMaterial
var _terrain_material: ShaderMaterial
var _water_material: ShaderMaterial
var _water_instance: MeshInstance3D
var _chunk_instances: Array[MeshInstance3D] = []

var _build_front := 1.0
var _playing := false
var _speed_multiplier := 1.0
var _timelapse_active := false
var _timelapse_elapsed := 0.0
var _render_view_busy := false
var _tick_timer := 0.0
var _mode_busy := false
var _winter_frost := false
var _commission_active := false
var _commission_was_visible := false


func _ready() -> void:
	_make_materials()
	_setup_environment()
	_setup_sun()
	_setup_camera()
	_setup_hud()
	_setup_feel()
	_setup_bridge()
	_setup_agent_layer()
	_setup_interior_props()
	_setup_photo_mode()
	_setup_codex()
	_setup_commission_panel()

	if _bridge != null:
		_start_control_server()
		_rebuild("m1")
		_refresh_commission_panel()


func _process(delta: float) -> void:
	if _camera != null and _sun != null:
		_camera.call("update_shadow_splits", _sun)

	# Zoom-reactive procedural mix + stone-setting ticks while the build plays.
	if _audio != null and _camera != null:
		_audio.call("set_camera_distance", float(_camera.call("get_view_distance")))

	if _timelapse_active:
		if _winter_frost:
			# Frost season also pauses the time-lapse replay (L4): the masons
			# have stopped raising courses, so the front must not creep up.
			if _hud != null:
				_hud.call(
					"set_tool_status",
					"Winter frost — time-lapse paused while fresh walls are thatched"
				)
		else:
			_timelapse_elapsed += delta
			var progress := clampf(_timelapse_elapsed / TIMELAPSE_SECONDS, 0.0, 1.0)
			_apply_build_front(progress)
			if _hud != null:
				_hud.call("set_build_front", _build_front)
			if _build_front > 0.01 and _audio != null:
				_tick_timer -= delta
				if _tick_timer <= 0.0:
					_audio.call("play_stone_tick", _build_front)
					_tick_timer = randf_range(0.08, 0.18)
			if progress >= 1.0:
				_stop_timelapse(true)
	elif _playing:
		if _winter_frost:
			# Frost season: the masons thatch the fresh wall tops and stop
			# raising courses until spring. Never advance the visible build.
			if _hud != null:
				_hud.call(
					"set_tool_status",
					"Winter frost — construction paused while fresh walls are thatched"
				)
		elif _build_front < 1.0:
			# Guard against a zero/huge multiplier; Instant is handled by snapping.
			var speed := maxf(_speed_multiplier, 0.01)
			_apply_build_front(
				minf(_build_front + delta * speed / BASE_BUILD_SECONDS, 1.0)
			)
			if _hud != null:
				_hud.call("set_build_front", _build_front)
		# Hold the finished wall at 1.0 instead of wrapping back to bare ground.
		if _build_front >= 1.0:
			_playing = false
			if _hud != null:
				_hud.call("set_playing", false)
		elif not _winter_frost and _build_front > 0.01 and _audio != null:
			_tick_timer -= delta
			if _tick_timer <= 0.0:
				_audio.call("play_stone_tick", _build_front)
				_tick_timer = randf_range(0.08, 0.18)

	_poll_control()


func _make_materials() -> void:
	_masonry_material = ShaderMaterial.new()
	_masonry_material.shader = MASONRY_SHADER
	_masonry_material.set_shader_parameter("build_front", 1.0)

	_terrain_material = ShaderMaterial.new()
	_terrain_material.shader = TERRAIN_SHADER

	_water_material = ShaderMaterial.new()
	_water_material.shader = WATER_SHADER


## Retint the shared terrain material and place the water plane for a site.
## Heights come from core/keep-core/src/terrain.rs:
##   promontory  plateau +10.5 m, river shelf bottom -12.7 m  -> river at -6.0 m
##   headland    upper shelf +17.5 m, sea-cliff foot  -16.0 m -> sea at -0.3 m
##   lake        island +4.2 m, lake basin -3.44 m            -> lake at -1.6 m
func _apply_site_look(site_id: String) -> void:
	var grass_tint := Color(1.0, 1.0, 1.0)
	var rock_tint := Color(1.0, 1.0, 1.0)
	var grass_lush := Color(0.24, 0.34, 0.16)
	var grass_dry := Color(0.34, 0.35, 0.19)
	var shore_height := -1000.0
	var wet_band := 0.0
	var water_level := 0.0
	var water_shallow := Color(0.16, 0.42, 0.40)
	var water_deep := Color(0.05, 0.18, 0.23)
	var water_foam := Color(0.84, 0.88, 0.84)

	match site_id:
		"coastal_headland":
			# Windswept coastal turf over slate-grey sea cliffs.
			grass_tint = Color(0.90, 1.02, 0.94)
			rock_tint = Color(0.88, 0.94, 1.03)
			grass_lush = Color(0.24, 0.33, 0.17)
			grass_dry = Color(0.36, 0.35, 0.21)
			shore_height = -0.3
			wet_band = 2.6
			water_level = -0.3
			water_shallow = Color(0.16, 0.40, 0.47)
			water_deep = Color(0.04, 0.14, 0.25)
			water_foam = Color(0.88, 0.91, 0.90)
		"lake_island":
			# Lush damp island grass rising out of a blue-green lake.
			grass_tint = Color(0.94, 1.12, 0.84)
			rock_tint = Color(0.95, 0.99, 0.98)
			grass_lush = Color(0.22, 0.36, 0.16)
			grass_dry = Color(0.32, 0.38, 0.20)
			shore_height = -1.6
			wet_band = 3.0
			water_level = -1.6
			water_shallow = Color(0.16, 0.44, 0.40)
			water_deep = Color(0.05, 0.19, 0.22)
			water_foam = Color(0.80, 0.84, 0.78)
		_:
			# m0a_promontory (and any unknown id): green upland meadow, warm pale
			# limestone cliff and a dark wet river bank in the lower shelf.
			grass_tint = Color(1.02, 1.06, 0.94)
			rock_tint = Color(1.20, 1.07, 0.88)
			grass_lush = Color(0.24, 0.34, 0.16)
			grass_dry = Color(0.35, 0.34, 0.19)
			shore_height = -6.0
			wet_band = 3.5
			water_level = -6.0
			water_shallow = Color(0.18, 0.34, 0.30)
			water_deep = Color(0.06, 0.16, 0.15)
			water_foam = Color(0.72, 0.76, 0.70)

	if _terrain_material != null:
		_terrain_material.set_shader_parameter("grass_lush", grass_lush)
		_terrain_material.set_shader_parameter("grass_dry", grass_dry)
		_terrain_material.set_shader_parameter("grass_tint", grass_tint)
		_terrain_material.set_shader_parameter("rock_tint", rock_tint)
		_terrain_material.set_shader_parameter("shore_height", shore_height)
		_terrain_material.set_shader_parameter("wet_shore_band", wet_band)

	_configure_water(water_level, water_shallow, water_deep, water_foam)


func _ensure_water() -> void:
	if _water_instance != null and is_instance_valid(_water_instance):
		return
	var plane := PlaneMesh.new()
	# Large enough to reach the fog/horizon from any orbit position; the
	# terrain mesh itself only spans 240 m.
	plane.size = Vector2(900.0, 900.0)
	_water_instance = MeshInstance3D.new()
	_water_instance.name = "Water"
	_water_instance.mesh = plane
	_water_instance.material_override = _water_material
	_water_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_water_instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
	add_child(_water_instance)


func _configure_water(
	level: float, shallow: Color, deep: Color, foam: Color
) -> void:
	_ensure_water()
	_water_instance.position = Vector3(0.0, level, 0.0)
	_water_instance.visible = true
	if _water_material != null:
		_water_material.set_shader_parameter("shallow_color", shallow)
		_water_material.set_shader_parameter("deep_color", deep)
		_water_material.set_shader_parameter("foam_color", foam)


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
	_sky_material = sky_material

	var environment := Environment.new()
	_environment = environment
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
	_sun.shadow_bias = 0.08
	_sun.shadow_normal_bias = 2.5
	_sun.shadow_blur = 1.2
	_sun.sky_mode = DirectionalLight3D.SKY_MODE_LIGHT_AND_SKY
	add_child(_sun)

	# Adopt the photo-mode lighting rig at the default golden-hour time so the
	# startup look, the time-of-day slider and the born-in-code shaders agree.
	_apply_sun_time(16.5)


## Position the sun, retint the sky/fog and realign the masonry bounce fill for
## an hour between 06:00 and 20:30 (M3.1 photo mode).
func _apply_sun_time(hour: float) -> void:
	if _sun == null:
		return
	var t := clampf((hour - 6.0) / 14.5, 0.0, 1.0)
	# A low, fast-setting arc: high and steady around noon, dropping to a warm
	# horizon at dawn and dusk.
	var arc := sin(PI * t)
	var elevation := 5.0 + 40.0 * pow(maxf(arc, 0.0), 7.0)
	var azimuth := lerpf(-20.0, 66.0, t)
	_sun.rotation_degrees = Vector3(-elevation, azimuth, 0.0)

	# 1.0 at midday, falling to 0.0 at the ends of the day.
	var midday := clampf(1.0 - absf(hour - 13.25) / 7.25, 0.0, 1.0)
	var sun_color := Color(1.0, 0.64, 0.40)
	sun_color = sun_color.lerp(Color(1.0, 0.78, 0.50), smoothstep(0.0, 0.45, midday))
	sun_color = sun_color.lerp(Color(1.0, 0.95, 0.88), smoothstep(0.55, 1.0, midday))
	_sun.light_color = sun_color
	_sun.light_energy = lerpf(1.15, 2.6, midday)

	if _sky_material != null:
		_sky_material.mie_color = sun_color
		_sky_material.rayleigh_color = (
			Color(0.22, 0.32, 0.56).lerp(Color(0.30, 0.42, 0.62), midday)
		)
		_sky_material.turbidity = lerpf(7.0, 4.6, midday)
	if _environment != null:
		_environment.fog_light_color = sun_color.lerp(Color(0.58, 0.58, 0.56), 0.4)
		_environment.fog_light_energy = lerpf(0.45, 0.35, midday)
		_environment.ambient_light_energy = lerpf(0.34, 0.58, midday)
		_environment.tonemap_exposure = lerpf(0.96, 1.06, midday)
		_environment.volumetric_fog_albedo = (
			sun_color.lerp(Color(0.90, 0.85, 0.78), 0.5)
		)

	# A directional light shines along its local -Z, so +Z points from the scene
	# toward the sun; the shader's stylised bounce uses the same direction.
	if _masonry_material != null:
		_masonry_material.set_shader_parameter(
			"sun_bounce_dir", _sun.global_transform.basis.z.normalized()
		)
		_masonry_material.set_shader_parameter("bounce_color", sun_color)


func _setup_camera() -> void:
	_camera = ORBIT_CAMERA_SCRIPT.new()
	_camera.name = "OrbitCamera"
	add_child(_camera)
	_camera.current = true
	_camera.connect("auto_orbit_cancelled", Callable(self, "_on_camera_auto_orbit_cancelled"))
	_camera.call("focus_on", Vector3(0.0, 8.0, 0.0), 90.0)


func _setup_hud() -> void:
	_hud = DEBUG_HUD_SCRIPT.new()
	_hud.name = "DebugHud"
	add_child(_hud)
	_hud.connect("build_front_changed", Callable(self, "_on_hud_build_front_changed"))
	_hud.connect("playing_changed", Callable(self, "_on_hud_playing_changed"))
	_hud.connect("speed_requested", Callable(self, "_on_hud_speed_requested"))
	_hud.connect("timelapse_requested", Callable(self, "_on_hud_timelapse_requested"))
	_hud.connect("mute_toggled", Callable(self, "_on_hud_mute_toggled"))
	_hud.connect("mode_requested", Callable(self, "_on_hud_mode_requested"))
	_hud.connect("style_requested", Callable(self, "_on_hud_style_requested"))
	_hud.connect("site_requested", Callable(self, "_on_hud_site_requested"))
	_hud.connect("tool_requested", Callable(self, "_on_hud_tool_requested"))
	_hud.connect("fix_requested", Callable(self, "_on_hud_fix_requested"))
	_hud.connect("undo_requested", Callable(self, "_on_undo_requested"))
	_hud.connect("save_requested", Callable(self, "_on_save_requested"))
	_hud.connect("load_requested", Callable(self, "_on_load_requested"))
	_hud.connect("cutaway_mode_requested", Callable(self, "_on_hud_cutaway_mode_requested"))
	_hud.connect(
		"cutaway_position_changed", Callable(self, "_on_hud_cutaway_position_changed")
	)
	_hud.connect("terrain_tool_requested", Callable(self, "_on_hud_terrain_tool_requested"))
	_hud.connect("photo_requested", Callable(self, "_on_hud_photo_requested"))
	_hud.connect("codex_requested", Callable(self, "_on_hud_codex_requested"))
	_hud.connect("commissions_requested", Callable(self, "_on_hud_commissions_requested"))


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
	_design_tool.connect(
		"tower_shaped_committed", Callable(self, "_on_tower_shaped_committed")
	)
	_design_tool.connect("gatehouse_committed", Callable(self, "_on_gatehouse_committed"))
	_design_tool.connect("ditch_committed", Callable(self, "_on_ditch_committed"))
	_design_tool.connect("keep_committed", Callable(self, "_on_keep_committed"))
	_design_tool.connect("range_committed", Callable(self, "_on_range_committed"))
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


## Procedural furniture for the dollhouse cutaway (M3.1).
func _setup_interior_props() -> void:
	_interior_props = INTERIOR_PROPS_SCRIPT.new()
	_interior_props.name = "InteriorProps"
	add_child(_interior_props)


## Photo-mode overlay (M3.1): time of day, FOV and clean PNG capture.
func _setup_photo_mode() -> void:
	_photo_mode = PHOTO_MODE_SCRIPT.new()
	_photo_mode.name = "PhotoMode"
	add_child(_photo_mode)
	_photo_mode.connect("sun_time_changed", Callable(self, "_on_photo_sun_time_changed"))
	_photo_mode.connect("fov_changed", Callable(self, "_on_photo_fov_changed"))
	_photo_mode.connect("capture_requested", Callable(self, "_on_photo_capture_requested"))
	_photo_mode.connect("exit_requested", Callable(self, "_on_photo_exit_requested"))
	if _camera != null:
		_default_fov = _camera.fov


## In-game historical codex (M3.1), fed by the bridge's codex entries.
func _setup_codex() -> void:
	_codex_panel = CODEX_PANEL_SCRIPT.new()
	_codex_panel.name = "CodexPanel"
	add_child(_codex_panel)
	_codex_panel.connect("close_requested", Callable(self, "_close_codex"))
	_refresh_codex_entries()


func _refresh_codex_entries() -> void:
	if _codex_panel == null or _bridge == null:
		return
	if not _bridge.has_method("list_codex_entries"):
		return
	var entries: Variant = _bridge.call("list_codex_entries")
	if entries is Array:
		_codex_panel.call("set_entries", entries)


## M3.2 commission picker + active-commission HUD.
func _setup_commission_panel() -> void:
	_commission_panel = COMMISSION_PANEL_SCRIPT.new()
	_commission_panel.name = "CommissionPanel"
	add_child(_commission_panel)
	_commission_panel.connect(
		"commission_selected", Callable(self, "_on_commission_selected")
	)
	_commission_panel.connect(
		"advance_season_requested", Callable(self, "_on_advance_season_requested")
	)
	_commission_panel.connect(
		"exit_commission_requested", Callable(self, "_on_exit_commission_requested")
	)
	_refresh_commission_entries()


func _refresh_commission_entries() -> void:
	if _commission_panel == null or _bridge == null:
		return
	if not _bridge.has_method("list_commissions"):
		return
	var entries: Variant = _bridge.call("list_commissions")
	if entries is Array:
		_commission_panel.call("set_commissions", entries)


## Pull the authoritative commission state and render it. Also drives the
## winter frost shader uniform so the season is visually unmistakable.
func _refresh_commission_panel() -> void:
	if _commission_panel == null or _bridge == null:
		return
	if not _bridge.has_method("get_commission_state"):
		return
	var state: Variant = _bridge.call("get_commission_state")
	if not (state is Dictionary):
		return
	var data: Dictionary = state
	_commission_active = bool(data.get("active", false))
	_commission_panel.call("refresh", data)
	_apply_commission_season(data)


func _apply_commission_season(state: Dictionary) -> void:
	var commission: Variant = state.get("commission", {})
	var frost := false
	if commission is Dictionary:
		frost = bool((commission as Dictionary).get("frost_active", false))
	_winter_frost = frost
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("winter_frost", 1.0 if frost else 0.0)


func _on_hud_commissions_requested() -> void:
	if _commission_panel == null:
		return
	if _commission_active:
		_refresh_commission_panel()
	else:
		_refresh_commission_entries()
		_commission_panel.call("open_picker")


## Build the in-game agent panel and the proposal ghost overlay root.
func _setup_agent_layer() -> void:
	_ghost_material = StandardMaterial3D.new()
	_ghost_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_ghost_material.albedo_color = Color(0.25, 0.85, 0.95, 0.42)
	_ghost_material.emission_enabled = true
	_ghost_material.emission = Color(0.25, 0.85, 0.95)
	_ghost_material.emission_energy_multiplier = 1.6
	_ghost_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_ghost_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED

	_ghost_root = Node3D.new()
	_ghost_root.name = "ProposalGhosts"
	add_child(_ghost_root)

	_agent_panel = AGENT_PANEL_SCRIPT.new()
	_agent_panel.name = "AgentPanel"
	add_child(_agent_panel)
	_agent_panel.connect(
		"permission_changed", Callable(self, "_on_agent_permission_changed")
	)
	_agent_panel.connect(
		"proposal_accept_requested", Callable(self, "_on_proposal_accept_requested")
	)
	_agent_panel.connect(
		"proposal_reject_requested", Callable(self, "_on_proposal_reject_requested")
	)
	_agent_panel.connect("ghost_toggled", Callable(self, "_on_ghost_toggled"))
	_agent_panel.connect("demo_proposal_requested", Callable(self, "_on_demo_proposal_requested"))
	_agent_panel.connect(
		"agent_pause_toggled", Callable(self, "_on_agent_pause_toggled")
	)
	_agent_panel.connect("request_submitted", Callable(self, "_on_request_submitted"))
	_agent_panel.connect(
		"proposal_changes_requested", Callable(self, "_on_proposal_changes_requested")
	)


func _start_control_server() -> void:
	if _bridge == null or not _bridge.has_method("start_control_server"):
		return
	# Headless verification must not hold the preferred port; an ephemeral bind
	# still exercises the exact same security and request path.
	var port := CONTROL_PORT
	if DisplayServer.get_name() == "headless":
		port = 0
	var result: Variant = _bridge.call("start_control_server", port)
	if not (result is Dictionary):
		push_warning("Keepwright: start_control_server() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		push_warning(
			"Keepwright: control server failed to start: %s" % str(data.get("error", "unknown"))
		)
		_agent_panel.call("set_server", "", false, "")
		return
	_control_url = str(data.get("url", ""))
	_control_token = str(data.get("token", ""))
	_agent_panel.call("set_server", _control_url, true, _control_token)
	print("Keepwright: control server listening on %s" % _control_url)


## Drain queued agent requests, refresh the scene/panel and fulfil render_view.
func _poll_control() -> void:
	if _bridge == null or not _bridge.has_method("poll_control"):
		return
	var result: Variant = _bridge.call("poll_control")
	if not (result is Dictionary):
		return
	var state: Dictionary = result

	var render_id := int(state.get("render_view_request_id", -1))
	if render_id >= 0 and not _render_view_busy:
		_fulfill_render_view(render_id, state)

	if bool(state.get("dirty", false)):
		var snapshot: Variant = _bridge.call("get_scene_snapshot")
		if snapshot is Dictionary:
			_apply_scene(snapshot as Dictionary, false, false)
		_refresh_agent_panel()

	if bool(state.get("proposals_changed", false)):
		_refresh_agent_panel()
		_refresh_ghosts()


## Capture a clean 3:2 render_view: hide HUD/agent overlays, wait one frame so
## the capture does not include them, crop before resizing (avoids squashing a
## 16:9 window) and restore the overlays afterwards.
func _fulfill_render_view(request_id: int, state: Dictionary) -> void:
	if _bridge == null or not _bridge.has_method("complete_render_view"):
		return
	_render_view_busy = true

	var yaw_deg := _render_view_option(state, "yaw_deg")
	var pitch_deg := _render_view_option(state, "pitch_deg")
	var distance_m := _render_view_option(state, "distance_m")
	var moved_camera := false
	var prev_yaw_deg := 0.0
	var prev_pitch_deg := 0.0
	var prev_distance_m := 0.0
	if _camera != null and (is_finite(yaw_deg) or is_finite(pitch_deg) or is_finite(distance_m)):
		prev_yaw_deg = rad_to_deg(float(_camera.get("_yaw")))
		prev_pitch_deg = rad_to_deg(float(_camera.get("_pitch")))
		prev_distance_m = float(_camera.get("_distance"))
		_camera.call("set_orbit_view", yaw_deg, pitch_deg, distance_m)
		moved_camera = true

	var hud_was_visible := false
	var panel_was_visible := false
	var photo_was_visible := false
	var codex_was_visible := false
	var commission_was_visible := false
	var prev_fov := _default_fov
	var prev_cutaway_mode := _cutaway_mode
	if _hud != null:
		hud_was_visible = _hud.visible
		_hud.visible = false
	if _agent_panel != null:
		panel_was_visible = _agent_panel.visible
		_agent_panel.visible = false
	if _photo_mode != null:
		photo_was_visible = _photo_mode.visible
		_photo_mode.visible = false
	if _codex_panel != null:
		codex_was_visible = _codex_panel.visible
		_codex_panel.visible = false
	if _commission_panel != null:
		commission_was_visible = _commission_panel.visible
		_commission_panel.visible = false
	# Capture the raw architectural form: no dollhouse cut and the default lens
	# (M7). Both are restored after the frame is read.
	if _cutaway_mode != 0:
		_cutaway_mode = 0
		_apply_cutaway()
	if _camera != null:
		prev_fov = _camera.fov
		_camera.fov = _default_fov

	# Let the renderer present one frame without the overlays before we read it.
	await RenderingServer.frame_post_draw

	var viewport := get_viewport()
	var image: Image = null
	if viewport != null and viewport.get_texture() != null:
		image = viewport.get_texture().get_image()

	if moved_camera and _camera != null:
		_camera.call("set_orbit_view", prev_yaw_deg, prev_pitch_deg, prev_distance_m)
	if _camera != null:
		_camera.fov = prev_fov
	if _cutaway_mode != prev_cutaway_mode:
		_cutaway_mode = prev_cutaway_mode
		_apply_cutaway()
	if _hud != null:
		_hud.visible = hud_was_visible
	if _agent_panel != null:
		_agent_panel.visible = panel_was_visible
	if _photo_mode != null:
		_photo_mode.visible = photo_was_visible
	if _codex_panel != null:
		_codex_panel.visible = codex_was_visible
	if _commission_panel != null:
		_commission_panel.visible = commission_was_visible

	if image == null or image.is_empty():
		_render_view_busy = false
		_bridge.call(
			"complete_render_view", request_id, "", "viewport capture is unavailable"
		)
		return

	image = _crop_to_aspect(image, float(RENDER_VIEW_SIZE.x) / float(RENDER_VIEW_SIZE.y))
	image.resize(RENDER_VIEW_SIZE.x, RENDER_VIEW_SIZE.y, Image.INTERPOLATE_BILINEAR)
	var png := image.save_png_to_buffer()
	var encoded := Marshalls.raw_to_base64(png)
	_bridge.call("complete_render_view", request_id, encoded, "")
	_render_view_busy = false


## Optional capture parameters may arrive either as flat keys
## (`render_view_yaw_deg`) or inside a nested `render_view` dictionary.
func _render_view_option(state: Dictionary, key: String) -> float:
	var nested: Variant = state.get("render_view", null)
	if nested is Dictionary and (nested as Dictionary).has(key):
		var nested_value: Variant = (nested as Dictionary).get(key)
		if nested_value is float or nested_value is int:
			return float(nested_value)
	var flat := "render_view_%s" % key
	var flat_value: Variant = state.get(flat, null)
	if flat_value is float or flat_value is int:
		return float(flat_value)
	return NAN


## Centre-crop to the requested aspect ratio before resizing so wide windows
## are not stretched into the 3:2 capture.
func _crop_to_aspect(image: Image, aspect: float) -> Image:
	var width := image.get_width()
	var height := image.get_height()
	if width <= 0 or height <= 0 or aspect <= 0.0:
		return image

	var target_width := int(round(float(height) * aspect))
	if target_width <= width:
		if target_width == width:
			return image
		var offset_x := int((width - target_width) * 0.5)
		return image.get_region(Rect2i(offset_x, 0, target_width, height))

	var target_height := int(round(float(width) / aspect))
	if target_height >= height:
		return image
	var offset_y := int((height - target_height) * 0.5)
	return image.get_region(Rect2i(0, offset_y, width, target_height))


func _refresh_agent_panel() -> void:
	if _agent_panel == null or _bridge == null:
		return
	if not _bridge.has_method("get_agent_panel_state"):
		return
	var state: Variant = _bridge.call("get_agent_panel_state")
	if state is Dictionary:
		_agent_panel.call("refresh", state as Dictionary)
		_ghost_visibility = _agent_panel.call("get_ghost_visibility")


func _refresh_ghosts() -> void:
	_clear_ghosts()
	if _bridge == null or _ghost_root == null:
		return
	if not _bridge.has_method("get_agent_panel_state"):
		return
	var state: Variant = _bridge.call("get_agent_panel_state")
	if not (state is Dictionary):
		return
	var proposals: Variant = (state as Dictionary).get("proposals", [])
	if not (proposals is Array):
		return
	for entry in (proposals as Array):
		if not (entry is Dictionary):
			continue
		var proposal: Dictionary = entry
		if str(proposal.get("status", "")) != "submitted":
			continue
		var proposal_id := str(proposal.get("id", ""))
		if not bool(_ghost_visibility.get(proposal_id, true)):
			continue
		var ghost: Variant = _bridge.call("build_proposal_ghost", proposal_id)
		if not (ghost is Dictionary) or not bool((ghost as Dictionary).get("ok", false)):
			continue
		_add_ghost_chunks((ghost as Dictionary).get("chunks", []))


func _add_ghost_chunks(chunks: Variant) -> void:
	if not (chunks is Array):
		return
	for entry in (chunks as Array):
		if not (entry is Dictionary):
			continue
		var mesh: Variant = (entry as Dictionary).get("lod0", null)
		if not (mesh is ArrayMesh):
			continue
		var instance := MeshInstance3D.new()
		instance.mesh = mesh as ArrayMesh
		instance.material_override = _ghost_material
		instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
		_ghost_root.add_child(instance)
		_ghost_nodes.append(instance)


func _clear_ghosts() -> void:
	for instance in _ghost_nodes:
		if is_instance_valid(instance):
			instance.queue_free()
	_ghost_nodes.clear()


func _on_demo_proposal_requested() -> void:
	if _bridge == null or not _bridge.has_method("create_demo_proposal"):
		return
	var result: Variant = _bridge.call("create_demo_proposal")
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call(
			"show_error",
			"Demo proposal failed: %s" % str(_dictionary_error(result))
		)
		return
	_hud.call("clear_error")
	_hud.call(
		"set_tool_status",
		"Demo proposal %s ready — review it in AGENTS & MCP"
		% str((result as Dictionary).get("proposal_id", ""))
	)
	_refresh_agent_panel()
	_refresh_ghosts()


func _on_proposal_accept_requested(proposal_id: String) -> void:
	if _bridge == null or not _bridge.has_method("accept_proposal"):
		return
	var result: Variant = _bridge.call("accept_proposal", proposal_id)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Proposal rejected: %s" % str(_dictionary_error(result)))
		return
	_apply_scene(result as Dictionary, false, false)
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Proposal %s accepted and applied" % proposal_id)
	_refresh_agent_panel()
	_refresh_ghosts()


func _on_proposal_reject_requested(proposal_id: String, reason: String) -> void:
	if _bridge == null or not _bridge.has_method("reject_proposal"):
		return
	var result: Variant = _bridge.call("reject_proposal", proposal_id, reason)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Reject failed: %s" % str(_dictionary_error(result)))
		return
	_hud.call("set_tool_status", "Proposal %s rejected" % proposal_id)
	_refresh_agent_panel()
	_refresh_ghosts()


func _on_agent_permission_changed(agent_id: String, permission: String) -> void:
	if _bridge == null or not _bridge.has_method("set_agent_permission"):
		return
	_bridge.call("set_agent_permission", agent_id, permission)
	_refresh_agent_panel()


func _on_agent_pause_toggled(agent_id: String, paused: bool) -> void:
	if _bridge == null:
		return
	var method := "pause_agent" if paused else "resume_agent"
	if not _bridge.has_method(method):
		return
	var result: Variant = _bridge.call(method, agent_id)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Agent update failed: %s" % _dictionary_error(result))
		return
	_hud.call("clear_error")
	_refresh_agent_panel()


func _on_request_submitted(prompt: String) -> void:
	if _bridge == null or not _bridge.has_method("submit_request"):
		return
	var result: Variant = _bridge.call("submit_request", prompt, "", 0)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Request failed: %s" % _dictionary_error(result))
		return
	_hud.call("clear_error")
	_hud.call(
		"set_tool_status",
		"Request %s queued for an agent" % str((result as Dictionary).get("request_id", ""))
	)
	_refresh_agent_panel()


func _on_proposal_changes_requested(proposal_id: String, feedback: String) -> void:
	if _bridge == null or not _bridge.has_method("ask_for_changes"):
		return
	var result: Variant = _bridge.call("ask_for_changes", proposal_id, feedback)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Change request failed: %s" % _dictionary_error(result))
		return
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Asked for changes to %s" % proposal_id)
	_refresh_agent_panel()


func _on_ghost_toggled(proposal_id: String, visible: bool) -> void:
	_ghost_visibility[proposal_id] = visible
	_refresh_ghosts()


func _dictionary_error(result: Variant) -> String:
	if result is Dictionary:
		return str((result as Dictionary).get("error", "unknown error"))
	return "no dictionary returned"


func _rebuild(mode: String) -> void:
	if _bridge == null:
		_hud.call("show_error", "Cannot build: KeepBridge is unavailable.")
		return
	if _mode_busy:
		return

	var method := "build_m1_edwardian"
	var label := "M1 Edwardian"
	match mode:
		"showcase":
			method = "build_m0a_showcase"
			label = "M0a Showcase"
		"m0b":
			method = "build_m0b_showcase"
			label = "M0b Showcase"
		"stress":
			method = "build_stress_200k"
			label = "200k Stress Test"
		"m2":
			method = "build_m2_baltic"
			label = "M2 Baltic Brick"
		_:
			method = "build_m1_edwardian"
			label = "M1 Edwardian"

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
	_refresh_agent_panel()
	_refresh_ghosts()
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
	if _timelapse_active:
		_stop_timelapse(false)
	_clear_world_meshes()
	_apply_site_look(str(data.get("site_id", "m0a_promontory")))
	_spawn_terrain(data.get("terrain"))
	_spawn_chunks(data.get("chunks"))

	# Dollhouse furniture is derived pure data; rebuild it for every scene edit.
	if _interior_props != null:
		var props: Variant = data.get("interior_props", [])
		if props is Array:
			_interior_props.call("build_props", props)
		else:
			_interior_props.call("clear_props")

	var block_count := int(data.get("block_count", 0))
	var chunk_count := int(data.get("chunk_count", 0))
	var triangle_count := int(data.get("triangle_count", 0))
	var layout_hash := str(data.get("layout_hash", ""))
	var document_hash := str(data.get("document_hash", ""))

	_hud.call(
		"set_scene_stats", block_count, chunk_count, layout_hash, triangle_count, document_hash
	)
	var assessment: Variant = data.get("assessment", {})
	if assessment is Dictionary:
		_hud.call("set_assessment", assessment as Dictionary)

	var historian: Variant = data.get("historian", {})
	if historian is Dictionary:
		_hud.call("set_historian", historian as Dictionary)

	# Mid-build salvage feedback (M3.2): an edit during the rising build may
	# dismantle already-laid stone, which is credited back to the commission.
	var salvage: Variant = data.get("salvage", null)
	if salvage is Dictionary and int((salvage as Dictionary).get("dismantled_blocks", 0)) > 0:
		var dismantled := int((salvage as Dictionary).get("dismantled_blocks", 0))
		var credit := int((salvage as Dictionary).get("salvage_credit_silver", 0))
		var preserved := int(
			round(float((salvage as Dictionary).get("preserved_fraction", 0.0)) * 100.0)
		)
		_hud.call(
			"show_warning",
			"Mid-build salvage: %d blocks dismantled (+%d silver), %d%% lower courses preserved"
			% [dismantled, credit, preserved]
		)
		_hud.call(
			"set_tool_status",
			"Mid-build salvage: %d blocks dismantled (+%d silver) — %d%% preserved"
			% [dismantled, credit, preserved]
		)
		_refresh_commission_panel()
	elif not data.has("salvage"):
		_hud.call("clear_warning")

	var style_id := str(data.get("style_id", "edwardian_welsh"))
	_hud.call("set_style_selection", style_id)
	if _masonry_material != null:
		var palette_index := 1 if str(data.get("material_palette", "limestone")) == "red_brick" else 0
		_masonry_material.set_shader_parameter("material_palette", palette_index)

	if reset_play:
		_hud.call("clear_error")
		_hud.call("set_playing", false)
		_hud.call("set_build_front", 1.0)
		_playing = false
		_apply_build_front(1.0)
	else:
		_apply_build_front(_build_front)

	var bounds := _compute_masonry_bounds(data)
	_masonry_bounds = bounds
	_update_ground_bounds(data)
	# Anchor the damp/moss course to the foot of this site's masonry.
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("wet_base_y", bounds.position.y)
	_apply_cutaway()
	_update_cutaway_range()
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

	# Every player edit lands here, so refresh the agent panel and proposal
	# ghosts too: stale proposal badges and Accept buttons must follow the live
	# document immediately (neither helper calls back into _apply_scene).
	if _agent_panel != null:
		_refresh_agent_panel()
		_refresh_ghosts()

	# Keep the commission card authoritative after every scene: this resyncs
	# frost/pause state on showcase loads (H1) and treasury/goal ticks after
	# ordinary edits, undo and accepted proposals (H2). The helper only reads
	# bridge state and never calls back into _apply_scene (no recursion).
	if _commission_panel != null:
		_refresh_commission_panel()


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
	# Scrubbing the slider takes manual control; stop any scripted replay.
	if _timelapse_active:
		_stop_timelapse(false)
	_apply_build_front(value)


func _on_hud_playing_changed(playing: bool) -> void:
	if playing and _timelapse_active:
		_stop_timelapse(false)
	_playing = playing
	_tick_timer = 0.0
	# Replaying a finished build starts cleanly from bare ground.
	if playing and _build_front >= 0.99:
		_apply_build_front(0.0)
		if _hud != null:
			_hud.call("set_build_front", 0.0)
	if _fx != null:
		_fx.call("set_build_front", _build_front, _playing)


## HUD speed preset. 0.0 is "Instant": snap to the completed wall.
func _on_hud_speed_requested(multiplier: float) -> void:
	if multiplier <= 0.0:
		_speed_multiplier = 1.0
		_playing = false
		if _timelapse_active:
			_stop_timelapse(false)
		if _hud != null:
			_hud.call("set_playing", false)
			_hud.call("set_build_front", 1.0)
			_hud.call("set_speed", 1.0)
		_apply_build_front(1.0)
		return
	_speed_multiplier = maxf(multiplier, 0.01)


func _on_hud_timelapse_requested() -> void:
	if _timelapse_active:
		_stop_timelapse(false)
		return
	_start_timelapse()


func _on_hud_mute_toggled(muted: bool) -> void:
	if _audio != null:
		_audio.call("set_muted", muted)
	if _hud != null:
		_hud.call("set_muted", muted)


func _start_timelapse() -> void:
	_playing = false
	_timelapse_active = true
	_timelapse_elapsed = 0.0
	_tick_timer = 0.0
	if _hud != null:
		_hud.call("set_playing", false)
		_hud.call("set_timelapse", true)
		_hud.call("set_build_front", 0.0)
	_apply_build_front(0.0)
	if _camera != null:
		_camera.call("start_auto_orbit", TIMELAPSE_YAW_RATE)


## Stop the replay. `complete` snaps the wall shut at 1.0 for a clean finish;
## otherwise the front holds where it was when the user took control.
func _stop_timelapse(complete: bool) -> void:
	_timelapse_active = false
	_timelapse_elapsed = 0.0
	if _camera != null:
		_camera.call("stop_auto_orbit")
	if _hud != null:
		_hud.call("set_timelapse", false)
	if complete:
		if _hud != null:
			_hud.call("set_build_front", 1.0)
		_apply_build_front(1.0)
	else:
		if _fx != null:
			_fx.call("set_build_front", _build_front, false)


func _on_camera_auto_orbit_cancelled() -> void:
	if _timelapse_active:
		_stop_timelapse(false)


func _apply_build_front(value: float) -> void:
	_build_front = clampf(value, 0.0, 1.0)
	if _bridge != null and _bridge.has_method("set_build_front_norm"):
		# Keep the authoritative mid-build salvage reference in sync with the
		# visible construction front before any interactive edit.
		_bridge.call("set_build_front_norm", _build_front)
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("build_front", _build_front)
	if _fx != null:
		# A time-lapse is a cinematic replay even though Play is not engaged;
		# scaffolding and dust must climb with the front either way.
		_fx.call("set_build_front", _build_front, _playing or _timelapse_active)


func _on_hud_mode_requested(mode: String) -> void:
	_rebuild(mode)


# ---------------------------------------------------------------------------
# M3.1: cutaway, terrain brushes, photo mode and the codex
# ---------------------------------------------------------------------------

func _on_hud_cutaway_mode_requested(mode: int) -> void:
	_set_cutaway_mode(mode)


func _on_hud_cutaway_position_changed(position: float) -> void:
	_set_cutaway_position(position)


func _default_storey_cut_y() -> float:
	if _masonry_bounds.size.y > 0.5:
		return _masonry_bounds.position.y + minf(5.5, _masonry_bounds.size.y * 0.45)
	return _ground_max_y + 2.0


func _set_cutaway_mode(mode: int) -> void:
	var clamped := clampi(mode, 0, 3)
	var changed := clamped != _cutaway_mode
	_cutaway_mode = clamped
	match clamped:
		1:
			# Horizontal plane: default to mid-first-storey of the castle so
			# the slice opens the masonry cleanly on every site elevation.
			if changed:
				_cutaway_pos = _default_storey_cut_y()
			_update_cutaway_range()
		2, 3:
			if changed:
				_cutaway_pos = 0.0
			_update_cutaway_range()
		_:
			_cutaway_pos = 100.0
			if _hud != null:
				var low := minf(_ground_min_y - 2.0, _masonry_bounds.position.y - 1.0)
				var high := maxf(
					_ground_max_y + 24.0,
					_masonry_bounds.position.y + _masonry_bounds.size.y + 4.0
				)
				_hud.call(
					"set_cutaway_range", low, high, clampf(_default_storey_cut_y(), low, high)
				)
	if _hud != null:
		_hud.call("set_cutaway_mode", clamped)
	_apply_cutaway()


## Sample the terrain surface to learn its true vertical range so the cutaway
## slider always spans the loaded site. Uses a bounded stride so a large terrain
## mesh never stalls an edit.
func _update_ground_bounds(data: Dictionary) -> void:
	var terrain: Variant = data.get("terrain", null)
	if terrain is ArrayMesh:
		var arrays: Array = (terrain as ArrayMesh).surface_get_arrays(0)
		if arrays.size() > Mesh.ARRAY_VERTEX:
			var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
			if not verts.is_empty():
				var stride := maxi(1, verts.size() / 2048)
				var heights := PackedFloat32Array()
				var index := 0
				while index < verts.size():
					heights.append(verts[index].y)
					index += stride
				heights.sort()
				_ground_min_y = heights[0]
				_ground_max_y = heights[heights.size() - 1]
				_ground_median_y = heights[heights.size() / 2]
				return
	# Fall back to the masonry foot when the terrain mesh is unavailable.
	_ground_min_y = _masonry_bounds.position.y
	_ground_max_y = _masonry_bounds.position.y
	_ground_median_y = _masonry_bounds.position.y


## Push a mode-appropriate slider range to the HUD. Horizontal cuts follow the
## site relief; vertical sections use a generous ±80 m window.
func _update_cutaway_range() -> void:
	if _hud == null:
		return
	match _cutaway_mode:
		1:
			var low := minf(_ground_min_y - 2.0, _masonry_bounds.position.y - 1.0)
			var high := maxf(
				_ground_max_y + 24.0,
				_masonry_bounds.position.y + _masonry_bounds.size.y + 4.0
			)
			_cutaway_pos = clampf(_cutaway_pos, low, high)
			_hud.call("set_cutaway_range", low, high, _cutaway_pos)
		2, 3:
			_cutaway_pos = clampf(_cutaway_pos, -80.0, 80.0)
			_hud.call("set_cutaway_range", -80.0, 80.0, _cutaway_pos)


func _set_cutaway_position(position: float) -> void:
	_cutaway_pos = position
	_apply_cutaway()


func _apply_cutaway() -> void:
	if _masonry_material != null:
		_masonry_material.set_shader_parameter("cutaway_mode", _cutaway_mode)
		_masonry_material.set_shader_parameter("cutaway_pos", _cutaway_pos)
	if _interior_props != null:
		_interior_props.call("set_cutaway", _cutaway_mode, _cutaway_pos)


func _on_hud_terrain_tool_requested(kind: String) -> void:
	_terrain_tool = kind
	if _design_tool != null:
		_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.NONE)
	if _hud != null:
		_hud.call(
			"set_tool_status",
			"Terrain: %s — LMB on the ground (Ctrl+Z undoes)" % kind.capitalize()
		)


## Apply the armed terrain brush at the ground point under a screen position.
func _sculpt_at_screen_position(screen_position: Vector2) -> void:
	if _bridge == null or not _bridge.has_method("sculpt_terrain"):
		return
	if not _bridge.has_method("pick_terrain"):
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	var from := camera.project_ray_origin(screen_position)
	var direction := camera.project_ray_normal(screen_position)
	var hit: Variant = _bridge.call("pick_terrain", from, direction)
	if not (hit is Dictionary) or not bool((hit as Dictionary).get("hit", false)):
		return
	var point: Variant = (hit as Dictionary).get("point", null)
	if not (point is Vector3):
		return
	var ground: Vector3 = point

	var delta := 1.5
	match _terrain_tool:
		"flatten":
			delta = ground.y
		"smooth":
			delta = 0.65
	var result: Variant = _bridge.call(
		"sculpt_terrain", _terrain_tool, Vector2(ground.x, ground.z), 8.0, delta
	)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Sculpt rejected: %s" % _dictionary_error(result))
		return
	_apply_scene(result as Dictionary, false, false)
	if _audio != null:
		_audio.call("play_place_thud", 0.62)
	_hud.call("clear_error")
	_hud.call(
		"set_tool_status",
		"Terrain %s at %.1f, %.1f m — Ctrl+Z to undo"
		% [_terrain_tool, ground.x, ground.z]
	)


func _on_hud_photo_requested() -> void:
	_set_photo_mode(not _photo_mode.visible)


func _on_photo_exit_requested() -> void:
	_set_photo_mode(false)


func _set_photo_mode(enabled: bool) -> void:
	if _photo_mode == null:
		return
	_photo_mode.visible = enabled
	if _hud != null:
		_hud.visible = not enabled
	if _agent_panel != null:
		_agent_panel.visible = not enabled
	if enabled:
		if _codex_panel != null:
			_codex_panel.visible = false
		if _commission_panel != null:
			_commission_was_visible = _commission_panel.visible
			_commission_panel.visible = false
		if _camera != null:
			_camera.fov = float(_photo_mode.call("get_fov"))
		_apply_sun_time(float(_photo_mode.call("get_sun_time")))
	else:
		if _commission_panel != null:
			_commission_panel.visible = _commission_was_visible
		if _camera != null:
			_camera.fov = _default_fov
		# Keep the chosen sun time: the scene stays photogenic after exiting.


func _on_photo_sun_time_changed(hour: float) -> void:
	_apply_sun_time(hour)


func _on_photo_fov_changed(fov: float) -> void:
	if _camera != null:
		_camera.fov = fov


func _on_photo_capture_requested() -> void:
	if _photo_mode == null:
		return
	# Hide the whole photo overlay (not just its bar) for a full frame so it can
	# never leak into the PNG, then capture and restore it (M3.1 M8).
	var was_visible := _photo_mode.visible
	_photo_mode.visible = false
	await get_tree().process_frame
	await RenderingServer.frame_post_draw
	var path := str(_photo_mode.call("capture_photo"))
	_photo_mode.visible = was_visible
	print("Keepwright: photo saved — %s" % path)


func _on_hud_codex_requested() -> void:
	_toggle_codex()


func _toggle_codex() -> void:
	if _codex_panel == null:
		return
	if not _codex_panel.visible and _photo_mode != null and _photo_mode.visible:
		_set_photo_mode(false)
	_codex_panel.visible = not _codex_panel.visible


func _close_codex() -> void:
	if _codex_panel != null:
		_codex_panel.visible = false


# ---------------------------------------------------------------------------
# M3.2: commissions, seasons and winter frost
# ---------------------------------------------------------------------------

func _on_commission_selected(commission_id: String) -> void:
	if _bridge == null or not _bridge.has_method("start_commission"):
		_hud.call(
			"show_error",
			"Cannot start commission: KeepBridge.start_commission() is unavailable."
		)
		return
	var result: Variant = _bridge.call("start_commission", commission_id)
	if not (result is Dictionary):
		_hud.call("show_error", "start_commission() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Commission rejected: %s" % _dictionary_error(result))
		return

	_apply_scene(data, true, true)
	_commission_active = true
	_apply_commission_season(data)
	_refresh_commission_panel()
	_refresh_agent_panel()
	_hud.call("clear_error")
	var title := commission_id
	var commission: Variant = data.get("commission", {})
	if commission is Dictionary:
		title = str((commission as Dictionary).get("title", commission_id))
	_hud.call("set_mode_label", "Commission — %s" % title)
	_hud.call("set_tool_status", "Commission accepted — raise the patron's castle")


func _on_advance_season_requested() -> void:
	if _bridge == null or not _bridge.has_method("advance_season"):
		_hud.call("show_error", "KeepBridge.advance_season() is unavailable.")
		return
	var result: Variant = _bridge.call("advance_season")
	if not (result is Dictionary):
		_hud.call("show_error", "advance_season() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Season failed: %s" % _dictionary_error(result))
		return

	var commission: Variant = data.get("commission", {})
	if commission is Dictionary:
		_apply_commission_season({"active": true, "commission": commission})
		var season_name := str((commission as Dictionary).get("season", "")).capitalize()
		if bool((commission as Dictionary).get("frost_active", false)):
			_hud.call(
				"set_tool_status",
				"%s — frost on the works; wall tops are thatched and construction is paused"
				% season_name
			)
		else:
			_hud.call(
				"set_tool_status",
				"%s — %s"
				% [season_name, str((commission as Dictionary).get("status_note", ""))]
			)
	_refresh_commission_panel()
	_hud.call("clear_error")


func _on_exit_commission_requested() -> void:
	if _bridge == null or not _bridge.has_method("end_commission"):
		_hud.call("show_error", "KeepBridge.end_commission() is unavailable.")
		return
	var result: Variant = _bridge.call("end_commission")
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Exit failed: %s" % _dictionary_error(result))
		return
	_commission_active = false
	_winter_frost = false
	_apply_commission_season({"active": false})
	if _commission_panel != null:
		_commission_panel.call("refresh", {"active": false})
		_commission_panel.visible = false
	_hud.call("clear_error")
	_hud.call("set_mode_label", "Sandbox")
	_hud.call("set_tool_status", "Returned to the free-build sandbox")


func _text_input_focused() -> bool:
	var focused := get_viewport().gui_get_focus_owner()
	return focused is LineEdit or focused is TextEdit


func _on_hud_tool_requested(tool: String) -> void:
	_terrain_tool = ""
	if _design_tool == null:
		return
	match tool:
		"wall":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.WALL)
			_hud.call("set_tool_status", "Tool: Wall — drag LMB between two points")
		"tower":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.TOWER)
			_hud.call("set_tool_status", "Tool: Round Tower — click LMB to place")
		"tower_d":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.TOWER_D)
			_hud.call("set_tool_status", "Tool: D-Tower — click LMB to place")
		"tower_square":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.TOWER_SQUARE)
			_hud.call("set_tool_status", "Tool: Square Tower — click LMB to place")
		"gatehouse":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.GATEHOUSE)
			_hud.call("set_tool_status", "Tool: Gatehouse — click LMB to place")
		"ditch":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.DITCH)
			_hud.call("set_tool_status", "Tool: Ditch — drag LMB across the approach")
		"keep":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.KEEP)
			_hud.call("set_tool_status", "Tool: Keep — click LMB to place a great keep")
		"hall":
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.HALL)
			_hud.call("set_tool_status", "Tool: Hall — click LMB to place a pitched-roof range")
		_:
			_design_tool.call("set_mode", DESIGN_TOOL_SCRIPT.Mode.NONE)
			_hud.call("set_tool_status", "Tool: View — orbit around the site")


func _on_hud_style_requested(style_id: String) -> void:
	if _bridge == null or not _bridge.has_method("set_style"):
		_hud.call("show_error", "Cannot switch style: KeepBridge.set_style() is unavailable.")
		return
	var result: Variant = _bridge.call("set_style", style_id)
	if not (result is Dictionary) or not bool((result as Dictionary).get("ok", false)):
		_hud.call("show_error", "Style rejected: %s" % _dictionary_error(result))
		return
	# Rebuild the scene dictionary so the shader palette and historian line follow
	# the new active pack without changing the document.
	if _bridge.has_method("get_scene_snapshot"):
		var snapshot: Variant = _bridge.call("get_scene_snapshot")
		if snapshot is Dictionary:
			_apply_scene(snapshot as Dictionary, false, false)
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Style: %s" % style_id)


func _on_hud_site_requested(site_id: String) -> void:
	if _bridge == null or not _bridge.has_method("load_site"):
		_hud.call("show_error", "Cannot switch sites: KeepBridge.load_site() is unavailable.")
		return
	var result: Variant = _bridge.call("load_site", site_id)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.load_site() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Site rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, true, true)
	_refresh_agent_panel()
	_refresh_ghosts()
	_hud.call("set_mode_label", "Empty site — %s" % site_id)
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Loaded empty site %s" % site_id)


func _on_hud_fix_requested(fix_json: String) -> void:
	if _bridge == null or not _bridge.has_method("apply_fix_json"):
		_hud.call("show_error", "Cannot fix: KeepBridge.apply_fix_json() is unavailable.")
		return
	var result: Variant = _bridge.call("apply_fix_json", fix_json)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.apply_fix_json() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Fix rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Structural fix applied — Ctrl+Z to undo")


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


func _on_tower_shaped_committed(
	center_m: Vector2,
	radius_m: float,
	height_m: float,
	plan: String,
	orientation_deg: int
) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 0.82)
	print(
		"Keepwright: %s tower committed — centre %s, radius %.1f m, height %.1f m"
		% [plan, str(center_m), radius_m, height_m]
	)
	if _bridge == null or not _bridge.has_method("add_tower_shaped"):
		_hud.call("set_tool_status", "%s tower committed: r %.1f m" % [plan, radius_m])
		return
	var result: Variant = _bridge.call(
		"add_tower_shaped", center_m, radius_m, height_m, plan, orientation_deg
	)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_tower_shaped() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Tower rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call("set_tool_status", "%s tower added — Ctrl+Z to undo" % plan)


func _on_gatehouse_committed(center_m: Vector2, orientation_deg: int) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 0.95)
	print(
		"Keepwright: gatehouse committed — centre %s, orientation %d°"
		% [str(center_m), orientation_deg]
	)
	if _bridge == null or not _bridge.has_method("add_gatehouse"):
		_hud.call("set_tool_status", "Gatehouse committed at %s" % str(center_m))
		return
	var result: Variant = _bridge.call("add_gatehouse", center_m, orientation_deg)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_gatehouse() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call(
			"show_error", "Gatehouse rejected: %s" % str(data.get("error", "unknown error"))
		)
		return
	_apply_scene(data, false, false)
	_hud.call("set_tool_status", "Gatehouse added — Ctrl+Z to undo")


func _on_ditch_committed(start_m: Vector2, end_m: Vector2, width_m: float, depth_m: float) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 0.70)
	var length := start_m.distance_to(end_m)
	print(
		"Keepwright: ditch committed — %.1f m long, %.1f m wide, %.1f m deep"
		% [length, width_m, depth_m]
	)
	if _bridge == null or not _bridge.has_method("add_ditch"):
		_hud.call("set_tool_status", "Ditch committed: %.1f m" % length)
		return
	var result: Variant = _bridge.call("add_ditch", start_m, end_m, width_m, depth_m)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_ditch() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Ditch rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call("set_tool_status", "Ditch added: %.1f m — Ctrl+Z to undo" % length)


func _on_keep_committed(center_m: Vector2, orientation_deg: int) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 1.1)
	print(
		"Keepwright: keep committed — centre %s, orientation %d°"
		% [str(center_m), orientation_deg]
	)
	if _bridge == null or not _bridge.has_method("add_keep"):
		_hud.call("set_tool_status", "Keep committed at %s" % str(center_m))
		return
	var result: Variant = _bridge.call("add_keep", center_m, orientation_deg)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_keep() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Keep rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call("set_tool_status", "Keep added — Ctrl+Z to undo")


func _on_range_committed(center_m: Vector2, kind: String, orientation_deg: int) -> void:
	if _audio != null:
		_audio.call("play_place_thud", 0.92)
	print(
		"Keepwright: %s range committed — centre %s, orientation %d°"
		% [kind, str(center_m), orientation_deg]
	)
	if _bridge == null or not _bridge.has_method("add_range"):
		_hud.call("set_tool_status", "%s range committed at %s" % [kind, str(center_m)])
		return
	var result: Variant = _bridge.call("add_range", center_m, kind, orientation_deg)
	if not (result is Dictionary):
		_hud.call("show_error", "KeepBridge.add_range() did not return a Dictionary.")
		return
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		_hud.call("show_error", "Range rejected: %s" % str(data.get("error", "unknown error")))
		return
	_apply_scene(data, false, false)
	_hud.call("set_tool_status", "%s range added — Ctrl+Z to undo" % kind)


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
	_refresh_agent_panel()
	_refresh_ghosts()
	_hud.call("clear_error")
	_hud.call("set_tool_status", "Loaded castle.keep.json")


func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		var mouse := event as InputEventMouseButton
		if (
			_terrain_tool != ""
			and mouse.button_index == MOUSE_BUTTON_LEFT
			and mouse.pressed
		):
			_sculpt_at_screen_position(mouse.position)
			get_viewport().set_input_as_handled()
		return
	if not (event is InputEventKey):
		return
	var key := event as InputEventKey
	if not key.pressed or key.echo:
		return
	if key.ctrl_pressed and key.keycode == KEY_Z:
		_on_undo_requested()
		get_viewport().set_input_as_handled()
		return
	if key.ctrl_pressed or key.alt_pressed:
		return
	match key.keycode:
		KEY_P:
			if not _text_input_focused():
				_on_hud_photo_requested()
				get_viewport().set_input_as_handled()
		KEY_C:
			if not _text_input_focused():
				_on_hud_codex_requested()
				get_viewport().set_input_as_handled()
		KEY_ESCAPE:
			if _photo_mode != null and _photo_mode.visible:
				_set_photo_mode(false)
				get_viewport().set_input_as_handled()
			elif _codex_panel != null and _codex_panel.visible:
				_close_codex()
				get_viewport().set_input_as_handled()
