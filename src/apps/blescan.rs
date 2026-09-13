//! BLE beacon scanner, with tracker highlighting.
//!
//! Listens only. Devices are keyed by advertised address; a Find My tag
//! rotates that address periodically, so the same tag eventually reappears as
//! a new entry. That is the privacy design working, not a bug.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
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
const LIST_TOP: i32 = ui::HEADER_H + 20;

/// Ticks (~25ms each) without a sighting before a device counts as stale and
/// may be evicted to make room.
const STALE_TICKS: u32 = 1200;

struct Seen {
    adv: Advert,
    hits: u16,
    /// Exponentially smoothed RSSI. Raw RSSI swings by tens of dB between
    /// adverts, so the displayed figure - and any ordering based on it -
    /// would jitter constantly without this.
    rssi_avg: i16,
    last_tick: u32,
    /// Set when the rendered text would change, so untouched rows are left
    /// alone instead of being cleared and redrawn into the same pixels.
    dirty: bool,
}

impl Seen {
    fn new(adv: Advert, tick: u32) -> Self {
        Self {
            rssi_avg: adv.rssi as i16,
            adv,
            hits: 1,
            last_tick: tick,
            dirty: true,
        }
    }

    fn update(&mut self, adv: Advert, tick: u32) {
        let before = self.rssi_avg;
        // Weighted towards history: four-sample time constant.
        self.rssi_avg = (self.rssi_avg * 3 + adv.rssi as i16) / 4;
        self.hits = self.hits.saturating_add(1);
        self.last_tick = tick;
        // Only a visible change is worth a redraw.
        if self.rssi_avg != before || self.adv.kind != adv.kind {
            self.dirty = true;
        }
        self.adv = adv;
    }
}

fn raw_button() -> Rectangle {
    Rectangle::new(Point::new(W - 112, 4), Size::new(52, 22))
}

fn row_rect(i: usize) -> Rectangle {
    Rectangle::new(
        Point::new(0, LIST_TOP + i as i32 * ROW_H),
        Size::new(W as u32, ROW_H as u32),
    )
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
    let mut raw_mode = false;
    ui::button(display, raw_button(), "RAW", DIM);
    ui::clear_body(display);
    let _ = Text::new(
        "listening for beacons...",
        Point::new(8, ui::HEADER_H + 14),
        ui::body_style(HOT),
    )
    .draw(display);

    ble::start_scan(ble_conn, delay);

    // Insertion order is never disturbed: a device keeps the slot it was
    // first drawn in, so the list stays readable while signals move around.
    let mut seen: heapless::Vec<Seen, MAX_SEEN> = heapless::Vec::new();
    let mut trackers: u16 = 0;
    let mut tick: u32 = 0;
    let mut packets: u32 = 0;
    let mut drawn_rows: usize = 0;
    let mut last_head: String<48> = String::new();

    loop {
        // Drain whatever the controller has queued.
        for _ in 0..16 {
            let Some(adv) = ble::poll_counted(ble_conn, &mut packets) else {
                break;
            };
            match seen.iter_mut().find(|s| s.adv.addr == adv.addr) {
                Some(s) => s.update(adv, tick),
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
                        evict(&mut seen, tick);
                    }
                    let _ = seen.push(Seen::new(adv, tick));
                }
            }
        }

        tick += 1;
        if tick % 16 == 0 {
            render(
                display,
                &mut seen,
                trackers,
                packets,
                raw_mode,
                &mut drawn_rows,
                &mut last_head,
            );
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
            if ui::hit(raw_button(), p) {
                raw_mode = !raw_mode;
                ui::flash(display, raw_button(), delay);
                ui::button(
                    display,
                    raw_button(),
                    if raw_mode { "CLASS" } else { "RAW" },
                    DIM,
                );
                ui::wait_release(touch, delay);
                // Every row's second line changes meaning in the other mode.
                for s in seen.iter_mut() {
                    s.dirty = true;
                }
            }
        }
        delay.delay_millis(25);
    }
}

