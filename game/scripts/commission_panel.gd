extends CanvasLayer
## M3.2 commissions UI.
##
## Built entirely in code (AGENTS.md rule 5). Two views share one CanvasLayer:
##
##   * a centred picker modal listing every curated commission brief, shown
##     from the DebugHud "Commissions" button while in the sandbox;
##   * a compact top-centre card for the live commission: patron, season/year,
##     treasury, workforce, patron patience, elapsed seasons, the season's
##     event, the goal checklist and the completion/failure banner.
##
## The panel is a pure view. It emits requests and `main.gd` forwards them to
## the authoritative KeepBridge session.

signal commission_selected(commission_id: String)
signal advance_season_requested()
signal exit_commission_requested()

const CARD_WIDTH := 460.0
const CARD_TOP := 14.0
const PICKER_SIZE := Vector2(780.0, 560.0)
const PANEL_BG := Color(0.055, 0.065, 0.08, 0.93)
const CARD_BG := Color(0.08, 0.09, 0.11, 0.90)
const BORDER := Color(0.55, 0.46, 0.32, 0.55)
const TEXT := Color(0.90, 0.88, 0.81)
const MUTED := Color(0.66, 0.64, 0.57)
const PARCHMENT := Color(0.95, 0.84, 0.62)
const GOOD := Color(0.62, 0.88, 0.54)
const BAD := Color(0.98, 0.48, 0.40)
const FROST := Color(0.66, 0.86, 1.0)
const STRAW := Color(0.86, 0.74, 0.46)

var _root: Control
var _active_card: PanelContainer
var _picker_root: Control
var _picker_frame: PanelContainer
var _picker_list: VBoxContainer
var _picker_empty: Label

var _title_label: Label
var _patron_label: Label
var _season_label: Label
var _ledger_label: Label
var _elapsed_label: Label
var _event_label: Label
var _goals_box: VBoxContainer
var _status_label: Label
var _next_button: Button
var _exit_button: Button

var _commissions: Array = []
var _active := false
var _active_id := ""


func _init() -> void:
	layer = 15
	_build()
	visible = false


func _ready() -> void:
	if _root == null:
		_build()


# ---------------------------------------------------------------------------
# Construction
# ---------------------------------------------------------------------------

func _build() -> void:
	if _root != null:
		return
	_root = Control.new()
	_root.name = "CommissionRoot"
	_root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(_root)

	_build_active_card()
	_build_picker()


func _build_active_card() -> void:
	_active_card = PanelContainer.new()
	_active_card.name = "CommissionCard"
	_active_card.set_anchors_preset(Control.PRESET_CENTER_TOP)
	_active_card.offset_left = -CARD_WIDTH * 0.5
	_active_card.offset_right = CARD_WIDTH * 0.5
	_active_card.offset_top = CARD_TOP
	_active_card.offset_bottom = CARD_TOP
	_active_card.custom_minimum_size = Vector2(CARD_WIDTH, 0.0)
	_active_card.mouse_filter = Control.MOUSE_FILTER_STOP
	_active_card.add_theme_stylebox_override("panel", _panel_style(CARD_BG, BORDER))
	_active_card.visible = false
	_root.add_child(_active_card)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 14)
	margin.add_theme_constant_override("margin_right", 14)
	margin.add_theme_constant_override("margin_top", 10)
	margin.add_theme_constant_override("margin_bottom", 10)
	_active_card.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 4)
	margin.add_child(column)

	_title_label = _make_label(16, PARCHMENT)
	column.add_child(_title_label)

	_patron_label = _make_label(12, TEXT)
	column.add_child(_patron_label)

	_season_label = _make_label(12, FROST)
	column.add_child(_season_label)

	column.add_child(HSeparator.new())

	_ledger_label = _make_label(12, TEXT)
	column.add_child(_ledger_label)

	_elapsed_label = _make_label(12, TEXT)
	column.add_child(_elapsed_label)

	_event_label = _make_label(11, STRAW)
	_event_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_event_label.custom_minimum_size = Vector2(CARD_WIDTH - 30.0, 0.0)
	_event_label.visible = false
	column.add_child(_event_label)

	column.add_child(HSeparator.new())

	_goals_box = VBoxContainer.new()
	_goals_box.name = "CommissionGoals"
	_goals_box.add_theme_constant_override("separation", 1)
	column.add_child(_goals_box)

	_status_label = _make_label(13, GOOD)
	_status_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_status_label.visible = false
	column.add_child(_status_label)

	var button_row := HBoxContainer.new()
	button_row.add_theme_constant_override("separation", 8)
	column.add_child(button_row)

	_next_button = Button.new()
	_next_button.text = "Next Season"
	_next_button.focus_mode = Control.FOCUS_NONE
	_next_button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_next_button.pressed.connect(func() -> void: advance_season_requested.emit())
	button_row.add_child(_next_button)

	_exit_button = Button.new()
	_exit_button.text = "Exit to Sandbox"
	_exit_button.focus_mode = Control.FOCUS_NONE
	_exit_button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_exit_button.pressed.connect(func() -> void: exit_commission_requested.emit())
	button_row.add_child(_exit_button)


