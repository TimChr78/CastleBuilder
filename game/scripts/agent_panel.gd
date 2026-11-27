extends CanvasLayer
## In-game agent panel (architecture section 7.7, M2.2).
##
## Built entirely in code and anchored top-right so it never overlaps the
## top-left DebugHud. It is a thin view over the session/proposal system:
## presence, pause/resume, permissions, the control-server address, the request
## inbox, Accept/Reject cards with historian deltas, and a live activity feed.
## It contains no LLM and never talks to the network.

signal permission_changed(agent_id: String, permission: String)
signal proposal_accept_requested(proposal_id: String)
signal proposal_reject_requested(proposal_id: String, reason: String)
signal ghost_toggled(proposal_id: String, visible: bool)
signal demo_proposal_requested
signal agent_pause_toggled(agent_id: String, paused: bool)
signal request_submitted(prompt: String)
signal proposal_changes_requested(proposal_id: String, feedback: String)

const PANEL_WIDTH := 372.0
const BODY_MAX_HEIGHT := 520.0
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
var _scroll: ScrollContainer
var _body: VBoxContainer
var _server_label: Label
var _copy_button: Button
var _demo_button: Button
var _agents_box: VBoxContainer
var _requests_box: VBoxContainer
var _requests_empty: Label
var _request_input: LineEdit
var _proposals_box: VBoxContainer
var _proposals_empty: Label
var _activity_box: VBoxContainer
var _activity_empty: Label

var _server_url := ""
var _token := ""
var _running := false
var _ghost_visibility: Dictionary = {}
var _feedback_drafts: Dictionary = {}


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

	# Every dynamic section lives inside this scroll box so a growing agent
	# roster, request inbox and activity feed can never overflow off-screen.
	_scroll = ScrollContainer.new()
	_scroll.name = "AgentPanelScroll"
	_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	_scroll.custom_minimum_size = Vector2(PANEL_WIDTH - 20.0, 0.0)
	_scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	column.add_child(_scroll)

	_body = VBoxContainer.new()
	_body.add_theme_constant_override("separation", 6)
	_body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_scroll.add_child(_body)

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
	_body.add_child(_section_label("REQUEST AGENT"))
	_request_input = LineEdit.new()
	_request_input.name = "AgentRequestInput"
	_request_input.placeholder_text = "Ask an agent to build or refine..."
	_request_input.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_request_input.text_submitted.connect(func(_text: String) -> void: _submit_request())
	_body.add_child(_request_input)
	var request_buttons := HBoxContainer.new()
	request_buttons.add_theme_constant_override("separation", 6)
	_body.add_child(request_buttons)
	var request_spacer := Control.new()
	request_spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	request_buttons.add_child(request_spacer)
	var send_request := _small_button("Send", ACCENT)
	send_request.pressed.connect(_submit_request)
	request_buttons.add_child(send_request)
	_requests_empty = _muted_label("No requests yet.")
	_body.add_child(_requests_empty)
	_requests_box = VBoxContainer.new()
	_requests_box.add_theme_constant_override("separation", 4)
	_body.add_child(_requests_box)

	_body.add_child(HSeparator.new())
	_body.add_child(_section_label("PENDING PROPOSALS"))
	_proposals_empty = _muted_label("No proposals waiting.")
	_body.add_child(_proposals_empty)
	_proposals_box = VBoxContainer.new()
	_proposals_box.add_theme_constant_override("separation", 6)
	_body.add_child(_proposals_box)

	_body.add_child(HSeparator.new())
	_body.add_child(_section_label("ACTIVITY"))
	_activity_empty = _muted_label("No activity yet.")
	_body.add_child(_activity_empty)
	_activity_box = VBoxContainer.new()
	_activity_box.add_theme_constant_override("separation", 2)
	_body.add_child(_activity_box)

	_resize_panel()


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
	_scroll.visible = not _scroll.visible
	_header.text = "AGENTS & MCP   %s" % ("▾" if _scroll.visible else "▸")
	_resize_panel()


## Clamp the scroll region to the live content height, capped so the panel can
## never grow past BODY_MAX_HEIGHT no matter how many entries arrive.
func _resize_panel() -> void:
	if _panel == null or _scroll == null or _body == null:
		return
	if not _scroll.visible:
		_scroll.custom_minimum_size.y = 0.0
		return
	var content_height := _body.get_combined_minimum_size().y
	_scroll.custom_minimum_size.y = clampf(content_height, 0.0, BODY_MAX_HEIGHT)


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
	_resize_panel()


func get_ghost_visibility() -> Dictionary:
	return _ghost_visibility.duplicate()


func refresh(state: Dictionary) -> void:
	_clear_children(_agents_box)
	_clear_children(_requests_box)
	_clear_children(_proposals_box)
	_clear_children(_activity_box)

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

	_refresh_requests(state.get("requests", []))
	_refresh_activity(state.get("activity", []))

	# Drop ghost-visibility state for proposals that are no longer pending.
	var live_ids := {}
	for entry in proposals:
		if entry is Dictionary and str((entry as Dictionary).get("status", "")) == "submitted":
			live_ids[str((entry as Dictionary).get("id", ""))] = true
	for proposal_id in _ghost_visibility.keys():
		if not live_ids.has(proposal_id):
			_ghost_visibility.erase(proposal_id)

	# Draft feedback text is per-proposal UI state; drop it once the proposal
	# leaves the pending set, exactly like ghost visibility.
	for proposal_id in _feedback_drafts.keys():
		if not live_ids.has(proposal_id):
			_feedback_drafts.erase(proposal_id)

	_resize_panel()


