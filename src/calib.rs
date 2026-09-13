//! Touch calibration screen.

use embedded_graphics::mono_font::{ascii::FONT_9X15_BOLD, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;
use heapless::String;

use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BG, DONE_RING, HOT, INK};

/// Valid samples required to accept a point. At roughly one per 10ms this is
/// about a quarter-second of contact: enough to average away the panel's
/// noise, short enough not to feel like a wait.
const HOLD_SAMPLES: u32 = 24;

/// Consecutive misses tolerated mid-hold before the attempt is abandoned.
///
/// A light press hovers near the driver's pressure floor, and PENIRQ briefly
/// deasserts while the Z channels are read, so isolated misses are normal and
/// must not throw away an otherwise good press.
const HOLD_MISS_LIMIT: u32 = 12;

/// Run the three-point calibration.
///
/// Three points rather than two: a pair of diagonal points cannot separate an
/// axis swap from an axis inversion, and this panel's axes are transposed
/// relative to the display. One pure-X and one pure-Y move resolve swap,
/// inversion and scale together.
pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
) -> Calibration
where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let _ = display.clear(BG);
    let loud = MonoTextStyle::new(&FONT_9X15_BOLD, DONE_RING);
    let body = ui::body_style(INK);

    let _ = Text::new("CALIBRATION", Point::new(57, 84), ui::title_style()).draw(display);
    let _ = Text::new("press and HOLD each target", Point::new(33, 116), body).draw(display);
    let _ = Text::new("until its ring turns", Point::new(54, 130), body).draw(display);
    let _ = Text::new("GREEN", Point::new(93, 152), loud).draw(display);

    let targets = [
        (Point::new(26, 210), (26i32, 210i32)),
        (Point::new(ui::W - 26, 210), (ui::W - 26, 210)),
        (Point::new(26, 296), (26, 296)),
    ];

    let mut raws = [(0u16, 0u16); 3];
    for (i, (at, _)) in targets.iter().enumerate() {
        let mut step: String<8> = String::new();
        let _ = core::fmt::Write::write_fmt(&mut step, format_args!("{} of 3", i + 1));
        let _ = display.fill_solid(
            &Rectangle::new(Point::new(88, 168), Size::new(64, 12)),
            BG,
        );
        let _ = Text::new(step.as_str(), Point::new(102, 178), body).draw(display);

        marker(display, *at, INK);
        raws[i] = hold(display, touch, delay, *at);
        clear_marker(display, *at);
        delay.delay_millis(200);
    }

    Calibration::from_points(
        raws[0],
        raws[1],
        raws[2],
        targets[0].1,
        targets[1].1,
        targets[2].1,
    )
}

/// Wait for a deliberate, sustained press and return its averaged position.
///
/// Averaging over the hold is what buys precision: a single sample carries the
/// panel's noise straight into the calibration constants.
fn hold<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    at: Point,
) -> (u16, u16)
where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
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
        marker(display, at, HOT);

        let (mut sx, mut sy, mut n, mut misses) = (0u32, 0u32, 0u32, 0u32);
        while n < HOLD_SAMPLES {
            match touch.sample() {
                Some(s) => {
                    sx += s.x as u32;
                    sy += s.y as u32;
                    n += 1;
                    misses = 0;
                    let grown = 4 + (n * 12 / HOLD_SAMPLES);
                    let _ = Circle::with_center(at, grown)
                        .into_styled(PrimitiveStyle::with_fill(HOT))
                        .draw(display);
                }
                None => {
                    misses += 1;
                    if misses > HOLD_MISS_LIMIT {
                        clear_marker(display, at);
                        marker(display, at, INK);
                        continue 'attempt;
                    }
                }
            }
            delay.delay_millis(10);
        }

        // Green confirms acceptance — the cue the instructions promise.
        clear_marker(display, at);
        let _ = Circle::with_center(at, 20)
            .into_styled(PrimitiveStyle::with_fill(DONE_RING))
            .draw(display);
        delay.delay_millis(280);

        while touch.is_touched() {
            delay.delay_millis(10);
        }
        return ((sx / n) as u16, (sy / n) as u16);
    }
}

fn marker<D: DrawTarget<Color = Rgb565>>(display: &mut D, at: Point, colour: Rgb565) {
    let stroke = PrimitiveStyle::with_stroke(colour, 2);
    let _ = Circle::with_center(at, 20).into_styled(stroke).draw(display);
    let _ = Line::new(at - Point::new(12, 0), at + Point::new(12, 0))
        .into_styled(stroke)
        .draw(display);
    let _ = Line::new(at - Point::new(0, 12), at + Point::new(0, 12))
        .into_styled(stroke)
        .draw(display);
}

/// Erase the whole marker footprint, including a fully grown progress disc.
fn clear_marker<D: DrawTarget<Color = Rgb565>>(display: &mut D, at: Point) {
    let _ = display.fill_solid(
        &Rectangle::new(at - Point::new(14, 14), Size::new(28, 28)),
        BG,
    );
}
