//! Wi-Fi scanner.
//!
//! Passive observation only: this listens for beacons and associates with
//! nothing. No credentials are involved and no network is joined.

extern crate alloc;

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use embassy_futures::block_on;
use esp_hal::delay::Delay;
use esp_println::println;
use esp_radio::wifi::scan::ScanConfig;
use esp_radio::wifi::WifiController;
use heapless::String;

use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BAD, DIM, GOOD, HOT, INK, W};

const ROW_H: i32 = 26;
const MAX_ROWS: usize = 9;

/// RSSI runs roughly -30 (excellent) to -90 (unusable).
fn rssi_colour(rssi: i8) -> Rgb565 {
    match rssi {
        r if r >= -55 => GOOD,
        r if r >= -70 => HOT,
        _ => BAD,
    }
}
fn rssi_fraction(rssi: i8) -> i32 {
    (((rssi as i32) + 90) * 100 / 60).clamp(0, 100)
}

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
    wifi: &mut WifiController<'_>,
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let back = ui::header(display, "WIFI SCAN");

    loop {
        ui::clear_body(display);
        let _ = Text::new(
            "scanning...",
            Point::new(8, ui::HEADER_H + 20),
            ui::body_style(HOT),
        )
        .draw(display);

        let config = ScanConfig::default().with_max(MAX_ROWS);
        match block_on(wifi.scan_async(&config)) {
            Ok(mut aps) => {
                aps.sort_by(|a, b| b.signal_strength.cmp(&a.signal_strength));
                println!("wifi: {} AP(s)", aps.len());
                ui::clear_body(display);
                render(display, &aps);
            }
            Err(e) => {
                println!("wifi scan failed: {e:?}");
                ui::clear_body(display);
                let _ = Text::new(
                    "scan failed",
                    Point::new(8, ui::HEADER_H + 20),
                    ui::body_style(BAD),
                )
                .draw(display);
            }
        }

        // Wait for a press: BACK leaves, anywhere else rescans.
        loop {
            let Some(s) = touch.sample() else {
                delay.delay_millis(15);
                continue;
            };
            let (mx, my) = cal.map((s.x, s.y));
            let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));
            if ui::hit(back, p) {
                ui::flash(display, back, delay);
                ui::wait_release(touch, delay);
                return;
            }
            ui::wait_release(touch, delay);
            break;
        }
    }
}

fn render<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    aps: &[esp_radio::wifi::ap::AccessPointInfo],
) {
    let mut head: String<24> = String::new();
    let _ = core::fmt::Write::write_fmt(&mut head, format_args!("{} networks", aps.len()));
    let _ = Text::new(
        head.as_str(),
        Point::new(8, ui::HEADER_H + 12),
        ui::body_style(INK),
    )
    .draw(display);

    for (i, ap) in aps.iter().take(MAX_ROWS).enumerate() {
        let y = ui::HEADER_H + 18 + i as i32 * ROW_H;

        // A hidden network reports an empty SSID.
        let name = ap.ssid.as_str();
        let shown = if name.is_empty() {
            "<hidden>"
        } else if name.len() > 20 {
            &name[..20]
        } else {
            name
        };
        let _ = Text::new(shown, Point::new(5, y + 9), ui::body_style(INK)).draw(display);

        let mut meta: String<24> = String::new();
        let _ = core::fmt::Write::write_fmt(
            &mut meta,
            format_args!(
                "ch{:<3} {:>4}dBm {}",
                ap.channel,
                ap.signal_strength,
                if ap.auth_method.is_some() { "*" } else { " " }
            ),
        );
        let _ = Text::new(meta.as_str(), Point::new(5, y + 20), ui::body_style(DIM)).draw(display);

        // Signal bar, right-aligned.
        let full = 70;
        let filled = full * rssi_fraction(ap.signal_strength) / 100;
        let _ = Rectangle::new(Point::new(W - full - 6, y + 3), Size::new(full as u32, 9))
            .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
            .draw(display);
        if filled > 0 {
            let _ = display.fill_solid(
                &Rectangle::new(Point::new(W - full - 6, y + 3), Size::new(filled as u32, 9)),
                rssi_colour(ap.signal_strength),
            );
        }
    }
}
