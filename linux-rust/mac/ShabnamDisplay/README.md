# Shabnam Display

A macOS menu-bar app that turns a **Shabnam laptop** into a second screen for a Mac over a
single USB-C cable — picture, sound, and the laptop's own keyboard and touchpad.

Plug the cable in, put the laptop into second-screen mode, and an extra 1920 × 1080 display
called "Shabnam" appears in macOS. Unplug it and the display vanishes; windows return to the
Mac's own screen.

This repository is the **Mac side** (the sender). The laptop side (the receiver, `subscreen-rx`,
written in Rust) is a separate program.

## How it works

```
 Mac                                                  Laptop
 ┌──────────────────────────────────────────┐         ┌─────────────────────────┐
 │ virtual display "Shabnam" (CoreGraphics)  │         │                         │
 │   └─ ScreenCaptureKit ─ VideoToolbox H.264├─ :7000 ─▶ hardware decoder ─ panel│
 │ system sound ─ 48 kHz 16-bit stereo PCM   ├─ :7001 ─▶ speakers / headphones   │
 │ CGEvent  ◀─ keyboard and touchpad events ─┤◀ :7000 ─┤ keyboard, touchpad      │
 └──────────────────────────────────────────┘  USB-C  └─────────────────────────┘
                                             (USB network link)
```

- The cable carries an ordinary network link. The laptop is `169.254.1.1` and hands the Mac an
  address in `10.55.0.0/24`.
- The app waits until the laptop is listening, then both ends prove to each other that they hold
  the same pairing key (HMAC-SHA256 challenge/response). **Nothing is sent and no input is
  accepted before that succeeds.**
- Only then is the virtual display created, captured at 60 Hz, encoded by the Mac's hardware
  H.264 encoder (no B-frames, about 120 Mbit/s target, quality ceiling for crisp text) and sent
  as an Annex-B byte stream.
- Sockets are pinned to the cable's network interface, so the stream can never wander onto Wi-Fi
  or reach anything other than the machine at the other end of the cable.
- When the link drops, held keys are released, capture stops and the display is removed. The
  app then goes back to waiting — no helper script or restart loop needed.

