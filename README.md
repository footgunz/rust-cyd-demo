# CYD Kitchen Sink

A menu-driven tour of the **ESP32-2432S028R** — the "Cheap Yellow Display"
(CYD) — written in `no_std` Rust on [`esp-hal`](https://github.com/esp-rs/esp-hal).

One firmware, eight screens: pressure-sensitive drawing, Wi-Fi and BLE
scanners, a passive ALPR-camera detector, and hardware diagnostics. It drives
both of the board's SPI buses at once (ILI9341 panel on SPI2, XPT2046 touch on
SPI3) alongside the ADC, the RGB LED, and the Wi-Fi/BLE radio.

Every hardware fact in here was confirmed on a physical board, not taken from a
datasheet — see [Hardware notes](#hardware-notes).

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

## Toolchain

The original ESP32 is an **Xtensa LX6** core. Mainline Rust does not target
Xtensa — support lives in Espressif's LLVM fork — so you cannot build this with
a stock `rustup` toolchain, and `rustup target add` will not help. (This is
specific to Xtensa; the newer RISC-V ESP32s — C3, C6, H2 — do build on upstream
Rust. The ESP32-S2/S3 are also Xtensa and need the same fork.)

The fork is installed by [`espup`](https://github.com/esp-rs/espup), which
provides an `esp` toolchain visible to `rustup`:

```sh
cargo install espup           # or grab a prebuilt binary from espup's releases
espup install                 # installs the 'esp' Rust toolchain + Xtensa LLVM + GCC
```

`espup install` downloads a few things (~1–2 GB): the Xtensa-enabled `rustc`,
the matching `rust-src`, Espressif's LLVM/clang, and the `xtensa-esp-elf` GCC
that provides the linker. It writes `~/export-esp.sh`, which sets `LIBCLANG_PATH`
and puts the GCC on `PATH`. **Source it in every shell before building:**

```sh
. ~/export-esp.sh
```

Skipping it produces cryptic `libclang`-not-found or `undefined reference`
linker errors rather than a clear message. `rust-toolchain.toml` pins
`channel = "esp"`, so cargo selects the right toolchain automatically once it is
installed; you do not pass `+esp` by hand.

The build target and `build-std` are set in `.cargo/config.toml` — target
`xtensa-esp32-none-elf`, and `build-std = ["alloc", "core"]` because the core is
`no_std` but the Wi-Fi/BLE stack needs an allocator. The standard library is not
available on this target; this is `no_std` throughout.

## Build and flash

Flashing uses [`espflash`](https://github.com/esp-rs/espflash) over USB serial
(`cargo install espflash`, or a prebuilt binary):

```sh
cargo build --release
espflash flash --chip esp32 --port /dev/ttyUSB0 --baud 115200 \
    target/xtensa-esp32-none-elf/release/cyd-rust
```

- Adjust `--port` to your serial device (`/dev/ttyUSB0`, `/dev/ttyACM0`,
  `COMx`). The board's CH340 USB-UART enumerates without extra drivers on modern
  systems.
- Flash at **115200** with the **default flash stub**. `--no-stub` fails on this
  board with an error on the `FlashEnd` command.
- Add `--monitor` to open the serial log after flashing (115200 8N1). Only one
  process may hold the port at a time.

`cargo run --release` builds, flashes and monitors in one step, via the runner
in `.cargo/config.toml`.

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

**RGB LED** (common anode, drive low): red 4, green 16, blue 17.

**Analog.** GPIO 34 is the documented LDR (light sensor) pin; GPIO 35 is the
other free ADC1 input. Both are input-only. Note that LDR population varies
between board revisions, so treat the light reading as present-if-populated.

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
