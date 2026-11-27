extends SceneTree
## Headless M0a contract test.
##
##   godot --headless --path game --script res://tests/run.gd
##
## Verifies that the KeepBridge extension is registered, that build_m0a_showcase()
## returns the documented dictionary shape and that the scene has actual content.
## Exits 0 on success, 1 on failure.

# Chunks span 16 m (CHUNK_MM); allow some slack for block extents and the
# difference between the block-centroid centre and the mesh AABB centre.
const MAX_LOD0_CENTER_OFFSET := 20.0

func _initialize() -> void:
	print("Keepwright M0b headless test")
	var failures: Array[String] = []
	var bridge: Object = null

	if not ClassDB.class_exists("KeepBridge"):
		failures.append("ClassDB.class_exists(\"KeepBridge\") is false — build core/keep-gd and check game/keep_gd.gdextension.")
	else:
		bridge = ClassDB.instantiate("KeepBridge")
		if bridge == null:
			failures.append("ClassDB.instantiate(\"KeepBridge\") returned null.")
		elif not bridge.has_method("build_m0a_showcase"):
			failures.append("KeepBridge has no build_m0a_showcase() method.")
		else:
			if bridge is Node:
				root.add_child(bridge as Node)
			var started := Time.get_ticks_msec()
			var result: Variant = bridge.call("build_m0a_showcase")
			var elapsed := Time.get_ticks_msec() - started
			_check_result(result, elapsed, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	if bridge != null:
		_check_wave2(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	_check_feel_scripts(failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	print("M0b feel scripts OK")
	quit(0)


func _for_each_failure(failures: Array[String]) -> void:
	for failure in failures:
		printerr("  FAIL: " + failure)


## M0b Wave 2 bridge checks: junction showcase, terrain picking, snapping and
## interactive add/undo round trips. Returns the bridge to the M0b showcase so
## later checks see the documented default document.
func _check_wave2(bridge: Object, failures: Array[String]) -> void:
	for method in [
		"build_m0b_showcase",
		"pick_terrain",
		"snap_point",
		"add_wall",
		"add_tower",
		"undo_last",
	]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	var m0b_started := Time.get_ticks_msec()
	var m0b_result: Variant = bridge.call("build_m0b_showcase")
	var m0b_elapsed := Time.get_ticks_msec() - m0b_started
	_check_result(m0b_result, m0b_elapsed, failures, "M0b showcase")
	if not (m0b_result is Dictionary):
		return

	var m0b: Dictionary = m0b_result
	var document_hash_before := str(m0b.get("document_hash", ""))
	if document_hash_before.is_empty():
		failures.append("build_m0b_showcase() returned an empty document_hash.")
	if str(m0b.get("layout_hash", "")).is_empty():
		failures.append("build_m0b_showcase() returned an empty layout_hash.")

	# Terrain picking: a downward ray from above the plateau must hit the
	# promontory; an upward ray must miss.
	var down: Variant = bridge.call(
		"pick_terrain", Vector3(0.0, 60.0, 0.0), Vector3(0.0, -1.0, 0.0)
	)
	if not (down is Dictionary):
		failures.append("pick_terrain() did not return a Dictionary.")
	else:
		var down_data: Dictionary = down
		if not bool(down_data.get("hit", false)):
			failures.append("pick_terrain() missed the plateau with a downward ray.")
		var down_point: Variant = down_data.get("point", null)
		if not (down_point is Vector3):
			failures.append("pick_terrain().point is not a Vector3.")
		elif absf((down_point as Vector3).x) > 0.5 or absf((down_point as Vector3).z) > 0.5:
			failures.append("pick_terrain() hit far from the ray axis: %s." % str(down_point))
	var up: Variant = bridge.call(
		"pick_terrain", Vector3(0.0, 60.0, 0.0), Vector3(0.0, 1.0, 0.0)
	)
	if up is Dictionary and bool((up as Dictionary).get("hit", false)):
		failures.append("pick_terrain() reported a hit for an upward ray.")

	# Snapping: just off the M0b drum-tower centre [19, 0] m.
	var snap: Variant = bridge.call("snap_point", Vector2(19.2, 0.15), 2.5)
	if not (snap is Dictionary):
		failures.append("snap_point() did not return a Dictionary.")
	else:
		var snap_data: Dictionary = snap
		if not bool(snap_data.get("snapped", false)):
			failures.append("snap_point() did not snap to the drum-tower centre.")
		var snap_point: Variant = snap_data.get("point", null)
		if not (snap_point is Vector2):
			failures.append("snap_point().point is not a Vector2.")
		elif (snap_point as Vector2).distance_to(Vector2(19.0, 0.0)) > 0.01:
			failures.append("snap_point() returned %s, expected (19, 0)." % str(snap_point))
		if str(snap_data.get("label", "")).is_empty():
			failures.append("snap_point() returned an empty label.")

	# Interactive edits: add a wall and a tower, then undo both.
	var wall_result: Variant = bridge.call(
		"add_wall", Vector2(-6.0, -7.0), Vector2(6.0, -7.0), 7.5, 1.8
	)
	_check_result(wall_result, 0, failures, "")
	var wall_document_hash := ""
	if wall_result is Dictionary:
		wall_document_hash = str((wall_result as Dictionary).get("document_hash", ""))
		if wall_document_hash.is_empty() or wall_document_hash == document_hash_before:
			failures.append("add_wall() did not change the document_hash.")

	var tower_result: Variant = bridge.call("add_tower", Vector2(9.0, -12.0), 2.5, 9.0)
	_check_result(tower_result, 0, failures, "")
	if tower_result is Dictionary:
		var tower_document_hash := str((tower_result as Dictionary).get("document_hash", ""))
		if tower_document_hash.is_empty() or tower_document_hash == wall_document_hash:
			failures.append("add_tower() did not change the document_hash.")

	var undo_tower: Variant = bridge.call("undo_last")
	if not (undo_tower is Dictionary):
		failures.append("undo_last() did not return a Dictionary.")
	elif not bool((undo_tower as Dictionary).get("undone", false)):
		failures.append("undo_last() did not report the tower as undone.")

	var undo_wall: Variant = bridge.call("undo_last")
	if not (undo_wall is Dictionary):
		failures.append("undo_last() did not return a Dictionary.")
	elif not bool((undo_wall as Dictionary).get("undone", false)):
		failures.append("undo_last() did not report the wall as undone.")
	elif str((undo_wall as Dictionary).get("document_hash", "")) != document_hash_before:
		failures.append("undo_last() did not restore the original document_hash.")

	var undo_empty: Variant = bridge.call("undo_last")
	if not (undo_empty is Dictionary):
		failures.append("undo_last() did not return a Dictionary when empty.")
	elif bool((undo_empty as Dictionary).get("undone", false)):
		failures.append("undo_last() reported an undo with an empty history.")

	if failures.is_empty():
		print(
			"M0b bridge OK: blocks=%d chunks=%d doc=%s"
			% [
				int(m0b.get("block_count", 0)),
				int(m0b.get("chunk_count", 0)),
				document_hash_before,
			]
		)


## M0b smoke checks: the FX/audio/design-tool scripts must load, instantiate and
## honour their documented public API even in --headless mode.
func _check_feel_scripts(failures: Array[String]) -> void:
	var fx_script: Variant = load("res://scripts/construction_fx.gd")
	if not (fx_script is GDScript):
		failures.append("construction_fx.gd did not load as a GDScript.")
	else:
		var fx: Variant = (fx_script as GDScript).new()
		if fx == null or not (fx is Node3D):
			failures.append("ConstructionFx is not a Node3D.")
		else:
			root.add_child(fx as Node3D)
			if not fx.has_method("configure_bounds"):
				failures.append("ConstructionFx has no configure_bounds().")
			if not fx.has_method("set_build_front"):
				failures.append("ConstructionFx has no set_build_front().")
			fx.call("configure_bounds", AABB(Vector3(-6.0, 0.0, -4.0), Vector3(12.0, 9.0, 8.0)))
			fx.call("set_build_front", 0.5, true)
			var scaffold_mid: bool = bool(fx.call("is_scaffold_visible"))
			var dust_mid: bool = bool(fx.call("is_dust_emitting"))
			fx.call("set_build_front", 1.0, false)
			var scaffold_done: bool = bool(fx.call("is_scaffold_visible"))
			var dust_done: bool = bool(fx.call("is_dust_emitting"))
			if not scaffold_mid:
				failures.append("ConstructionFx shows no scaffolding mid-build.")
			if not dust_mid:
				failures.append("ConstructionFx emits no dust mid-build.")
			if scaffold_done:
				failures.append("ConstructionFx did not strike its scaffolding at build_front 1.0.")
			if dust_done:
				failures.append("ConstructionFx keeps emitting dust after completion.")
			(fx as Node).free()

	var audio_script: Variant = load("res://scripts/build_audio.gd")
	if not (audio_script is GDScript):
		failures.append("build_audio.gd did not load as a GDScript.")
	else:
		var audio: Variant = (audio_script as GDScript).new()
		if audio == null or not (audio is Node):
			failures.append("BuildAudio is not a Node.")
		else:
			root.add_child(audio as Node)
			for method in ["play_place_thud", "play_stone_tick", "set_camera_distance"]:
				if not audio.has_method(method):
					failures.append("BuildAudio has no %s()." % method)
			audio.call("play_place_thud", 1.0)
			audio.call("play_stone_tick", 0.4)
			audio.call("set_camera_distance", 60.0)
			if not bool(audio.call("is_headless")):
				failures.append("BuildAudio did not detect --headless mode.")
			(audio as Node).free()

	var design_script: Variant = load("res://scripts/design_tool.gd")
	if not (design_script is GDScript):
		failures.append("design_tool.gd did not load as a GDScript.")
	else:
		var tool: Variant = (design_script as GDScript).new()
		if tool == null or not (tool is Node3D):
			failures.append("DesignTool is not a Node3D.")
		else:
			root.add_child(tool as Node3D)
			for method in ["set_mode", "get_mode", "set_ground_height", "set_defaults"]:
				if not tool.has_method(method):
					failures.append("DesignTool has no %s()." % method)
			for signal_name in ["wall_committed", "tower_committed", "snap_feedback"]:
				if not tool.has_signal(signal_name):
					failures.append("DesignTool has no %s signal." % signal_name)
			tool.call("set_ground_height", 4.0)
			tool.call("set_mode", 1)
			if int(tool.call("get_mode")) != 1:
				failures.append("DesignTool.set_mode(1) did not stick.")
			tool.call("set_mode", 2)
			if int(tool.call("get_mode")) != 2:
				failures.append("DesignTool.set_mode(2) did not stick.")
			tool.call("set_mode", 0)
			(tool as Node).free()


func _check_result(
	result: Variant,
	elapsed: int,
	failures: Array[String],
	label: String = "M0a showcase"
) -> void:
	if not (result is Dictionary):
		failures.append("%s did not return a Dictionary." % label)
		return

	var data: Dictionary = result

	var terrain: Variant = data.get("terrain", null)
	if terrain == null:
		failures.append("\"terrain\" is null.")
	elif not (terrain is ArrayMesh):
		failures.append("\"terrain\" is not an ArrayMesh.")

	var chunks: Variant = data.get("chunks", null)
	if chunks == null or not (chunks is Array):
		failures.append("\"chunks\" is not an Array.")
	elif (chunks as Array).is_empty():
		failures.append("\"chunks\" is empty.")
	else:
		var index := 0
		for entry in (chunks as Array):
			if not (entry is Dictionary):
				failures.append("chunks[%d] is not a Dictionary." % index)
				break
			var chunk: Dictionary = entry
			for key in ["key", "center", "lod0", "lod1", "lod2", "block_count"]:
				if not chunk.has(key):
					failures.append("chunks[%d] is missing key \"%s\"." % [index, key])
					break
			var lod0: Variant = chunk.get("lod0", null)
			if lod0 != null and not (lod0 is ArrayMesh):
				failures.append("chunks[%d].lod0 is not an ArrayMesh." % index)
			elif lod0 is ArrayMesh:
				# keep-core emits world-space chunk meshes, so lod0's AABB centre
				# must sit near the chunk centre. A local-space mesh would give an
				# AABB centred on the origin and silently break LOD culling (B1).
				var center: Variant = chunk.get("center", null)
				if center is Vector3:
					var center_vec: Vector3 = center
					var aabb_center: Vector3 = (lod0 as ArrayMesh).get_aabb().get_center()
					var offset: float = aabb_center.distance_to(center_vec)
					if offset > MAX_LOD0_CENTER_OFFSET:
						failures.append(
							"chunks[%d].lod0 AABB centre %s is %.2f m from chunk centre %s (max %.1f m)."
							% [index, str(aabb_center), offset, str(center_vec), MAX_LOD0_CENTER_OFFSET]
						)
			for lod_key in ["lod1", "lod2"]:
				var lod_mesh: Variant = chunk.get(lod_key, null)
				if lod_mesh == null:
					failures.append("chunks[%d].%s is null." % [index, lod_key])
				elif not (lod_mesh is ArrayMesh):
					failures.append("chunks[%d].%s is not an ArrayMesh." % [index, lod_key])
			index += 1

	var block_count := int(data.get("block_count", 0))
	var chunk_count := int(data.get("chunk_count", 0))
	var triangle_count := int(data.get("triangle_count", 0))
	var layout_hash := str(data.get("layout_hash", ""))

	if block_count <= 0:
		failures.append("block_count is not > 0 (got %d)." % block_count)
	if chunk_count <= 0:
		failures.append("chunk_count is not > 0 (got %d)." % chunk_count)

	if failures.is_empty() and not label.is_empty():
		print("%s OK: blocks=%d chunks=%d tris=%d hash=%s build_ms=%d" % [
			label, block_count, chunk_count, triangle_count, layout_hash, elapsed
		])
