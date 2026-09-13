//! Analog input readout.
//!
//! The 2432S028R is documented with an LDR on GPIO 34, but on this unit that
//! pin sits at a fixed level and does not respond to light — see the README.
//! The screen therefore shows raw counts rather than pretending to be a lux
//! meter, and tracks the observed range so any real movement is obvious.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;
use heapless::String;

use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BG, DIM, GOOD, INK, W};

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
    read: &mut dyn FnMut() -> (u16, u16),
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let back = ui::header(display, "ANALOG");
    ui::clear_body(display);

    let _ = Text::new(
        "GPIO34 is the documented LDR pin;",
        Point::new(6, ui::HEADER_H + 14),
        ui::body_style(DIM),
    )
    .draw(display);
    let _ = Text::new(
        "it does not respond to light here.",
        Point::new(6, ui::HEADER_H + 26),
        ui::body_style(DIM),
    )
    .draw(display);
    let _ = Text::new(
        "GPIO35 is unconnected and free.",
        Point::new(6, ui::HEADER_H + 38),
        ui::body_style(DIM),
    )
    .draw(display);

    let body = Rectangle::new(
        Point::new(0, ui::HEADER_H + 46),
        Size::new(W as u32, 120),
    );

    // Track extremes so a genuine change stands out from noise.
    let (mut lo34, mut hi34, mut lo35, mut hi35) = (u16::MAX, 0u16, u16::MAX, 0u16);

    loop {
        if let Some(s) = touch.sample() {
            let (mx, my) = cal.map((s.x, s.y));
            let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));
            if ui::hit(back, p) {
                ui::flash(display, back, delay);
                ui::wait_release(touch, delay);
                return;
            }
        }

        let (a, b) = read();
        lo34 = lo34.min(a);
        hi34 = hi34.max(a);
        lo35 = lo35.min(b);
        hi35 = hi35.max(b);

        let _ = display.fill_solid(&body, BG);
        let mut y = body.top_left.y + 14;
        for (name, v, lo, hi) in [
            ("GPIO34", a, lo34, hi34),
            ("GPIO35", b, lo35, hi35),
        ] {
            let mut line: String<48> = String::new();
            let _ = core::fmt::Write::write_fmt(
                &mut line,
                format_args!("{name}  {v:>4}   seen {lo}-{hi}"),
            );
            let _ = Text::new(line.as_str(), Point::new(6, y), ui::body_style(INK)).draw(display);

            // Bar across the full 12-bit range.
            let full = W - 16;
            let filled = (v as i32 * full / 4095).clamp(0, full);
            let bar = Rectangle::new(Point::new(8, y + 6), Size::new(full as u32, 10));
            let _ = bar
                .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
                .draw(display);
            if filled > 0 {
                let _ = display.fill_solid(
                    &Rectangle::new(Point::new(8, y + 6), Size::new(filled as u32, 10)),
                    GOOD,
                );
            }
            y += 40;
        }

        delay.delay_millis(120);
    }
}
