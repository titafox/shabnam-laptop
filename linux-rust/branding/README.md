Shabnam brand assets for the laptop system, derived from the ShabnamTV materials in
`~/Projects/shabnamtv/branding/` (same droplet mark, same colours; the word "TV" removed).

- `mark.png`            the droplet, transparent background
- `wordmark-white.png`  "Shabnam" for dark backgrounds; `wordmark-navy.png` for light ones
- `wallpaper.jpg`       desktop wallpaper, 1920x1080
- `splash.png`          start-up screen; `splash.bgra` is the same picture as raw pixels for
                        the start-up program (regenerate: `magick splash.png -depth 8 bgra:splash.bgra`)
- `bootlogo.png`        the picture shown from power-on until the system takes over the screen. It lives
                        inside the boot partition (two 1920x1080 24-bit pictures, `logo.bmp` and
                        `logo_kernel.bmp`). Original partition: `../../backup-boot-20261120/boot.img`,
                        also on the laptop at `/data/void-boot/boot-orig.img`.