func _build_picker() -> void:
	_picker_root = Control.new()
	_picker_root.name = "CommissionPicker"
	_picker_root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_picker_root.mouse_filter = Control.MOUSE_FILTER_STOP
	_picker_root.visible = false
	_root.add_child(_picker_root)

	var backdrop := ColorRect.new()
	backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	backdrop.color = Color(0.02, 0.03, 0.04, 0.62)
	backdrop.mouse_filter = Control.MOUSE_FILTER_STOP
	_picker_root.add_child(backdrop)

	_picker_frame = PanelContainer.new()
	_picker_frame.set_anchors_preset(Control.PRESET_CENTER)
	_picker_frame.offset_left = -PICKER_SIZE.x * 0.5
	_picker_frame.offset_right = PICKER_SIZE.x * 0.5
	_picker_frame.offset_top = -PICKER_SIZE.y * 0.5
	_picker_frame.offset_bottom = PICKER_SIZE.y * 0.5
	_picker_frame.custom_minimum_size = PICKER_SIZE
	_picker_frame.add_theme_stylebox_override("panel", _panel_style(PANEL_BG, BORDER))
	_picker_root.add_child(_picker_frame)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 16)
	margin.add_theme_constant_override("margin_right", 16)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 12)
	_picker_frame.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 8)
	margin.add_child(column)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	column.add_child(header)

	var title := _make_label(17, PARCHMENT)
	title.text = "COMMISSIONS"
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)

	var hint := _make_label(11, MUTED)
	hint.text = "Accept a patron's brief to leave the sandbox"
	header.add_child(hint)

	var close_button := Button.new()
	close_button.text = "×"
	close_button.focus_mode = Control.FOCUS_NONE
	close_button.custom_minimum_size = Vector2(30.0, 24.0)
	close_button.pressed.connect(close_picker)
	header.add_child(close_button)

	column.add_child(HSeparator.new())

	var scroll := ScrollContainer.new()
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	column.add_child(scroll)

	_picker_list = VBoxContainer.new()
	_picker_list.name = "CommissionList"
	_picker_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_picker_list.add_theme_constant_override("separation", 8)
	scroll.add_child(_picker_list)

	_picker_empty = _make_label(12, MUTED)
	_picker_empty.text = "No commissions available."
	_picker_empty.visible = false
	column.add_child(_picker_empty)


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

## Feed the curated briefs returned by KeepBridge.list_commissions().
func set_commissions(entries: Array) -> void:
	_commissions = entries.duplicate()
	if _picker_list != null and _picker_root != null and _picker_root.visible:
		_rebuild_picker()


## Show the picker modal (sandbox only; an active commission keeps its card).
func open_picker() -> void:
	if _root == null:
		_build()
	if _active:
		visible = true
		_active_card.visible = true
		return
	_active_card.visible = false
	_rebuild_picker()
	_picker_root.visible = true
	# The picker is a modal: sit above the AgentPanel (layer 20) and
	# PhotoMode (layer 30) so it can never be covered.
	layer = 38
	visible = true


## Hide the picker modal without changing commission state.
func close_picker() -> void:
	if _picker_root != null:
		_picker_root.visible = false
	# Back to the compact top-centre card layer.
	layer = 15
	if not _active and _active_card != null:
		_active_card.visible = false
		visible = false


func is_picker_open() -> bool:
	return _picker_root != null and _picker_root.visible


func is_commission_active() -> bool:
	return _active


func get_commission_count() -> int:
	return _commissions.size()


func get_active_commission_id() -> String:
	return _active_id


## Render `{ active, commission? }` from KeepBridge.get_commission_state().
func refresh(state: Dictionary) -> void:
	if _root == null:
		_build()
	var commission: Dictionary = {}
	var nested: Variant = state.get("commission", null)
	if nested is Dictionary:
		commission = nested
	elif state.has("id"):
		commission = state

	var active := bool(state.get("active", not commission.is_empty()))
	if commission.is_empty():
		active = false
	_active = active

	if not active:
		_active_id = ""
		_active_card.visible = false
		if not is_picker_open():
			visible = false
		return

	_active_card.visible = true
	_picker_root.visible = false
	visible = true
	_populate_active(commission)


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------

