//! Flock ALPR camera detector screen.
//!
//! Enables Wi-Fi promiscuous mode, hops the channels Flock cameras use, and
//! lists any transmitter whose OUI is a known Flock prefix. See `crate::flock`
//! for the detection method and its provenance.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;
use esp_println::println;
use esp_radio::wifi::sniffer::Sniffer;
use esp_radio::wifi::{SecondaryChannel, WifiController};
use heapless::String;

use crate::flock::{self, Hit, MAX_HITS};
use crate::touch::{Calibration, Xpt2046};
use crate::ui::{self, BAD, BG, DIM, GOOD, INK, W};

const ROW_H: i32 = 24;
const VISIBLE: usize = 8;
const LIST_TOP: i32 = ui::HEADER_H + 22;
/// Ticks (~30ms) between channel hops. Matches the camera's ~125ms hop; two
/// dwell periods per channel, as flock-you found, catches it sooner.
const HOP_TICKS: u32 = 8;

pub fn run<D, SPI, IRQ>(
    display: &mut D,
    touch: &mut Xpt2046<SPI, IRQ>,
    delay: &mut Delay,
    cal: Calibration,
    wifi: &mut WifiController<'_>,
    sniffer: &mut Sniffer<'_>,
) where
    D: DrawTarget<Color = Rgb565>,
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    let back = ui::header(display, "FLOCK DETECT");
    ui::clear_body(display);
    let _ = Text::new(
        "passive sniff - hopping 11/6/1",
        Point::new(6, ui::HEADER_H + 14),
        ui::body_style(DIM),
    )
    .draw(display);

    flock::reset();
    sniffer.set_receive_cb(flock::on_frame);
    if sniffer.set_promiscuous_mode(true).is_err() {
        println!("flock: could not enter promiscuous mode");
    }

    let mut tick: u32 = 0;
    let mut hop_i: usize = 0;
    let _ = wifi.set_channel(flock::HOP_CHANNELS[0], SecondaryChannel::None);

    let mut last_rev = u32::MAX;
    let mut last_head: String<48> = String::new();
    let mut drawn: usize = 0;

    loop {
        // Hop channels so a camera on any of the three is seen.
        if tick % HOP_TICKS == 0 {
            hop_i = (hop_i + 1) % flock::HOP_CHANNELS.len();
            let _ = wifi.set_channel(flock::HOP_CHANNELS[hop_i], SecondaryChannel::None);
        }

        // Snapshot the shared table under a short critical section, then draw
        // outside it so the SPI writes never block the sniffer callback.
        let (hits, frames, rev) = snapshot();
        if rev != last_rev {
            render(display, &hits, frames, &mut drawn, &mut last_head);
            last_rev = rev;
        }

        if let Some(s) = touch.sample() {
            let (mx, my) = cal.map((s.x, s.y));
            let p = Point::new(mx.clamp(0, W - 1), my.clamp(0, ui::H - 1));
            if ui::hit(back, p) {
                // Leave promiscuous mode so the other radio screens behave.
                let _ = sniffer.set_promiscuous_mode(false);
                ui::flash(display, back, delay);
                ui::wait_release(touch, delay);
                return;
            }
        }

        tick += 1;
        delay.delay_millis(30);
    }
}

fn snapshot() -> ([Option<Hit>; MAX_HITS], u32, u32) {
    critical_section::with(|cs| {
        let mut state = flock::FLOCK.borrow_ref_mut(cs);
        // Clear the dirty flags as we copy, so a row is only redrawn once per
        // change.
        let out = state.hits;
        for slot in state.hits.iter_mut().flatten() {
            slot.dirty = false;
        }
        (out, state.frames, state.revision)
    })
}

fn render<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    hits: &[Option<Hit>; MAX_HITS],
    frames: u32,
    drawn: &mut usize,
    last_head: &mut String<48>,
) {
    let count = hits.iter().flatten().count();

    let mut head: String<48> = String::new();
    let _ = core::fmt::Write::write_fmt(
        &mut head,
        format_args!("{count} cameras   {frames} frames"),
    );
    if head != *last_head {
        pad(&mut head);
        let _ = Text::new(
            head.as_str(),
            Point::new(6, ui::HEADER_H + 12),
            ui::body_style_opaque(if count > 0 { BAD } else { INK }),
        )
        .draw(display);
        *last_head = head;
    }

    if count == 0 {
        let _ = Text::new(
            "no Flock hardware detected",
            Point::new(6, LIST_TOP + 10),
            ui::body_style_opaque(GOOD),
        )
        .draw(display);
        *drawn = 0;
        return;
    }

    let mut i = 0usize;
    for h in hits.iter().flatten().take(VISIBLE) {
        let y = LIST_TOP + i as i32 * ROW_H;
        let m = h.mac;

        let mut l1: String<48> = String::new();
        let _ = core::fmt::Write::write_fmt(
            &mut l1,
            format_args!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  ch{:<2} {:>4}",
                m[0], m[1], m[2], m[3], m[4], m[5], h.channel, h.rssi
            ),
        );
        pad(&mut l1);
        let _ = Text::new(
            l1.as_str(),
            Point::new(6, y + 9),
            ui::body_style_opaque(BAD),
        )
        .draw(display);

        let mut l2: String<48> = String::new();
        let _ = core::fmt::Write::write_fmt(
            &mut l2,
            format_args!("   Flock OUI match   x{}", h.count),
        );
        pad(&mut l2);
        let _ = Text::new(
            l2.as_str(),
            Point::new(6, y + 18),
            ui::body_style_opaque(DIM),
        )
        .draw(display);
        i += 1;
    }

    // Clear rows left behind if the count fell (e.g. after a reset).
    for j in i..*drawn {
        let _ = display.fill_solid(
            &Rectangle::new(
                Point::new(0, LIST_TOP + j as i32 * ROW_H),
                Size::new(W as u32, ROW_H as u32),
            ),
            BG,
        );
    }
    *drawn = i;
}

/// Pad to a constant width so opaque redraws cover any longer previous text.
fn pad(s: &mut String<48>) {
    while s.len() < ui::COLS {
        if s.push(' ').is_err() {
            break;
        }
    }
}
