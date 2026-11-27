extends Node3D
## M5 Release 2 co-play LAN transport and 3D presence markers.
##
## This node is completely idle in single player: no peer is created and no
## network traffic happens until `host_session()` or `join_session()` is called.
## It owns only transport concerns. The authoritative Castle Document stays in
## `KeepBridge`; this script forwards snapshots, command batches, undo requests,
## clock heartbeats, hash checks and throttled presence updates, and renders a
## lightweight marker for every remote participant.
##
## Wire rules (architecture section 15.1):
## * commands, snapshots and hash checks travel reliably;
## * presence updates travel unreliably at roughly 10 Hz;
## * only commands and presence are sent, never generated layout or meshes.

signal session_state_changed(state: Dictionary)
signal snapshot_received(snapshot_json: String)
signal remote_batch_received(author_id: String, ops_json: String)
signal remote_undo_received(author_id: String)
signal clock_sync_received(build_front: float, playing: bool, speed: float)
signal hash_check_received(doc_hash: String, layout_hash: String)
signal presence_received(
	player_id: String, cam_pos: Vector3, cam_target: Vector3, status: String
)
signal status_message(text: String)
## Emitted when a peer completes the ENet handshake (host only).
signal peer_joined(peer_id: int)
## Emitted when a connected peer leaves (host only).
signal peer_left(peer_id: int)
## Emitted when a peer submits its join request (host only). `player_id` is the
## host-assigned id (`peer-<n>`), never a name chosen by the guest.
signal guest_join_requested(peer_id: int, player_id: String, player_name: String)
## Emitted when a registered peer leaves so the host can drop its player.
signal peer_left_player(peer_id: int, player_id: String)

const DEFAULT_PORT := 24565
const PRESENCE_INTERVAL_SEC := 0.10
const MARKER_HEIGHT_OFFSET := 0.55

var _peer: ENetMultiplayerPeer
var _hosting := false
var _connected := false
var _player_name := "Mason"
var _local_player_id := "host"
var _port := DEFAULT_PORT
var _max_players := 2
var _signals_connected := false
var _last_presence_sec := -1.0
var _status := "Not connected"
var _remote_players: Dictionary = {}
var _peer_player_ids: Dictionary = {}
var _markers: Dictionary = {}
var _marker_root: Node3D


func _ready() -> void:
	_marker_root = Node3D.new()
	_marker_root.name = "PresenceMarkers"
	add_child(_marker_root)


func _process(_delta: float) -> void:
	# Keep marker labels readable from the player's camera without any network
	# work. Presence traffic itself is driven by `send_presence()` callers.
	pass


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

func host_session(port: int, player_name: String, max_players: int = 2) -> Dictionary:
	disconnect_session()
	var peer := ENetMultiplayerPeer.new()
	var peer_count := maxi(max_players - 1, 1)
	var error := peer.create_server(port, peer_count)
	if error != OK:
		var failed := {
			"ok": false,
			"error": "could not host on port %d: %s" % [port, error_string(error)],
		}
		_set_status(str(failed["error"]))
		return failed

	_peer = peer
	_hosting = true
	_connected = true
	_port = port
	_max_players = max_players
	_player_name = player_name
	_local_player_id = "host"
	# Our RPC handlers already forward/authorise every message; let Godot's
	# server relay do nothing so nothing is ever relayed behind our back (H3).
	if multiplayer is SceneMultiplayer:
		(multiplayer as SceneMultiplayer).server_relay = false
	multiplayer.multiplayer_peer = peer
	_connect_multiplayer_signals()
	_set_status("Hosting on port %d as %s" % [port, player_name])
	var result := {
		"ok": true,
		"hosting": true,
		"port": port,
		"max_players": max_players,
		"player_id": _local_player_id,
	}
	session_state_changed.emit(get_status())
	return result


func join_session(address: String, port: int, player_name: String) -> Dictionary:
	disconnect_session()
	var peer := ENetMultiplayerPeer.new()
	var error := peer.create_client(address, port)
	if error != OK:
		var failed := {
			"ok": false,
			"error": "could not join %s:%d: %s" % [address, port, error_string(error)],
		}
		_set_status(str(failed["error"]))
		return failed

	_peer = peer
	_hosting = false
	_connected = true
	_port = port
	_player_name = player_name
	_local_player_id = _slug(player_name)
	if multiplayer is SceneMultiplayer:
		(multiplayer as SceneMultiplayer).server_relay = false
	multiplayer.multiplayer_peer = peer
	_connect_multiplayer_signals()
	_set_status("Connecting to %s:%d…" % [address, port])
	var result := {
		"ok": true,
		"hosting": false,
		"address": address,
		"port": port,
		"player_id": _local_player_id,
	}
	session_state_changed.emit(get_status())
	return result


