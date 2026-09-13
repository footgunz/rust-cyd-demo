//! CYD Touch Lab — a pressure-sensitive drawing demo for the
//! ESP32-2432S028R, in no_std Rust on esp-hal.
//!
//! Exercises both SPI buses at once: the ILI9341 panel on SPI2 and the
//! XPT2046 touch controller on SPI3. Boots into a hold-to-confirm
//! calibration, then a paint surface where stroke width follows pressure.

#![no_std]
#![no_main]
#![deny(clippy::mem_forget, reason = "esp_hal types may hold DMA buffers")]

use core::fmt::Write as _;

use cyd_rust::touch::{Calibration, Xpt2046};
use embedded_graphics::draw_target::DrawTargetExt;
use embedded_graphics::mono_font::{ascii::FONT_6X10, ascii::FONT_9X15_BOLD, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use embedded_hal::digital::InputPin;
use embedded_hal::spi::SpiDevice;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use esp_println::println;
use heapless::String;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ILI9341Rgb565;
use mipidsi::options::{ColorOrder, Orientation};
use mipidsi::Builder;

// Required by the ESP-IDF bootloader that ships on this board.
esp_bootloader_esp_idf::esp_app_desc!();

const W: i32 = 240;
const H: i32 = 320;

const HEADER_H: i32 = 34;
const PALETTE_Y: i32 = HEADER_H;
const PALETTE_H: i32 = 30;
const STATUS_Y: i32 = PALETTE_Y + PALETTE_H;
const STATUS_H: i32 = 18;
const CANVAS_Y: i32 = STATUS_Y + STATUS_H;

const SWATCHES: usize = 8;
const SWATCH_W: i32 = W / SWATCHES as i32;

/// Slack added around a button when hit-testing. Drawn size is unchanged, so
/// controls stay visually tidy while being far easier to land on.
const HIT_SLACK: i32 = 7;

/// Valid samples required to accept a calibration point. At roughly one per
/// 10ms this is about a quarter-second of contact — enough to average away
/// the panel's noise, short enough not to feel like a wait.
const HOLD_SAMPLES: u32 = 24;

/// Consecutive misses tolerated mid-hold before the attempt is abandoned.
///
/// A light press hovers near the driver's pressure floor, and PENIRQ briefly
/// deasserts while the Z channels are read, so isolated misses are normal and
/// must not throw away an otherwise good press.
const HOLD_MISS_LIMIT: u32 = 12;

const PALETTE: [Rgb565; SWATCHES - 1] = [
    Rgb565::new(31, 0, 0),
    Rgb565::new(31, 30, 0),
    Rgb565::new(28, 63, 0),
    Rgb565::new(0, 63, 12),
    Rgb565::new(0, 50, 31),
    Rgb565::new(8, 16, 31),
    Rgb565::new(31, 0, 24),
];
const RAINBOW: [Rgb565; 12] = [
    Rgb565::new(31, 0, 0),
    Rgb565::new(31, 16, 0),
    Rgb565::new(31, 32, 0),
    Rgb565::new(31, 55, 0),
    Rgb565::new(16, 63, 0),
    Rgb565::new(0, 63, 8),
    Rgb565::new(0, 60, 20),
    Rgb565::new(0, 45, 31),
    Rgb565::new(0, 20, 31),
    Rgb565::new(12, 8, 31),
    Rgb565::new(24, 0, 31),
    Rgb565::new(31, 0, 16),
];

const BG: Rgb565 = Rgb565::new(2, 4, 6);
const INK: Rgb565 = Rgb565::new(26, 53, 28);
const DIM: Rgb565 = Rgb565::new(14, 28, 16);
const HOT: Rgb565 = Rgb565::new(31, 40, 0);
/// Ring colour once a hold is accepted — the state the on-screen text names.
const DONE: Rgb565 = Rgb565::new(0, 63, 14);
const CANVAS_BG: Rgb565 = Rgb565::BLACK;

fn clear_button() -> Rectangle {
    Rectangle::new(Point::new(112, 5), Size::new(60, 24))
}
fn recal_button() -> Rectangle {
    Rectangle::new(Point::new(176, 5), Size::new(60, 24))
}
fn canvas_rect() -> Rectangle {
    Rectangle::new(
        Point::new(0, CANVAS_Y),
        Size::new(W as u32, (H - CANVAS_Y) as u32),
    )
}

/// Hit-test with slack, so a control is forgiving without looking bloated.
fn hit(r: Rectangle, p: Point) -> bool {
    let tl = r.top_left;
    p.x >= tl.x - HIT_SLACK
        && p.x < tl.x + r.size.width as i32 + HIT_SLACK
        && p.y >= tl.y - HIT_SLACK
        && p.y < tl.y + r.size.height as i32 + HIT_SLACK
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    println!();
    println!("=== CYD Touch Lab ===");

    let out = OutputConfig::default();
    let _backlight = Output::new(peripherals.GPIO21, Level::High, out);

    // --- Panel: ILI9341 on SPI2 (HSPI) ------------------------------------
    let lcd_spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .expect("spi2")
    .with_sck(peripherals.GPIO14)
    .with_mosi(peripherals.GPIO13)
    .with_miso(peripherals.GPIO12);

    let lcd_dev = ExclusiveDevice::new(
        lcd_spi,
        Output::new(peripherals.GPIO15, Level::High, out),
        Delay::new(),
    )
    .expect("lcd device");

    let mut lcd_buf = [0u8; 512];
    let di = SpiInterface::new(
        lcd_dev,
        Output::new(peripherals.GPIO2, Level::Low, out),
        &mut lcd_buf,
    );

    let mut delay = Delay::new();
    let mut display = Builder::new(ILI9341Rgb565, di)
        .display_size(240, 320)
        // Verified on hardware: this panel is BGR, and its MADCTL MX bit is
        // inverted relative to mipidsi's default.
        .color_order(ColorOrder::Bgr)
        .orientation(Orientation::new().flip_horizontal())
        .init(&mut delay)
        .expect("display init");

    // --- Touch: XPT2046 on SPI3 (VSPI) ------------------------------------
    // A separate bus from the panel on this board. 2 MHz: the XPT2046 is
    // specified far below the panel's clock.
    let touch_spi = Spi::new(
        peripherals.SPI3,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(2))
            .with_mode(Mode::_0),
    )
    .expect("spi3")
    .with_sck(peripherals.GPIO25)
    .with_mosi(peripherals.GPIO32)
    .with_miso(peripherals.GPIO39);

    let touch_dev = ExclusiveDevice::new(
        touch_spi,
        Output::new(peripherals.GPIO33, Level::High, out),
        Delay::new(),
    )
    .expect("touch device");

    // GPIO 36 has no internal pull resistor; the board supplies its own
    // pull-up on PENIRQ, which measures clean.
    let irq = Input::new(
        peripherals.GPIO36,
        InputConfig::default().with_pull(Pull::Up),
    );
    let mut touch = Xpt2046::new(touch_dev, irq);

    let mut cal = calibrate(&mut display, &mut touch, &mut delay);
    loop {
        // paint() only returns when the user asks to recalibrate.
        paint(&mut display, &mut touch, &mut delay, cal);
        cal = calibrate(&mut display, &mut touch, &mut delay);
    }
}

