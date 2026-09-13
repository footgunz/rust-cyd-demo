# CLAUDE.md

Guidance for AI agents (and humans) working in this repository.

## What this is

`no_std` Rust firmware for the **ESP32-2432S028R** ("Cheap Yellow Display"), a
menu-driven kitchen-sink demo built on `esp-hal` 1.x. One binary,
`src/bin/main.rs`, brings up the hardware and dispatches to one screen at a time
from a menu. See `README.md` for the user-facing tour.

## Toolchain — this is the part that trips people up

The ESP32 is an **Xtensa** target that mainline Rust does not support. It needs
Espressif's forked toolchain, installed with `espup`, and its environment must
be sourced in **every shell** before any cargo command:

```sh
. ~/export-esp.sh
```

Without it you get cryptic `libclang` or linker errors. The pinned toolchain is
`channel = "esp"` (see `rust-toolchain.toml`); the target is
`xtensa-esp32-none-elf`, and `build-std = ["alloc", "core"]` (Wi-Fi needs
`alloc`) — both set in `.cargo/config.toml`, do not remove them.

## Build, flash, verify

```sh
cargo build --release
espflash flash --chip esp32 --port /dev/ttyUSB0 --baud 115200 \
    target/xtensa-esp32-none-elf/release/cyd-rust
```

- Flash at **115200** and use the **default stub**. `--no-stub` fails on this
  board with an error on the `FlashEnd` command.
- The `--port` above is the Linux name. On macOS it is `/dev/cu.usbserial-XXXX`
  or `/dev/cu.wchusbserial*` — discover it with `ls /dev/cu.*`. Setting
  `export ESPFLASH_PORT=<device>` once avoids editing the command and stops
  `espflash` prompting interactively (which blocks a non-interactive agent).
- To read the serial log, add `--monitor`, or connect at 115200. Only **one**
  process may hold the port — a stray monitor makes `espflash` panic with a
  slice-index error. Find it with `lsof <device>` (`fuser` on Linux).
- There is no host-side test suite: this is embedded firmware. "Verify" means
  flashing to hardware and reading the serial log or the screen. Do not claim a
  change works without doing so, and say plainly when you could not.

## Architecture

- `src/lib.rs` — the crate library; pin map plus `pub mod` for every module.
- `src/ui.rs` — shared chrome. Use `ui::header`, `ui::button`, the colour
  constants, and `ui::body_style_opaque` for text that redraws without a flash.
  New screens should reuse these, not reinvent layout.
- `src/menu.rs` — `Choice` enum and the `ITEMS` table. **Its array length is
  hard-coded** (`[(Choice, &str, &str); N]`); bump `N` when adding an item or it
  will not compile.
- `src/apps/*` — one module per screen, each a `run(...)` that loops until the
  user presses BACK, then returns to the menu.
- Peripherals that need concrete `esp-hal` types (ADC, LEDs) are passed to a
  screen as a **closure** from `main`, keeping app modules free of the type
  plumbing.

### Adding a screen

1. New module in `src/apps/`, `pub mod` it in `src/apps/mod.rs`.
2. Add a `Choice` variant and an `ITEMS` row in `src/menu.rs`; bump the array
   length.
3. Dispatch it in the `match` in `main.rs`.

## Radio notes (hard-won; do not regress)

- Wi-Fi/BLE need a heap (`esp-alloc`) and a scheduler (`esp-rtos`), started in
  `main` before `esp_radio::wifi::new`. `set_config` is what actually starts the
  Wi-Fi driver.
- **BLE** is driven over raw HCI in `src/ble.rs`. After `Reset` you must send
  **Set Event Mask and LE Set Event Mask** before enabling scan, or no
  advertising reports ever arrive even though every command returns success.
  This was a real bug; keep the masks.
- Apple manufacturer type `0x12` is emitted by all Find My devices (phones,
  Macs), not just tags. The payload **length** `0x19` distinguishes a real
  separated tag. Do not label every `0x12` an AirTag.
- **FLOCK DETECT** uses promiscuous mode; the callback is a bare `fn`, so it
  reaches state through the `critical_section`-guarded static in `src/flock.rs`.
  Do the minimum in the callback; draw from a snapshot in the app.

## Hardware quirks

Verified empirically, not from the datasheet. See `README.md` for the pinout;
the load-bearing ones for code:

- Panel is **BGR** and horizontally **mirrored** — needs `ColorOrder::Bgr` and
  `Orientation::new().flip_horizontal()`.
- Touch axes are **transposed** vs the display — three-point calibration.
- Board revisions vary in what's populated (LDR, RGB channels), and individual
  units have defects. Prove a peripheral works on the actual board before
  building a feature on it — don't trust the pinout alone. Diagnostic screens
  (TOUCH DIAG, ANALOG) exist for exactly this.

## Conventions

- Match the surrounding style: comments explain *why*, not *what*. Several
  comments encode a measured fact or a debugging conclusion — preserve that
  reasoning when editing nearby code.
- Keep changes scoped. This repo values working-on-hardware over theoretical
  correctness; if you cannot flash and check, say so.

## AGENTS.md

`AGENTS.md` is a symlink to this file. Edit `CLAUDE.md`; the symlink follows.
