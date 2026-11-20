# Protocol

The sender (Mac) is the TCP client; the receiver (laptop) listens. Everything runs over the USB
network link that the cable provides.

| Revision | Sender | Receiver | Adds |
|---------:|--------|----------|------|
| 1 | Shabnam Display 0.1 – 0.2 | subscreen-rx 0.1 | video, input, sound (§1 – §5) |
| 2 | Shabnam Display 0.3 | subscreen-rx 0.2 | the control channel (§6) |

Revisions mix freely: the control channel is a separate, optional connection, and §1 – §5 are
unchanged. A sender that does not know it never connects to it; a receiver that does not know
it refuses the connection and the sender carries on without.

| | |
|---|---|
| Receiver address | `169.254.1.1` (also reachable as `10.55.0.1`) |
| Sender address | assigned by the receiver, in `10.55.0.0/24` |
| Port 7000 | video (sender → receiver) and input events (receiver → sender) |
| Port 7001 | sound (sender → receiver) |
| Port 7002 | control and status (both directions), optional — §6 |
| Transport | TCP, `TCP_NODELAY`, no encryption, no framing layer |

The receiver binds its listeners to the USB interface. The sender finds the interface that
holds a `10.55.0.x` address and pins its sockets to it (`IP_BOUND_IF`); if no interface has such
an address it does not try to connect at all.

## 1. Handshake (every connection, every port)

Both ends hold the same pairing key `K` (the whole contents of the key file, at least 32
bytes). Neither side sends it.

```
sender   → receiver   Ns                                  16 random bytes
receiver → sender     Nr ‖ HMAC-SHA256(K, "rx" ‖ Ns ‖ Nr)   16 + 32 bytes
sender   → receiver   HMAC-SHA256(K, "tx" ‖ Nr ‖ Ns)        32 bytes
```

- `"rx"` and `"tx"` are the two ASCII bytes; `‖` is concatenation.
- The sender verifies the receiver's tag before sending its own; the receiver verifies the
  sender's tag before treating the connection as valid. Comparisons are constant-time.
- Each side waits at most 5 seconds for the other's message. On any failure the connection is
  closed and nothing else is sent.
- The handshake authenticates the peers at connection time. It does not encrypt or
  authenticate the data that follows — the link is assumed to be a point-to-point cable.

## 2. Video — port 7000, sender → receiver

A continuous **H.264 Annex-B elementary stream**: NAL units, each preceded by the start code
`00 00 00 01`. TCP writes do not line up with frames; the receiver's parser finds the
boundaries.

- 1920 × 1080, 60 Hz nominal, 4:2:0 8-bit, BT.709, video range.
- High profile, no B-frames (no reordering delay).
- The first frame on a connection is an IDR frame, preceded by SPS and PPS. SPS/PPS are
  repeated before every later keyframe.
- After each frame the sender appends an access unit delimiter NAL, `00 00 00 01 09 F0`, so the
  parser knows the frame is complete and can decode it immediately instead of waiting for the
  next one to begin.
- Frames are only sent when the picture changes, plus up to ten repeats of a still picture
  (about 30 per second) so the encoder can refine it to full quality. Long gaps between frames
  are normal.
- No timestamps are carried; the receiver shows each frame as soon as it is decoded and drops
  frames rather than queueing them.

## 3. Input — port 7000, receiver → sender

On the same connection, in the other direction: a stream of fixed **8-byte records**, little
endian — a Linux `struct input_event` without its timestamp.

| Offset | Size | Field |
|-------:|-----:|-------|
| 0 | 2 | `type` (u16) |
| 2 | 2 | `code` (u16) |
| 4 | 4 | `value` (i32) |

| `type` | Meaning | `code` / `value` |
|-------:|---------|------------------|
| 1 `EV_KEY` | key or button | Linux key code; value 1 = press, 0 = release, 2 = auto-repeat. Codes `0x110`–`0x117` are pointer buttons (left, right, middle, …) |
| 2 `EV_REL` | relative motion | code 0 = X, 1 = Y, 8 = vertical wheel, 6 = horizontal wheel; value = signed delta |
| 0 `EV_SYN` | end of batch | motion collected since the last sync is applied now |