func _refresh_requests(requests: Variant) -> void:
	var open_requests: Array = []
	var resolved_requests: Array = []
	if requests is Array:
		for entry in (requests as Array):
			if not (entry is Dictionary):
				continue
			var request: Dictionary = entry
			if str(request.get("status", "")) == "resolved":
				resolved_requests.append(request)
			else:
				open_requests.append(request)
	open_requests.append_array(resolved_requests)

	var shown := mini(open_requests.size(), 4)
	for index in shown:
		_requests_box.add_child(_request_row(open_requests[index]))
	_requests_box.visible = shown > 0
	_requests_empty.visible = shown == 0


func _request_row(request: Dictionary) -> Control:
	var card := PanelContainer.new()
	card.add_theme_stylebox_override("panel", _panel_style(CARD_BG, BORDER))
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 2)
	card.add_child(column)

	var status := str(request.get("status", "open"))
	var header := Label.new()
	header.text = "[%s] %s" % [status, str(request.get("prompt", ""))]
	header.add_theme_font_size_override("font_size", 11)
	header.add_theme_color_override("font_color", _status_color(status))
	header.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	column.add_child(header)

	var reply := str(request.get("reply", ""))
	if not reply.is_empty():
		var reply_label := _muted_label("↳ %s" % reply)
		reply_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		column.add_child(reply_label)
	return card


func _refresh_activity(entries: Variant) -> void:
	var shown := 0
	if entries is Array:
		for entry in (entries as Array):
			if shown >= 8:
				break
			if not (entry is Dictionary):
				continue
			var activity: Dictionary = entry
			var line := _muted_label(
				"[%s] %s: %s"
				% [
					str(activity.get("actor", "?")),
					str(activity.get("action", "?")),
					str(activity.get("detail", "")),
				]
			)
			line.add_theme_font_size_override("font_size", 10)
			line.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
			_activity_box.add_child(line)
			shown += 1
	_activity_box.visible = shown > 0
	_activity_empty.visible = shown == 0


func _submit_request() -> void:
	var prompt := _request_input.text.strip_edges()
	if prompt.is_empty():
		return
	_request_input.text = ""
	request_submitted.emit(prompt)


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

	var paused := str(agent.get("state", "")) == "paused"
	var toggle := _small_button("Resume" if paused else "Pause", GOOD if paused else MUTED)
	toggle.pressed.connect(func() -> void: agent_pause_toggled.emit(agent_id, not paused))
	row.add_child(toggle)
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

	var stale := bool(proposal.get("stale", false))
	if stale:
		var stale_badge := Label.new()
		stale_badge.text = "STALE — castle changed"
		stale_badge.add_theme_font_size_override("font_size", 10)
		stale_badge.add_theme_color_override("font_color", BAD)
		column.add_child(stale_badge)

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

	var historian_before := int(proposal.get("historian_before", 0))
	var historian_after := int(proposal.get("historian_after", 0))
	var historian_delta := int(
		proposal.get("historian_delta", historian_after - historian_before)
	)
	var historian := Label.new()
	historian.text = (
		"Historian  %d → %d  (%+d)"
		% [historian_before, historian_after, historian_delta]
	)
	historian.add_theme_font_size_override("font_size", 11)
	historian.add_theme_color_override("font_color", GOOD if historian_delta >= 0 else BAD)
	column.add_child(historian)

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
	accept.disabled = stale
	accept.pressed.connect(func() -> void: proposal_accept_requested.emit(proposal_id))
	controls.add_child(accept)

	var reject := _small_button("Reject", BAD)
	reject.pressed.connect(
		func() -> void:
			proposal_reject_requested.emit(proposal_id, "Rejected in the agent panel")
	)
	controls.add_child(reject)

	# Inline "ask for changes" row; blank feedback falls back to a polite
	# default so the request is always actionable.
	var changes := HBoxContainer.new()
	changes.add_theme_constant_override("separation", 6)
	column.add_child(changes)
	var feedback := LineEdit.new()
	feedback.placeholder_text = "Ask for changes..."
	feedback.text = str(_feedback_drafts.get(proposal_id, ""))
	feedback.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	feedback.text_changed.connect(
		func(new_text: String) -> void:
			_feedback_drafts[proposal_id] = new_text
	)
	changes.add_child(feedback)
	var send_changes := _small_button("Send", ACCENT)
	send_changes.pressed.connect(
		func() -> void:
			var text := feedback.text.strip_edges()
			if text.is_empty():
				text = "Please revise this proposal"
			feedback.text = ""
			_feedback_drafts.erase(proposal_id)
			proposal_changes_requested.emit(proposal_id, text)
	)
	changes.add_child(send_changes)
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


func _status_color(status: String) -> Color:
	match status:
		"resolved":
			return MUTED
		"acknowledged":
			return GOOD
		_:
			return ACCENT


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