func disconnect_session() -> void:
	if _peer != null:
		_peer.close()
		_peer = null
	if multiplayer != null:
		multiplayer.multiplayer_peer = null
	_hosting = false
	_connected = false
	_remote_players.clear()
	_peer_player_ids.clear()
	_clear_markers()
	_set_status("Not connected")
	session_state_changed.emit(get_status())


func is_hosting() -> bool:
	return _hosting


func is_guest() -> bool:
	return _connected and not _hosting


func is_connected_session() -> bool:
	if not _connected or _peer == null:
		return false
	if _hosting:
		return true
	return (
		_peer.get_connection_status() == MultiplayerPeer.CONNECTION_CONNECTED
	)


func get_status() -> Dictionary:
	var players: Array = []
	for peer_id in _remote_players:
		players.append((_remote_players[peer_id] as Dictionary).duplicate(true))
	return {
		"connected": is_connected_session(),
		"hosting": _hosting,
		"guest": is_guest(),
		"player_name": _player_name,
		"player_id": _local_player_id,
		"port": _port,
		"max_players": _max_players,
		"peer_id": 1 if _hosting else 0,
		"players": players,
		"status": _status,
	}


func get_local_player_id() -> String:
	return _local_player_id


## Host only: forget an unregistered peer and disconnect it (for example when
## the session is full and `register_player` refused the join).
func reject_peer(peer_id: int) -> void:
	if not _hosting:
		return
	_peer_player_ids.erase(peer_id)
	_remote_players.erase(peer_id)
	if multiplayer.multiplayer_peer is ENetMultiplayerPeer:
		(multiplayer.multiplayer_peer as ENetMultiplayerPeer).disconnect_peer(peer_id)


func send_snapshot(peer_id: int, snapshot_json: String) -> void:
	if not is_hosting() or snapshot_json.is_empty():
		return
	_receive_snapshot.rpc_id(peer_id, snapshot_json)


## Host only: push the authoritative snapshot to every connected guest.
func broadcast_snapshot_to_all(snapshot_json: String) -> void:
	if not is_hosting() or snapshot_json.is_empty():
		return
	for peer_id in multiplayer.get_peers():
		_receive_snapshot.rpc_id(peer_id, snapshot_json)


func broadcast_batch(author_id: String, ops_json: String) -> void:
	if not is_connected_session():
		return
	if _hosting:
		for peer_id in multiplayer.get_peers():
			_apply_host_batch.rpc_id(peer_id, author_id, ops_json)
	else:
		_receive_batch.rpc_id(1, ops_json)


func broadcast_undo(author_id: String) -> void:
	if not is_connected_session():
		return
	if _hosting:
		for peer_id in multiplayer.get_peers():
			_apply_host_undo.rpc_id(peer_id, author_id)
	else:
		_receive_undo.rpc_id(1)


func broadcast_clock(build_front: float, playing: bool, speed: float) -> void:
	if not is_connected_session():
		return
	if _hosting:
		for peer_id in multiplayer.get_peers():
			_receive_clock.rpc_id(peer_id, build_front, playing, speed)
	else:
		_receive_clock.rpc_id(1, build_front, playing, speed)


func broadcast_hash_check(doc_hash: String, layout_hash: String) -> void:
	if not is_connected_session():
		return
	if _hosting:
		for peer_id in multiplayer.get_peers():
			_receive_hash_check.rpc_id(peer_id, doc_hash, layout_hash)
	else:
		_receive_hash_check.rpc_id(1, doc_hash, layout_hash)


## Throttled (~10 Hz) presence beacon. Safe to call every frame: off-interval
## calls are dropped before any packet is built.
func send_presence(cam_pos: Vector3, cam_target: Vector3, status: String) -> void:
	if not is_connected_session():
		return
	var now := float(Time.get_ticks_msec()) / 1000.0
	if _last_presence_sec >= 0.0 and now - _last_presence_sec < PRESENCE_INTERVAL_SEC:
		return
	_last_presence_sec = now
	_receive_presence.rpc(_local_player_id, cam_pos, cam_target, status)


## Create, move and remove lightweight coloured 3D markers for every remote
## participant. `players` entries are the dictionaries from
## `KeepBridge.coplay_state()["players"]`.
func update_remote_markers(players: Array) -> void:
	if _marker_root == null:
		return
	var seen: Dictionary = {}
	for entry in players:
		if not (entry is Dictionary):
			continue
		var player: Dictionary = entry
		var id := str(player.get("id", ""))
		if id.is_empty() or id == _local_player_id:
			continue
		seen[id] = true
		var marker: Node3D = _markers.get(id, null)
		if marker == null:
			marker = _build_marker(id, player)
			_marker_root.add_child(marker)
			_markers[id] = marker
		_update_marker(marker, player)

	for id in _markers.keys():
		if not seen.has(id):
			var stale: Node = _markers[id]
			_markers.erase(id)
			if is_instance_valid(stale):
				stale.queue_free()