Other types are ignored. The receiver keeps some keys for itself (brightness, volume, mute, lid)
and does not forward them.

The sender maps key codes by physical position (US layout) to macOS virtual key codes; Ctrl →
Control, Alt → Option, Super → Command. Play/pause, next and previous travel as macOS media
keys.

## 4. Sound — port 7001, sender → receiver

After the same handshake: raw **PCM, 48 000 Hz, signed 16-bit little endian, stereo
interleaved** (left, right), no header, no framing. The receiver reads it in 10 ms blocks
(1920 bytes) and discards blocks when more than about 120 ms is queued, so delay cannot build
up. Volume is applied on the receiver.

Where the sender gets the samples from (listening in on the system sound, or taking it away
from the Mac's own speakers) is a local choice and does not change the stream.

The sound connection is optional and independent: the sender opens it only while the video
link is up and sound is not switched off, and re-opens it if it drops.

## 5. Connection lifetime

- The video connection is the session. When it closes (cable pulled, receiver left
  second-screen mode, either program quit), the sender releases any keys and buttons still held,
  removes the virtual display and closes the sound connection, then starts polling again about
  once a second.
- The receiver serves one sender at a time.

## 6. Control — port 7002, both directions (revision 2; control revision 2 since subscreen-rx 0.3)

A third connection (video port + 2), after the same handshake as the others and bound to the
cable interface like them. It lets the sender adjust the laptop and shows it the laptop's state.
It is optional and independent, like sound: if it is refused or drops, picture, input and sound
are unaffected. The receiver serves one control connection; a newer one replaces the older.

The port is served in both of the laptop's modes, by one program at a time:

- **Second-screen mode:** by `subscreen-rx` itself. The sender opens the connection while the
  video link is up and tries again every few seconds while that lasts.
- **The laptop's own desktop** (since subscreen-rx 0.3): by `subscreen-rx control`, which the
  session script starts with the desktop and stops before second-screen mode. It opens nothing
  but this port — no display, decoder, input device or sound device — keeps trying if the port
  cannot be had yet, and listens afresh when the cable's network interface is re-created. Volume
  and mute are then the desktop's sound output (PipeWire), mapped to the same 16 steps
  (step × 100 / 16 percent, rounded), so the laptop's own volume keys show up in the status.
  The sender, when it has no video link, tries this port every few seconds while the cable
  network is present; when a second-screen session starts, that session's control connection
  replaces it.

Which of the two is answering is flag bit 6 of the status message.

Both directions carry the same framing:

| Offset | Size | Field |
|-------:|-----:|-------|
| 0 | 1 | `type` |
| 1 | 1 | `length` of the payload, 0 – 32 |
| 2 | `length` | payload |

Rules for both ends:

- A `length` above 32 is a protocol violation: the connection is closed.
- Unknown `type`s are skipped (their payload is read and dropped).
- A payload shorter than the type needs makes the message void; it is skipped. Payload bytes
  beyond the ones defined here are ignored, so later revisions can append fields.
- Numbers are clamped to their range by the side that acts on them.

### Sender → receiver

| `type` | Name | Payload | Effect |
|-------:|------|---------|--------|
| `0x01` | set brightness | 1 byte, percent | Panel backlight, clamped to 0 – 100. The receiver never goes below 4 % of the panel's range, and does not light a panel that is switched off (lid closed). |
| `0x02` | set volume | 1 byte, step | Speaker volume, clamped to 0 – 16 (the same 16 steps as the laptop's volume keys). Does not change mute. |
| `0x03` | set mute | 1 byte: 1 = mute, 0 = un-mute | Any other value: ignored. |
| `0x04` | leave | none | The receiver ends second-screen mode exactly as the laptop's Win+Esc does: it exits, all connections close, and the laptop returns to its own desktop. Ignored on the laptop's own desktop. |
| `0x05` | get status | none | A status message follows (within the rate limit below). |
| `0x06` | become second screen | none | *Control revision 2.* On its own desktop the laptop does what its Win+Esc does there: the desktop ends and second-screen mode starts; the control connection drops and the sender connects as for any session. Ignored in second-screen mode. |
| `0x07` | set screen | 1 byte: 1 = on, 0 = off | *Control revision 2.* Backlight power only; brightness is kept. "On" is ignored while the lid is closed. Any other value: ignored. |
| `0x08` | end session | 1 byte: 1 = power off, 2 = restart | *Control revision 2.* The laptop shuts down or restarts the way its own power buttons do. Any other value, or no value: ignored. |

These eight are the only things a message can cause. The receiver acts on the type and on one
clamped integer; no part of a message is ever used as text, a path or a program argument (the
desktop's volume is set by starting `wpctl` directly with fixed arguments and a percentage
printed from the clamped step; mode changes write one of three fixed words for the session
script).

Guard rails on the receiver: when one read from the socket yields several commands, only the
newest of each kind is carried out; `0x07` is carried out at most once every 0.5 s and `0x06` /
`0x08` together at most once every 5 s (what comes sooner is dropped, not queued); each of
`0x06`–`0x08` is written to the receiver's log.

Compatibility: a receiver of control revision 1 (subscreen-rx 0.2) skips `0x06`–`0x08` as
unknown types, so a sender must offer them only when the status says revision 2 or later; a
sender of revision 1 never sends them and ignores the two new flag bits.

### Receiver → sender

| `type` | Name | Payload |
|-------:|------|---------|
| `0x81` | status | 9 bytes, below |

| Offset | Field |
|-------:|-------|
| 0 | protocol revision of the control channel: 1 (subscreen-rx 0.2) or 2 (0.3) |
| 1 | brightness, percent 0 – 100 |
| 2 | volume, step 0 – *steps* |
| 3 | *steps*: the number of volume steps (16); 0 makes the message void |
| 4 | flags, below |
| 5 | battery, percent 0 – 100 (meaningful only with flag bit 1) |
| 6, 7, 8 | receiver version: major, minor, patch |

| Flag bit | Meaning |
|---------:|---------|
| 0 | sound is muted |
| 1 | a battery is present and byte 5 is valid |
| 2 | the battery is charging |
| 3 | external power is connected (charging, or full) |
| 4 | the lid state is known (in second-screen mode: the sensor has reported since the receiver started) |
| 5 | the lid is closed (meaningful only with bit 4) |
| 6 | *control revision 2:* the laptop is on its own desktop (the control-only receiver is answering); 0 in second-screen mode |
| 7 | *control revision 2:* the screen (backlight) is switched off |

The receiver sends a status once when the connection is established, then whenever any field
changes — including brightness, volume, mute and screen changed on the laptop itself (on its own
desktop the sound output is looked at about once a second while a sender is connected) — and when
asked with `0x05`; in every case **at most one status a second**. The sender should therefore
show a value the user has just set at once, and not let a status that was already on its way
move the control back (the app holds a user-set value for 2 s or until the receiver confirms
it, then asks for the status once more).

## 7. Content measurement (receiver, revision 2 — log only)

Not part of the wire format: since subscreen-rx 0.2 the receiver notes, for every decoded
frame, when it arrived and how many encoded bytes came in since the previous one, and writes
one line to its log every 10 seconds:

```
content: text-like | 88 frames in 10.0 s, 26% active, 8 burst(s), longest run 0.3 s | frame bytes p50 400 p95 6000 max 6000 | <1k 80, <16k 8, <128k 0, larger 0 | 0.04 Mbit/s
```

`idle` / `text-like` / `video-like` is a guess from those two numbers alone (how much of the
window was spent in unbroken runs of frames, and whether the frames had real size). Nothing
acts on it yet.
