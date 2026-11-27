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

	if bridge != null:
		_check_m3_1(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	if bridge != null:
		_check_m3_2(bridge, failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	_check_m4(failures)

	if not failures.is_empty():
		_for_each_failure(failures)
		quit(1)
		return

	_check_m5(failures)

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


func _check_m4(failures: Array[String]) -> void:
	# --- Early-Z shader split -----------------------------------------
	var masonry_source := _read_text("res://shaders/masonry.gdshader", failures)
	if not masonry_source.is_empty():
		if masonry_source.find("cull_back") == -1:
			failures.append("masonry.gdshader does not use cull_back for the hot path.")
		if masonry_source.find("cull_disabled") != -1:
			failures.append("masonry.gdshader still declares cull_disabled.")
		if masonry_source.find("discard") != -1:
			failures.append("masonry.gdshader still discards fragments in the hot path.")

	var cutaway_source := _read_text("res://shaders/masonry_cutaway.gdshader", failures)
	if not cutaway_source.is_empty():
		if cutaway_source.find("cull_disabled") == -1:
			failures.append("masonry_cutaway.gdshader does not use cull_disabled.")
		if cutaway_source.find("cutaway_mode") == -1:
			failures.append("masonry_cutaway.gdshader has no cutaway_mode uniform.")
		if cutaway_source.find("discard") == -1:
			failures.append("masonry_cutaway.gdshader never clips the active plane.")
		for token in ["winter_frost", "build_front", "material_palette"]:
			if cutaway_source.find(token) == -1:
				failures.append(
					"masonry_cutaway.gdshader is missing shared token \"%s\"." % token
				)

	var hud_script: Variant = load("res://scripts/debug_hud.gd")
	if hud_script is GDScript:
		var hud: Variant = (hud_script as GDScript).new()
		if hud != null:
			if not hud.has_signal("settings_requested"):
				failures.append("DebugHud has no settings_requested signal.")
			root.add_child(hud as CanvasLayer)
			(hud as Node).free()

	# --- Accessibility settings panel ---------------------------------
	var settings_script: Variant = load("res://scripts/settings_panel.gd")
	if not (settings_script is GDScript):
		failures.append("settings_panel.gd did not load as a GDScript.")
	else:
		var panel: Variant = (settings_script as GDScript).new()
		if panel == null or not (panel is CanvasLayer):
			failures.append("SettingsPanel is not a CanvasLayer.")
		else:
			root.add_child(panel as CanvasLayer)
			for signal_name in [
				"ui_scale_changed",
				"ghost_palette_changed",
				"reduced_motion_changed",
				"mute_changed",
			]:
				if not panel.has_signal(signal_name):
					failures.append("SettingsPanel has no %s signal." % signal_name)
			for method in [
				"set_ui_scale",
				"get_ui_scale",
				"set_high_contrast",
				"is_high_contrast",
				"set_reduced_motion",
				"is_reduced_motion",
				"set_muted",
				"is_muted",
				"save_settings",
				"load_settings",
				"get_credits_text",
				"open",
				"close",
				"is_open",
			]:
				if not panel.has_method(method):
					failures.append("SettingsPanel has no %s()." % method)

			panel.call("set_ui_scale", 1.3)
			if not is_equal_approx(float(panel.call("get_ui_scale")), 1.3):
				failures.append("SettingsPanel.set_ui_scale(1.3) did not stick.")
			panel.call("set_high_contrast", true)
			if not bool(panel.call("is_high_contrast")):
				failures.append("SettingsPanel.set_high_contrast(true) did not stick.")
			panel.call("set_reduced_motion", true)
			if not bool(panel.call("is_reduced_motion")):
				failures.append("SettingsPanel.set_reduced_motion(true) did not stick.")
			panel.call("set_muted", true)
			if not bool(panel.call("is_muted")):
				failures.append("SettingsPanel.set_muted(true) did not stick.")

			var credits := str(panel.call("get_credits_text"))
			for token in ["AI", "CC0", "PROCEDURAL", "shader"]:
				if credits.find(token) == -1:
					failures.append(
						"SettingsPanel credits text is missing \"%s\"." % token
					)

			var test_settings_path := "user://settings_test.json"
			if not bool(panel.call("save_settings", test_settings_path)):
				failures.append("SettingsPanel.save_settings() failed.")
			# Wipe the in-memory state, then prove the file round-trips.
			panel.call("set_ui_scale", 1.0)
			panel.call("set_high_contrast", false)
			panel.call("set_reduced_motion", false)
			panel.call("set_muted", false)
			if not bool(panel.call("load_settings", test_settings_path)):
				failures.append("SettingsPanel.load_settings() failed.")
			DirAccess.remove_absolute(ProjectSettings.globalize_path(test_settings_path))
			if not is_equal_approx(float(panel.call("get_ui_scale")), 1.3):
				failures.append("SettingsPanel did not restore ui_scale from disk.")
			if not bool(panel.call("is_high_contrast")):
				failures.append("SettingsPanel did not restore high_contrast from disk.")
			if not bool(panel.call("is_reduced_motion")):
				failures.append("SettingsPanel did not restore reduced_motion from disk.")
			if not bool(panel.call("is_muted")):
				failures.append("SettingsPanel did not restore mute state from disk.")

			panel.call("open")
			if not bool(panel.call("is_open")):
				failures.append("SettingsPanel.open() did not open the modal.")
			panel.call("close")
			if bool(panel.call("is_open")):
				failures.append("SettingsPanel.close() did not close the modal.")
			(panel as Node).free()

	# --- main.gd wiring ------------------------------------------------
	var main_script: Variant = load("res://scripts/main.gd")
	if main_script is GDScript:
		var host: Variant = (main_script as GDScript).new()
		if host == null or not (host is Node3D):
			failures.append("main.gd did not instantiate as a Node3D for M4b checks.")
		else:
			for method in [
				"_setup_settings_panel",
				"_toggle_settings",
				"_close_settings",
				"_apply_ui_scale",
				"_apply_ghost_palette",
				"_apply_reduced_motion",
				"_active_masonry_material",
			]:
				if not host.has_method(method):
					failures.append("main.gd has no %s()." % method)
			(host as Node).free()

	if failures.is_empty():
		print("M4b accessibility OK: early-Z cutaway split, settings modal, persistence")


## M5 Release 2: two independent bridges prove snapshot totality and hash
## agreement, then the LAN transport and modal prove a clean idle state.
func _check_m5(failures: Array[String]) -> void:
	var bridge_methods := [
		"coplay_state",
		"coplay_set_max_players",
		"coplay_register_player",
		"coplay_remove_player",
		"coplay_set_player_role",
		"coplay_update_presence",
		"coplay_claim_element",
		"coplay_release_claim",
		"coplay_snapshot_json",
		"coplay_apply_snapshot_json",
		"coplay_apply_remote_batch_json",
		"coplay_apply_remote_undo",
		"coplay_verify_hashes",
	]
	if not ClassDB.class_exists("KeepBridge"):
		failures.append("KeepBridge is not registered for M5 checks.")
		return

	var host: Object = ClassDB.instantiate("KeepBridge")
	var guest: Object = ClassDB.instantiate("KeepBridge")
	if host == null or guest == null:
		failures.append("Could not instantiate two independent KeepBridge sessions.")
		return
	root.add_child(host as Node)
	root.add_child(guest as Node)

	for method in bridge_methods:
		if not host.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		(host as Node).free()
		(guest as Node).free()
		return

	# --- Host builds the M1 showcase and registers both players ---------
	var built: Variant = host.call("build_m1_edwardian")
	if not (built is Dictionary) or str((built as Dictionary).get("document_hash", "")).is_empty():
		failures.append("M5 host build_m1_edwardian() returned no document_hash.")
		(host as Node).free()
		(guest as Node).free()
		return
	var cap: Variant = host.call("coplay_set_max_players", 2)
	if int(cap) != 2:
		failures.append("coplay_set_max_players(2) returned %s, expected 2." % str(cap))

	var host_reg: Variant = host.call(
		"coplay_register_player", "alex", "Alex", "#e8b04b", "host"
	)
	_require_ok(host_reg, "host registration", failures)
	var guest_reg: Variant = host.call(
		"coplay_register_player", "sam", "Sam", "#4b9be8", "builder"
	)
	_require_ok(guest_reg, "guest registration", failures)
	var claim: Variant = host.call("coplay_claim_element", 1, "sam", "north wall")
	_require_ok(claim, "soft claim", failures)

	var host_state_result: Variant = host.call("coplay_state")
	if not (host_state_result is Dictionary):
		failures.append("coplay_state() did not return a Dictionary.")
		(host as Node).free()
		(guest as Node).free()
		return
	var host_state: Dictionary = host_state_result
	if int(host_state.get("max_players", 0)) != 2:
		failures.append("coplay_state() max_players is not 2.")
	if (host_state.get("players", []) as Array).size() != 2:
		failures.append("coplay_state() did not list both players.")
	if (host_state.get("claims", []) as Array).size() != 1:
		failures.append("coplay_state() did not list the soft claim.")
	if int(host_state.get("generator_version", 0)) != 1:
		failures.append("coplay_state() generator_version is not 1.")
	var host_doc_hash := str(host_state.get("document_hash", ""))
	if host_doc_hash.is_empty():
		failures.append("coplay_state() returned an empty document_hash.")

	# --- Guest rebuilds from the snapshot and agrees on both hashes -----
	var snapshot_json := str(host.call("coplay_snapshot_json"))
	if snapshot_json.is_empty():
		failures.append("coplay_snapshot_json() returned an empty string.")
		(host as Node).free()
		(guest as Node).free()
		return
	var snapshot: Variant = JSON.parse_string(snapshot_json)
	if not (snapshot is Dictionary):
		failures.append("coplay_snapshot_json() did not emit valid JSON.")
		(host as Node).free()
		(guest as Node).free()
		return
	var host_layout_hash := str((snapshot as Dictionary).get("layout_hash", ""))
	if host_layout_hash.is_empty():
		failures.append("Snapshot has an empty layout_hash.")

	var applied: Variant = guest.call("coplay_apply_snapshot_json", snapshot_json)
	_require_ok(applied, "snapshot apply", failures)
	var guest_state: Variant = guest.call("coplay_state")
	if guest_state is Dictionary:
		if str((guest_state as Dictionary).get("document_hash", "")) != host_doc_hash:
			failures.append("Guest document_hash differs from the host after applying the snapshot.")
		if (guest_state as Dictionary).get("players", []).size() != 2:
			failures.append("Guest did not restore both players from the snapshot.")
		if (guest_state as Dictionary).get("claims", []).size() != 1:
			failures.append("Guest did not restore the soft claim from the snapshot.")

	var verify: Variant = guest.call("coplay_verify_hashes", host_doc_hash, host_layout_hash)
	if not (verify is Dictionary) or str((verify as Dictionary).get("kind", "")) != "match":
		failures.append(
			"Guest coplay_verify_hashes() did not report 'match' after the snapshot."
		)

	# --- The same ordered wall batch inside both sessions ----------------
	var ops := [
		{
			"op": "create_wall_run",
			"name": "Remote East",
			"start_mm": [60000, 60000],
			"end_mm": [66000, 60000],
			"height_mm": 6000,
			"thickness_mm": 1800,
		}
	]
	var ops_json := JSON.stringify(ops)
	var host_batch: Variant = host.call("coplay_apply_remote_batch_json", "sam", ops_json)
	_require_ok(host_batch, "host remote batch", failures)
	var guest_batch: Variant = guest.call("coplay_apply_remote_batch_json", "sam", ops_json)
	_require_ok(guest_batch, "guest remote batch", failures)
	if host_batch is Dictionary and guest_batch is Dictionary:
		var host_batch_doc := str((host_batch as Dictionary).get("document_hash", ""))
		var guest_batch_doc := str((guest_batch as Dictionary).get("document_hash", ""))
		var host_batch_layout := str((host_batch as Dictionary).get("layout_hash", ""))
		var guest_batch_layout := str((guest_batch as Dictionary).get("layout_hash", ""))
		if host_batch_doc != guest_batch_doc:
			failures.append("Host and guest document hashes diverged after the shared batch.")
		if host_batch_layout != guest_batch_layout:
			failures.append("Host and guest layout hashes diverged after the shared batch.")
		var verify_after: Variant = guest.call(
			"coplay_verify_hashes", host_batch_doc, host_batch_layout
		)
		if not (verify_after is Dictionary) or str((verify_after as Dictionary).get("kind", "")) != "match":
			failures.append("Hashes no longer match after the shared remote batch.")

	# --- A viewer cannot send commands -----------------------------------
	guest.call("coplay_set_max_players", 3)
	var viewer_reg: Variant = guest.call(
		"coplay_register_player", "val", "Val", "#8be84b", "viewer"
	)
	_require_ok(viewer_reg, "viewer registration", failures)
	var refused: Variant = guest.call("coplay_apply_remote_batch_json", "val", ops_json)
	if not (refused is Dictionary) or bool((refused as Dictionary).get("ok", true)):
		failures.append("coplay_apply_remote_batch_json() accepted a viewer-authored batch.")

	# --- Transport and modal instantiate and stay idle -------------------
	var net_script: Variant = load("res://scripts/coplay_net.gd")
	if not (net_script is GDScript):
		failures.append("coplay_net.gd did not load as a GDScript.")
	else:
		var net: Variant = (net_script as GDScript).new()
		if net == null or not (net is Node3D):
			failures.append("CoplayNet is not a Node3D.")
		else:
			root.add_child(net as Node3D)
			for method in [
				"host_session",
				"join_session",
				"disconnect_session",
				"is_hosting",
				"is_guest",
				"is_connected_session",
				"get_status",
				"send_snapshot",
				"broadcast_snapshot_to_all",
				"broadcast_batch",
				"broadcast_undo",
				"broadcast_clock",
				"broadcast_hash_check",
				"send_presence",
				"update_remote_markers",
			]:
				if not net.has_method(method):
					failures.append("CoplayNet has no %s() method." % method)
			for signal_name in [
				"session_state_changed",
				"snapshot_received",
				"remote_batch_received",
				"remote_undo_received",
				"clock_sync_received",
				"hash_check_received",
				"guest_join_requested",
				"status_message",
			]:
				if not net.has_signal(signal_name):
					failures.append("CoplayNet has no %s signal." % signal_name)
			var net_status: Variant = net.call("get_status")
			if not (net_status is Dictionary):
				failures.append("CoplayNet.get_status() did not return a Dictionary.")
			else:
				if bool((net_status as Dictionary).get("connected", true)):
					failures.append("CoplayNet reports a live session before host/join.")
				if bool((net_status as Dictionary).get("hosting", true)):
					failures.append("CoplayNet reports hosting before host_session().")
				if not ((net_status as Dictionary).get("players", null) is Array):
					failures.append("CoplayNet.get_status() has no players array.")
			if bool(net.call("is_hosting")):
				failures.append("CoplayNet.is_hosting() is true in idle single player.")
			if bool(net.call("is_connected_session")):
				failures.append("CoplayNet.is_connected_session() is true in idle single player.")
			net.call("disconnect_session")
			(net as Node).free()

	var panel_script: Variant = load("res://scripts/coplay_panel.gd")
	if not (panel_script is GDScript):
		failures.append("coplay_panel.gd did not load as a GDScript.")
	else:
		var panel: Variant = (panel_script as GDScript).new()
		if panel == null or not (panel is CanvasLayer):
			failures.append("CoplayPanel is not a CanvasLayer.")
		else:
			root.add_child(panel as CanvasLayer)
			if int(panel.layer) != 37:
				failures.append("CoplayPanel layer is not 37.")
			if bool(panel.visible):
				failures.append("CoplayPanel is visible by default.")
			for signal_name in [
				"host_requested",
				"join_requested",
				"disconnect_requested",
				"close_requested",
			]:
				if not panel.has_signal(signal_name):
					failures.append("CoplayPanel has no %s signal." % signal_name)
			for method in [
				"open",
				"close",
				"is_open",
				"toggle",
				"set_session_state",
				"set_players",
				"get_player_name",
				"get_port",
				"get_address",
				"get_max_players",
			]:
				if not panel.has_method(method):
					failures.append("CoplayPanel has no %s() method." % method)
			if bool(panel.call("is_open")):
				failures.append("CoplayPanel.is_open() is true by default.")
			if int(panel.call("get_max_players")) != 2:
				failures.append("CoplayPanel does not default to 2 players.")
			if int(panel.call("get_port")) != 24565:
				failures.append("CoplayPanel does not default to port 24565.")
			panel.call(
				"set_session_state",
				{
					"connected": false,
					"hosting": false,
					"port": 24565,
					"players": [],
					"status": "Not connected",
				}
			)
			(panel as Node).free()

	(host as Node).free()
	(guest as Node).free()

	if failures.is_empty():
		print(
			"M5 co-play OK: snapshot hash match, shared batch determinism, viewer gate, idle transport"
		)


func _read_text(path: String, failures: Array[String]) -> String:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		failures.append("%s could not be opened for inspection." % path)
		return ""
	var text := file.get_as_text()
	file.close()
	return text


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


## M3.1: authoritative terrain sculpting, dollhouse interior props, the
## historical codex and the three new UI scripts (interior props, photo mode,
## codex panel).
func _check_m3_1(bridge: Object, failures: Array[String]) -> void:
	for method in ["sculpt_terrain", "list_codex_entries"]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	var base_result: Variant = bridge.call("build_m2_baltic")
	if not (base_result is Dictionary):
		failures.append("build_m2_baltic(M3.1) did not return a Dictionary.")
		return
	var base_data: Dictionary = base_result

	# 1. Dollhouse furniture covers every furnished room type in M2.
	var props: Variant = base_data.get("interior_props", null)
	if not (props is Array):
		failures.append("scene dictionary is missing \"interior_props\" as an Array.")
	elif (props as Array).is_empty():
		failures.append("build_m2_baltic() returned no interior_props.")
	else:
		var rooms := {}
		var valid_kinds := [
			"table", "bench", "bed", "altar", "cauldron", "barrel", "chest"
		]
		var index := 0
		for entry in (props as Array):
			if not (entry is Dictionary):
				failures.append("interior_props[%d] is not a Dictionary." % index)
				break
			var prop: Dictionary = entry
			for key in ["kind", "room_type", "position", "yaw_deg", "element_id"]:
				if not prop.has(key):
					failures.append(
						"interior_props[%d] is missing \"%s\"." % [index, key]
					)
			var position: Variant = prop.get("position", null)
			if position is Vector3:
				rooms[str(prop.get("room_type", ""))] = true
			else:
				failures.append("interior_props[%d].position is not a Vector3." % index)
			if not valid_kinds.has(str(prop.get("kind", ""))):
				failures.append(
					"interior_props[%d] has unknown kind \"%s\"."
					% [index, str(prop.get("kind", ""))]
				)
			index += 1
			if index >= 400:
				break
		for room in ["great_hall", "chapel", "kitchen", "undercroft", "solar"]:
			if not rooms.has(room):
				failures.append("interior_props is missing room type \"%s\"." % room)

	# 2. Sculpting is authoritative: the rendered terrain rises, hashes into the
	# document and Ctrl+Z restores both the surface and the hash. Picking reads
	# the raw site terrain (sculpts are derived earthworks), so the mesh itself
	# is the honest probe here.
	var before_hash := str(base_data.get("document_hash", ""))
	var probe := Vector2(0.0, 24.0)
	var before_height := _terrain_mesh_height(base_data, probe)
	if is_nan(before_height):
		failures.append("build_m2_baltic() terrain mesh has no vertices near the sculpt probe.")
	else:
		var sculpt: Variant = bridge.call("sculpt_terrain", "raise", probe, 8.0, 1.5)
		if not (sculpt is Dictionary) or not bool((sculpt as Dictionary).get("ok", false)):
			var sculpt_error := "did not return a Dictionary"
			if sculpt is Dictionary:
				sculpt_error = str((sculpt as Dictionary).get("error", "unknown error"))
			failures.append("sculpt_terrain(raise) failed: %s" % sculpt_error)
		else:
			var sculpt_data: Dictionary = sculpt
			if str(sculpt_data.get("document_hash", "")) == before_hash:
				failures.append("sculpt_terrain() did not change the document hash.")
			var raised := _terrain_mesh_height(sculpt_data, probe)
			if is_nan(raised) or raised <= before_height + 0.2:
				failures.append(
					"sculpt_terrain(raise) did not raise the terrain (%.2f -> %.2f)."
					% [before_height, raised]
				)
			var undo: Variant = bridge.call("undo_last")
			if not (undo is Dictionary) or not bool((undo as Dictionary).get("undone", false)):
				failures.append("undo_last() did not reverse the terrain sculpt.")
			else:
				var undo_data: Dictionary = undo
				if str(undo_data.get("document_hash", "")) != before_hash:
					failures.append(
						"undo_last() did not restore the pre-sculpt document hash."
					)
				var restored := _terrain_mesh_height(undo_data, probe)
				if not is_nan(restored) and absf(restored - before_height) > 0.2:
					failures.append(
						"undo_last() did not restore the terrain height (%.2f vs %.2f)."
						% [restored, before_height]
					)

	# 3. Codex data contract: eight entries with titles and citations.
	var entries: Variant = bridge.call("list_codex_entries")
	if not (entries is Array):
		failures.append("list_codex_entries() did not return an Array.")
	elif (entries as Array).size() != 8:
		failures.append(
			"list_codex_entries() returned %d entries, expected 8."
			% (entries as Array).size()
		)
	else:
		for entry in (entries as Array):
			if not (entry is Dictionary):
				failures.append("A codex entry is not a Dictionary.")
				break
			var item: Dictionary = entry
			if str(item.get("title", "")).strip_edges().is_empty():
				failures.append(
					"Codex entry \"%s\" has an empty title." % str(item.get("id", ""))
				)
			var citations: Variant = item.get("citations", null)
			if not (citations is Array) or (citations as Array).is_empty():
				failures.append(
					"Codex entry \"%s\" has no citations." % str(item.get("id", ""))
				)
			if not (item.get("related_style_ids", null) is Array):
				failures.append(
					"Codex entry \"%s\" has no related_style_ids array."
					% str(item.get("id", ""))
				)

	# 4. The new UI scripts build and run headlessly.
	_check_interior_props_script(failures)
	_check_photo_mode_script(failures)
	_check_codex_panel_script(entries, failures)
	_check_m3_1_main_methods(failures)

	if failures.is_empty():
		print(
			"M3.1 OK: interior props (5 rooms), terrain sculpt+undo, 8 codex entries, "
			+ "photo/codex/cutaway UI"
		)


## Highest terrain-mesh vertex (metres) within `radius` of an XZ point.
func _terrain_mesh_height(data: Dictionary, point_xz: Vector2, radius: float = 2.0) -> float:
	var terrain: Variant = data.get("terrain", null)
	if not (terrain is ArrayMesh):
		return NAN
	var mesh: ArrayMesh = terrain
	if mesh.get_surface_count() == 0:
		return NAN
	var arrays: Array = mesh.surface_get_arrays(0)
	var vertices: Variant = arrays[Mesh.ARRAY_VERTEX]
	if not (vertices is PackedVector3Array):
		return NAN
	var best := -INF
	for vertex in (vertices as PackedVector3Array):
		if Vector2(vertex.x, vertex.z).distance_to(point_xz) <= radius:
			best = maxf(best, vertex.y)
	if best == -INF:
		return NAN
	return best


func _check_interior_props_script(failures: Array[String]) -> void:
	var script: Variant = load("res://scripts/interior_props.gd")
	if not (script is GDScript):
		failures.append("interior_props.gd did not load as a GDScript.")
		return
	var builder: Variant = (script as GDScript).new()
	if builder == null or not (builder is Node3D):
		failures.append("InteriorProps is not a Node3D.")
		return
	root.add_child(builder as Node3D)
	for method in ["build_props", "set_cutaway", "get_prop_count", "get_room_types"]:
		if not builder.has_method(method):
			failures.append("InteriorProps has no %s()." % method)
	var sample := [
		{
			"kind": "table",
			"room_type": "great_hall",
			"position": Vector3(0.0, 1.0, 0.0),
			"yaw_deg": 0,
			"element_id": 30,
		},
		{
			"kind": "bed",
			"room_type": "solar",
			"position": Vector3(0.0, 6.0, 0.0),
			"yaw_deg": 90,
			"element_id": 20,
		},
	]
	builder.call("build_props", sample)
	if int(builder.call("get_prop_count")) != 2:
		failures.append("InteriorProps.get_prop_count() != 2 after build_props().")
	builder.call("set_cutaway", 0, 100.0)
	if bool(builder.call("is_props_visible")):
		failures.append("InteriorProps is visible with the cutaway off.")
	builder.call("set_cutaway", 1, 3.0)
	if not bool(builder.call("is_props_visible")):
		failures.append("InteriorProps did not become visible with a cutaway.")
	if int(builder.call("get_visible_prop_count")) != 1:
		failures.append("InteriorProps did not hide the prop above the cut plane.")
	(builder as Node).free()


func _check_photo_mode_script(failures: Array[String]) -> void:
	var script: Variant = load("res://scripts/photo_mode.gd")
	if not (script is GDScript):
		failures.append("photo_mode.gd did not load as a GDScript.")
		return
	var photo: Variant = (script as GDScript).new()
	if photo == null or not (photo is CanvasLayer):
		failures.append("PhotoMode is not a CanvasLayer.")
		return
	root.add_child(photo as CanvasLayer)
	for signal_name in [
		"sun_time_changed", "fov_changed", "capture_requested", "exit_requested"
	]:
		if not photo.has_signal(signal_name):
			failures.append("PhotoMode has no %s signal." % signal_name)
	for method in ["set_sun_time", "get_sun_time", "set_fov", "get_fov", "capture_photo"]:
		if not photo.has_method(method):
			failures.append("PhotoMode has no %s()." % method)
	photo.call("set_sun_time", 7.5)
	if not is_equal_approx(float(photo.call("get_sun_time")), 7.5):
		failures.append("PhotoMode.set_sun_time(7.5) did not stick.")
	photo.call("set_fov", 62.0)
	if not is_equal_approx(float(photo.call("get_fov")), 62.0):
		failures.append("PhotoMode.set_fov(62) did not stick.")
	if not (photo.call("capture_photo") is String):
		failures.append("PhotoMode.capture_photo() did not return a String.")
	(photo as Node).free()


func _check_codex_panel_script(entries: Variant, failures: Array[String]) -> void:
	var script: Variant = load("res://scripts/codex_panel.gd")
	if not (script is GDScript):
		failures.append("codex_panel.gd did not load as a GDScript.")
		return
	var panel: Variant = (script as GDScript).new()
	if panel == null or not (panel is CanvasLayer):
		failures.append("CodexPanel is not a CanvasLayer.")
		return
	root.add_child(panel as CanvasLayer)
	if not panel.has_signal("close_requested"):
		failures.append("CodexPanel has no close_requested signal.")
	for method in ["set_entries", "select_entry", "get_selected_id", "open", "close"]:
		if not panel.has_method(method):
			failures.append("CodexPanel has no %s()." % method)
	if entries is Array:
		panel.call("set_entries", entries)
		if int(panel.call("get_entry_count")) != (entries as Array).size():
			failures.append("CodexPanel.get_entry_count() does not match the entries.")
		if not (entries as Array).is_empty():
			var first: Variant = (entries as Array)[0]
			if first is Dictionary:
				var first_id := str((first as Dictionary).get("id", ""))
				panel.call("select_entry", first_id)
				if str(panel.call("get_selected_id")) != first_id:
					failures.append("CodexPanel.select_entry() did not stick.")
	panel.call("open")
	if not bool(panel.call("is_open")):
		failures.append("CodexPanel.open() did not open the panel.")
	panel.call("close")
	if bool(panel.call("is_open")):
		failures.append("CodexPanel.close() did not close the panel.")
	(panel as Node).free()


func _check_m3_1_main_methods(failures: Array[String]) -> void:
	var script: Variant = load("res://scripts/main.gd")
	if not (script is GDScript):
		failures.append("main.gd did not load as a GDScript.")
		return
	var host: Variant = (script as GDScript).new()
	if host == null or not (host is Node3D):
		failures.append("main.gd did not instantiate as a Node3D.")
		return
	for method in [
		"_apply_sun_time",
		"_set_cutaway_mode",
		"_set_cutaway_position",
		"_sculpt_at_screen_position",
		"_set_photo_mode",
		"_toggle_codex",
	]:
		if not host.has_method(method):
			failures.append("main.gd has no %s()." % method)
	(host as Node).free()


## M3.2: curated commissions, the season calendar with winter frost, mid-build
## salvage accounting and the commission panel UI.
func _check_m3_2(bridge: Object, failures: Array[String]) -> void:
	for method in [
		"list_commissions",
		"start_commission",
		"end_commission",
		"advance_season",
		"get_commission_state",
		"set_build_front_norm",
	]:
		if not bridge.has_method(method):
			failures.append("KeepBridge has no %s() method." % method)
	if not failures.is_empty():
		return

	# 1. All four curated briefs carry the picker's display contract.
	var commissions: Variant = bridge.call("list_commissions")
	if not (commissions is Array) or (commissions as Array).size() != 4:
		failures.append("list_commissions() did not return four commissions.")
		return
	for entry in (commissions as Array):
		if not (entry is Dictionary):
			failures.append("A commission brief is not a Dictionary.")
			continue
		var brief: Dictionary = entry
		for key in [
			"id",
			"title",
			"patron",
			"description",
			"site_id",
			"style_id",
			"starting_silver",
			"season_limit",
			"starting_workforce",
			"min_structure_score",
			"min_historian_score",
		]:
			if not brief.has(key):
				failures.append(
					"Commission brief \"%s\" is missing \"%s\"."
					% [str(brief.get("id", "")), key]
				)

	# 2. Starting Harlech switches to the headland and Edwardian Welsh style.
	var started := _require_ok(
		bridge.call("start_commission", "harlech_watch"),
		"start_commission(harlech_watch)",
		failures
	)
	if started.is_empty():
		return
	if str(started.get("site_id", "")) != "coastal_headland":
		failures.append(
			"start_commission(harlech_watch) reported site_id %s."
			% str(started.get("site_id", ""))
		)
	if str(started.get("style_id", "")) != "edwardian_welsh":
		failures.append(
			"start_commission(harlech_watch) reported style_id %s."
			% str(started.get("style_id", ""))
		)

	var commission: Variant = started.get("commission", null)
	if not (commission is Dictionary):
		failures.append("start_commission() returned no commission Dictionary.")
		return
	var state: Dictionary = commission
	if str(state.get("id", "")) != "harlech_watch":
		failures.append("start_commission() returned the wrong commission id.")
	if str(state.get("season", "")) != "spring":
		failures.append(
			"A new commission did not start in spring (got %s)."
			% str(state.get("season", ""))
		)
	var goals: Variant = state.get("goals", null)
	if not (goals is Array) or (goals as Array).is_empty():
		failures.append("start_commission() returned no goals.")

	var status: Variant = bridge.call("get_commission_state")
	if not (status is Dictionary) or not bool((status as Dictionary).get("active", false)):
		failures.append("get_commission_state() did not report an active commission.")

	# 3. Spring -> summer -> autumn -> winter, with frost in winter.
	var season_state: Dictionary = state
	for expected in ["summer", "autumn", "winter"]:
		var advanced := _require_ok(
			bridge.call("advance_season"), "advance_season()", failures
		)
		if advanced.is_empty():
			return
		var next_commission: Variant = advanced.get("commission", null)
		if not (next_commission is Dictionary):
			failures.append("advance_season() returned no commission Dictionary.")
			return
		season_state = next_commission
		if str(season_state.get("season", "")) != expected:
			failures.append(
				"advance_season() reached %s, expected %s."
				% [str(season_state.get("season", "")), expected]
			)
	if not bool(season_state.get("frost_active", false)):
		failures.append("Winter did not report frost_active.")

	# 4. Mid-build salvage: raising a wall at a half-built front reports a
	# salvage diff, but undoing it must never award extra silver (B1).
	bridge.call("set_build_front_norm", 0.5)
	var wall_result := _require_ok(
		bridge.call("add_wall", Vector2(-10.0, 0.0), Vector2(10.0, 0.0), 8.0, 2.0),
		"add_wall(mid-build)",
		failures
	)
	if wall_result.is_empty():
		return
	var edit_salvage: Variant = wall_result.get("salvage", null)
	if not (edit_salvage is Dictionary):
		failures.append("A mid-build edit returned no salvage Dictionary.")
	var before_undo: Variant = bridge.call("get_commission_state")
	var treasury_before_undo := 0
	if before_undo is Dictionary:
		var before_commission: Variant = (before_undo as Dictionary).get("commission", {})
		if before_commission is Dictionary:
			treasury_before_undo = int((before_commission as Dictionary).get("treasury_silver", 0))
	var undo_result: Variant = bridge.call("undo_last")
	if not (undo_result is Dictionary):
		failures.append("undo_last() did not return a Dictionary during salvage.")
		return
	var undo_data: Dictionary = undo_result
	if not bool(undo_data.get("undone", false)):
		failures.append("undo_last() refused the mid-build salvage edit.")
	if undo_data.has("salvage"):
		failures.append("undo_last() awarded mid-build salvage; undo must not mint silver (B1).")
	var after_undo: Variant = bridge.call("get_commission_state")
	if after_undo is Dictionary:
		var after_commission: Variant = (after_undo as Dictionary).get("commission", {})
		if after_commission is Dictionary:
			var treasury_after_undo := int(
				(after_commission as Dictionary).get("treasury_silver", 0)
			)
			if treasury_after_undo > treasury_before_undo:
				failures.append(
					"undo_last() increased the treasury (%d -> %d)."
					% [treasury_before_undo, treasury_after_undo]
				)
	if edit_salvage is Dictionary:
		var salvage_data: Dictionary = edit_salvage
		for key in [
			"unchanged_built_blocks",
			"dismantled_blocks",
			"added_blocks",
			"salvage_credit_silver",
			"preserved_fraction",
		]:
			if not salvage_data.has(key):
				failures.append("salvage is missing \"%s\"." % key)
		if float(salvage_data.get("preserved_fraction", 1.0)) > 1.0:
			failures.append("Mid-build salvage preserved_fraction exceeds one.")

	# 5. Ending the commission returns cleanly to the sandbox.
	_require_ok(bridge.call("end_commission"), "end_commission()", failures)
	var sandbox: Variant = bridge.call("get_commission_state")
	if not (sandbox is Dictionary) or bool((sandbox as Dictionary).get("active", true)):
		failures.append("end_commission() did not return to the sandbox.")

	# 6. The commission panel instantiates, exposes its API and refreshes.
	var script: Variant = load("res://scripts/commission_panel.gd")
	if not (script is GDScript):
		failures.append("commission_panel.gd did not load as a GDScript.")
	else:
		var panel: Variant = (script as GDScript).new()
		if panel == null or not (panel is CanvasLayer):
			failures.append("CommissionPanel is not a CanvasLayer.")
		else:
			root.add_child(panel as CanvasLayer)
			for signal_name in [
				"commission_selected",
				"advance_season_requested",
				"exit_commission_requested",
			]:
				if not panel.has_signal(signal_name):
					failures.append("CommissionPanel has no %s signal." % signal_name)
			for method in [
				"set_commissions",
				"open_picker",
				"close_picker",
				"refresh",
				"is_picker_open",
			]:
				if not panel.has_method(method):
					failures.append("CommissionPanel has no %s()." % method)
			panel.call("set_commissions", commissions)
			if int(panel.call("get_commission_count")) != 4:
				failures.append("CommissionPanel did not keep the four briefs.")
			panel.call("refresh", {"active": true, "commission": season_state})
			if not bool(panel.call("is_commission_active")):
				failures.append("CommissionPanel did not activate on refresh().")
			panel.call("refresh", {"active": false})
			if bool(panel.call("is_commission_active")):
				failures.append("CommissionPanel did not clear on sandbox refresh().")
			panel.call("open_picker")
			if not bool(panel.call("is_picker_open")):
				failures.append("CommissionPanel.open_picker() did not open the picker.")
			panel.call("close_picker")
			if bool(panel.call("is_picker_open")):
				failures.append("CommissionPanel.close_picker() did not close the picker.")
			(panel as Node).free()

	if failures.is_empty():
		print(
			"M3.2 commissions OK: 4 briefs, seasons spring→winter, frost, "
			+ "mid-build salvage, panel"
		)


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
