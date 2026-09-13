//! BLE beacon scanner, with Find My / AirTag highlighting.
//!
//! Listens only. Devices are keyed by advertised address, so a Find My beacon
//! that rotates its address will reappear as a new entry — that is the
//! privacy design working, not a bug.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use heapless::String;

use crate::ble::{self, Advert, Kind};
use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BAD, BG, DIM, GOOD, HOT, INK, W};

const MAX_SEEN: usize = 12;
const ROW_H: i32 = 22;
const VISIBLE: usize = 9;

struct Seen {
    adv: Advert,
    hits: u16,
}

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
    ble_conn: &mut BleConnector<'_>,
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let back = ui::header(display, "BLE SCAN");
    ui::clear_body(display);
    let _ = Text::new(
        "listening for beacons...",
        Point::new(8, ui::HEADER_H + 14),
        ui::body_style(HOT),
    )
    .draw(display);

    ble::start_scan(ble_conn);

    let mut seen: heapless::Vec<Seen, MAX_SEEN> = heapless::Vec::new();
    let mut trackers: u16 = 0;
    let mut ticks: u32 = 0;

    loop {
        // Drain whatever the controller has queued.
        for _ in 0..16 {
            let Some(adv) = ble::poll(ble_conn) else { break };
            match seen.iter_mut().find(|s| s.adv.addr == adv.addr) {
                Some(s) => {
                    s.adv = adv;
                    s.hits = s.hits.saturating_add(1);
                }
                None => {
                    if adv.kind.is_tracker() {
                        trackers = trackers.saturating_add(1);
                        println!(
                            "BLE tracker: {:02x?} rssi {} {}",
                            adv.addr,
                            adv.rssi,
                            adv.kind.label()
                        );
                    }
                    if seen.len() == MAX_SEEN {
                        // Evict the weakest so strong, nearby devices persist.
                        if let Some((i, _)) = seen
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, s)| s.adv.rssi)
                        {
                            seen.swap_remove(i);
                        }
                    }
                    let _ = seen.push(Seen { adv, hits: 1 });
                }
            }
        }

        ticks += 1;
        if ticks % 8 == 0 {
            seen.sort_unstable_by(|a, b| b.adv.rssi.cmp(&a.adv.rssi));
            render(display, &seen, trackers);
        }

        if let Some(s) = touch.sample() {
            let (mx, my) = cal.map((s.x, s.y));
            let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));
            if ui::hit(back, p) {
                ble::stop_scan(ble_conn);
                ui::flash(display, back, delay);
                ui::wait_release(touch, delay);
                return;
            }
        }
        delay.delay_millis(25);
    }
}

fn kind_colour(k: Kind) -> Rgb565 {
    match k {
        Kind::FindMy => BAD, // deliberately loud: this is the interesting one
        Kind::AppleNearby | Kind::AirDrop | Kind::Apple => HOT,
        Kind::IBeacon => GOOD,
        _ => DIM,
    }
}

fn render<D: DrawTarget<Color = Rgb565>>(display: &mut D, seen: &[Seen], trackers: u16) {
    let body = Rectangle::new(
        Point::new(0, ui::HEADER_H),
        Size::new(W as u32, (ui::H - ui::HEADER_H) as u32),
    );
    let _ = display.fill_solid(&body, BG);

    let mut head: String<40> = String::new();
    let _ = core::fmt::Write::write_fmt(
        &mut head,
        format_args!("{} devices   {} Find My", seen.len(), trackers),
    );
    let _ = Text::new(
        head.as_str(),
        Point::new(6, ui::HEADER_H + 12),
        ui::body_style(if trackers > 0 { BAD } else { INK }),
    )
    .draw(display);

    for (i, s) in seen.iter().take(VISIBLE).enumerate() {
        let y = ui::HEADER_H + 24 + i as i32 * ROW_H;
        let a = s.adv.addr;

        // Address, plus a marker when it is a randomised one.
        let mut line: String<48> = String::new();
        let _ = core::fmt::Write::write_fmt(
            &mut line,
            format_args!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}{} {:>4}",
                a[5],
                a[4],
                a[3],
                a[2],
                a[1],
                a[0],
                if s.adv.random_addr { "~" } else { " " },
                s.adv.rssi
            ),
        );
        let _ = Text::new(line.as_str(), Point::new(6, y + 8), ui::body_style(INK)).draw(display);

        // Second line: what it looks like, and its name if it gave one.
        let mut meta: String<48> = String::new();
        let name = s.adv.name_str();
        if name.is_empty() {
            let _ = core::fmt::Write::write_fmt(
                &mut meta,
                format_args!("  {} x{}", s.adv.kind.label(), s.hits),
            );
        } else {
            let _ = core::fmt::Write::write_fmt(
                &mut meta,
                format_args!("  {} {}", s.adv.kind.label(), name),
            );
        }
        let _ = Text::new(
            meta.as_str(),
            Point::new(6, y + 17),
            ui::body_style(kind_colour(s.adv.kind)),
        )
        .draw(display);
    }

    let _ = Text::new(
        "~ = randomised address",
        Point::new(6, ui::H - 6),
        ui::body_style(DIM),
    )
    .draw(display);
    let _ = Rectangle::new(Point::new(0, ui::H - 18), Size::new(0, 0))
        .into_styled(PrimitiveStyle::with_stroke(DIM, 0))
        .draw(display);
}