# ---------------------------------------------------------------------------
# RPC handlers (same node path on every peer)
# ---------------------------------------------------------------------------

@rpc("any_peer", "reliable")
func _submit_join(player_name: String) -> void:
	# Only the host assigns identity. The id is always derived from the ENet
	# peer id, so a guest can never impersonate another player.
	if not _hosting:
		return
	var sender_id := multiplayer.get_remote_sender_id()
	if sender_id <= 0:
		return
	var assigned_id := "peer-%d" % sender_id
	_peer_player_ids[sender_id] = assigned_id
	_receive_assigned_id.rpc_id(sender_id, assigned_id)
	guest_join_requested.emit(sender_id, assigned_id, player_name)


## Host -> guest: tell the guest the identity the host assigned it, so presence
## and local registration use the same id the host knows (H1).
@rpc("any_peer", "reliable")
func _receive_assigned_id(player_id: String) -> void:
	if _hosting or multiplayer.get_remote_sender_id() != 1:
		return
	if player_id.is_empty():
		return
	_local_player_id = player_id


@rpc("any_peer", "reliable")
func _receive_snapshot(snapshot_json: String) -> void:
	# Only the host (peer 1) may push authoritative state, and never while we
	# are the host ourselves (B1).
	if multiplayer.get_remote_sender_id() != 1 or _hosting:
		return
	snapshot_received.emit(snapshot_json)


## Guest -> host: submit a command batch. The author is resolved from the
## host-assigned peer table, never trusted from the packet (H1).
@rpc("any_peer", "reliable")
func _receive_batch(ops_json: String) -> void:
	if not _hosting:
		return
	var sender_id := multiplayer.get_remote_sender_id()
	var author_id := str(_peer_player_ids.get(sender_id, ""))
	if author_id.is_empty():
		return
	remote_batch_received.emit(author_id, ops_json)


## Host -> guests: apply an already validated batch. Only peer 1 may send it.
@rpc("any_peer", "reliable")
func _apply_host_batch(author_id: String, ops_json: String) -> void:
	if _hosting or multiplayer.get_remote_sender_id() != 1:
		return
	remote_batch_received.emit(author_id, ops_json)


## Guest -> host: request an undo. The author is host-assigned, as above.
@rpc("any_peer", "reliable")
func _receive_undo() -> void:
	if not _hosting:
		return
	var sender_id := multiplayer.get_remote_sender_id()
	var author_id := str(_peer_player_ids.get(sender_id, ""))
	if author_id.is_empty():
		return
	remote_undo_received.emit(author_id)


## Host -> guests: apply an accepted undo. Only peer 1 may send it.
@rpc("any_peer", "reliable")
func _apply_host_undo(author_id: String) -> void:
	if _hosting or multiplayer.get_remote_sender_id() != 1:
		return
	remote_undo_received.emit(author_id)


@rpc("any_peer", "unreliable")
func _receive_presence(
	player_id: String, cam_pos: Vector3, cam_target: Vector3, status: String
) -> void:
	var sender := multiplayer.get_remote_sender_id()
	var resolved_id := player_id
	if _hosting:
		# Guests cannot choose their own presence identity; use the host's map.
		resolved_id = str(_peer_player_ids.get(sender, ""))
		if resolved_id.is_empty():
			return
		for peer_id in multiplayer.get_peers():
			if peer_id != sender:
				_receive_presence.rpc_id(peer_id, resolved_id, cam_pos, cam_target, status)
	_remote_players[sender] = {
		"peer_id": sender,
		"id": resolved_id,
		"position": cam_pos,
		"target": cam_target,
		"status": status,
	}
	presence_received.emit(resolved_id, cam_pos, cam_target, status)


@rpc("any_peer", "unreliable")
func _receive_clock(build_front: float, playing: bool, speed: float) -> void:
	# Only the host may drive the shared clock (B1).
	if multiplayer.get_remote_sender_id() != 1 or _hosting:
		return
	clock_sync_received.emit(build_front, playing, speed)


@rpc("any_peer", "reliable")
func _receive_hash_check(doc_hash: String, layout_hash: String) -> void:
	# Only the host may ask for a divergence check (B1).
	if multiplayer.get_remote_sender_id() != 1 or _hosting:
		return
	hash_check_received.emit(doc_hash, layout_hash)


# ---------------------------------------------------------------------------
# Multiplayer signal plumbing
# ---------------------------------------------------------------------------

func _connect_multiplayer_signals() -> void:
	if _signals_connected:
		return
	_signals_connected = true
	multiplayer.peer_connected.connect(_on_peer_connected)
	multiplayer.peer_disconnected.connect(_on_peer_disconnected)
	multiplayer.connected_to_server.connect(_on_connected_to_server)
	multiplayer.connection_failed.connect(_on_connection_failed)
	multiplayer.server_disconnected.connect(_on_server_disconnected)