func _populate_active(commission: Dictionary) -> void:
	_active_id = str(commission.get("id", ""))
	var title := str(commission.get("title", "Commission"))
	var patron := str(commission.get("patron", ""))
	_title_label.text = title
	_title_label.add_theme_color_override("font_color", PARCHMENT)
	_patron_label.text = "Patron — %s" % patron

	var season_name := str(commission.get("season", "spring")).capitalize()
	var year := int(commission.get("year", 1))
	var frost := bool(commission.get("frost_active", false))
	var badge := "%s · Year %d" % [season_name, year]
	if frost:
		badge += " — FROST"
		if bool(commission.get("walls_thatched", false)):
			badge += " · WALLS THATCHED"
	_season_label.text = badge
	_season_label.add_theme_color_override("font_color", FROST if frost else STRAW)

	var treasury := int(commission.get("treasury_silver", 0))
	var workforce := int(commission.get("workforce", 0))
	var patience := int(commission.get("patron_patience", 0))
	var elapsed := int(commission.get("seasons_elapsed", 0))
	var limit := int(commission.get("season_limit", 0))
	_ledger_label.text = "Treasury %d silver    Workforce %d masons    Patience %d%%" % [
		treasury, workforce, patience
	]
	_elapsed_label.text = "Elapsed %d / %d seasons    Spent %d silver    Salvaged %d silver" % [
		elapsed, limit, int(commission.get("spent_silver", 0)), int(commission.get("salvage_silver", 0))
	]

	var event: Variant = commission.get("active_event", {})
	var event_text := ""
	if event is Dictionary and not (event as Dictionary).is_empty():
		var event_title := str((event as Dictionary).get("title", ""))
		var event_description := str((event as Dictionary).get("description", ""))
		if not event_title.is_empty():
			event_text = "%s — %s" % [event_title, event_description]
	_event_label.text = event_text
	_event_label.visible = not event_text.is_empty()

	for child in _goals_box.get_children():
		child.queue_free()
	var goals: Variant = commission.get("goals", [])
	if goals is Array:
		for entry in (goals as Array):
			if not (entry is Dictionary):
				continue
			var goal: Dictionary = entry
			var met := bool(goal.get("met", false))
			var row := _make_label(11, GOOD if met else MUTED)
			row.text = "%s  %s" % ["✓" if met else "○", str(goal.get("label", ""))]
			_goals_box.add_child(row)

	var completed := bool(commission.get("completed", false))
	var failed := bool(commission.get("failed", false))
	_status_label.visible = completed or failed
	if completed:
		_status_label.text = "COMMISSION COMPLETE!"
		_status_label.add_theme_color_override("font_color", GOOD)
	elif failed:
		_status_label.text = "COMMISSION FAILED"
		_status_label.add_theme_color_override("font_color", BAD)
	_next_button.disabled = completed or failed


func _rebuild_picker() -> void:
	if _picker_list == null:
		return
	for child in _picker_list.get_children():
		child.queue_free()
	_picker_empty.visible = _commissions.is_empty()
	for entry in _commissions:
		if entry is Dictionary:
			_picker_list.add_child(_commission_card(entry))


func _commission_card(brief: Dictionary) -> Control:
	var card := PanelContainer.new()
	card.add_theme_stylebox_override("panel", _panel_style(CARD_BG, BORDER))
	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 12)
	margin.add_theme_constant_override("margin_right", 12)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)
	card.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 3)
	margin.add_child(column)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	column.add_child(header)

	var title := _make_label(14, PARCHMENT)
	title.text = str(brief.get("title", brief.get("id", "Commission")))
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)

	var accept := Button.new()
	accept.text = "Accept Commission"
	accept.focus_mode = Control.FOCUS_NONE
	accept.custom_minimum_size = Vector2(150.0, 0.0)
	accept.pressed.connect(_on_accept_pressed.bind(str(brief.get("id", ""))))
	header.add_child(accept)

	var patron := _make_label(11, TEXT)
	patron.text = "Patron — %s" % str(brief.get("patron", ""))
	column.add_child(patron)

	var site := _make_label(11, MUTED)
	site.text = "Site — %s    Style — %s    Budget — %d silver    Season limit — %d" % [
		str(brief.get("site_id", "")),
		str(brief.get("style_id", "")),
		int(brief.get("starting_silver", 0)),
		int(brief.get("season_limit", 0)),
	]
	column.add_child(site)

	var description := _make_label(11, TEXT)
	description.text = str(brief.get("description", ""))
	description.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	description.custom_minimum_size = Vector2(PICKER_SIZE.x - 80.0, 0.0)
	column.add_child(description)

	return card


func _on_accept_pressed(commission_id: String) -> void:
	if commission_id.is_empty():
		return
	_picker_root.visible = false
	commission_selected.emit(commission_id)


func _make_label(font_size: int, color: Color) -> Label:
	var label := Label.new()
	label.add_theme_font_size_override("font_size", font_size)
	label.add_theme_color_override("font_color", color)
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return label


func _panel_style(bg: Color, border: Color) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = bg
	style.border_color = border
	style.set_border_width_all(1)
	style.set_corner_radius_all(6)
	style.content_margin_left = 2.0
	style.content_margin_right = 2.0
	return style
