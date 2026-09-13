//! Live raw touch readout — the on-screen twin of the `touch-probe` binary.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle};
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
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let back = ui::header(display, "TOUCH DIAG");
    ui::clear_body(display);

    let _ = Text::new(
        "raw ADC counts and the mapped",
        Point::new(8, ui::HEADER_H + 14),
        ui::body_style(DIM),
    )
    .draw(display);
    let _ = Text::new(
        "pixel. drag to watch it track.",
        Point::new(8, ui::HEADER_H + 26),
        ui::body_style(DIM),
    )
    .draw(display);

    let readout = Rectangle::new(
        Point::new(0, ui::HEADER_H + 34),
        Size::new(W as u32, 46),
    );
    let field = Rectangle::new(
        Point::new(0, ui::HEADER_H + 84),
        Size::new(W as u32, (ui::H - ui::HEADER_H - 84) as u32),
    );
    let _ = field
        .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
        .draw(display);

    let mut last_dot: Option<Point> = None;

    loop {
        match touch.sample() {
            Some(s) => {
                let (mx, my) = cal.map((s.x, s.y));
                let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));

                if ui::hit(back, p) {
                    ui::flash(display, back, delay);
                    ui::wait_release(touch, delay);
                    return;
                }

                let _ = display.fill_solid(&readout, BG);
                let mut l1: String<48> = String::new();
                let _ = core::fmt::Write::write_fmt(
                    &mut l1,
                    format_args!("raw  x{:>5}  y{:>5}  z{:>5}", s.x, s.y, s.z),
                );
                let mut l2: String<48> = String::new();
                let _ = core::fmt::Write::write_fmt(
                    &mut l2,
                    format_args!(
                        "px   x{:>5}  y{:>5}  axes:{}",
                        p.x,
                        p.y,
                        if cal.is_swapped() { "swap" } else { "direct" }
                    ),
                );
                let _ = Text::new(
                    l1.as_str(),
                    Point::new(8, readout.top_left.y + 14),
                    ui::body_style(INK),
                )
                .draw(display);
                let _ = Text::new(
                    l2.as_str(),
                    Point::new(8, readout.top_left.y + 30),
                    ui::body_style(GOOD),
                )
                .draw(display);

                // Track the finger inside the field box only.
                if p.y >= field.top_left.y + 2 {
                    if let Some(prev) = last_dot {
                        let _ = Circle::with_center(prev, 7)
                            .into_styled(PrimitiveStyle::with_fill(BG))
                            .draw(display);
                    }
                    let _ = Circle::with_center(p, 7)
                        .into_styled(PrimitiveStyle::with_fill(GOOD))
                        .draw(display);
                    last_dot = Some(p);
                }
            }
            None => delay.delay_millis(10),
        }
    }
}
