extends Node
## M0b zero-budget procedural construction audio.
##
## Everything is synthesised live into an AudioStreamGenerator: no .wav/.ogg
## files, no external libraries, no paid samples. Two voices cover the M0b
## feel pass:
##
##   play_place_thud(pitch_scale)  — resonant low-mid stone thud on snap/place
##   play_stone_tick(progress)     — crisp chisel/settle click during build
##
## Ticks are gently detuned by construction progress and attenuated by camera
## distance (set_camera_distance) so close inspection sounds intimate while a
## wide panorama keeps the mix calm. In --headless mode every call is a no-op.
##
## Voices are kept in parallel typed arrays (not per-voice Dictionaries) so the
## 22 kHz inner synthesis loop stays fast and allocation-free.

const MIX_RATE := 22050
const INV_MIX_RATE := 1.0 / 22050.0
const MAX_VOICES := 12
const TICK_MIN_GAP_MS := 55

const THUD_DURATION := 0.62
const TICK_DURATION := 0.10

const VOICE_THUD := 0
const VOICE_TICK := 1

var _headless := false
var _headless_checked := false
var _player: AudioStreamPlayer
var _generator: AudioStreamGenerator
var _playback: AudioStreamGeneratorPlayback

# Parallel voice state. Index i in every array belongs to the same voice.
var _voice_kind := PackedInt32Array()
var _voice_amp := PackedFloat32Array()
var _voice_pitch := PackedFloat32Array()
var _voice_t := PackedFloat32Array()
var _voice_duration := PackedFloat32Array()
var _voice_phase_a := PackedFloat32Array()
var _voice_phase_b := PackedFloat32Array()
var _voice_noise := PackedInt32Array()

var _camera_distance := 90.0
var _last_tick_ms := -100000


func _ready() -> void:
	_ensure_setup()


func _ensure_setup() -> void:
	if _headless_checked:
		return
	_headless_checked = true
	_headless = DisplayServer.get_name() == "headless"
	if _headless:
		return

	_generator = AudioStreamGenerator.new()
	_generator.mix_rate = MIX_RATE
	# 40 ms keeps thuds and ticks immediate instead of queueing behind silence.
	_generator.buffer_length = 0.04

	_player = AudioStreamPlayer.new()
	_player.name = "Generator"
	_player.stream = _generator
	_player.volume_db = -3.0
	add_child(_player)


func is_headless() -> bool:
	_ensure_setup()
	return _headless


## Resonant stone thud for snapping/placing a wall or tower. Pitch scales with
## element size (bigger element → lower, heavier thud).
func play_place_thud(pitch_scale: float = 1.0) -> void:
	_ensure_setup()
	if _headless or not _ensure_playback():
		return
	var attenuation := clampf(120.0 / maxf(_camera_distance + 40.0, 1.0), 0.35, 1.0)
	_add_voice(VOICE_THUD, 0.62 * attenuation, clampf(pitch_scale, 0.5, 2.0))
	# Fill the generator immediately so the attack transient is not lost to an
	# underrun on the very first mix callback.
	_pump_audio(4096)


## Subtle chisel/settle click while masonry is rising. `progress` is the
## 0..1 build front; it nudges the pitch upward as the wall extends.
func play_stone_tick(progress: float) -> void:
	_ensure_setup()
	if _headless:
		return
	var now := Time.get_ticks_msec()
	if now - _last_tick_ms < TICK_MIN_GAP_MS:
		return
	_last_tick_ms = now
	if not _ensure_playback():
		return

	var attenuation := clampf(70.0 / maxf(_camera_distance + 25.0, 1.0), 0.12, 1.0)
	var pitch := 0.86 + clampf(progress, 0.0, 1.0) * 0.55
	pitch *= randf_range(0.94, 1.07)
	_add_voice(VOICE_TICK, 0.30 * attenuation, pitch)
	_pump_audio(4096)


## Zoom-reactive mix control, fed from the orbit camera each frame.
func set_camera_distance(distance: float) -> void:
	_camera_distance = maxf(distance, 1.0)


