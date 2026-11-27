extends CanvasLayer
## M3.1 in-game historical codex.
##
## A centred, modal reading panel fed by `keep-gd`'s `list_codex_entries()`.
## The left column lists every entry; the right column renders the selected
## entry's title, category/period badge, summary, architectural body and
## numbered scholarly citations. Styling mirrors AgentPanel (dark slate with a
## warm parchment accent) and all UI is built in code (AGENTS.md rule 5).

signal close_requested
signal entry_selected(entry_id: String)

const PANEL_SIZE := Vector2(760.0, 520.0)
const LIST_WIDTH := 218.0
const PANEL_BG := Color(0.06, 0.065, 0.08, 0.96)
const CARD_BG := Color(0.10, 0.105, 0.125, 0.94)
const BORDER := Color(0.55, 0.46, 0.32, 0.60)
const TEXT := Color(0.90, 0.88, 0.81)
const MUTED := Color(0.66, 0.64, 0.57)
const PARCHMENT := Color(0.95, 0.84, 0.62)
const ACCENT := Color(0.86, 0.72, 0.46)

var _panel: PanelContainer
var _list_box: VBoxContainer
var _detail_box: VBoxContainer
var _empty_label: Label
var _entry_buttons: Dictionary = {}
var _entries: Array = []
var _selected_id := ""


func _init() -> void:
	layer = 40
	visible = false
	_build()


func _ready() -> void:
	if _panel == null:
		_build()


func _build() -> void:
	if _panel != null:
		return
	var root := Control.new()
	root.name = "CodexRoot"
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	var backdrop := ColorRect.new()
	backdrop.name = "CodexBackdrop"
	backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	backdrop.color = Color(0.02, 0.03, 0.04, 0.55)
	backdrop.mouse_filter = Control.MOUSE_FILTER_STOP
	root.add_child(backdrop)

	_panel = PanelContainer.new()
	_panel.name = "CodexFrame"
	_panel.set_anchors_preset(Control.PRESET_CENTER)
	_panel.offset_left = -PANEL_SIZE.x * 0.5
	_panel.offset_right = PANEL_SIZE.x * 0.5
	_panel.offset_top = -PANEL_SIZE.y * 0.5
	_panel.offset_bottom = PANEL_SIZE.y * 0.5
	_panel.custom_minimum_size = PANEL_SIZE
	_panel.add_theme_stylebox_override("panel", _panel_style(PANEL_BG, BORDER))
	root.add_child(_panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 16)
	margin.add_theme_constant_override("margin_right", 16)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 14)
	_panel.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 8)
	margin.add_child(column)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	column.add_child(header)

	var title := Label.new()
	title.text = "HISTORICAL CODEX"
	title.add_theme_font_size_override("font_size", 16)
	title.add_theme_color_override("font_color", PARCHMENT)
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)

	var hint := Label.new()
	hint.text = "C / Esc to close"
	hint.add_theme_font_size_override("font_size", 11)
	hint.add_theme_color_override("font_color", MUTED)
	header.add_child(hint)

	var close_button := Button.new()
	close_button.text = "×"
	close_button.focus_mode = Control.FOCUS_NONE
	close_button.custom_minimum_size = Vector2(30.0, 24.0)
	close_button.add_theme_font_size_override("font_size", 16)
	close_button.pressed.connect(_on_close_pressed)
	header.add_child(close_button)

	column.add_child(HSeparator.new())

	var content := HBoxContainer.new()
	content.add_theme_constant_override("separation", 12)
	content.size_flags_vertical = Control.SIZE_EXPAND_FILL
	column.add_child(content)

	# --- Left column: entry list --------------------------------------
	var list_scroll := ScrollContainer.new()
	list_scroll.name = "CodexEntryList"
	list_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	list_scroll.custom_minimum_size = Vector2(LIST_WIDTH, 0.0)
	list_scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	content.add_child(list_scroll)

	_list_box = VBoxContainer.new()
	_list_box.add_theme_constant_override("separation", 4)
	_list_box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	list_scroll.add_child(_list_box)

	content.add_child(VSeparator.new())

	# --- Right column: selected entry ---------------------------------
	var detail_scroll := ScrollContainer.new()
	detail_scroll.name = "CodexEntryDetail"
	detail_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	detail_scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	detail_scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	content.add_child(detail_scroll)

	_detail_box = VBoxContainer.new()
	_detail_box.add_theme_constant_override("separation", 8)
	_detail_box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	detail_scroll.add_child(_detail_box)

	_empty_label = Label.new()
	_empty_label.text = "No codex entries loaded."
	_empty_label.add_theme_color_override("font_color", MUTED)
	_empty_label.add_theme_font_size_override("font_size", 12)
	_detail_box.add_child(_empty_label)


func _panel_style(bg: Color, border: Color) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = bg
	style.border_color = border
	style.set_border_width_all(1)
	style.set_corner_radius_all(8)
	style.content_margin_left = 4.0
	style.content_margin_right = 4.0
	style.content_margin_top = 4.0
	style.content_margin_bottom = 4.0
	return style


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