/// Make room, preferring to drop a stale non-tracker over anything else.
///
/// `remove` rather than `swap_remove`: swapping would move an unrelated
/// device into the freed slot and visibly reshuffle the list.
fn evict(seen: &mut heapless::Vec<Seen, MAX_SEEN>, tick: u32) {
    let victim = seen
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.adv.kind.is_tracker())
        .filter(|(_, s)| tick.saturating_sub(s.last_tick) > STALE_TICKS)
        .min_by_key(|(_, s)| s.rssi_avg)
        // Nothing stale: fall back to the weakest non-tracker, then anything.
        .or_else(|| {
            seen.iter()
                .enumerate()
                .filter(|(_, s)| !s.adv.kind.is_tracker())
                .min_by_key(|(_, s)| s.rssi_avg)
        })
        .or_else(|| seen.iter().enumerate().min_by_key(|(_, s)| s.rssi_avg))
        .map(|(i, _)| i);

    if let Some(i) = victim {
        seen.remove(i);
        // Everything below shifted up a slot.
        for s in seen.iter_mut().skip(i) {
            s.dirty = true;
        }
    }
}

fn kind_colour(k: Kind) -> Rgb565 {
    match k {
        Kind::FindMyTag => BAD, // deliberately loud: this is the interesting one
        Kind::FindMyDevice => DIM, // a phone or laptop, not a tag
        Kind::Tile | Kind::SmartTag | Kind::FastPair => BAD,
        Kind::AppleNearby | Kind::AirDrop | Kind::Apple => HOT,
        Kind::IBeacon | Kind::MetaGlasses | Kind::Flipper => GOOD,
        _ => DIM,
    }
}

#[allow(clippy::too_many_arguments)]
fn render<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    seen: &mut [Seen],
    trackers: u16,
    packets: u32,
    raw_mode: bool,
    drawn_rows: &mut usize,
    last_head: &mut String<48>,
) {
    // Header line, redrawn only when its text changes.
    let mut head: String<48> = String::new();
    let _ = core::fmt::Write::write_fmt(
        &mut head,
        format_args!("{} dev  {} tags  {} hci", seen.len(), trackers, packets),
    );
    if head != *last_head {
        let _ = display.fill_solid(
            &Rectangle::new(Point::new(0, ui::HEADER_H), Size::new(W as u32, 16)),
            BG,
        );
        let _ = Text::new(
            head.as_str(),
            Point::new(6, ui::HEADER_H + 12),
            ui::body_style(if trackers > 0 { BAD } else { INK }),
        )
        .draw(display);
        *last_head = head;
    }

    // Nothing at all, and no HCI traffic either: say so, rather than showing
    // an empty list that looks like a quiet room.
    if seen.is_empty() && packets == 0 {
        let _ = Text::new(
            "no HCI packets - scan not running",
            Point::new(6, LIST_TOP + 10),
            ui::body_style(BAD),
        )
        .draw(display);
        return;
    }

    for (i, s) in seen.iter_mut().take(VISIBLE).enumerate() {
        if !s.dirty {
            continue;
        }
        let r = row_rect(i);
        let _ = display.fill_solid(&r, BG);
        let y = r.top_left.y;
        let a = s.adv.addr;

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
                s.rssi_avg
            ),
        );
        let _ = Text::new(line.as_str(), Point::new(6, y + 9), ui::body_style(INK)).draw(display);

        let mut meta: String<48> = String::new();
        if raw_mode {
            // No classification and no filtering: useful when the classifier
            // itself is the suspect.
            let _ = core::fmt::Write::write_fmt(
                &mut meta,
                format_args!(
                    "  x{}  {}",
                    s.hits,
                    if s.adv.random_addr { "random" } else { "public" }
                ),
            );
            let _ = Text::new(meta.as_str(), Point::new(6, y + 18), ui::body_style(DIM))
                .draw(display);
        } else {
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
                Point::new(6, y + 18),
                ui::body_style(kind_colour(s.adv.kind)),
            )
            .draw(display);
        }
        s.dirty = false;
    }

    // Clear any rows left behind when the list shrinks.
    let now = seen.len().min(VISIBLE);
    for i in now..*drawn_rows {
        let _ = display.fill_solid(&row_rect(i), BG);
    }
    *drawn_rows = now;
}