func _on_peer_connected(peer_id: int) -> void:
	if not _hosting:
		return
	_set_status("Peer %d joined" % peer_id)
	peer_joined.emit(peer_id)
	session_state_changed.emit(get_status())


func _on_peer_disconnected(peer_id: int) -> void:
	if not _hosting:
		return
	var player_id := str(_peer_player_ids.get(peer_id, ""))
	_peer_player_ids.erase(peer_id)
	_remote_players.erase(peer_id)
	_set_status("Peer %d left" % peer_id)
	peer_left.emit(peer_id)
	if not player_id.is_empty():
		peer_left_player.emit(peer_id, player_id)
	session_state_changed.emit(get_status())


func _on_connected_to_server() -> void:
	_connected = true
	_set_status("Connected as %s" % _player_name)
	# Ask the host to admit us; it assigns the authoritative player id (H1).
	_submit_join.rpc_id(1, _player_name)
	session_state_changed.emit(get_status())


func _on_connection_failed() -> void:
	_connected = false
	_set_status("Connection failed")
	session_state_changed.emit(get_status())


func _on_server_disconnected() -> void:
	_connected = false
	_hosting = false
	_remote_players.clear()
	_peer_player_ids.clear()
	if multiplayer != null:
		multiplayer.multiplayer_peer = null
	if _peer != null:
		_peer.close()
		_peer = null
	_clear_markers()
	_set_status("Host disconnected — session ended")
	session_state_changed.emit(get_status())


# ---------------------------------------------------------------------------
# Presence markers
# ---------------------------------------------------------------------------

func _build_marker(player_id: String, player: Dictionary) -> Node3D:
	var marker := Node3D.new()
	marker.name = "Presence_%s" % player_id
	var color := _parse_color(str(player.get("color_hex", "#888888")))

	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = color
	material.emission_enabled = true
	material.emission = color
	material.emission_energy_multiplier = 1.6

	var body := MeshInstance3D.new()
	var sphere := SphereMesh.new()
	sphere.radius = 0.28
	sphere.height = 0.56
	body.mesh = sphere
	body.material_override = material
	marker.add_child(body)

	# A small cone showing the camera look direction.
	var cone := MeshInstance3D.new()
	var cone_mesh := CylinderMesh.new()
	cone_mesh.top_radius = 0.0
	cone_mesh.bottom_radius = 0.16
	cone_mesh.height = 0.55
	cone.mesh = cone_mesh
	cone.material_override = material
	cone.position = Vector3(0.0, 0.0, -0.42)
	cone.rotation_degrees = Vector3(-90.0, 0.0, 0.0)
	marker.add_child(cone)

	var label := Label3D.new()
	label.name = "Name"
	label.text = str(player.get("name", player_id))
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.font_size = 28
	label.pixel_size = 0.004
	label.modulate = color
	label.outline_size = 6
	label.position = Vector3(0.0, MARKER_HEIGHT_OFFSET, 0.0)
	marker.add_child(label)

	return marker


func _update_marker(marker: Node3D, player: Dictionary) -> void:
	var position := _mm_to_metres(player.get("camera_pos_mm", []))
	var target := _mm_to_metres(player.get("camera_target_mm", []))
	marker.position = position
	var direction := target - position
	if direction.length_squared() > 0.0001:
		marker.basis = Basis.looking_at(direction.normalized(), Vector3.UP)
	var label: Node = marker.get_node_or_null("Name")
	if label != null:
		label.text = str(player.get("name", ""))
		var status := str(player.get("status", ""))
		if not status.is_empty():
			label.text = "%s\n%s" % [label.text, status]


func _clear_markers() -> void:
	for id in _markers:
		var marker: Node = _markers[id]
		if is_instance_valid(marker):
			marker.queue_free()
	_markers.clear()


func _mm_to_metres(value: Variant) -> Vector3:
	if value is Array and (value as Array).size() >= 3:
		var triple: Array = value
		return Vector3(
			float(triple[0]) / 1000.0,
			float(triple[1]) / 1000.0,
			float(triple[2]) / 1000.0
		)
	return Vector3.ZERO


func _parse_color(hex: String) -> Color:
	var text := hex.strip_edges()
	if text.is_empty():
		return Color(0.53, 0.53, 0.53)
	return Color.from_string(text, Color(0.53, 0.53, 0.53))


func _slug(name: String) -> String:
	var slug := name.strip_edges().to_lower().replace(" ", "-")
	if slug.is_empty():
		return "guest"
	return slug


func _set_status(text: String) -> void:
	_status = text
	status_message.emit(text)
