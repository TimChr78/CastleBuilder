extends CanvasLayer
## In-game agent panel (architecture section 7.7, M1 basics).
##
## Built entirely in code and anchored top-right so it never overlaps the
## top-left DebugHud. It is a thin view over the session/proposal system:
## presence, permissions, the control-server address, and Accept/Reject cards
## for submitted proposals. It contains no LLM and never talks to the network.

signal permission_changed(agent_id: String, permission: String)
signal proposal_accept_requested(proposal_id: String)
signal proposal_reject_requested(proposal_id: String, reason: String)
signal ghost_toggled(proposal_id: String, visible: bool)
signal demo_proposal_requested

const PANEL_WIDTH := 372.0
const PERMISSIONS := ["Proposal-Only", "Direct-Edit", "Read-Only"]
const PERMISSION_KEYS := ["proposal_only", "direct_edit", "read_only"]
const PANEL_BG := Color(0.055, 0.065, 0.08, 0.92)
const CARD_BG := Color(0.09, 0.105, 0.13, 0.94)
const BORDER := Color(0.32, 0.46, 0.58, 0.55)
const TEXT := Color(0.86, 0.90, 0.94)
const MUTED := Color(0.60, 0.67, 0.74)
const ACCENT := Color(0.35, 0.82, 0.95)
const GOOD := Color(0.46, 0.88, 0.58)
const BAD := Color(0.95, 0.46, 0.42)

var _panel: PanelContainer
var _header: Button
var _body: VBoxContainer
var _server_label: Label
var _copy_button: Button
var _demo_button: Button
var _agents_box: VBoxContainer
var _proposals_box: VBoxContainer
var _proposals_empty: Label

var _server_url := ""
var _token := ""
var _running := false
var _ghost_visibility: Dictionary = {}


## Build immediately so the panel is usable even when a headless test creates
## it without ever entering the scene tree (no `_ready()` callback).
func _init() -> void:
	layer = 20
	_build()


func _ready() -> void:
	if _panel == null:
		_build()


func _build() -> void:
	if _panel != null:
		return
	var root := Control.new()
	root.name = "AgentPanelRoot"
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	_panel = PanelContainer.new()
	_panel.name = "AgentPanelFrame"
	_panel.set_anchors_preset(Control.PRESET_TOP_RIGHT)
	_panel.offset_left = -(PANEL_WIDTH + 16.0)
	_panel.offset_right = -16.0
	_panel.offset_top = 16.0
	_panel.offset_bottom = 16.0
	_panel.grow_vertical = Control.GROW_DIRECTION_END
	_panel.custom_minimum_size = Vector2(PANEL_WIDTH, 0.0)
	_panel.add_theme_stylebox_override("panel", _panel_style(PANEL_BG, BORDER))
	root.add_child(_panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 10)
	margin.add_theme_constant_override("margin_right", 10)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 10)
	_panel.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 7)
	margin.add_child(column)

	_header = Button.new()
	_header.name = "AgentPanelHeader"
	_header.text = "AGENTS & MCP   ▾"
	_header.alignment = HORIZONTAL_ALIGNMENT_LEFT
	_header.add_theme_font_size_override("font_size", 14)
	_header.add_theme_color_override("font_color", ACCENT)
	_header.add_theme_stylebox_override("normal", _panel_style(Color(0, 0, 0, 0), Color(0, 0, 0, 0)))
	_header.add_theme_stylebox_override("hover", _panel_style(Color(0.15, 0.22, 0.28, 0.55), BORDER))
	_header.add_theme_stylebox_override("pressed", _panel_style(Color(0.12, 0.18, 0.23, 0.65), BORDER))
	_header.pressed.connect(_toggle_collapsed)
	column.add_child(_header)

	_body = VBoxContainer.new()
	_body.add_theme_constant_override("separation", 6)
	column.add_child(_body)

	_server_label = Label.new()
	_server_label.text = "Control server: not running"
	_server_label.add_theme_font_size_override("font_size", 12)
	_server_label.add_theme_color_override("font_color", MUTED)
	_body.add_child(_server_label)

	var buttons := HBoxContainer.new()
	buttons.add_theme_constant_override("separation", 6)
	_body.add_child(buttons)

	_copy_button = _small_button("Copy Token", ACCENT)
	_copy_button.tooltip_text = "Copy the per-session bearer token to the clipboard"
	_copy_button.pressed.connect(_on_copy_token)
	buttons.add_child(_copy_button)

	_demo_button = _small_button("Demo Proposal", GOOD)
	_demo_button.tooltip_text = "Ask the built-in Mason agent for a flanking tower proposal"
	_demo_button.pressed.connect(func() -> void: demo_proposal_requested.emit())
	buttons.add_child(_demo_button)

	_body.add_child(HSeparator.new())
	_body.add_child(_section_label("CONNECTED AGENTS"))
	_agents_box = VBoxContainer.new()
	_agents_box.add_theme_constant_override("separation", 4)
	_body.add_child(_agents_box)

	_body.add_child(HSeparator.new())
	_body.add_child(_section_label("PENDING PROPOSALS"))
	_proposals_empty = Label.new()
	_proposals_empty.text = "No proposals waiting."
	_proposals_empty.add_theme_font_size_override("font_size", 11)
	_proposals_empty.add_theme_color_override("font_color", MUTED)
	_body.add_child(_proposals_empty)
	_proposals_box = VBoxContainer.new()
	_proposals_box.add_theme_constant_override("separation", 6)
	_body.add_child(_proposals_box)


