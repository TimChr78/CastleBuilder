extends CanvasLayer
## M5 Release 2 co-play modal.
##
## A centred, code-built modal (AGENTS.md rule 5) that hosts or joins a LAN
## session and shows who is connected. It never touches authoritative castle
## state: `main.gd` owns the `CoplayNet` transport and the `KeepBridge`
## registration, and this panel only collects input and renders status.
##
## Styling mirrors SettingsPanel and CodexPanel (dark slate, warm parchment
## accent) so every modal reads as one family.

signal host_requested(port: int, player_name: String, max_players: int)
signal join_requested(address: String, port: int, player_name: String)
signal disconnect_requested()
signal close_requested()

const PANEL_SIZE := Vector2(560.0, 420.0)
const DEFAULT_PORT := 24565
const DEFAULT_ADDRESS := "127.0.0.1"
const DEFAULT_NAME := "Mason"
const PANEL_BG := Color(0.06, 0.065, 0.08, 0.96)
const BORDER := Color(0.55, 0.46, 0.32, 0.60)
const TEXT := Color(0.90, 0.88, 0.81)
const MUTED := Color(0.66, 0.64, 0.57)
const PARCHMENT := Color(0.95, 0.84, 0.62)
const ACCENT := Color(0.86, 0.72, 0.46)
const OK_GREEN := Color(0.62, 0.88, 0.54)

var _panel: PanelContainer
var _name_edit: LineEdit
var _port_edit: LineEdit
var _address_edit: LineEdit
var _max_option: OptionButton
var _host_button: Button
var _join_button: Button
var _disconnect_button: Button
var _status_label: Label
var _players_label: Label
var _players_list: VBoxContainer
var _connected := false


func _init() -> void:
	layer = 37
	visible = false
	_build()


func _ready() -> void:
	if _panel == null:
		_build()


