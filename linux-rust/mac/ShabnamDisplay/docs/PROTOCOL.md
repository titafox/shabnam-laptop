# Protocol

The sender (Mac) is the TCP client; the receiver (laptop) listens. Everything runs over the USB
network link that the cable provides.

| | |
|---|---|
| Receiver address | `169.254.1.1` (also reachable as `10.55.0.1`) |
| Sender address | assigned by the receiver, in `10.55.0.0/24` |
| Port 7000 | video (sender → receiver) and input events (receiver → sender) |
| Port 7001 | sound (sender → receiver) |
| Transport | TCP, `TCP_NODELAY`, no encryption, no framing layer |

The receiver binds its listeners to the USB interface. The sender finds the interface that
holds a `10.55.0.x` address and pins its sockets to it (`IP_BOUND_IF`); if no interface has such
an address it does not try to connect at all.

## 1. Handshake (every connection, both ports)

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