func _section_label(text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.add_theme_font_size_override("font_size", 10)
	label.add_theme_color_override("font_color", MUTED)
	return label


func _small_button(text: String, color: Color) -> Button:
	var button := Button.new()
	button.text = text
	button.add_theme_font_size_override("font_size", 11)
	button.add_theme_color_override("font_color", color)
	button.add_theme_stylebox_override("normal", _panel_style(CARD_BG, BORDER))
	button.add_theme_stylebox_override("hover", _panel_style(Color(0.16, 0.22, 0.27, 0.95), ACCENT))
	button.add_theme_stylebox_override("pressed", _panel_style(Color(0.22, 0.30, 0.36, 0.95), ACCENT))
	return button


func _panel_style(fill: Color, border: Color) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = fill
	style.border_color = border
	style.set_border_width_all(1)
	style.set_corner_radius_all(6)
	style.content_margin_left = 8
	style.content_margin_right = 8
	style.content_margin_top = 5
	style.content_margin_bottom = 5
	return style


func _toggle_collapsed() -> void:
	_body.visible = not _body.visible
	_header.text = "AGENTS & MCP   %s" % ("▾" if _body.visible else "▸")


## Public API used by main.gd.
func set_server(url: String, running: bool, token: String) -> void:
	_server_url = url
	_running = running
	_token = token
	_copy_button.disabled = token.is_empty()
	if running and not url.is_empty():
		_server_label.text = "Control server: %s" % url
		_server_label.add_theme_color_override("font_color", GOOD)
	else:
		_server_label.text = "Control server: stopped"
		_server_label.add_theme_color_override("font_color", BAD)


func get_ghost_visibility() -> Dictionary:
	return _ghost_visibility.duplicate()


func refresh(state: Dictionary) -> void:
	_clear_children(_agents_box)
	_clear_children(_proposals_box)

	var agents: Array = state.get("agents", [])
	if agents.is_empty():
		_agents_box.add_child(_muted_label("No agents connected yet."))
	else:
		for entry in agents:
			if entry is Dictionary:
				_agents_box.add_child(_agent_row(entry as Dictionary))

	var pending := 0
	var proposals: Array = state.get("proposals", [])
	for entry in proposals:
		if not (entry is Dictionary):
			continue
		var proposal: Dictionary = entry
		if str(proposal.get("status", "")) != "submitted":
			continue
		pending += 1
		_proposals_box.add_child(_proposal_card(proposal))
	_proposals_box.visible = pending > 0
	_proposals_empty.visible = pending == 0

	# Drop ghost-visibility state for proposals that are no longer pending.
	var live_ids := {}
	for entry in proposals:
		if entry is Dictionary and str((entry as Dictionary).get("status", "")) == "submitted":
			live_ids[str((entry as Dictionary).get("id", ""))] = true
	for proposal_id in _ghost_visibility.keys():
		if not live_ids.has(proposal_id):
			_ghost_visibility.erase(proposal_id)


func _agent_row(agent: Dictionary) -> Control:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)

	var dot := ColorRect.new()
	dot.custom_minimum_size = Vector2(10.0, 10.0)
	dot.color = _parse_color(str(agent.get("color", "#888888")))
	dot.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	row.add_child(dot)

	var name_label := Label.new()
	name_label.text = str(agent.get("name", agent.get("id", "agent")))
	name_label.add_theme_font_size_override("font_size", 12)
	name_label.add_theme_color_override("font_color", TEXT)
	name_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_child(name_label)

	var state_label := Label.new()
	state_label.text = _state_text(str(agent.get("state", "idle")))
	state_label.add_theme_font_size_override("font_size", 11)
	state_label.add_theme_color_override("font_color", MUTED)
	row.add_child(state_label)

	var agent_id := str(agent.get("id", ""))
	var picker := OptionButton.new()
	picker.add_theme_font_size_override("font_size", 10)
	for item in PERMISSIONS:
		picker.add_item(item)
	picker.select(_permission_index(str(agent.get("permission", "proposal_only"))))
	picker.item_selected.connect(
		func(index: int) -> void:
			permission_changed.emit(agent_id, PERMISSION_KEYS[clampi(index, 0, 2)])
	)
	row.add_child(picker)
	return row


