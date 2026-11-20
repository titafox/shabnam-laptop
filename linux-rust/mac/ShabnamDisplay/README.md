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

## Menu and settings

The menu-bar icon is a droplet: faint outline = switched off, outline = waiting for the laptop,
filled = connected, outline with a mark = something needs attention.

The menu is short on purpose:

| Item | What it does |
|------|--------------|
| Status line | `Waiting for laptop`, `Connected · 1080p · 60 fps · 42 Mbit/s`, or `Error: …` |
| Connect / Disconnect | Stop or resume looking for the laptop, for this run |
| Laptop battery, brightness, volume, mute | The laptop's own state and controls, e.g. `Shabnam 82% · charging` — shown while connected to a laptop whose receiver offers the control channel (subscreen-rx 0.2+) |
| Return Shabnam to its own desktop | Ends second-screen mode on the laptop, as its Win+Esc does |
| Sound | Where the Mac's sound is heard — see [Sound modes](#sound-modes) |
| Resolution | How large the desktop appears on the laptop |
| Settings… (⌘,) | Everything else |

**Settings** has seven panes; changes apply at once, without dropping the link, and are stored
in the app's defaults domain.

| Pane | Contents |
|------|----------|
| General | Launch at login · connect automatically · icon only or icon and bitrate in the menu bar · notifications on connect/disconnect (off by default) |
| Display | Shabnam laptop: battery, brightness, volume, mute, return to its own desktop (greyed out without the control channel) · Resolution: 1920 × 1080 HiDPI (default), 1920 × 1080, 1600 × 900 HiDPI, 1280 × 720 HiDPI · frame rate cap 60/30 · picture quality · show the Mac's pointer · Arrange Displays… |
| Sound | The three [sound modes](#sound-modes) |
| Input | Accept the laptop's keyboard and touchpad (off: events are read and discarded) · Win/Alt mapping · media keys · pointer speed · scroll speed · reverse scroll direction |
| Connection | Laptop address and port · the cable interface in use · pairing key (reveal, import, generate) · live statistics · Copy Diagnostics |
| Permissions | Status of each permission and buttons to the right pane of System Settings |
| About | Version, licence, project page |

Picture quality presets (the laptop's panel is always fed 1920 × 1080):

| Preset | Average bitrate | Max QP |
|--------|----------------:|-------:|
| Sharpest (default) | 120 Mbit/s | 18 |
| Balanced | 60 Mbit/s | 24 |
| Economy | 25 Mbit/s | 30 |

Modifier keys: Win → Command, Alt → Option, Ctrl → Control by default; Win and Alt can be
swapped so that the keys sit in the order of a Mac keyboard.

**Copy Diagnostics** puts a short text block on the clipboard — version, state, statistics,
settings, permission status. It contains no key material, user name, computer name or home
folder path. A log is kept in `~/Library/Logs/ShabnamDisplay/` (about 1 MB per file, three old
files); it never contains the key either.

The interface is in English and Simplified Chinese.

### Behaviour worth knowing

- Only one copy runs: starting the app a second time brings up the first one's Settings window.
- Before the Mac sleeps the session is ended cleanly; after waking the app looks for the laptop
  again.
- On Quit, and when the app is told to terminate (SIGTERM), the Mac's own sound is restored
  first; the extra screen belongs to the process and disappears with it.

## Requirements

- macOS 14 or later (developed on Apple silicon). The "Shabnam only" sound mode needs macOS 14.2.
- Swift 5.9+ toolchain (Xcode or the Command Line Tools). No Xcode project and no third-party
  dependencies.
- A Shabnam laptop running `subscreen-rx`, and a USB-C cable.

## Build

```sh
make app          # release build + "build/Shabnam Display.app" (signed, see below)
make run          # build and launch
make install      # copy the app to ~/Applications
swift test        # handshake, control channel codec and state, settings, input mapping, presets, diagnostics
make check        # tests + every UI string present in both languages
```

`make app` also builds `audio-tap-probe`, a small diagnostic for the "Shabnam only" sound mode
(see below); it is not part of the app bundle.

or by hand: `swift build -c release`, then `scripts/bundle.sh`. The version shown in
Settings → About comes from the `VERSION` file; the build number is `BUILD_NUMBER` or the
number of commits.

Developer switches of the app binary:

```sh
APP="build/Shabnam Display.app/Contents/MacOS/ShabnamDisplay"
"$APP" --ui-preview display      # only the Settings window (general, display, audio, input,
                                 # connection, permissions, about): connects to nothing, creates
                                 # no display, leaves a running copy alone, and keeps its settings
                                 # in a scratch domain
"$APP" --ui-preview display --fake-laptop   # the same, with made-up figures in the laptop's controls
"$APP" --probe-display-modes     # create a short-lived test display and report which of the
                                 # resolution choices macOS accepts
```

The bare binary (`swift run`) works too, but macOS then attributes the permissions to the
terminal it was started from.

### Signing and permissions

macOS ties privacy permissions to the code signature. An ad-hoc signature changes with every
build, so with ad-hoc signing **macOS asks for the permissions again after each rebuild**
(remove the stale entry in System Settings and add the app again). Signing with a stable
identity avoids that: the permissions then survive rebuilds.

`scripts/bundle.sh` chooses the identity in this order:

1. the `CODESIGN_IDENTITY` environment variable;
2. the first line of `.codesign-identity` in the project root — a machine-local file that is
   git-ignored, because identity names usually contain a person's name;
3. ad hoc.

```sh
security find-identity -v -p codesigning            # what your keychain offers
echo "<SHA-1 hash or name from that list>" > .codesign-identity
```

No identity yet? Any code-signing certificate will do, including a self-signed one: open
**Keychain Access → Certificate Assistant → Create a Certificate…**, choose identity type
*Self-Signed Root* and certificate type *Code Signing*, then use its name as above. (A
self-signed signature keeps permissions stable on your own Mac; it does not make the app
trusted on anyone else's.)

Switching from one identity to another (including from ad hoc) counts as a new app once: grant
the permissions one more time and they stay.

## Permissions

| Permission | Why | Where |
|------------|-----|-------|
| Screen Recording | Capture the virtual display and the system sound | System Settings → Privacy & Security → Screen & System Audio Recording |
| Accessibility | Replay the laptop's keyboard and touchpad as macOS input | System Settings → Privacy & Security → Accessibility |
| System Audio Recording | Only for the "Shabnam only" sound mode: take the Mac's sound away from its own speakers | same pane as Screen Recording ("System Audio Recording Only" list); macOS may ask the first time the mode is used |

The app does not connect to the laptop until Screen Recording is granted, so no display is
created that cannot be captured. Without Accessibility the picture and sound still work; only
the laptop's keyboard and touchpad are ignored by macOS. macOS usually wants the app restarted
after Screen Recording is granted. If System Audio Recording is refused, the "Shabnam only" mode
sends silence; choose another mode or grant it.

### "The switch is on but the app still says the permission is missing"

This happens once when the app's signature changes (for example from ad hoc to a certificate):
macOS keeps the old record for the bundle id, shows its switch as on, and still refuses the new
build. Quit the app, clear its records, start it again and grant the permissions afresh:

```sh
tccutil reset ScreenCapture org.shabnam.display
tccutil reset Accessibility org.shabnam.display
tccutil reset AudioCapture  org.shabnam.display
```

## Sound modes

| Mode | Heard on | How |
|------|----------|-----|
| Sound: Mac and Shabnam | both | ScreenCaptureKit listens in on the system sound; the Mac keeps playing |
| Sound: Shabnam only (Mac muted) | the laptop | a Core Audio *process tap* takes the sound of every app and mutes it on the Mac while it is being read |
| Sound: off | the Mac | nothing is sent |

Switching takes effect at once and does not interrupt the picture. On the wire all modes are
the same 48 kHz 16-bit stereo stream.

About "Shabnam only":

- No driver and no administrator password: it uses the public tap API of macOS 14.2+
  (`CATapDescription`, `AudioHardwareCreateProcessTap`, a private aggregate device).
- The tap sits **before** the Mac's volume control. The Mac's volume keys and mute do not
  change what the laptop receives; set the loudness with the laptop's own volume keys.
- The Mac is only muted while sound is really reaching the laptop. The tap starts after the
  sound connection is up and stops the moment the link drops, the mode is changed, or the app
  quits or is terminated — the Mac's own sound returns immediately.
- If the app is killed outright, the muting ends with the process (macOS mutes only while the
  tap is being read). The tap object itself can linger in the audio server; the app removes
  such leftovers at its next start.
- When the Mac's output device changes (headphones, another device), the tap is rebuilt for
  the new device.
- If the tap cannot be created, the session falls back to "Mac and Shabnam" and says so in the
  log.

`audio-tap-probe` exercises this without the laptop:

```sh
swift build -c release
.build/release/audio-tap-probe            # plays a system sound, prints the captured peak level
.build/release/audio-tap-probe --volume   # same at low Mac volume and muted (settings restored)
.build/release/audio-tap-probe --listen   # 6 s of tap: play music, the Mac should fall silent
```

Do not run the probe while the app itself is using the "Shabnam only" mode: it clears taps
carrying the app's name, which would interrupt the app's sound until its next reconnect.

Run from a terminal it uses the terminal's permissions, not the app's.

## Pairing

Both machines hold the same secret key file (at least 32 bytes; random):

- Mac: `~/.config/subscreen/key`, readable by the owner only.
- Laptop: the file given to `subscreen-rx --key` (default `/root/subscreen.key`).

**Settings → Connection → Generate new key…** writes 32 fresh random bytes on the Mac; **Import
from File…** installs a key file you already have (for example the laptop's). Copy the file
between the machines over a channel you trust; until both sides match, each will refuse the
other. The app
never displays or logs the key, and the key file must never be committed anywhere
(`.gitignore` excludes the usual names as a safety net).

## Settings

All settings are stored in the app's defaults domain (`org.shabnam.display`) and can be
inspected with `defaults read org.shabnam.display`. The laptop's address and port (default
`169.254.1.1`, `7000`; sound is port + 1, control port + 2) are under Settings → Connection.

The project's web address for the About pane is a single constant, `ProjectInfo.homepage` in
`Sources/ShabnamDisplay/Support/ProjectInfo.swift` (unset by default).

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

The resolution choices were checked on macOS 27.2 with `--probe-display-modes`: all four are
accepted (1920 × 1080 at 3840 × 2160 and at 1920 × 1080 pixels, 1600 × 900 at 3200 × 1800,
1280 × 720 at 2560 × 1440).

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
VERSION                            the app's version number
scripts/check-strings.py           checks the string tables against the sources
scripts/make-icon.swift            regenerates Resources/AppIcon.icns from Resources/mark.png
Resources/                         Info.plist, icon, en / zh-Hans strings
Sources/CVirtualDisplay/           header for the undocumented CoreGraphics classes
Sources/ShabnamAudio/              system audio tap ("Shabnam only" sound mode)
Sources/audio-tap-probe/           diagnostic for the tap
Sources/ShabnamDisplay/
  App/        main, AppDelegate, status item and menu, session state machine and statistics,
              the laptop's controls (LaptopControl), Settings window (SettingsUI/ holds one
              SwiftUI view per pane)
  Link/       cable interface lookup, pairing key file, handshake, authenticated socket,
              control channel wire format
  Display/    virtual display
  Media/      screen capture, H.264 encoder + Annex-B framing, sound sender and sound modes
  Input/      Linux → macOS key tables, event injection
  Support/    settings, permissions, login item, notifications, diagnostics, localisation,
              logging, project facts
Tests/ShabnamDisplayTests/         handshake against a reference receiver; control channel
                                   messages and UI state; settings and
                                   migration; modifier mapping; presets; diagnostics; log rotation
docs/PROTOCOL.md                   wire format
```

## Security notes

- Mutual authentication precedes everything; a peer without the key receives no screen data
  and cannot inject input.
- The link is **not encrypted**. It is designed for a direct cable between two machines and the
  sockets are bound to that interface; do not route it over a shared network.
- A paired laptop can type and click on the Mac — that is the feature. Treat the key file like
  a password.
- The control channel uses the same handshake and the same cable-only sockets. What the Mac can
  make the laptop do is a fixed list — backlight, volume, mute, leave second-screen mode — each
  carrying at most one clamped number.

## Licence

MIT — see [LICENSE](LICENSE).
