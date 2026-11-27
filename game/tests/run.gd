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
	print("Keepwright M0a headless test")
	var failures: Array[String] = []

	if not ClassDB.class_exists("KeepBridge"):
		failures.append("ClassDB.class_exists(\"KeepBridge\") is false — build core/keep-gd and check game/keep_gd.gdextension.")
	else:
		var bridge: Object = ClassDB.instantiate("KeepBridge")
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
		for failure in failures:
			printerr("  FAIL: " + failure)
		quit(1)
		return

	quit(0)


func _check_result(result: Variant, elapsed: int, failures: Array[String]) -> void:
	if not (result is Dictionary):
		failures.append("build_m0a_showcase() did not return a Dictionary.")
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

	if failures.is_empty():
		print("M0a OK: blocks=%d chunks=%d tris=%d hash=%s build_ms=%d" % [
			block_count, chunk_count, triangle_count, layout_hash, elapsed
		])
