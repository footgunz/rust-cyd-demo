//! RGB LED control.
//!
//! The LED is common anode, so a pin is driven low to light it. On this unit
//! the red channel (GPIO 4) is dead — driving it alone produces no light —
//! which the screen says outright so it does not read as a bug.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;

use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BAD, BG, DIM, INK, W};

const CHANNELS: [(&str, Rgb565); 3] = [
    ("RED  (GPIO4)", Rgb565::new(31, 0, 0)),
    ("GREEN (GPIO16)", Rgb565::new(0, 63, 0)),
    ("BLUE  (GPIO17)", Rgb565::new(0, 0, 31)),
];

fn row(i: usize) -> Rectangle {
    Rectangle::new(
        Point::new(16, ui::HEADER_H + 40 + i as i32 * 52),
        Size::new(W as u32 - 32, 40),
    )
}

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
    set: &mut dyn FnMut(usize, bool),
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let back = ui::header(display, "LEDS");
    ui::clear_body(display);

    let _ = Text::new(
        "tap to toggle each channel",
        Point::new(8, ui::HEADER_H + 14),
        ui::body_style(DIM),
    )
    .draw(display);
    let _ = Text::new(
        "red is dead on this unit",
        Point::new(8, ui::HEADER_H + 26),
        ui::body_style(BAD),
    )
    .draw(display);

    let mut on = [false; 3];
    for i in 0..3 {
        set(i, false);
        draw_row(display, i, on[i]);
    }

    loop {
        let Some(s) = touch.sample() else {
            delay.delay_millis(15);
            continue;
        };
        let (mx, my) = cal.map((s.x, s.y));
        let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));

        if ui::hit(back, p) {
            // Leave the LED off rather than stranding it lit.
            for i in 0..3 {
                set(i, false);
            }
            ui::flash(display, back, delay);
            ui::wait_release(touch, delay);
            return;
        }

        for i in 0..3 {
            if ui::hit(row(i), p) {
                on[i] = !on[i];
                set(i, on[i]);
                draw_row(display, i, on[i]);
            }
        }
        ui::wait_release(touch, delay);
    }
}

fn draw_row<D: DrawTarget<Color = Rgb565>>(display: &mut D, i: usize, on: bool) {
    let r = row(i);
    let (label, colour) = CHANNELS[i];
    let _ = display.fill_solid(&r, if on { colour } else { BG });
    let _ = r
        .into_styled(PrimitiveStyle::with_stroke(if on { INK } else { DIM }, 2))
        .draw(display);
    let _ = Text::new(
        label,
        Point::new(r.top_left.x + 10, r.top_left.y + 18),
        ui::body_style(INK),
    )
    .draw(display);
    let _ = Text::new(
        if on { "ON" } else { "off" },
        Point::new(r.top_left.x + 10, r.top_left.y + 32),
        ui::body_style(if on { INK } else { DIM }),
    )
    .draw(display);
}
