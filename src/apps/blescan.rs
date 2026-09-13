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

// Ticks are ~25ms. How long since the last sighting before a device is
// considered fading, then gone. A device that has gone quiet stays in the
// list rather than vanishing, so a beacon that drops in and out keeps its
// slot instead of appearing to be a new device each time.
const IDLE_TICKS: u32 = 120; // ~3s
const GONE_TICKS: u32 = 600; // ~15s

/// Presence of a device, from how recently it was last heard.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Here,
    Fading,
    Gone,
}

impl Presence {
    fn from_age(age: u32) -> Self {
        if age < IDLE_TICKS {
            Presence::Here
        } else if age < GONE_TICKS {
            Presence::Fading
        } else {
            Presence::Gone
        }
    }
    fn marker(self) -> char {
        match self {
            Presence::Here => '*',
            Presence::Fading => '-',
            Presence::Gone => 'x',
        }
    }
}

struct Seen {
    adv: Advert,
    hits: u16,
    /// Exponentially smoothed RSSI. Raw RSSI swings by tens of dB between
    /// adverts, so the displayed figure - and any ordering based on it -
    /// would jitter constantly without this.
    rssi_avg: i16,
    last_tick: u32,
    /// Last text actually drawn for this row. Redrawing is driven by
    /// comparing against these rather than by guessing what changed.
    l1: String<48>,
    l2: String<48>,
}

impl Seen {
    fn new(adv: Advert, tick: u32) -> Self {
        Self {
            rssi_avg: adv.rssi as i16,
            adv,
            hits: 1,
            last_tick: tick,
            l1: String::new(),
            l2: String::new(),
        }
    }

    fn presence(&self, tick: u32) -> Presence {
        Presence::from_age(tick.saturating_sub(self.last_tick))
    }

    fn update(&mut self, adv: Advert, tick: u32) {
        // Weighted towards history: four-sample time constant.
        self.rssi_avg = (self.rssi_avg * 3 + adv.rssi as i16) / 4;
        self.hits = self.hits.saturating_add(1);
        self.last_tick = tick;
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
                    if seen.len() == MAX_SEEN && !evict(&mut seen, tick) {
                        // Everything on screen is still live; keep it.
                        continue;
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
                tick,
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
                    s.l2.clear();
                }
            }
        }
        delay.delay_millis(25);
    }
}

/// Make room for a new device, if anything can fairly be dropped.
///
/// Only devices that have gone are candidates: a beacon that drops in and
/// out should keep its slot and be marked, not be evicted and then reappear
/// as a new entry. Trackers are dropped last. Returns false when everything
/// present is still worth keeping, in which case the new device is ignored
/// until a slot frees up.
///
/// `remove` rather than `swap_remove`: swapping would move an unrelated
/// device into the freed slot and visibly reshuffle the list.
fn evict(seen: &mut heapless::Vec<Seen, MAX_SEEN>, tick: u32) -> bool {
    let victim = seen
        .iter()
        .enumerate()
        .filter(|(_, s)| s.presence(tick) == Presence::Gone)
        .min_by_key(|(_, s)| (s.adv.kind.is_tracker(), s.last_tick))
        .map(|(i, _)| i);

    match victim {
        Some(i) => {
            seen.remove(i);
            // Everything below shifted up a slot; force those rows to redraw.
            for s in seen.iter_mut().skip(i) {
                s.l1.clear();
                s.l2.clear();
            }
            true
        }
        None => false,
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
    tick: u32,
    drawn_rows: &mut usize,
    last_head: &mut String<48>,
) {
    // Header line, redrawn only when its text changes.
    let mut head: String<48> = String::new();
    let here = seen
        .iter()
        .filter(|s| s.presence(tick) != Presence::Gone)
        .count();
    let _ = core::fmt::Write::write_fmt(
        &mut head,
        format_args!("{here}/{} dev  {trackers} tags  {packets} hci", seen.len()),
    );
    if head != *last_head {
        pad(&mut head);
        let _ = Text::new(
            head.as_str(),
            Point::new(3, ui::HEADER_H + 12),
            ui::body_style_opaque(if trackers > 0 { BAD } else { INK }),
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
            ui::body_style_opaque(BAD),
        )
        .draw(display);
        return;
    }

    for (i, s) in seen.iter_mut().take(VISIBLE).enumerate() {
        let y = LIST_TOP + i as i32 * ROW_H;
        let presence = s.presence(tick);
        let gone = presence == Presence::Gone;
        let a = s.adv.addr;

        let mut l1: String<48> = String::new();
        let _ = core::fmt::Write::write_fmt(
            &mut l1,
            format_args!(
                "{} {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}{} {:>4}",
                presence.marker(),
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

        let mut l2: String<48> = String::new();
        if raw_mode {
            // No classification and no filtering: useful when the classifier
            // itself is the suspect.
            let _ = core::fmt::Write::write_fmt(
                &mut l2,
                format_args!(
                    "   x{}  {}",
                    s.hits,
                    if s.adv.random_addr { "random" } else { "public" }
                ),
            );
        } else {
            let name = s.adv.name_str();
            if name.is_empty() {
                let _ = core::fmt::Write::write_fmt(
                    &mut l2,
                    format_args!("   {} x{}", s.adv.kind.label(), s.hits),
                );
            } else {
                let _ = core::fmt::Write::write_fmt(
                    &mut l2,
                    format_args!("   {} {}", s.adv.kind.label(), name),
                );
            }
        }
        pad(&mut l1);
        pad(&mut l2);

        // Opaque text overwrites in place, so a changed row never blinks.
        if l1 != s.l1 {
            let _ = Text::new(
                l1.as_str(),
                Point::new(3, y + 9),
                ui::body_style_opaque(if gone { DIM } else { INK }),
            )
            .draw(display);
            s.l1 = l1;
        }
        if l2 != s.l2 {
            let colour = if gone { DIM } else { kind_colour(s.adv.kind) };
            let _ = Text::new(
                l2.as_str(),
                Point::new(3, y + 18),
                ui::body_style_opaque(colour),
            )
            .draw(display);
            s.l2 = l2;
        }
    }

    // Clear any rows left behind when the list shrinks.
    let now = seen.len().min(VISIBLE);
    for i in now..*drawn_rows {
        let _ = display.fill_solid(&row_rect(i), BG);
    }
    *drawn_rows = now;
}

/// Pad to a constant width so an opaque redraw fully covers whatever was
/// underneath; a shorter new string would otherwise leave stale characters.
fn pad(s: &mut String<48>) {
    while s.len() < ui::COLS {
        if s.push(' ').is_err() {
            break;
        }
    }
}