func active_voice_count() -> int:
	return _voice_t.size()


func _ensure_playback() -> bool:
	if _headless or _player == null:
		return false
	if _playback == null:
		# play() is only legal once the player has entered the tree; a probe or
		# very first frame call is silently skipped instead of spamming errors.
		if not _player.is_inside_tree():
			return false
		_player.play()
		_playback = _player.get_stream_playback() as AudioStreamGeneratorPlayback
	return _playback != null


func _add_voice(kind: int, amplitude: float, pitch: float) -> void:
	if _voice_t.size() >= MAX_VOICES:
		_remove_voice(0)
	_voice_kind.append(kind)
	_voice_amp.append(amplitude)
	_voice_pitch.append(pitch)
	_voice_t.append(0.0)
	_voice_duration.append(THUD_DURATION if kind == VOICE_THUD else TICK_DURATION)
	_voice_phase_a.append(0.0)
	_voice_phase_b.append(0.0)
	_voice_noise.append(12345 + randi() % 1_000_000)


func _remove_voice(index: int) -> void:
	_voice_kind.remove_at(index)
	_voice_amp.remove_at(index)
	_voice_pitch.remove_at(index)
	_voice_t.remove_at(index)
	_voice_duration.remove_at(index)
	_voice_phase_a.remove_at(index)
	_voice_phase_b.remove_at(index)
	_voice_noise.remove_at(index)


func _process(_delta: float) -> void:
	_pump_audio(2048)


func _pump_audio(max_frames: int) -> void:
	if _headless or _playback == null:
		return
	var frames := mini(_playback.get_frames_available(), max_frames)
	if frames <= 0:
		return

	var buffer := PackedVector2Array()
	buffer.resize(frames)
	for i in range(frames):
		var sample := 0.0
		for voice in range(_voice_t.size()):
			sample += _sample_voice(voice)
		# Soft clip keeps stacked voices from crackling.
		sample = sample / (1.0 + absf(sample))
		buffer[i] = Vector2(sample, sample)

	_playback.push_buffer(buffer)

	for i in range(_voice_t.size() - 1, -1, -1):
		if _voice_t[i] >= _voice_duration[i]:
			_remove_voice(i)


func _sample_voice(index: int) -> float:
	var t := _voice_t[index]
	if t >= _voice_duration[index]:
		return 0.0

	var amp := _voice_amp[index]
	var pitch := _voice_pitch[index]
	var out := 0.0

	if _voice_kind[index] == VOICE_THUD:
		# Body: a low fundamental that drops as it decays, plus an inharmonic
		# partial and a very short noise transient for the stone-on-stone hit.
		var env := exp(-t * 7.0) * minf(t * 120.0, 1.0)
		var freq := (92.0 * pitch) * exp(-t * 6.5) + 40.0
		_voice_phase_a[index] += TAU * freq * INV_MIX_RATE
		_voice_phase_b[index] += TAU * (freq * 1.67) * INV_MIX_RATE
		var body := sin(_voice_phase_a[index]) * 0.78
		body += sin(_voice_phase_b[index]) * 0.22
		var transient := _next_noise(index) * exp(-t * 70.0)
		out = (body * 0.85 + transient * 0.30) * env * amp
	else:
		# Tick: high, dry, mostly noise with two short ringing partials.
		var env := exp(-t * 30.0) * minf(t * 400.0, 1.0)
		var freq := 1450.0 * pitch
		_voice_phase_a[index] += TAU * freq * INV_MIX_RATE
		_voice_phase_b[index] += TAU * (freq * 2.71) * INV_MIX_RATE
		var noise := _next_noise(index)
		out = (
			sin(_voice_phase_a[index]) * 0.40
			+ sin(_voice_phase_b[index]) * 0.18
			+ noise * 0.42
		) * env * amp

	_voice_t[index] = t + INV_MIX_RATE
	return out


func _next_noise(index: int) -> float:
	var state := _voice_noise[index]
	state = int((state * 1664525 + 1013904223) & 0x7fffffff)
	_voice_noise[index] = state
	return float(state) / 1073741823.5 - 1.0