func _build() -> void:
	if _panel != null:
		return
	var root := Control.new()
	root.name = "CoplayRoot"
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	var backdrop := ColorRect.new()
	backdrop.name = "CoplayBackdrop"
	backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	backdrop.color = Color(0.02, 0.03, 0.04, 0.55)
	backdrop.mouse_filter = Control.MOUSE_FILTER_STOP
	root.add_child(backdrop)

	_panel = PanelContainer.new()
	_panel.name = "CoplayFrame"
	_panel.set_anchors_preset(Control.PRESET_CENTER)
	_panel.offset_left = -PANEL_SIZE.x * 0.5
	_panel.offset_right = PANEL_SIZE.x * 0.5
	_panel.offset_top = -PANEL_SIZE.y * 0.5
	_panel.offset_bottom = PANEL_SIZE.y * 0.5
	_panel.custom_minimum_size = PANEL_SIZE
	_panel.add_theme_stylebox_override("panel", _panel_style())
	root.add_child(_panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 18)
	margin.add_theme_constant_override("margin_right", 18)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 14)
	_panel.add_child(margin)

	var column := VBoxContainer.new()
	column.name = "CoplayColumn"
	column.add_theme_constant_override("separation", 8)
	margin.add_child(column)

	# --- Header -------------------------------------------------------
	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	column.add_child(header)

	var title := Label.new()
	title.text = "CO-PLAY — BUILD TOGETHER"
	title.add_theme_font_size_override("font_size", 16)
	title.add_theme_color_override("font_color", PARCHMENT)
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)

	var hint := Label.new()
	hint.text = "Esc to close"
	hint.add_theme_font_size_override("font_size", 11)
	hint.add_theme_color_override("font_color", MUTED)
	header.add_child(hint)

	var close_button := Button.new()
	close_button.name = "CoplayClose"
	close_button.text = "×"
	close_button.focus_mode = Control.FOCUS_NONE
	close_button.custom_minimum_size = Vector2(30.0, 24.0)
	close_button.add_theme_font_size_override("font_size", 16)
	close_button.pressed.connect(close)
	header.add_child(close_button)

	column.add_child(HSeparator.new())

	# --- Identity -----------------------------------------------------
	_name_edit = _make_edit_row(column, "Player Name", DEFAULT_NAME)
	_name_edit.name = "PlayerNameEdit"

	# --- Host ---------------------------------------------------------
	var host_row := _make_label_row(column, "Host LAN")
	_port_edit = LineEdit.new()
	_port_edit.name = "HostPortEdit"
	_port_edit.text = str(DEFAULT_PORT)
	_port_edit.custom_minimum_size = Vector2(76.0, 0.0)
	host_row.add_child(_port_edit)

	_max_option = OptionButton.new()
	_max_option.name = "MaxPlayersOption"
	_max_option.add_item("2 Players (Recommended)", 2)
	_max_option.add_item("4 Players", 4)
	_max_option.focus_mode = Control.FOCUS_NONE
	_max_option.custom_minimum_size = Vector2(190.0, 0.0)
	host_row.add_child(_max_option)

	_host_button = Button.new()
	_host_button.name = "HostButton"
	_host_button.text = "Host LAN"
	_host_button.focus_mode = Control.FOCUS_NONE
	_host_button.custom_minimum_size = Vector2(96.0, 0.0)
	_host_button.pressed.connect(_on_host_pressed)
	host_row.add_child(_host_button)

	# --- Join ---------------------------------------------------------
	var join_row := _make_label_row(column, "Join Address")
	_address_edit = LineEdit.new()
	_address_edit.name = "JoinAddressEdit"
	_address_edit.text = DEFAULT_ADDRESS
	_address_edit.custom_minimum_size = Vector2(160.0, 0.0)
	_address_edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	join_row.add_child(_address_edit)

	_join_button = Button.new()
	_join_button.name = "JoinButton"
	_join_button.text = "Join LAN"
	_join_button.focus_mode = Control.FOCUS_NONE
	_join_button.custom_minimum_size = Vector2(96.0, 0.0)
	_join_button.pressed.connect(_on_join_pressed)
	join_row.add_child(_join_button)

	column.add_child(HSeparator.new())

	# --- Session status ----------------------------------------------
	_status_label = Label.new()
	_status_label.name = "CoplayStatus"
	_status_label.text = "Not connected — host a LAN session or join a friend."
	_status_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_status_label.add_theme_font_size_override("font_size", 12)
	_status_label.add_theme_color_override("font_color", MUTED)
	column.add_child(_status_label)

	_players_label = Label.new()
	_players_label.text = "Connected players"
	_players_label.add_theme_font_size_override("font_size", 12)
	_players_label.add_theme_color_override("font_color", ACCENT)
	column.add_child(_players_label)

	_players_list = VBoxContainer.new()
	_players_list.name = "CoplayPlayers"
	_players_list.add_theme_constant_override("separation", 3)
	_players_list.custom_minimum_size = Vector2(0.0, 84.0)
	column.add_child(_players_list)

	_disconnect_button = Button.new()
	_disconnect_button.name = "DisconnectButton"
	_disconnect_button.text = "Disconnect"
	_disconnect_button.focus_mode = Control.FOCUS_NONE
	_disconnect_button.disabled = true
	_disconnect_button.pressed.connect(_on_disconnect_pressed)
	column.add_child(_disconnect_button)

	_sync_widgets()


func _panel_style() -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = PANEL_BG
	style.border_color = BORDER
	style.set_border_width_all(1)
	style.set_corner_radius_all(8)
	style.content_margin_left = 4.0
	style.content_margin_right = 4.0
	style.content_margin_top = 4.0
	style.content_margin_bottom = 4.0
	return style


func _make_label_row(parent: VBoxContainer, label_text: String) -> HBoxContainer:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)
	parent.add_child(row)
	var label := Label.new()
	label.text = label_text
	label.add_theme_font_size_override("font_size", 12)
	label.add_theme_color_override("font_color", TEXT)
	label.custom_minimum_size = Vector2(112.0, 0.0)
	row.add_child(label)
	return row


func _make_edit_row(parent: VBoxContainer, label_text: String, value: String) -> LineEdit:
	var row := _make_label_row(parent, label_text)
	var edit := LineEdit.new()
	edit.text = value
	edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_child(edit)
	return edit


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