## Replace the entry list. `entries` is the array returned by
## `KeepBridge.list_codex_entries()`.
func set_entries(entries: Array) -> void:
	_entries = entries
	_entry_buttons.clear()
	for child in _list_box.get_children():
		child.queue_free()
	for index in _entries.size():
		var entry: Variant = _entries[index]
		if not (entry is Dictionary):
			continue
		var data: Dictionary = entry
		var entry_id := str(data.get("id", ""))
		var button := Button.new()
		button.text = str(data.get("title", entry_id))
		button.alignment = HORIZONTAL_ALIGNMENT_LEFT
		button.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		button.toggle_mode = true
		button.focus_mode = Control.FOCUS_NONE
		button.custom_minimum_size = Vector2(0.0, 34.0)
		button.add_theme_font_size_override("font_size", 12)
		button.add_theme_color_override("font_color", TEXT)
		button.add_theme_color_override("font_pressed_color", PARCHMENT)
		button.pressed.connect(_on_entry_pressed.bind(entry_id))
		_list_box.add_child(button)
		_entry_buttons[entry_id] = button

	var first_id := ""
	for entry in _entries:
		if entry is Dictionary and not str((entry as Dictionary).get("id", "")).is_empty():
			first_id = str((entry as Dictionary).get("id", ""))
			break
	if not first_id.is_empty():
		select_entry(first_id)


## Render one entry by id. Unknown ids leave the panel unchanged.
func select_entry(entry_id: String) -> void:
	var data := _find_entry(entry_id)
	if data.is_empty():
		return
	_selected_id = entry_id
	for key in _entry_buttons:
		var button: Button = _entry_buttons[key]
		button.set_pressed_no_signal(str(key) == entry_id)
	_render_detail(data)
	entry_selected.emit(entry_id)


func get_selected_id() -> String:
	return _selected_id


func get_entry_count() -> int:
	return _entries.size()


func is_open() -> bool:
	return visible


func open() -> void:
	visible = true


func close() -> void:
	visible = false


func toggle() -> void:
	visible = not visible


# ---------------------------------------------------------------------------
# Internals
# ---------------------------------------------------------------------------

func _find_entry(entry_id: String) -> Dictionary:
	for entry in _entries:
		if entry is Dictionary and str((entry as Dictionary).get("id", "")) == entry_id:
			return entry as Dictionary
	return {}


func _render_detail(data: Dictionary) -> void:
	for child in _detail_box.get_children():
		child.queue_free()

	var title := Label.new()
	title.text = str(data.get("title", ""))
	title.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	title.add_theme_font_size_override("font_size", 19)
	title.add_theme_color_override("font_color", PARCHMENT)
	_detail_box.add_child(title)

	var badge_row := HBoxContainer.new()
	badge_row.add_theme_constant_override("separation", 6)
	_detail_box.add_child(badge_row)
	_add_badge(badge_row, str(data.get("category", "")), ACCENT)
	_add_badge(badge_row, str(data.get("period", "")), Color(0.72, 0.80, 0.88))

	var summary := Label.new()
	summary.text = str(data.get("summary", ""))
	summary.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	summary.add_theme_font_size_override("font_size", 13)
	summary.add_theme_color_override("font_color", Color(0.84, 0.82, 0.74))
	_detail_box.add_child(summary)

	_detail_box.add_child(HSeparator.new())

	var body := Label.new()
	body.text = str(data.get("body", ""))
	body.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	body.add_theme_font_size_override("font_size", 13)
	body.add_theme_color_override("font_color", TEXT)
	body.custom_minimum_size = Vector2(0.0, 0.0)
	_detail_box.add_child(body)

	var citations: Variant = data.get("citations", [])
	if citations is Array and not (citations as Array).is_empty():
		var heading := Label.new()
		heading.text = "Sources"
		heading.add_theme_font_size_override("font_size", 13)
		heading.add_theme_color_override("font_color", ACCENT)
		_detail_box.add_child(heading)
		var index := 1
		for citation in (citations as Array):
			var line := Label.new()
			line.text = "%d. %s" % [index, str(citation)]
			line.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
			line.add_theme_font_size_override("font_size", 12)
			line.add_theme_color_override("font_color", MUTED)
			_detail_box.add_child(line)
			index += 1

	var related: Variant = data.get("related_style_ids", [])
	if related is Array and not (related as Array).is_empty():
		var styles := PackedStringArray()
		for style_id in (related as Array):
			styles.append(str(style_id).replace("_", " ").capitalize())
		var note := Label.new()
		note.text = "Explains: %s" % ", ".join(styles)
		note.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		note.add_theme_font_size_override("font_size", 11)
		note.add_theme_color_override("font_color", Color(0.58, 0.62, 0.66))
		_detail_box.add_child(note)


func _add_badge(row: HBoxContainer, text: String, color: Color) -> void:
	if text.strip_edges().is_empty():
		return
	var badge := PanelContainer.new()
	var style := StyleBoxFlat.new()
	style.bg_color = CARD_BG
	style.border_color = color
	style.set_border_width_all(1)
	style.set_corner_radius_all(4)
	style.content_margin_left = 6.0
	style.content_margin_right = 6.0
	style.content_margin_top = 1.0
	style.content_margin_bottom = 1.0
	badge.add_theme_stylebox_override("panel", style)
	var label := Label.new()
	label.text = text
	label.add_theme_font_size_override("font_size", 11)
	label.add_theme_color_override("font_color", color)
	badge.add_child(label)
	row.add_child(badge)


func _on_entry_pressed(entry_id: String) -> void:
	select_entry(entry_id)


func _on_close_pressed() -> void:
	close_requested.emit()