/// Three-point calibration.
///
/// Three points rather than two: a pair of diagonal points cannot separate an
/// axis swap from an axis inversion, and this panel's axes do not match the
/// display's. One pure-X move and one pure-Y move resolve swap, inversion and
/// scale together.
fn calibrate<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
) -> Calibration
where
    D: DrawTarget<Color = Rgb565>,
    SPI: SpiDevice,
    IRQ: InputPin,
{
    let _ = display.clear(BG);
    let title = MonoTextStyle::new(&FONT_9X15_BOLD, INK);
    let body = MonoTextStyle::new(&FONT_6X10, INK);
    let loud = MonoTextStyle::new(&FONT_9X15_BOLD, DONE);

    let _ = Text::new("CALIBRATION", Point::new(57, 84), title).draw(display);
    let _ = Text::new("press and HOLD each target", Point::new(33, 116), body).draw(display);
    let _ = Text::new("until its ring turns", Point::new(54, 130), body).draw(display);
    let _ = Text::new("GREEN", Point::new(93, 152), loud).draw(display);

    let targets = [
        (Point::new(26, 210), (26i32, 210i32)),
        (Point::new(W - 26, 210), (W - 26, 210)),
        (Point::new(26, 296), (26, 296)),
    ];

    let mut raws = [(0u16, 0u16); 3];
    for (i, (at, _)) in targets.iter().enumerate() {
        let mut step: String<8> = String::new();
        let _ = write!(step, "{} of 3", i + 1);
        let _ = display.fill_solid(
            &Rectangle::new(Point::new(88, 168), Size::new(64, 12)),
            BG,
        );
        let _ = Text::new(step.as_str(), Point::new(102, 178), body).draw(display);

        draw_marker(display, *at, INK);
        raws[i] = hold_to_confirm(display, touch, delay, *at);
        clear_marker(display, *at);
        delay.delay_millis(200);
    }

    let cal = Calibration::from_points(
        raws[0],
        raws[1],
        raws[2],
        targets[0].1,
        targets[1].1,
        targets[2].1,
    );
    println!(
        "calibrated: raw {:?} {:?} {:?} axes={}",
        raws[0],
        raws[1],
        raws[2],
        if cal.is_swapped() { "swapped" } else { "direct" }
    );
    cal
}

