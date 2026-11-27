extends SceneTree
## Headless M1 contract test.
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
	print("Keepwright M1 headless test")
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

	if bridge != null:
		_check_wave2b(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	if bridge != null:
		_check_m1(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	if bridge != null:
		_check_m2(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	if bridge != null:
		_check_control_and_proposals(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	if bridge != null:
		_check_m2_2(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	_check_feel_scripts(failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	_check_visual_polish(failures)

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

	_check_elements(m0b, failures)
	_check_lod_chunks(m0b, failures)

	if failures.is_empty():
		print(
			"M0b bridge OK: blocks=%d chunks=%d doc=%s"
			% [
				int(m0b.get("block_count", 0)),
				int(m0b.get("chunk_count", 0)),
				document_hash_before,
			]
		)


## M1.1 Wave 2b: session-backed save/load round trip and compensating undo.
##
## Saves a freshly built castle, resets to a different showcase, restores the
## save and requires identical document/layout hashes and block count. Then
## checks that undo restores the pre-edit document while the append-only log
## grows by one entry.
func _check_wave2b(bridge: Object, failures: Array[String]) -> void:
	for method in ["save_castle_json", "load_castle_json"]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	# Fresh session: known hash, empty history.
	var base_result: Variant = bridge.call("build_m0b_showcase")
	if not (base_result is Dictionary):
		failures.append("build_m0b_showcase() did not return a Dictionary for the save/load test.")
		return
	var base: Dictionary = base_result
	var base_hash := str(base.get("document_hash", ""))
	var base_history := int(base.get("history_len", -1))
	if base_history != 0:
		failures.append(
			"build_m0b_showcase() started with history_len %d, expected 0." % base_history
		)

	var wall_result: Variant = bridge.call(
		"add_wall", Vector2(-6.0, -7.0), Vector2(6.0, -7.0), 7.5, 1.8
	)
	if not (wall_result is Dictionary) or not bool((wall_result as Dictionary).get("ok", false)):
		failures.append("add_wall() was rejected in the save/load test: %s" % str(wall_result))
		return

	var tower_result: Variant = bridge.call("add_tower", Vector2(9.0, -12.0), 2.5, 9.0)
	if not (tower_result is Dictionary) or not bool((tower_result as Dictionary).get("ok", false)):
		failures.append("add_tower() was rejected in the save/load test: %s" % str(tower_result))
		return
	var built: Dictionary = tower_result
	var saved_hash := str(built.get("document_hash", ""))
	var saved_layout := str(built.get("layout_hash", ""))
	var saved_blocks := int(built.get("block_count", -1))
	var saved_history := int(built.get("history_len", -1))
	if saved_history != base_history + 2:
		failures.append(
			"history_len after two edits is %d, expected %d."
			% [saved_history, base_history + 2]
		)

	var save_result: Variant = bridge.call("save_castle_json")
	if not (save_result is Dictionary):
		failures.append("save_castle_json() did not return a Dictionary.")
		return
	var save_data: Dictionary = save_result
	if not bool(save_data.get("ok", false)):
		failures.append("save_castle_json() failed: %s" % str(save_data.get("error", "")))
		return
	var saved_json := str(save_data.get("json", ""))
	if saved_json.is_empty():
		failures.append("save_castle_json() returned an empty json payload.")
		return
	if str(save_data.get("document_hash", "")) != saved_hash:
		failures.append("save_castle_json().document_hash does not match the live scene hash.")
	if int(save_data.get("history_len", -1)) != saved_history:
		failures.append(
			"save_castle_json().history_len does not match the live history length."
		)

	# Reset to a different castle, then restore from the saved JSON.
	var other_result: Variant = bridge.call("build_m0a_showcase")
	if (
		other_result is Dictionary
		and str((other_result as Dictionary).get("document_hash", "")) == saved_hash
	):
		failures.append("build_m0a_showcase() did not reset to a different document.")

	var load_result: Variant = bridge.call("load_castle_json", saved_json)
	if not (load_result is Dictionary):
		failures.append("load_castle_json() did not return a Dictionary.")
		return
	var loaded: Dictionary = load_result
	if not bool(loaded.get("ok", false)):
		failures.append("load_castle_json() failed: %s" % str(loaded.get("error", "")))
		return
	if str(loaded.get("document_hash", "")) != saved_hash:
		failures.append(
			"load_castle_json() restored document_hash %s, expected %s."
			% [str(loaded.get("document_hash", "")), saved_hash]
		)
	if str(loaded.get("layout_hash", "")) != saved_layout:
		failures.append(
			"load_castle_json() restored layout_hash %s, expected %s."
			% [str(loaded.get("layout_hash", "")), saved_layout]
		)
	if int(loaded.get("block_count", -1)) != saved_blocks:
		failures.append(
			"load_castle_json() restored block_count %d, expected %d."
			% [int(loaded.get("block_count", -1)), saved_blocks]
		)
	if int(loaded.get("history_len", -1)) != saved_history:
		failures.append(
			"load_castle_json() restored history_len %d, expected %d."
			% [int(loaded.get("history_len", -1)), saved_history]
		)

	# A tampered save must be rejected and must leave the live session untouched.
	var live_before_tamper: Variant = bridge.call("save_castle_json")
	if (
		not (live_before_tamper is Dictionary)
		or not bool((live_before_tamper as Dictionary).get("ok", false))
	):
		failures.append("save_castle_json() failed before the tampered-load test.")
		return
	var live_hash_before_tamper := str(
		(live_before_tamper as Dictionary).get("document_hash", "")
	)
	var live_history_before_tamper := int(
		(live_before_tamper as Dictionary).get("history_len", -1)
	)

	var tampered_value: Variant = JSON.parse_string(saved_json)
	if not (tampered_value is Dictionary):
		failures.append("Saved JSON did not parse for the tampered-load test.")
	else:
		var tampered: Dictionary = tampered_value
		tampered["document_hash"] = int(tampered.get("document_hash", 0)) ^ 1
		var tampered_result: Variant = bridge.call(
			"load_castle_json", JSON.stringify(tampered)
		)
		if not (tampered_result is Dictionary):
			failures.append(
				"load_castle_json() did not return a Dictionary for a tampered save."
			)
		else:
			var tampered_data: Dictionary = tampered_result
			if bool(tampered_data.get("ok", true)):
				failures.append("load_castle_json() accepted a tampered document_hash.")
			if str(tampered_data.get("error", "")).is_empty():
				failures.append(
					"load_castle_json() returned an empty error for a tampered save."
				)

	var live_after_tamper: Variant = bridge.call("save_castle_json")
	if (
		not (live_after_tamper is Dictionary)
		or not bool((live_after_tamper as Dictionary).get("ok", false))
	):
		failures.append("save_castle_json() failed after the rejected tampered load.")
	else:
		var live_after: Dictionary = live_after_tamper
		if str(live_after.get("document_hash", "")) != live_hash_before_tamper:
			failures.append("A rejected tampered load changed the live document_hash.")
		if int(live_after.get("history_len", -1)) != live_history_before_tamper:
			failures.append("A rejected tampered load changed the live history_len.")

	# Compensating undo: restores the pre-edit document, appends to history.
	var pre_edit_hash := str(loaded.get("document_hash", ""))
	var pre_edit_layout := str(loaded.get("layout_hash", ""))
	var pre_edit_history := int(loaded.get("history_len", 0))
	var edit_result: Variant = bridge.call("add_tower", Vector2(24.0, -8.0), 2.0, 8.0)
	if not (edit_result is Dictionary) or not bool((edit_result as Dictionary).get("ok", false)):
		failures.append("add_tower() was rejected before the undo test: %s" % str(edit_result))
		return
	var edited: Dictionary = edit_result
	var edited_history := int(edited.get("history_len", -1))
	if str(edited.get("document_hash", "")) == pre_edit_hash:
		failures.append("add_tower() did not change the document before the undo test.")

	var undo_result: Variant = bridge.call("undo_last")
	if not (undo_result is Dictionary):
		failures.append("undo_last() did not return a Dictionary in the undo test.")
		return
	var undone: Dictionary = undo_result
	if not bool(undone.get("undone", false)):
		failures.append(
			"undo_last() refused the compensating undo: %s" % str(undone.get("reason", ""))
		)
		return
	if str(undone.get("document_hash", "")) != pre_edit_hash:
		failures.append("undo_last() did not restore the pre-edit document_hash.")
	if str(undone.get("layout_hash", "")) != pre_edit_layout:
		failures.append("undo_last() did not restore the pre-edit layout_hash.")
	if int(undone.get("history_len", -1)) != edited_history + 1:
		failures.append(
			"undo_last() did not append to history (history_len %d, expected %d)."
			% [int(undone.get("history_len", -1)), edited_history + 1]
		)
	if int(undone.get("history_len", 0)) <= pre_edit_history:
		failures.append("undo_last() truncated history instead of appending.")
	if base_hash == saved_hash:
		failures.append("save/load fixture did not change the document from its base state.")

	if failures.is_empty():
		print(
			"M1.1 save/load + compensating undo OK: doc=%s history=%d"
			% [saved_hash, int(undone.get("history_len", -1))]
		)


## M1.2 Wave 2b: default Edwardian showcase, site switching, shaped towers,
## gatehouses, ditches and the structural assessment one-click fix.
func _check_m1(bridge: Object, failures: Array[String]) -> void:
	for method in [
		"build_m1_edwardian",
		"load_site",
		"list_sites",
		"add_tower_shaped",
		"add_gatehouse",
		"add_ditch",
		"get_assessment",
		"apply_fix_json",
	]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	# 1. The default M1 Edwardian showcase builds, validates and meshes.
	var m1_started := Time.get_ticks_msec()
	var m1_result: Variant = bridge.call("build_m1_edwardian")
	var m1_elapsed := Time.get_ticks_msec() - m1_started
	_check_result(m1_result, m1_elapsed, failures, "M1 Edwardian showcase")
	if not (m1_result is Dictionary):
		return
	var m1: Dictionary = m1_result
	print("M1 Edwardian build_ms=%d" % m1_elapsed)

	if str(m1.get("site_id", "")) != "coastal_headland":
		failures.append(
			"build_m1_edwardian() reported site_id %s, expected coastal_headland."
			% str(m1.get("site_id", ""))
		)
	var report: Variant = m1.get("assessment", null)
	if not (report is Dictionary):
		failures.append("build_m1_edwardian() is missing an assessment Dictionary.")
	else:
		var assessment: Dictionary = report
		if not bool(assessment.get("passed", false)):
			failures.append(
				"build_m1_edwardian() assessment did not pass: %s"
				% str(assessment.get("findings", []))
			)
		if int(assessment.get("score", -1)) != 100:
			failures.append(
				"build_m1_edwardian() assessment score is %d, expected 100."
				% int(assessment.get("score", -1))
			)
	if int(m1.get("block_count", 0)) <= 0:
		failures.append("build_m1_edwardian() produced no masonry blocks.")
	if int(m1.get("chunk_count", 0)) <= 0:
		failures.append("build_m1_edwardian() produced no chunks.")
	_check_lod_chunks(m1, failures)

	# The gatehouse must contribute scaffolding footprints.
	var element_kinds := {}
	var elements: Variant = m1.get("elements", [])
	if elements is Array:
		for entry in (elements as Array):
			if entry is Dictionary:
				element_kinds[str((entry as Dictionary).get("kind", ""))] = true
	if not element_kinds.has("tower"):
		failures.append("M1 elements export no tower/gatehouse footprints.")
	if not element_kinds.has("wall"):
		failures.append("M1 elements export no wall footprints.")

	# 2. Site listing and switching.
	var sites: Variant = bridge.call("list_sites")
	if not (sites is Array) or (sites as Array).size() != 3:
		failures.append("list_sites() did not return three curated sites.")
	else:
		for index in 3:
			var site: Variant = (sites as Array)[index]
			if not (site is Dictionary):
				failures.append("list_sites()[%d] is not a Dictionary." % index)
				continue
			for key in ["id", "name", "description"]:
				if str((site as Dictionary).get(key, "")).is_empty():
					failures.append("list_sites()[%d] is missing \"%s\"." % [index, key])

	var lake_data := _require_ok(
		bridge.call("load_site", "lake_island"), "load_site(lake_island)", failures
	)
	if lake_data.is_empty():
		return
	if str(lake_data.get("site_id", "")) != "lake_island":
		failures.append(
			"load_site(lake_island) reported site_id %s." % str(lake_data.get("site_id", ""))
		)
	if int(lake_data.get("block_count", -1)) != 0:
		failures.append("load_site(lake_island) did not start from an empty castle.")

	var unknown: Variant = bridge.call("load_site", "atlantis")
	if not (unknown is Dictionary):
		failures.append("load_site() did not return a Dictionary for an unknown site.")
	elif bool((unknown as Dictionary).get("ok", true)):
		failures.append("load_site() accepted an unknown site id.")
	elif str((unknown as Dictionary).get("error", "")).is_empty():
		failures.append("load_site() returned an empty error for an unknown site.")

	# Save/load round trip on lake_island: build a wall, save, reset elsewhere,
	# restore, and require identical document/layout hashes and site id.
	_require_ok(bridge.call("load_site", "lake_island"), "load_site(lake_island) round trip", failures)
	var lake_wall := _require_ok(
		bridge.call("add_wall", Vector2(-8.0, 0.0), Vector2(8.0, 0.0), 8.0, 2.0),
		"add_wall(lake_island save/load)",
		failures
	)
	var lake_saved_hash := str(lake_wall.get("document_hash", ""))
	var lake_saved_layout := str(lake_wall.get("layout_hash", ""))
	var lake_save := _require_ok(
		bridge.call("save_castle_json"), "save_castle_json(lake_island)", failures
	)
	var lake_json := str(lake_save.get("json", ""))
	if lake_json.is_empty():
		failures.append("save_castle_json(lake_island) returned an empty json payload.")
	var reset_result: Variant = bridge.call("build_m0b_showcase")
	if not (reset_result is Dictionary) or str(
		(reset_result as Dictionary).get("document_hash", "")
	) == lake_saved_hash:
		failures.append("build_m0b_showcase() did not reset before the lake_island load.")
	var lake_loaded := _require_ok(
		bridge.call("load_castle_json", lake_json), "load_castle_json(lake_island)", failures
	)
	if not lake_loaded.is_empty():
		if str(lake_loaded.get("site_id", "")) != "lake_island":
			failures.append(
				"load_castle_json(lake_island) restored site_id %s."
				% str(lake_loaded.get("site_id", ""))
			)
		if str(lake_loaded.get("document_hash", "")) != lake_saved_hash:
			failures.append("load_castle_json(lake_island) did not restore document_hash.")
		if str(lake_loaded.get("layout_hash", "")) != lake_saved_layout:
			failures.append("load_castle_json(lake_island) did not restore layout_hash.")

	# 3. Gatehouse, ditch and square tower commands each change the document.
	var gatehouse_data := _require_ok(
		bridge.call("add_gatehouse", Vector2(0.0, 0.0), 0), "add_gatehouse()", failures
	)
	var gatehouse_hash := str(gatehouse_data.get("document_hash", ""))
	if gatehouse_hash.is_empty():
		failures.append("add_gatehouse() did not change the document_hash.")

	var ditch_data := _require_ok(
		bridge.call(
			"add_ditch", Vector2(-20.0, -10.0), Vector2(-20.0, 10.0), 5.0, 2.0
		),
		"add_ditch()",
		failures
	)
	var ditch_hash := str(ditch_data.get("document_hash", ""))
	if ditch_hash.is_empty() or ditch_hash == gatehouse_hash:
		failures.append("add_ditch() did not change the document_hash.")

	# Undo the ditch through KeepBridge: it must restore the pre-ditch document
	# while appending to the append-only history.
	var undo_ditch: Variant = bridge.call("undo_last")
	if not (undo_ditch is Dictionary):
		failures.append("undo_last() did not return a Dictionary for the ditch.")
	elif not bool((undo_ditch as Dictionary).get("undone", false)):
		failures.append("undo_last() refused to undo the ditch.")
	elif str((undo_ditch as Dictionary).get("document_hash", "")) != gatehouse_hash:
		failures.append("undo_last() did not restore the document_hash after undoing the ditch.")

	# Re-create the ditch so the remaining checks keep one in the document.
	ditch_data = _require_ok(
		bridge.call(
			"add_ditch", Vector2(-20.0, -10.0), Vector2(-20.0, 10.0), 5.0, 2.0
		),
		"add_ditch() re-create",
		failures
	)
	ditch_hash = str(ditch_data.get("document_hash", ""))

	var square_data := _require_ok(
		bridge.call(
			"add_tower_shaped", Vector2(14.0, 4.0), 3.0, 12.0, "square", 0
		),
		"add_tower_shaped(square)",
		failures
	)
	var square_hash := str(square_data.get("document_hash", ""))
	if square_hash.is_empty() or square_hash == ditch_hash:
		failures.append("add_tower_shaped() did not change the document_hash.")

	# A D-shaped tower rotated 90 degrees around +X.
	var d_shaped_data := _require_ok(
		bridge.call(
			"add_tower_shaped", Vector2(20.0, -4.0), 3.0, 12.0, "d_shaped", 90
		),
		"add_tower_shaped(d_shaped)",
		failures
	)
	var d_shaped_hash := str(d_shaped_data.get("document_hash", ""))
	if d_shaped_hash.is_empty() or d_shaped_hash == square_hash:
		failures.append("add_tower_shaped(d_shaped) did not change the document_hash.")

	# 4. Structural assessment + one-click fix on a thin 12 m x 1.0 m wall.
	_require_ok(bridge.call("load_site", "lake_island"), "load_site(lake_island) reset", failures)
	var thin_data := _require_ok(
		bridge.call("add_wall", Vector2(-6.0, 0.0), Vector2(6.0, 0.0), 12.0, 1.0),
		"add_wall(thin)",
		failures
	)
	if thin_data.is_empty():
		return

	var broken: Variant = bridge.call("get_assessment")
	if not (broken is Dictionary):
		failures.append("get_assessment() did not return a Dictionary.")
		return
	var broken_report: Dictionary = broken
	if bool(broken_report.get("passed", true)):
		failures.append("A 12 m x 1.0 m wall unexpectedly passed the assessment.")
	var fix_json := ""
	var findings: Variant = broken_report.get("findings", [])
	if findings is Array:
		for entry in (findings as Array):
			if not (entry is Dictionary):
				continue
			var finding: Dictionary = entry
			if bool(finding.get("has_fix", false)):
				var candidate := str(finding.get("fix_json", ""))
				if not candidate.is_empty():
					fix_json = candidate
					break
	if fix_json.is_empty():
		failures.append("The slender-wall finding carried no fix_json.")
		return

	var fixed_data := _require_ok(
		bridge.call("apply_fix_json", fix_json), "apply_fix_json()", failures
	)
	if fixed_data.is_empty():
		return
	var fixed_report: Variant = fixed_data.get("assessment", null)
	if not (fixed_report is Dictionary):
		fixed_report = bridge.call("get_assessment")
	if not (fixed_report is Dictionary):
		failures.append("No assessment available after applying the fix.")
		return
	if not bool((fixed_report as Dictionary).get("passed", false)):
		failures.append(
			"Assessment still fails after apply_fix_json(): %s"
			% str((fixed_report as Dictionary).get("findings", []))
		)
	if int((fixed_report as Dictionary).get("score", -1)) != 100:
		failures.append(
			"Assessment score after the fix is %d, expected 100."
			% int((fixed_report as Dictionary).get("score", -1))
		)

	if failures.is_empty():
		print(
			"M1.2 bridge OK: blocks=%d chunks=%d sites=3 fix=ok"
			% [int(m1.get("block_count", 0)), int(m1.get("chunk_count", 0))]
		)


## M2.1 Wave 2b: Baltic brick showcase, historian scores, style switching and
## the Keep/Hall placement commands.
func _check_m2(bridge: Object, failures: Array[String]) -> void:
	for method in ["build_m2_baltic", "set_style", "add_keep", "add_range"]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	var started := Time.get_ticks_msec()
	var result: Variant = bridge.call("build_m2_baltic")
	var elapsed := Time.get_ticks_msec() - started
	_check_result(result, elapsed, failures, "M2 Baltic showcase")
	if not (result is Dictionary):
		return
	var m2: Dictionary = result

	if str(m2.get("site_id", "")) != "lake_island":
		failures.append(
			"build_m2_baltic() reported site_id %s, expected lake_island."
			% str(m2.get("site_id", ""))
		)
	if str(m2.get("style_id", "")) != "baltic_brick":
		failures.append(
			"build_m2_baltic() reported style_id %s, expected baltic_brick."
			% str(m2.get("style_id", ""))
		)
	if str(m2.get("material_palette", "")) != "red_brick":
		failures.append(
			"build_m2_baltic() reported material_palette %s, expected red_brick."
			% str(m2.get("material_palette", ""))
		)
	if int(m2.get("block_count", 0)) <= 0:
		failures.append("build_m2_baltic() produced no masonry blocks.")

	var historian: Variant = m2.get("historian", null)
	if not (historian is Dictionary):
		failures.append("build_m2_baltic() is missing a historian Dictionary.")
	else:
		var report: Dictionary = historian
		if int(report.get("overall_score", 0)) < 80:
			failures.append(
				"build_m2_baltic() historian overall_score is %d, expected >= 80."
				% int(report.get("overall_score", 0))
			)
		for key in [
			"authenticity_score",
			"defensive_score",
			"domestic_score",
			"findings",
		]:
			if not report.has(key):
				failures.append("build_m2_baltic() historian is missing \"%s\"." % key)

	var style_result := _require_ok(
		bridge.call("set_style", "edwardian_welsh"), "set_style(edwardian_welsh)", failures
	)
	if not style_result.is_empty():
		if str(style_result.get("style_id", "")) != "edwardian_welsh":
			failures.append("set_style() did not report the Edwardian style.")
		if str(style_result.get("material_palette", "")) != "limestone":
			failures.append("set_style() did not report the limestone palette.")
		if not (style_result.get("assessment", null) is Dictionary):
			failures.append("set_style() returned no assessment Dictionary.")
		if not (style_result.get("historian", null) is Dictionary):
			failures.append("set_style() returned no historian Dictionary.")

	var keep_result := _require_ok(
		bridge.call("add_keep", Vector2(30.0, 0.0), 0), "add_keep()", failures
	)
	var keep_hash := str(keep_result.get("document_hash", ""))
	if keep_hash.is_empty() or keep_hash == str(m2.get("document_hash", "")):
		failures.append("add_keep() did not change the document_hash.")

	var range_result := _require_ok(
		bridge.call("add_range", Vector2(30.0, 16.0), "hall", 0), "add_range(hall)", failures
	)
	var range_hash := str(range_result.get("document_hash", ""))
	if range_hash.is_empty() or range_hash == keep_hash:
		failures.append("add_range() did not change the document_hash.")

	if failures.is_empty():
		print(
			"M2.1 bridge OK: blocks=%d historian=%d palette=red_brick keep+hall=ok"
			% [
				int(m2.get("block_count", 0)),
				int((historian as Dictionary).get("overall_score", 0)),
			]
		)


## M1.3 Wave 2b: localhost control server, demo proposals, ghost overlays and
## the in-game agent panel.
func _check_control_and_proposals(bridge: Object, failures: Array[String]) -> void:
	for method in [
		"start_control_server",
		"poll_control",
		"complete_render_view",
		"get_agent_panel_state",
		"set_agent_permission",
		"accept_proposal",
		"reject_proposal",
		"build_proposal_ghost",
		"create_demo_proposal",
		"get_scene_snapshot",
	]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	var base_result: Variant = bridge.call("build_m1_edwardian")
	if not (base_result is Dictionary):
		failures.append("build_m1_edwardian(control test) did not return a Dictionary.")
		return
	var base: Dictionary = base_result
	var base_hash := str(base.get("document_hash", ""))
	var base_history := int(base.get("history_len", -1))

	# 1. Ephemeral loopback control server with a 32-hex session token.
	var server := _require_ok(
		bridge.call("start_control_server", 0), "start_control_server(0)", failures
	)
	if server.is_empty():
		return
	var port := int(server.get("port", 0))
	var url := str(server.get("url", ""))
	var token := str(server.get("token", ""))
	if port <= 0:
		failures.append("start_control_server(0) did not return an ephemeral port.")
	if url != "127.0.0.1:%d" % port:
		failures.append("start_control_server() URL %s does not match port %d." % [url, port])
	if not url.begins_with("127.0.0.1:"):
		failures.append("start_control_server() did not bind loopback: %s" % url)
	if token.length() != 32 or not token.is_valid_hex_number():
		failures.append("start_control_server() token is not 32 hex characters: %s" % token)

	var poll: Variant = bridge.call("poll_control")
	if not (poll is Dictionary):
		failures.append("poll_control() did not return a Dictionary.")
	else:
		var poll_data: Dictionary = poll
		for key in ["dirty", "proposals_changed", "render_view_request_id"]:
			if not poll_data.has(key):
				failures.append("poll_control() is missing \"%s\"." % key)

	var panel_state: Variant = bridge.call("get_agent_panel_state")
	if not (panel_state is Dictionary):
		failures.append("get_agent_panel_state() did not return a Dictionary.")
	else:
		var panel_data: Dictionary = panel_state
		if not bool(panel_data.get("server_running", false)):
			failures.append("get_agent_panel_state() reports the server as stopped.")
		if str(panel_data.get("server_url", "")) != url:
			failures.append("get_agent_panel_state() server_url does not match the server.")
		var agents: Variant = panel_data.get("agents", null)
		if not (agents is Array) or (agents as Array).is_empty():
			failures.append("get_agent_panel_state() returned no agents.")
		var listed: Variant = panel_data.get("proposals", null)
		if not (listed is Array):
			failures.append("get_agent_panel_state().proposals is not an Array.")

	# 2. Demo proposal -> non-empty ghost chunks -> reject leaves the document.
	var demo := _require_ok(
		bridge.call("create_demo_proposal"), "create_demo_proposal()", failures
	)
	if demo.is_empty():
		return
	var demo_id := str(demo.get("proposal_id", ""))
	if demo_id.is_empty():
		failures.append("create_demo_proposal() returned an empty proposal_id.")
	if str(demo.get("semantic_diff", "")).is_empty():
		failures.append("create_demo_proposal() returned an empty semantic_diff.")

	var ghost := _require_ok(
		bridge.call("build_proposal_ghost", demo_id), "build_proposal_ghost()", failures
	)
	if not ghost.is_empty():
		var chunks: Variant = ghost.get("chunks", null)
		if not (chunks is Array) or (chunks as Array).is_empty():
			failures.append("build_proposal_ghost() returned no chunks.")
		if int(ghost.get("block_count", 0)) <= 0:
			failures.append("build_proposal_ghost() returned no blocks.")

	var pending_state: Variant = bridge.call("get_agent_panel_state")
	if pending_state is Dictionary:
		var found_submitted := false
		for entry in ((pending_state as Dictionary).get("proposals", []) as Array):
			if entry is Dictionary and str((entry as Dictionary).get("id", "")) == demo_id:
				found_submitted = str((entry as Dictionary).get("status", "")) == "submitted"
		if not found_submitted:
			failures.append("get_agent_panel_state() did not list the demo as submitted.")

	var rejected := _require_ok(
		bridge.call("reject_proposal", demo_id, "Not this time"),
		"reject_proposal()",
		failures
	)
	if not rejected.is_empty():
		var after_reject := _require_ok(
			bridge.call("save_castle_json"), "save_castle_json(after reject)", failures
		)
		if not after_reject.is_empty():
			if str(after_reject.get("document_hash", "")) != base_hash:
				failures.append("reject_proposal() changed the live document hash.")
			if int(after_reject.get("history_len", -1)) != base_history:
				failures.append("reject_proposal() changed the command history.")

	# 3. A second demo proposal accepted commits as one attributed batch.
	var demo_two := _require_ok(
		bridge.call("create_demo_proposal"), "create_demo_proposal(second)", failures
	)
	if demo_two.is_empty():
		return
	var demo_two_id := str(demo_two.get("proposal_id", ""))
	if demo_two_id == demo_id:
		failures.append("create_demo_proposal() reused the rejected proposal id.")

	var accepted := _require_ok(
		bridge.call("accept_proposal", demo_two_id), "accept_proposal()", failures
	)
	if not accepted.is_empty():
		if str(accepted.get("document_hash", "")) == base_hash:
			failures.append("accept_proposal() did not change the document hash.")
		if int(accepted.get("history_len", -1)) != base_history + 1:
			failures.append(
				"accept_proposal() did not append exactly one history entry (got %d)."
				% int(accepted.get("history_len", -1))
			)
		if int(accepted.get("block_count", 0)) <= 0:
			failures.append("accept_proposal() returned no masonry after applying.")

	# 4. The panel script builds, exposes its API and survives a refresh.
	var panel_script: Variant = load("res://scripts/agent_panel.gd")
	if not (panel_script is GDScript):
		failures.append("agent_panel.gd did not load as a GDScript.")
	else:
		var panel: Variant = (panel_script as GDScript).new()
		if panel == null or not (panel is CanvasLayer):
			failures.append("AgentPanel is not a CanvasLayer.")
		else:
			root.add_child(panel as CanvasLayer)
			for method in ["set_server", "refresh", "get_ghost_visibility"]:
				if not panel.has_method(method):
					failures.append("AgentPanel has no %s()." % method)
			for signal_name in [
				"permission_changed",
				"proposal_accept_requested",
				"proposal_reject_requested",
				"ghost_toggled",
				"demo_proposal_requested",
			]:
				if not panel.has_signal(signal_name):
					failures.append("AgentPanel has no %s signal." % signal_name)
			panel.call("set_server", "127.0.0.1:17731", true, "token")
			panel.call("refresh", {"agents": [], "proposals": []})
			if not (panel.call("get_ghost_visibility") is Dictionary):
				failures.append("AgentPanel.get_ghost_visibility() is not a Dictionary.")
			(panel as Node).free()

	if failures.is_empty():
		print(
			"M1.3 control + proposals OK: %s token=%s… ghost+accept/reject"
			% [url, token.substr(0, 6)]
		)


func _require_ok(result: Variant, label: String, failures: Array[String]) -> Dictionary:
	if not (result is Dictionary):
		failures.append("%s did not return a Dictionary." % label)
		return {}
	var data: Dictionary = result
	if not bool(data.get("ok", false)):
		failures.append("%s was rejected: %s" % [label, str(data.get("error", "unknown error"))])
		return {}
	return data


## M2.2 Wave 2b: pause/resume, the request inbox, ask-for-changes, historian
## deltas, stale proposals and the activity feed.
func _check_m2_2(bridge: Object, failures: Array[String]) -> void:
	for method in [
		"pause_agent",
		"resume_agent",
		"disconnect_agent",
		"submit_request",
		"ask_for_changes",
		"ack_request",
		"resolve_request",
	]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	var base_result: Variant = bridge.call("build_m1_edwardian")
	if not (base_result is Dictionary):
		failures.append("build_m1_edwardian(M2.2 test) did not return a Dictionary.")
		return

	# 1. Pause/resume is reflected in the panel state.
	var paused := _require_ok(
		bridge.call("pause_agent", "mason"), "pause_agent(mason)", failures
	)
	if not paused.is_empty():
		if str(paused.get("state", "")) != "paused":
			failures.append("pause_agent() did not report the paused state.")
		var paused_panel: Variant = bridge.call("get_agent_panel_state")
		if paused_panel is Dictionary:
			var paused_state := _agent_state(paused_panel as Dictionary, "mason")
			if paused_state.is_empty():
				failures.append("get_agent_panel_state() lost the paused agent.")
			elif paused_state != "paused":
				failures.append("get_agent_panel_state() did not reflect the paused agent.")

	var resumed := _require_ok(
		bridge.call("resume_agent", "mason"), "resume_agent(mason)", failures
	)
	if not resumed.is_empty() and str(resumed.get("state", "")) == "paused":
		failures.append("resume_agent() left the agent paused.")

	# 2. Request inbox round trip.
	var submitted := _require_ok(
		bridge.call("submit_request", "Add a gatehouse on the south approach", "", 0),
		"submit_request()",
		failures
	)
	if submitted.is_empty():
		return
	var request_id := str(submitted.get("request_id", ""))
	if request_id.is_empty():
		failures.append("submit_request() returned an empty request_id.")
	if str(submitted.get("status", "")) != "open":
		failures.append("submit_request() did not return an open request.")

	var acked := _require_ok(
		bridge.call("ack_request", request_id, "mcp-agent"), "ack_request()", failures
	)
	if not acked.is_empty() and str(acked.get("status", "")) != "acknowledged":
		failures.append("ack_request() did not mark the request acknowledged.")

	var resolved := _require_ok(
		bridge.call("resolve_request", request_id, "mcp-agent", "Gatehouse placed"),
		"resolve_request()",
		failures
	)
	if not resolved.is_empty():
		if str(resolved.get("status", "")) != "resolved":
			failures.append("resolve_request() did not mark the request resolved.")
		if str(resolved.get("reply", "")) != "Gatehouse placed":
			failures.append("resolve_request() did not record the reply.")

	var panel_after_requests: Variant = bridge.call("get_agent_panel_state")
	if not (panel_after_requests is Dictionary):
		failures.append("get_agent_panel_state() did not return a Dictionary after requests.")
	else:
		var panel_data: Dictionary = panel_after_requests
		var requests: Variant = panel_data.get("requests", null)
		if not (requests is Array):
			failures.append("get_agent_panel_state().requests is not an Array.")
		elif not _has_request(requests as Array, request_id):
			failures.append("get_agent_panel_state() did not list the submitted request.")
		var activity: Variant = panel_data.get("activity", null)
		if not (activity is Array) or (activity as Array).is_empty():
			failures.append("get_agent_panel_state() returned an empty activity feed.")

	# 3. Ask-for-changes is linked to a demo proposal, whose panel entry carries
	# historian deltas and a stale flag.
	var demo := _require_ok(
		bridge.call("create_demo_proposal"), "create_demo_proposal(M2.2)", failures
	)
	if not demo.is_empty():
		var demo_id := str(demo.get("proposal_id", ""))
		var demo_entry := _proposal_entry(bridge, demo_id)
		if demo_entry.is_empty():
			failures.append("get_agent_panel_state() did not list the demo proposal.")
		else:
			for key in ["historian_before", "historian_after", "historian_delta"]:
				if not demo_entry.has(key):
					failures.append("Demo proposal is missing \"%s\"." % key)
				elif not (demo_entry.get(key) is int or demo_entry.get(key) is float):
					failures.append("Demo proposal \"%s\" is not numeric." % key)
			if bool(demo_entry.get("stale", true)):
				failures.append("A fresh demo proposal was reported as stale.")

		var changes := _require_ok(
			bridge.call("ask_for_changes", demo_id, "Please make the tower taller"),
			"ask_for_changes()",
			failures
		)
		if not changes.is_empty():
			if str(changes.get("proposal_id", "")) != demo_id:
				failures.append("ask_for_changes() did not link the request to the proposal.")
			if str(changes.get("prompt", "")) != "Please make the tower taller":
				failures.append("ask_for_changes() did not keep the feedback text.")

		# Editing the castle after the proposal was begun must mark it stale.
		var edit := _require_ok(
			bridge.call("add_tower", Vector2(40.0, 0.0), 2.0, 8.0),
			"add_tower(stale)",
			failures
		)
		if not edit.is_empty():
			var stale_entry := _proposal_entry(bridge, demo_id)
			if stale_entry.is_empty():
				failures.append("The stale test lost the demo proposal.")
			elif not bool(stale_entry.get("stale", false)):
				failures.append("agent_panel_state() did not mark a superseded proposal stale.")

	# 4. Disconnect removes the agent.
	var disconnected := _require_ok(
		bridge.call("disconnect_agent", "mason"), "disconnect_agent(mason)", failures
	)
	if not disconnected.is_empty():
		var after_disconnect: Variant = bridge.call("get_agent_panel_state")
		if (
			after_disconnect is Dictionary
			and not _agent_state(after_disconnect as Dictionary, "mason").is_empty()
		):
			failures.append("disconnect_agent() did not remove the agent from the panel.")

	# 5. The upgraded panel renders the full M2.2 state without errors.
	var panel_script: Variant = load("res://scripts/agent_panel.gd")
	if not (panel_script is GDScript):
		failures.append("agent_panel.gd did not load as a GDScript.")
	elif panel_after_requests is Dictionary:
		var panel: Variant = (panel_script as GDScript).new()
		if panel == null or not (panel is CanvasLayer):
			failures.append("AgentPanel is not a CanvasLayer.")
		else:
			root.add_child(panel as CanvasLayer)
			for signal_name in [
				"agent_pause_toggled",
				"request_submitted",
				"proposal_changes_requested",
			]:
				if not panel.has_signal(signal_name):
					failures.append("AgentPanel has no %s signal." % signal_name)
			panel.call("set_server", "127.0.0.1:17731", true, "token")
			panel.call("refresh", panel_after_requests as Dictionary)
			if not (panel.call("get_ghost_visibility") is Dictionary):
				failures.append("AgentPanel.get_ghost_visibility() is not a Dictionary.")
			(panel as Node).free()

	if failures.is_empty():
		print(
			"M2.2 agent control OK: pause/resume, request inbox, ask-for-changes, stale+activity"
		)


func _agent_state(state: Dictionary, agent_id: String) -> String:
	for entry in state.get("agents", []):
		if entry is Dictionary and str((entry as Dictionary).get("id", "")) == agent_id:
			return str((entry as Dictionary).get("state", ""))
	return ""


func _has_request(requests: Array, request_id: String) -> bool:
	for entry in requests:
		if entry is Dictionary and str((entry as Dictionary).get("id", "")) == request_id:
			return true
	return false


func _proposal_entry(bridge: Object, proposal_id: String) -> Dictionary:
	var state: Variant = bridge.call("get_agent_panel_state")
	if not (state is Dictionary):
		return {}
	var listed: Variant = (state as Dictionary).get("proposals", [])
	if not (listed is Array):
		return {}
	for entry in (listed as Array):
		if entry is Dictionary and str((entry as Dictionary).get("id", "")) == proposal_id:
			return entry as Dictionary
	return {}


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
			if not fx.has_method("configure_elements"):
				failures.append("ConstructionFx has no configure_elements().")
			else:
				var sample_elements: Array = [
					{
						"kind": "wall",
						"start": Vector2(-4.0, 0.0),
						"end": Vector2(4.0, 0.0),
						"thickness": 1.5,
						"base_y": 0.0,
						"start_y": 0.0,
						"end_y": 0.0,
						"height": 8.0,
					},
					{
						"kind": "tower",
						"center": Vector2(5.0, 0.0),
						"radius": 2.0,
						"base_y": 0.0,
						"height": 9.0,
					},
				]
				fx.call(
					"configure_elements",
					sample_elements,
					AABB(Vector3(-6.0, 0.0, -4.0), Vector3(16.0, 9.0, 8.0))
				)
				fx.call("set_build_front", 0.5, true)
				if not bool(fx.call("is_scaffold_visible")):
					failures.append("ConstructionFx shows no scaffolding for element footprints.")
				if not bool(fx.call("is_dust_emitting")):
					failures.append("ConstructionFx emits no dust for element footprints.")
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
			for method in ["set_muted", "is_muted"]:
				if not audio.has_method(method):
					failures.append("BuildAudio has no %s()." % method)
			audio.call("set_muted", true)
			if not bool(audio.call("is_muted")):
				failures.append("BuildAudio.set_muted(true) did not stick.")
			audio.call("set_muted", false)
			if bool(audio.call("is_muted")):
				failures.append("BuildAudio.set_muted(false) did not clear the mute.")
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
			for signal_name in [
				"wall_committed",
				"tower_committed",
				"tower_shaped_committed",
				"gatehouse_committed",
				"ditch_committed",
				"snap_feedback",
			]:
				if not tool.has_signal(signal_name):
					failures.append("DesignTool has no %s signal." % signal_name)
			tool.call("set_ground_height", 4.0)
			for mode in [1, 2, 3, 4, 5, 6]:
				tool.call("set_mode", mode)
				if int(tool.call("get_mode")) != mode:
					failures.append("DesignTool.set_mode(%d) did not stick." % mode)
			tool.call("set_mode", 0)
			(tool as Node).free()


## M1.4 Wave 2b: LOD0 anti-moiré setup, speed/time-lapse HUD controls, the
## scriptable orbit and the clean render_view capture helpers.
func _check_visual_polish(failures: Array[String]) -> void:
	# 16x anisotropic filtering keeps grazing masonry crisp at distance.
	var af := int(
		ProjectSettings.get_setting(
			"rendering/textures/default_filters/anisotropic_filtering_level", 0
		)
	)
	if af != 4:
		failures.append(
			"anisotropic_filtering_level is %d, expected 4 (16x AF)." % af
		)

	# LOD0 must band-limit its procedural detail with screen-space derivatives
	# and carry the weathering terms.
	var shader_file := FileAccess.open("res://shaders/masonry.gdshader", FileAccess.READ)
	if shader_file == null:
		failures.append("masonry.gdshader could not be opened for inspection.")
	else:
		var source := shader_file.get_as_text()
		shader_file.close()
		for token in [
			"dFdx(uv.x)",
			"grain_visible",
			"weathered_stone(",
			"fresh_lift",
			"wet_base_y",
		]:
			if source.find(token) == -1:
				failures.append("masonry.gdshader is missing \"%s\"." % token)

	# HUD speed / time-lapse / mute contract.
	var hud_script: Variant = load("res://scripts/debug_hud.gd")
	if not (hud_script is GDScript):
		failures.append("debug_hud.gd did not load as a GDScript.")
	else:
		var hud: Variant = (hud_script as GDScript).new()
		if hud == null or not (hud is CanvasLayer):
			failures.append("DebugHud is not a CanvasLayer.")
		else:
			root.add_child(hud as CanvasLayer)
			for signal_name in ["speed_requested", "timelapse_requested", "mute_toggled"]:
				if not hud.has_signal(signal_name):
					failures.append("DebugHud has no %s signal." % signal_name)
			for method in [
				"set_speed",
				"get_speed_multiplier",
				"set_timelapse",
				"is_timelapse_active",
				"set_muted",
				"is_muted",
			]:
				if not hud.has_method(method):
					failures.append("DebugHud has no %s()." % method)
			hud.call("set_speed", 20.0)
			if not is_equal_approx(float(hud.call("get_speed_multiplier")), 20.0):
				failures.append("DebugHud.set_speed(20) did not stick.")
			hud.call("set_timelapse", true)
			if not bool(hud.call("is_timelapse_active")):
				failures.append("DebugHud.set_timelapse(true) did not stick.")
			hud.call("set_timelapse", false)
			if bool(hud.call("is_timelapse_active")):
				failures.append("DebugHud.set_timelapse(false) did not clear.")
			hud.call("set_muted", true)
			if not bool(hud.call("is_muted")):
				failures.append("DebugHud.set_muted(true) did not stick.")
			(hud as Node).free()

	# Orbit camera scripted time-lapse orbit and explicit capture framing.
	var camera_script: Variant = load("res://scripts/orbit_camera.gd")
	if not (camera_script is GDScript):
		failures.append("orbit_camera.gd did not load as a GDScript.")
	else:
		var cam: Variant = (camera_script as GDScript).new()
		if cam == null or not (cam is Camera3D):
			failures.append("OrbitCamera is not a Camera3D.")
		else:
			root.add_child(cam as Camera3D)
			for method in [
				"start_auto_orbit",
				"stop_auto_orbit",
				"is_auto_orbiting",
				"set_orbit_view",
			]:
				if not cam.has_method(method):
					failures.append("OrbitCamera has no %s()." % method)
			if not cam.has_signal("auto_orbit_cancelled"):
				failures.append("OrbitCamera has no auto_orbit_cancelled signal.")
			cam.call("start_auto_orbit", 0.28)
			if not bool(cam.call("is_auto_orbiting")):
				failures.append("OrbitCamera.start_auto_orbit() did not start.")
			cam.call("stop_auto_orbit")
			if bool(cam.call("is_auto_orbiting")):
				failures.append("OrbitCamera.stop_auto_orbit() did not stop.")
			cam.call("set_orbit_view", 90.0, 20.0, 50.0)
			if not is_equal_approx(float(cam.call("get_view_distance")), 50.0):
				failures.append("OrbitCamera.set_orbit_view() did not apply the distance.")
			(cam as Node).free()

	# main.gd helpers used by the time-lapse replay and capture crop.
	var main_script: Variant = load("res://scripts/main.gd")
	if not (main_script is GDScript):
		failures.append("main.gd did not load as a GDScript.")
	else:
		var main_node: Variant = (main_script as GDScript).new()
		if main_node == null or not (main_node is Node3D):
			failures.append("main.gd did not instantiate as a Node3D.")
		else:
			for method in [
				"_on_hud_speed_requested",
				"_on_hud_timelapse_requested",
				"_start_timelapse",
				"_stop_timelapse",
				"_fulfill_render_view",
				"_crop_to_aspect",
			]:
				if not main_node.has_method(method):
					failures.append("main.gd has no %s()." % method)
			(main_node as Node).free()

	if failures.is_empty():
		print("M1.4 visual polish OK: 16x AF, LOD0 band-limit, speed/time-lapse, mute, clean capture")


## Contract for the construction-FX element footprints emitted by keep-gd.
func _check_elements(data: Dictionary, failures: Array[String]) -> void:
	if not data.has("elements"):
		failures.append("build_m0b_showcase() is missing \"elements\".")
		return
	var elements: Variant = data.get("elements", null)
	if not (elements is Array):
		failures.append("build_m0b_showcase().elements is not an Array.")
		return
	var element_array: Array = elements
	if element_array.is_empty():
		failures.append("build_m0b_showcase().elements is empty.")
		return

	var index := 0
	for entry in element_array:
		if not (entry is Dictionary):
			failures.append("elements[%d] is not a Dictionary." % index)
			index += 1
			continue
		var element: Dictionary = entry
		var kind := str(element.get("kind", ""))
		if kind == "wall":
			for key in ["start", "end", "thickness", "base_y", "start_y", "end_y", "height"]:
				if not element.has(key):
					failures.append("elements[%d] wall is missing \"%s\"." % [index, key])
			if element.has("start") and not (element.get("start") is Vector2):
				failures.append("elements[%d].start is not a Vector2." % index)
			if element.has("end") and not (element.get("end") is Vector2):
				failures.append("elements[%d].end is not a Vector2." % index)
		elif kind == "tower":
			for key in ["center", "radius", "base_y", "height"]:
				if not element.has(key):
					failures.append("elements[%d] tower is missing \"%s\"." % [index, key])
			if element.has("center") and not (element.get("center") is Vector2):
				failures.append("elements[%d].center is not a Vector2." % index)
		else:
			failures.append("elements[%d] has unexpected kind \"%s\"." % [index, kind])
		index += 1

	if failures.is_empty():
		print("Construction elements OK: %d footprints" % element_array.size())


## Spawn the M0b chunks through main.gd's real `_spawn_chunks()` and confirm all
## three LODs share one `custom_aabb` and zero visibility fade margins.
func _check_lod_chunks(data: Dictionary, failures: Array[String]) -> void:
	var chunks: Variant = data.get("chunks", null)
	if not (chunks is Array) or (chunks as Array).is_empty():
		failures.append("Cannot check LOD culling: build has no chunks.")
		return

	var main_script: Variant = load("res://scripts/main.gd")
	if not (main_script is GDScript):
		failures.append("main.gd did not load as a GDScript.")
		return
	var host: Variant = (main_script as GDScript).new()
	if host == null or not (host is Node3D):
		failures.append("main.gd did not instantiate as a Node3D.")
		return

	var host_node: Node3D = host
	# Not added to the SceneTree, so _ready() does not run; _spawn_chunks only
	# builds child MeshInstance3D nodes, which is exactly what we want to test.
	host_node.call("_spawn_chunks", chunks)

	var groups := {}
	for child in host_node.get_children():
		if not (child is MeshInstance3D):
			continue
		var parts := str(child.name).split("_LOD")
		if parts.size() != 2:
			continue
		if not groups.has(parts[0]):
			groups[parts[0]] = []
		groups[parts[0]].append(child)

	if groups.is_empty():
		failures.append("main.gd._spawn_chunks() created no LOD instances.")
		host_node.free()
		return

	for chunk_key in groups:
		var lods: Array = groups[chunk_key]
		if lods.size() != 3:
			failures.append(
				"Chunk %s spawned %d LOD instances, expected 3." % [chunk_key, lods.size()]
			)
			continue
		var reference: MeshInstance3D = lods[0]
		for entry in lods:
			var lod: MeshInstance3D = entry
			if (
				not lod.custom_aabb.position.is_equal_approx(reference.custom_aabb.position)
				or not lod.custom_aabb.size.is_equal_approx(reference.custom_aabb.size)
			):
				failures.append(
					"Chunk %s LODs do not share an identical custom_aabb." % chunk_key
				)
				break
			if (
				not is_zero_approx(lod.visibility_range_begin_margin)
				or not is_zero_approx(lod.visibility_range_end_margin)
			):
				failures.append(
					"Chunk %s %s has non-zero visibility fade margins." % [chunk_key, lod.name]
				)
				break
			# The shader keys LOD0's per-block UV path against the LOD1/LOD2
			# world-space coursed-ashlar path on this instance uniform.
			var expected_lod := int(str(lod.name).split("_LOD")[1])
			if int(lod.get_instance_shader_parameter("lod_level")) != expected_lod:
				failures.append(
					"Chunk %s %s has wrong lod_level." % [chunk_key, lod.name]
				)
				break

	if failures.is_empty():
		print("LOD culling OK: %d chunks share one AABB, zero fade margins and per-LOD lod_level" % groups.size())
	host_node.free()


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
