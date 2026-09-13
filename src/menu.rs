//! Main menu for the kitchen-sink demo.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;

use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BG, DIM, INK, PANEL, W};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Paint,
    WifiScan,
    BleScan,
    TouchDiag,
    Analog,
    Leds,
    Calibrate,
}

const ITEMS: [(Choice, &str, &str); 7] = [
    (Choice::Paint, "PAINT", "pressure-sensitive drawing"),
    (Choice::WifiScan, "WIFI SCAN", "nearby access points"),
    (Choice::BleScan, "BLE SCAN", "beacons and item trackers"),
    (Choice::TouchDiag, "TOUCH DIAG", "live raw touch values"),
    (Choice::Analog, "ANALOG", "ADC inputs"),
    (Choice::Leds, "LEDS", "RGB channel control"),
    (Choice::Calibrate, "CALIBRATE", "redo touch calibration"),
];

const TOP: i32 = 46;
const ROW_H: i32 = 36;
const GAP: i32 = 2;

fn row(i: usize) -> Rectangle {
    Rectangle::new(
        Point::new(8, TOP + i as i32 * (ROW_H + GAP)),
        Size::new(W as u32 - 16, ROW_H as u32),
    )
}

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
) -> Choice
where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    draw(display);

    loop {
        let Some(s) = touch.sample() else {
            delay.delay_millis(15);
            continue;
        };
        let (mx, my) = cal.map((s.x, s.y));
        let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));

        for (i, (choice, _, _)) in ITEMS.iter().enumerate() {
            if ui::hit(row(i), p) {
                ui::flash(display, row(i), delay);
                ui::wait_release(touch, delay);
                return *choice;
            }
        }
    }
}

fn draw<D: DrawTarget<Color = Rgb565>>(display: &mut D) {
    let _ = display.clear(BG);
    let _ = display.fill_solid(
        &Rectangle::new(Point::zero(), Size::new(W as u32, 38)),
        PANEL,
    );
    let _ = Text::new("CYD KITCHEN SINK", Point::new(8, 18), ui::title_style()).draw(display);
    let _ = Text::new(
        "ESP32-2432S028R - no_std Rust",
        Point::new(8, 31),
        ui::body_style(DIM),
    )
    .draw(display);

    for (i, (_, name, hint)) in ITEMS.iter().enumerate() {
        let r = row(i);
        let _ = r
            .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
            .draw(display);
        let _ = Text::new(
            name,
            Point::new(r.top_left.x + 10, r.top_left.y + 15),
            ui::body_style(INK),
        )
        .draw(display);
        let _ = Text::new(
            hint,
            Point::new(r.top_left.x + 10, r.top_left.y + 27),
            ui::body_style(DIM),
        )
        .draw(display);
    }
}