/// Wait for a deliberate, sustained press and return its averaged position.
///
/// Averaging over the hold is what buys precision: a single sample carries the
/// panel's noise straight into the calibration constants.
fn hold_to_confirm<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    at: Point,
) -> (u16, u16)
where
    D: DrawTarget<Color = Rgb565>,
    SPI: SpiDevice,
    IRQ: InputPin,
{
    // Ignore a finger already down from the previous target.
    while touch.is_touched() {
        delay.delay_millis(10);
    }
    delay.delay_millis(150);

    'attempt: loop {
        if touch.sample().is_none() {
            delay.delay_millis(8);
            continue;
        }

        // Let the contact settle before anything is counted; the first
        // readings of a press are the least trustworthy.
        delay.delay_millis(40);
        // Amber the moment contact registers, so the press is acknowledged
        // before the hold has earned anything.
        draw_marker(display, at, HOT);

        let (mut sx, mut sy, mut n, mut misses) = (0u32, 0u32, 0u32, 0u32);
        while n < HOLD_SAMPLES {
            match touch.sample() {
                Some(s) => {
                    sx += s.x as u32;
                    sy += s.y as u32;
                    n += 1;
                    misses = 0;
                    // Disc grows with progress: the hold is visibly going
                    // somewhere rather than just being amber.
                    let grown = 4 + (n * 12 / HOLD_SAMPLES);
                    let _ = Circle::with_center(at, grown)
                        .into_styled(PrimitiveStyle::with_fill(HOT))
                        .draw(display);
                }
                None => {
                    misses += 1;
                    if misses > HOLD_MISS_LIMIT {
                        // A real lift: reset and wait for a fresh press.
                        clear_marker(display, at);
                        draw_marker(display, at, INK);
                        continue 'attempt;
                    }
                }
            }
            delay.delay_millis(10);
        }

        // Green confirms acceptance — the exact cue the instructions promise.
        clear_marker(display, at);
        let _ = Circle::with_center(at, 20)
            .into_styled(PrimitiveStyle::with_fill(DONE))
            .draw(display);
        delay.delay_millis(280);

        while touch.is_touched() {
            delay.delay_millis(10);
        }
        return ((sx / n) as u16, (sy / n) as u16);
    }
}

/// Erase the whole marker footprint, including a fully grown progress disc.
fn clear_marker<D: DrawTarget<Color = Rgb565>>(display: &mut D, at: Point) {
    let _ = display.fill_solid(
        &Rectangle::new(at - Point::new(14, 14), Size::new(28, 28)),
        BG,
    );
}

fn draw_marker<D: DrawTarget<Color = Rgb565>>(display: &mut D, at: Point, colour: Rgb565) {
    let stroke = PrimitiveStyle::with_stroke(colour, 2);
    let _ = Circle::with_center(at, 20).into_styled(stroke).draw(display);
    let _ = Line::new(at - Point::new(12, 0), at + Point::new(12, 0))
        .into_styled(stroke)
        .draw(display);
    let _ = Line::new(at - Point::new(0, 12), at + Point::new(0, 12))
        .into_styled(stroke)
        .draw(display);
}

fn paint<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: SpiDevice,
    IRQ: InputPin,
{
    let mut colour_index = 0usize;
    let mut rainbow_step = 0usize;
    let mut last: Option<Point> = None;
    let mut strokes: u32 = 0;

    draw_chrome(display, colour_index);
    let _ = display.fill_solid(&canvas_rect(), CANVAS_BG);

    loop {
        let Some(s) = touch.sample() else {
            last = None;
            delay.delay_millis(4);
            continue;
        };

        let (mx, my) = cal.map((s.x, s.y));
        let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, H - 1));

        if p.y < PALETTE_Y {
            if hit(recal_button(), p) {
                flash(display, recal_button(), delay);
                while touch.is_touched() {
                    delay.delay_millis(10);
                }
                return;
            }
            if hit(clear_button(), p) {
                flash(display, clear_button(), delay);
                let _ = display.fill_solid(&canvas_rect(), CANVAS_BG);
                strokes = 0;
                draw_chrome(display, colour_index);
            }
            last = None;
        } else if p.y < STATUS_Y {
            let picked = (p.x / SWATCH_W).clamp(0, SWATCHES as i32 - 1) as usize;
            if picked != colour_index {
                colour_index = picked;
                draw_palette(display, colour_index);
            }
            last = None;
        } else if p.y >= CANVAS_Y {
            // Pressure drives stroke width. Range from measurements on this
            // panel: the proxy idles near 0 and reads ~1000 under a fingertip.
            let width = 2 + ((s.z as u32).saturating_sub(700) / 180).min(9);
            let colour = if colour_index == SWATCHES - 1 {
                rainbow_step = (rainbow_step + 1) % RAINBOW.len();
                RAINBOW[rainbow_step]
            } else {
                PALETTE[colour_index]
            };

            // Clip to the canvas so a stroke can never scribble on the UI.
            let mut canvas = display.clipped(&canvas_rect());
            let _ = Circle::with_center(p, width)
                .into_styled(PrimitiveStyle::with_fill(colour))
                .draw(&mut canvas);
            if let Some(prev) = last {
                // Join to the previous point: sampling is slower than a
                // moving finger, so without this a quick stroke is dotted.
                let _ = Line::new(prev, p)
                    .into_styled(PrimitiveStyle::with_stroke(colour, width))
                    .draw(&mut canvas);
            }
            last = Some(p);
            strokes = strokes.saturating_add(1);
            draw_status(display, p, s.z, width, strokes);
        } else {
            last = None;
        }
    }
}

