# CYD Kitchen Sink

A menu-driven tour of the **ESP32-2432S028R** — the "Cheap Yellow Display"
(CYD) — written in `no_std` Rust on [`esp-hal`](https://github.com/esp-rs/esp-hal).

One firmware, eight screens: pressure-sensitive drawing, Wi-Fi and BLE
scanners, a passive ALPR-camera detector, and hardware diagnostics. It drives
both of the board's SPI buses at once (ILI9341 panel on SPI2, XPT2046 touch on
SPI3) alongside the ADC, the RGB LED, and the Wi-Fi/BLE radio.

Every hardware fact in here was confirmed on a physical board, not taken from a
datasheet — see [Hardware notes](#hardware-notes), including the ones this
particular unit gets wrong.

## Screens

| Screen | What it does |
| --- | --- |
| **PAINT** | Pressure-sensitive drawing. Eight colours (one cycles hue), pressure sets stroke width. |
| **WIFI SCAN** | Nearby access points, sorted by signal, with a per-network signal bar. |
| **BLE SCAN** | BLE beacons and item trackers. Flags AirTags / Find My tags, Tile, Galaxy SmartTag and Fast Pair; a presence column tracks devices dropping in and out. |
| **FLOCK DETECT** | Passive detector for Flock Safety ALPR cameras — promiscuous sniff of known transmitter OUIs. Receives only. |
| **TOUCH DIAG** | Live raw touch values and the mapped pixel, with a dot tracking your finger. |
| **ANALOG** | The two free ADC inputs, with observed min/max ranges. |
| **LEDS** | Toggle each RGB channel. |
| **CALIBRATE** | Redo touch calibration. |

## Build and flash

The ESP32 is an Xtensa part, which mainline Rust does not target, so this needs
Espressif's forked toolchain via [`espup`](https://github.com/esp-rs/espup).
Install it once, then source its environment in **every** shell:

```sh
espup install                 # one time: installs the 'esp' Rust toolchain
. ~/export-esp.sh             # every shell: sets LIBCLANG_PATH and the GCC path
```

Then build and flash over USB (`espflash` installs with `cargo install espflash`
or as a prebuilt binary):

```sh
cargo build --release
espflash flash --chip esp32 --port /dev/ttyUSB0 --baud 115200 \
    target/xtensa-esp32-none-elf/release/cyd-rust
```

Add `--monitor` to watch the serial log. `cargo run --release` does both in one
step via the runner configured in `.cargo/config.toml`.

## Layout

```
src/lib.rs          board pin map, all hardware-verified
src/ui.rs           shared chrome: header, buttons, colours, layout
src/menu.rs         the main menu and dispatch enum
src/calib.rs        three-point touch calibration
src/touch.rs        XPT2046 driver: median filter, pressure gate, calibration
src/ble.rs          passive BLE scanner over raw HCI, with tracker signatures
src/flock.rs        Flock OUI table and the promiscuous-mode detection callback
src/apps/           one module per screen
src/bin/main.rs     brings up the hardware and runs the menu loop
```

Screens are dispatched from `menu::Choice`. Adding one is three steps: a module
under `src/apps/`, a `Choice` variant, and a row in `menu::ITEMS`. Hardware that
needs concrete `esp-hal` types (the ADC, the LEDs) is passed to a screen as a
closure, so the app modules stay free of peripheral plumbing.

## Hardware notes

All measured on the board.

**Display (SPI2 / HSPI).** sclk 14, mosi 13, miso 12, cs 15, dc 2, backlight 21.
The panel is wired **BGR** (`ColorOrder::Rgb` swaps red and blue), and its
MADCTL MX bit is inverted versus `mipidsi`'s default, so
`Orientation::new().flip_horizontal()` is required or the image comes out
mirrored left-to-right.

**Touch (SPI3 / VSPI).** sclk 25, mosi 32, miso 39, cs 33, PENIRQ 36. This is a
**separate bus** from the panel — the detail that breaks most shared-bus
examples written for TFT_eSPI. GPIO 36 and 39 are input-only with no internal
pull resistors; the board supplies its own pull-up on PENIRQ. The touch axes are
**transposed** relative to the display, so calibration takes three points rather
than two — two diagonal points cannot separate an axis swap from an inversion.

**RGB LED** (common anode, drive low): red 4, green 16, blue 17. On the
reference unit the **red channel is dead** — driving GPIO 4 alone produces no
light. Cosmetic; the code says so on the LEDS screen rather than hiding it.

**No light sensor.** GPIO 34 is the documented LDR pin, but on the reference unit
it sits at a fixed ~1.9 V and does not respond to light (verified by sealing the
board in a box: 0.1 σ of movement). Likely an unpopulated footprint. GPIO 35 is
unconnected and free for an external sensor.

**Audio.** GPIO 26 is DAC2 and feeds the on-board amp and 2-pin JST speaker
header. (DAC1 / GPIO 25 is used by the touch clock.) Not yet driven by any
screen.

## Flashing over USB/IP in WSL

If you flash through `usbipd` from Windows, the link drops packets on sustained
transfers: reads above ~64 KB fail at any baud rate, so dump flash in 64 KB
chunks. A ~600 KB firmware image writes fine. Use `espflash`'s default stub —
`--no-stub` fails here with an error on the `FlashEnd` command. Anything holding
the serial port open (a monitor, a logging script) makes `espflash` panic with a
confusing slice-index error; check with `fuser /dev/ttyUSB0`.

## Credits and legality

The **FLOCK DETECT** OUI list and detection method come from the
[flock-you](https://github.com/colonelpanichacks/flock-you) project and
@NitekryDPaul's research. The **BLE SCAN** tracker signatures were cross-checked
against [BLE-Hound](https://github.com/GH0ST3CH/BLE-Hound).

The radio screens are **passive**: they listen, they never transmit, associate,
or connect. They are for research and education. You are responsible for using
them lawfully. Flock hardware is deployed mainly in the US and Canada, so the
OUI list will not match anything elsewhere.

## License

MIT — see [LICENSE](LICENSE).