Wire format details: [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Menu

| Item | What it does |
|------|--------------|
| Status line | `Waiting for laptop`, `Connected · 1080p60 · 42 Mbit/s`, or `Error: …` |
| Connect / Disconnect | Stop or resume looking for the laptop (remembered across launches) |
| Resolution | `1920 × 1080 HiDPI` (drawn at 3840 × 2160, averaged 2:1 — sharper; default) or plain `1920 × 1080` |
| Send audio | Play the Mac's sound output on the laptop |
| Launch at login | Registers the app as a login item |
| Pairing key… | Shows where the key file is; reveal it in Finder or generate a new one |
| Permissions… | Shows whether Screen Recording and Accessibility are granted and opens System Settings |

The interface is in English and Simplified Chinese.

## Requirements

- macOS 14 or later (developed on Apple silicon).
- Swift 5.9+ toolchain (Xcode or the Command Line Tools). No Xcode project and no third-party
  dependencies.
- A Shabnam laptop running `subscreen-rx`, and a USB-C cable.

## Build

```sh
make app          # release build + "build/Shabnam Display.app" (ad-hoc signed)
make run          # build and launch
make install      # copy the app to ~/Applications
swift test        # handshake and record-format tests
```

or by hand: `swift build -c release`, then `scripts/bundle.sh`.

The bare binary (`swift run`) works too, but macOS then attributes the permissions to the
terminal it was started from.

### Signing and permissions

The bundle is signed ad hoc by default. macOS ties privacy permissions to the code signature,
and an ad-hoc signature changes with every build — so **after each rebuild macOS will ask for
Screen Recording and Accessibility again** (remove the stale entry in System Settings and add
the app again). To avoid that, sign with a stable identity from your keychain:

```sh
CODESIGN_IDENTITY="Your Signing Identity" scripts/bundle.sh
```

## Permissions

| Permission | Why | Where |
|------------|-----|-------|
| Screen Recording | Capture the virtual display and the system sound | System Settings → Privacy & Security → Screen & System Audio Recording |
| Accessibility | Replay the laptop's keyboard and touchpad as macOS input | System Settings → Privacy & Security → Accessibility |

The app does not connect to the laptop until Screen Recording is granted, so no display is
created that cannot be captured. Without Accessibility the picture and sound still work; only
the laptop's keyboard and touchpad are ignored by macOS. macOS usually wants the app restarted
after Screen Recording is granted.

## Pairing

Both machines hold the same secret key file (at least 32 bytes; random):

- Mac: `~/.config/subscreen/key`, readable by the owner only.
- Laptop: the file given to `subscreen-rx --key` (default `/root/subscreen.key`).

**Pairing key… → Generate new key…** writes 32 fresh random bytes on the Mac. Copy that file to
the laptop over a channel you trust; until both sides match, each will refuse the other. The app
never displays or logs the key, and the key file must never be committed anywhere
(`.gitignore` excludes the usual names as a safety net).

## Settings

Stored in the app's defaults domain (`org.shabnam.display`). Two values have no UI:

```sh
defaults write org.shabnam.display host 169.254.1.1   # the laptop's address on the cable
defaults write org.shabnam.display port 7000          # video/input port; sound is port + 1
```

## Private API caveat

macOS has no public API for creating a display in software. This app uses
`CGVirtualDisplay`, a set of CoreGraphics classes that ship in the system but are **not
documented or declared in the SDK**; `Sources/CVirtualDisplay/include/CGVirtualDisplay.h` is a
reconstruction of their interface, as commonly used by other virtual-display tools.

What that means:

- **Mac App Store:** not eligible. App Review rejects private API use, and the app also needs
  to run unsandboxed to post input events.
- **Developer ID / notarisation:** notarisation is a malware scan, not an API review, and other
  apps built on the same classes are distributed this way — but that has **not been tested for
  this project**, and nothing guarantees it stays possible.
- **Stability:** Apple can change or remove these classes in any macOS update without notice.
  If that happens the app reports `Error: could not create the display`.

## Target hardware

Developed for the Shabnam laptop: a 14-inch 1920 × 1080 notebook built on the Rockchip RK3568,
running Linux, with the receiver decoding H.264 on the SoC's hardware decoder (Rockchip MPP) and
scanning frames out directly through DRM/KMS. The Mac side makes few assumptions beyond the
resolution, the cable addressing and the protocol, so other receivers that speak
[the protocol](docs/PROTOCOL.md) should work.

## Project layout

```
Package.swift
Makefile, scripts/bundle.sh        build and bundle
scripts/make-icon.swift            regenerates Resources/AppIcon.icns from Resources/mark.png
Resources/                         Info.plist, icon, en / zh-Hans strings
Sources/CVirtualDisplay/           header for the undocumented CoreGraphics classes
Sources/ShabnamDisplay/
  App/        main, AppDelegate, status item and menu, session state machine, windows
  Link/       cable interface lookup, pairing key file, handshake, authenticated socket
  Display/    virtual display
  Media/      screen capture, H.264 encoder + Annex-B framing, sound
  Input/      Linux → macOS key tables, event injection
  Support/    settings, permissions, login item, localisation, logging
Tests/ShabnamDisplayTests/         handshake against a reference receiver; record parsing
docs/PROTOCOL.md                   wire format
```

## Security notes

- Mutual authentication precedes everything; a peer without the key receives no screen data
  and cannot inject input.
- The link is **not encrypted**. It is designed for a direct cable between two machines and the
  sockets are bound to that interface; do not route it over a shared network.
- A paired laptop can type and click on the Mac — that is the feature. Treat the key file like
  a password.

## Licence

MIT — see [LICENSE](LICENSE).
