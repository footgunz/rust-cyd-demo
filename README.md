# CYD Touch Lab

A pressure-sensitive drawing demo for the **ESP32-2432S028R**, better known as
the "Cheap Yellow Display" (CYD), written in `no_std` Rust on `esp-hal`.

It drives both of the board's SPI buses at once: the ILI9341 panel on SPI2 and
the XPT2046 touch controller on SPI3. Boots into a three-point touch
calibration, then a paint surface where stroke width follows how hard you
press.

## Build and flash

The ESP32 is an Xtensa part, which mainline Rust does not target, so this needs
Espressif's forked toolchain via [`espup`](https://github.com/esp-rs/espup).
Every shell needs the environment first:

```sh
. ~/export-esp.sh
cargo build --release
espflash flash --chip esp32 --port /dev/ttyUSB0 --baud 115200 \
    target/xtensa-esp32-none-elf/release/cyd-rust
```

Pass `--monitor` to espflash to watch the serial output.

## Layout

| Path | Contents |
| --- | --- |
| `src/lib.rs` | Pin map for the board, all values confirmed on hardware |
| `src/touch.rs` | XPT2046 driver: median filter, pressure gate, calibration |
| `src/bin/main.rs` | The Touch Lab demo |
| `src/bin/touch_probe.rs` | Diagnostic dumping raw touch channels and PENIRQ |

When the hardware misbehaves, reach for the probe rather than guessing:

```sh
cargo build --release --bin touch-probe
espflash flash --chip esp32 --port /dev/ttyUSB0 --baud 115200 \
    target/xtensa-esp32-none-elf/release/touch-probe
```

## Using the demo

Calibration asks for three presses. Press and **hold** each target until its
ring turns green; the ring goes amber on contact and fills as the hold
progresses.

Then:

- **Eight swatches** — seven solid colours plus a striped slot that cycles hue
  as you draw.
- **CLEAR** wipes the canvas, **RECAL** re-runs calibration without reflashing.
- The status line shows live position, pressure, stroke width and stroke count.
- Press harder for a wider stroke, up to 11px.

## Hardware notes

Everything here was measured on the board, not taken from a datasheet.

**Display (SPI2/HSPI).** sclk 14, mosi 13, miso 12, cs 15, dc 2, backlight 21.
The panel is wired **BGR** — `ColorOrder::Rgb` renders red and blue swapped.
Its MADCTL MX bit is inverted relative to `mipidsi`'s default, so
`Orientation::new().flip_horizontal()` is required or the image comes out
mirrored left-to-right.

**Touch (SPI3/VSPI).** sclk 25, mosi 32, miso 39, cs 33, PENIRQ 36. This is a
*separate* bus from the panel, which is what breaks most shared-bus examples
written for TFT_eSPI. GPIO 36 and 39 are input-only and have no internal pull
resistors; the board supplies its own pull-up on PENIRQ, which measures clean.

Measured: Z1 idles at 2 and reads ~720 under a fingertip, so the driver gates
on `z1 > 80` *in addition to* PENIRQ. PENIRQ alone is not sufficient — it
deasserts briefly while the Z channels are read.

Calibration uses three points rather than two. Two diagonal points cannot
distinguish an axis swap from an axis inversion, and on this panel the touch
axes are **transposed** relative to the display. One pure-X and one pure-Y move
resolve swap, inversion and scale together. Averaging samples across the hold
gives repeatability of 1-2px, at roughly 15 ADC counts per pixel.

**On-board RGB LED** (common anode, drive low): red 4, green 16, blue 17.
Note that the **red channel is dead on this particular unit** — driving GPIO 4
alone, with 16 and 17 held off, produces no light. Cosmetic only.

## Flashing over USB/IP in WSL

The link drops packets on sustained transfers. Reads above ~64KB fail at any
baud rate, so dump flash in 64KB chunks; a ~100KB firmware image writes fine.
Use espflash's default stub — `--no-stub` fails here with an error on the
`FlashEnd` command.

Anything holding the serial port open (a monitor, a logging script) will make
espflash panic with a confusing slice-index error. Check with
`fuser /dev/ttyUSB0`.