fn draw_chrome<D: DrawTarget<Color = Rgb565>>(display: &mut D, colour_index: usize) {
    let _ = display.fill_solid(
        &Rectangle::new(Point::zero(), Size::new(W as u32, HEADER_H as u32)),
        BG,
    );
    let _ = Text::new(
        "TOUCH LAB",
        Point::new(8, 21),
        MonoTextStyle::new(&FONT_9X15_BOLD, INK),
    )
    .draw(display);

    button(display, clear_button(), "CLEAR");
    button(display, recal_button(), "RECAL");

    draw_palette(display, colour_index);
    let _ = display.fill_solid(
        &Rectangle::new(
            Point::new(0, STATUS_Y),
            Size::new(W as u32, STATUS_H as u32),
        ),
        BG,
    );
}

fn button<D: DrawTarget<Color = Rgb565>>(display: &mut D, r: Rectangle, label: &str) {
    let _ = r
        .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
        .draw(display);
    // FONT_6X10 is 6px per glyph; centre the label in the button.
    let tx = r.top_left.x + (r.size.width as i32 - label.len() as i32 * 6) / 2;
    let _ = Text::new(
        label,
        Point::new(tx, r.top_left.y + 16),
        MonoTextStyle::new(&FONT_6X10, INK),
    )
    .draw(display);
}

fn draw_palette<D: DrawTarget<Color = Rgb565>>(display: &mut D, selected: usize) {
    for i in 0..SWATCHES {
        let x = i as i32 * SWATCH_W;
        let cell = Rectangle::new(
            Point::new(x, PALETTE_Y),
            Size::new(SWATCH_W as u32, PALETTE_H as u32),
        );

        if i == SWATCHES - 1 {
            // Rainbow slot: striped, so it reads as "not one colour".
            let stripe = PALETTE_H / RAINBOW.len() as i32 + 1;
            for (n, c) in RAINBOW.iter().enumerate() {
                let _ = display.fill_solid(
                    &Rectangle::new(
                        Point::new(x, PALETTE_Y + n as i32 * stripe),
                        Size::new(SWATCH_W as u32, stripe as u32),
                    ),
                    *c,
                );
            }
        } else {
            let _ = display.fill_solid(&cell, PALETTE[i]);
        }

        // Selection marker: an inset frame, so it never hides the colour.
        let frame = if i == selected { INK } else { BG };
        let _ = cell
            .into_styled(PrimitiveStyle::with_stroke(frame, 2))
            .draw(display);
    }
}

fn draw_status<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    p: Point,
    pressure: u16,
    width: u32,
    strokes: u32,
) {
    let _ = display.fill_solid(
        &Rectangle::new(
            Point::new(0, STATUS_Y),
            Size::new(W as u32, STATUS_H as u32),
        ),
        BG,
    );
    let mut line: String<48> = String::new();
    let _ = write!(
        line,
        "x{:>3} y{:>3}  p{:>4}  w{:>2}  n{}",
        p.x, p.y, pressure, width, strokes
    );
    let _ = Text::new(
        line.as_str(),
        Point::new(6, STATUS_Y + 13),
        MonoTextStyle::new(&FONT_6X10, DIM),
    )
    .draw(display);
}

fn flash<D: DrawTarget<Color = Rgb565>>(display: &mut D, r: Rectangle, delay: &mut Delay) {
    let _ = r.into_styled(PrimitiveStyle::with_fill(HOT)).draw(display);
    delay.delay_millis(90);
}