func _proposal_card(proposal: Dictionary) -> Control:
	var card := PanelContainer.new()
	card.add_theme_stylebox_override("panel", _panel_style(CARD_BG, BORDER))
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 4)
	card.add_child(column)

	var title := Label.new()
	title.text = str(proposal.get("title", "Proposal"))
	title.add_theme_font_size_override("font_size", 13)
	title.add_theme_color_override("font_color", ACCENT)
	title.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	column.add_child(title)

	var author := Label.new()
	author.text = "by %s" % str(proposal.get("author", "agent"))
	author.add_theme_font_size_override("font_size", 10)
	author.add_theme_color_override("font_color", MUTED)
	column.add_child(author)

	var diff := Label.new()
	diff.text = str(proposal.get("semantic_diff", ""))
	diff.add_theme_font_size_override("font_size", 11)
	diff.add_theme_color_override("font_color", TEXT)
	diff.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	column.add_child(diff)

	var before := int(proposal.get("structure_before", 0))
	var after := int(proposal.get("structure_after", 0))
	var delta := int(proposal.get("structure_delta", after - before))
	var structure := Label.new()
	structure.text = "Structure  %d → %d  (%+d)" % [before, after, delta]
	structure.add_theme_font_size_override("font_size", 11)
	structure.add_theme_color_override("font_color", GOOD if delta >= 0 else BAD)
	column.add_child(structure)

	var summary := Label.new()
	summary.text = str(proposal.get("summary", ""))
	summary.add_theme_font_size_override("font_size", 10)
	summary.add_theme_color_override("font_color", MUTED)
	summary.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	column.add_child(summary)

	var proposal_id := str(proposal.get("id", ""))
	var controls := HBoxContainer.new()
	controls.add_theme_constant_override("separation", 6)
	column.add_child(controls)

	var ghost := CheckButton.new()
	ghost.text = "Ghost"
	ghost.add_theme_font_size_override("font_size", 11)
	ghost.button_pressed = bool(_ghost_visibility.get(proposal_id, true))
	ghost.toggled.connect(
		func(pressed: bool) -> void:
			_ghost_visibility[proposal_id] = pressed
			ghost_toggled.emit(proposal_id, pressed)
	)
	controls.add_child(ghost)

	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	controls.add_child(spacer)

	var accept := _small_button("Accept", GOOD)
	accept.pressed.connect(func() -> void: proposal_accept_requested.emit(proposal_id))
	controls.add_child(accept)

	var reject := _small_button("Reject", BAD)
	reject.pressed.connect(
		func() -> void:
			proposal_reject_requested.emit(proposal_id, "Rejected in the agent panel")
	)
	controls.add_child(reject)
	return card


func _muted_label(text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.add_theme_font_size_override("font_size", 11)
	label.add_theme_color_override("font_color", MUTED)
	return label


func _clear_children(node: Node) -> void:
	for child in node.get_children():
		node.remove_child(child)
		child.queue_free()


func _on_copy_token() -> void:
	if _token.is_empty():
		return
	DisplayServer.clipboard_set(_token)
	_copy_button.text = "Copied!"


func _permission_index(permission: String) -> int:
	var index := PERMISSION_KEYS.find(permission)
	return index if index >= 0 else 0


func _state_text(state: String) -> String:
	match state:
		"thinking":
			return "Thinking"
		"building_proposal":
			return "Building"
		"waiting":
			return "Waiting"
		"paused":
			return "Paused"
		_:
			return "Idle"


func _parse_color(text: String) -> Color:
	var value := Color.from_string(text, Color(0.53, 0.53, 0.53))
	return value