func is_open() -> bool:
	return visible


func open() -> void:
	visible = true


func close() -> void:
	visible = false
	close_requested.emit()


func toggle() -> void:
	if visible:
		close()
	else:
		open()


func get_player_name() -> String:
	return _name_edit.text.strip_edges() if _name_edit != null else DEFAULT_NAME


func get_port() -> int:
	var text := _port_edit.text.strip_edges() if _port_edit != null else ""
	var value := int(text)
	return value if value > 0 else DEFAULT_PORT


func get_address() -> String:
	var text := _address_edit.text.strip_edges() if _address_edit != null else ""
	return text if not text.is_empty() else DEFAULT_ADDRESS


func get_max_players() -> int:
	if _max_option == null:
		return 2
	return int(_max_option.get_selected_id())


## Update connection state and the connected-player list. `state` carries the
## keys from `CoplayNet.get_status()` and, when supplied by `main.gd`, a
## `players` array from `KeepBridge.coplay_state()`.
func set_session_state(state: Dictionary) -> void:
	_connected = bool(state.get("connected", false))
	var hosting := bool(state.get("hosting", false))
	var status := str(state.get("status", ""))
	if _connected:
		var mode := "Hosting" if hosting else "Connected"
		var port := int(state.get("port", DEFAULT_PORT))
		if status.is_empty():
			status = "%s on port %d" % [mode, port]
		_status_label.add_theme_color_override("font_color", OK_GREEN)
	elif status.is_empty():
		status = "Not connected — host a LAN session or join a friend."
		_status_label.add_theme_color_override("font_color", MUTED)
	_status_label.text = status
	var players: Variant = state.get("players", [])
	if players is Array:
		set_players(players)
	_sync_widgets()


func set_players(players: Array) -> void:
	if _players_list == null:
		return
	for child in _players_list.get_children():
		child.queue_free()
	if players.is_empty():
		var empty := Label.new()
		empty.text = "  Nobody yet"
		empty.add_theme_font_size_override("font_size", 11)
		empty.add_theme_color_override("font_color", MUTED)
		_players_list.add_child(empty)
		return
	for entry in players:
		if not (entry is Dictionary):
			continue
		_players_list.add_child(_player_row(entry as Dictionary))


func _player_row(player: Dictionary) -> HBoxContainer:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)

	var dot := Label.new()
	dot.text = "●"
	dot.add_theme_font_size_override("font_size", 14)
	dot.add_theme_color_override(
		"font_color", Color.from_string(str(player.get("color_hex", "#888888")), Color.GRAY)
	)
	row.add_child(dot)

	var name_label := Label.new()
	name_label.text = str(player.get("name", player.get("id", "Player")))
	name_label.add_theme_font_size_override("font_size", 12)
	name_label.add_theme_color_override("font_color", TEXT)
	name_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_child(name_label)

	var role_label := Label.new()
	role_label.text = str(player.get("role", "builder"))
	role_label.add_theme_font_size_override("font_size", 11)
	role_label.add_theme_color_override("font_color", MUTED)
	row.add_child(role_label)
	return row


# ---------------------------------------------------------------------------
# Internals
# ---------------------------------------------------------------------------

func _sync_widgets() -> void:
	if _host_button != null:
		_host_button.disabled = _connected
	if _join_button != null:
		_join_button.disabled = _connected
	if _disconnect_button != null:
		_disconnect_button.disabled = not _connected
	if _name_edit != null:
		_name_edit.editable = not _connected
	if _port_edit != null:
		_port_edit.editable = not _connected
	if _address_edit != null:
		_address_edit.editable = not _connected
	if _max_option != null:
		_max_option.disabled = _connected


func _on_host_pressed() -> void:
	host_requested.emit(get_port(), get_player_name(), get_max_players())


func _on_join_pressed() -> void:
	join_requested.emit(get_address(), get_port(), get_player_name())


func _on_disconnect_pressed() -> void:
	disconnect_requested.emit()
