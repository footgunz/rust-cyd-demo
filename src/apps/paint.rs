//! Pressure-sensitive drawing surface.

use embedded_graphics::draw_target::DrawTargetExt;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;
use heapless::String;

use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BG, DIM, H, W};

const PALETTE_Y: i32 = ui::HEADER_H;
const PALETTE_H: i32 = 30;
const STATUS_Y: i32 = PALETTE_Y + PALETTE_H;
const STATUS_H: i32 = 18;
const CANVAS_Y: i32 = STATUS_Y + STATUS_H;

const SWATCHES: usize = 8;
const SWATCH_W: i32 = W / SWATCHES as i32;

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

fn clear_button() -> Rectangle {
    Rectangle::new(Point::new(120, 4), Size::new(52, 22))
}
fn canvas_rect() -> Rectangle {
    Rectangle::new(
        Point::new(0, CANVAS_Y),
        Size::new(W as u32, (H - CANVAS_Y) as u32),
    )
}

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let mut colour_index = 0usize;
    let mut rainbow_step = 0usize;
    let mut last: Option<Point> = None;
    let mut strokes: u32 = 0;

    let back = chrome(display, colour_index);
    let _ = display.fill_solid(&canvas_rect(), Rgb565::BLACK);

    loop {
        let Some(s) = touch.sample() else {
            last = None;
            delay.delay_millis(4);
            continue;
        };

        let (mx, my) = cal.map((s.x, s.y));
        let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, H - 1));

        if p.y < PALETTE_Y {
            if ui::hit(back, p) {
                ui::flash(display, back, delay);
                ui::wait_release(touch, delay);
                return;
            }
            if ui::hit(clear_button(), p) {
                ui::flash(display, clear_button(), delay);
                let _ = display.fill_solid(&canvas_rect(), Rgb565::BLACK);
                strokes = 0;
                chrome(display, colour_index);
            }
            last = None;
        } else if p.y < STATUS_Y {
            let picked = (p.x / SWATCH_W).clamp(0, SWATCHES as i32 - 1) as usize;
            if picked != colour_index {
                colour_index = picked;
                palette(display, colour_index);
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
            status(display, p, s.z, width, strokes);
        } else {
            last = None;
        }
    }
}

fn chrome<D: DrawTarget<Color = Rgb565>>(display: &mut D, colour_index: usize) -> Rectangle {
    let back = ui::header(display, "PAINT");
    ui::button(display, clear_button(), "CLEAR", DIM);
    palette(display, colour_index);
    let _ = display.fill_solid(
        &Rectangle::new(
            Point::new(0, STATUS_Y),
            Size::new(W as u32, STATUS_H as u32),
        ),
        BG,
    );
    back
}

fn palette<D: DrawTarget<Color = Rgb565>>(display: &mut D, selected: usize) {
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
        let frame = if i == selected { ui::INK } else { BG };
        let _ = cell
            .into_styled(PrimitiveStyle::with_stroke(frame, 2))
            .draw(display);
    }
}

fn status<D: DrawTarget<Color = Rgb565>>(
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
    let _ = core::fmt::Write::write_fmt(
        &mut line,
        format_args!(
            "x{:>3} y{:>3}  p{:>4}  w{:>2}  n{}",
            p.x, p.y, pressure, width, strokes
        ),
    );
    let _ = Text::new(
        line.as_str(),
        Point::new(6, STATUS_Y + 13),
        ui::body_style(DIM),
    )
    .draw(display);
}
