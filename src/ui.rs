//! Shared look and feel for the kitchen-sink demo.
//!
//! Every screen is built from these pieces so the apps stay consistent and
//! none of them has to re-derive layout constants.

use embedded_graphics::mono_font::{
    ascii::FONT_6X10, ascii::FONT_9X15_BOLD, MonoTextStyle, MonoTextStyleBuilder,
};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_hal::delay::Delay;

use crate::touch::Xpt2046;

pub const W: i32 = 240;
pub const H: i32 = 320;
pub const HEADER_H: i32 = 30;

/// Slack added around a control when hit-testing. Drawn size is unchanged, so
/// controls stay visually tidy while being far easier to land on.
pub const HIT_SLACK: i32 = 7;

pub const BG: Rgb565 = Rgb565::new(2, 4, 6);
pub const PANEL: Rgb565 = Rgb565::new(5, 10, 12);
pub const INK: Rgb565 = Rgb565::new(26, 53, 28);
pub const DIM: Rgb565 = Rgb565::new(14, 28, 16);
pub const HOT: Rgb565 = Rgb565::new(31, 40, 0);
pub const GOOD: Rgb565 = Rgb565::new(0, 63, 14);
/// Same green as [`GOOD`], named for the calibration ring it confirms.
pub const DONE_RING: Rgb565 = GOOD;
pub const BAD: Rgb565 = Rgb565::new(31, 12, 4);

pub fn title_style() -> MonoTextStyle<'static, Rgb565> {
    MonoTextStyle::new(&FONT_9X15_BOLD, INK)
}
pub fn body_style(colour: Rgb565) -> MonoTextStyle<'static, Rgb565> {
    MonoTextStyle::new(&FONT_6X10, colour)
}

/// Body text that paints its own background.
///
/// Redrawing changing text by clearing the area first and then drawing makes
/// the region visibly blink. An opaque style overwrites the glyph cells in
/// place instead, so a row can update without any flash — provided the string
/// is padded to a constant width, or leftovers from a longer previous string
/// survive underneath.
pub fn body_style_opaque(colour: Rgb565) -> MonoTextStyle<'static, Rgb565> {
    MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(colour)
        .background_color(BG)
        .build()
}

/// FONT_6X10 glyphs are 6px wide, so this many fill the 240px width.
pub const COLS: usize = 39;

pub fn hit(r: Rectangle, p: Point) -> bool {
    let tl = r.top_left;
    p.x >= tl.x - HIT_SLACK
        && p.x < tl.x + r.size.width as i32 + HIT_SLACK
        && p.y >= tl.y - HIT_SLACK
        && p.y < tl.y + r.size.height as i32 + HIT_SLACK
}

pub fn back_button() -> Rectangle {
    Rectangle::new(Point::new(W - 56, 4), Size::new(52, 22))
}

/// Draw a screen header with a BACK control, and return that control's area.
pub fn header<D: DrawTarget<Color = Rgb565>>(display: &mut D, title: &str) -> Rectangle {
    let _ = display.fill_solid(
        &Rectangle::new(Point::zero(), Size::new(W as u32, HEADER_H as u32)),
        PANEL,
    );
    let _ = Text::new(title, Point::new(8, 20), title_style()).draw(display);
    let back = back_button();
    button(display, back, "BACK", DIM);
    back
}

pub fn button<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    r: Rectangle,
    label: &str,
    frame: Rgb565,
) {
    let _ = r
        .into_styled(PrimitiveStyle::with_stroke(frame, 1))
        .draw(display);
    // FONT_6X10 is 6px per glyph; centre the label.
    let tx = r.top_left.x + (r.size.width as i32 - label.len() as i32 * 6) / 2;
    let ty = r.top_left.y + (r.size.height as i32 + 8) / 2;
    let _ = Text::new(label, Point::new(tx, ty), body_style(INK)).draw(display);
}

/// Briefly invert a control so a press is acknowledged.
pub fn flash<D: DrawTarget<Color = Rgb565>>(display: &mut D, r: Rectangle, delay: &mut Delay) {
    let _ = r.into_styled(PrimitiveStyle::with_fill(HOT)).draw(display);
    delay.delay_millis(90);
}

pub fn clear_body<D: DrawTarget<Color = Rgb565>>(display: &mut D) {
    let _ = display.fill_solid(
        &Rectangle::new(
            Point::new(0, HEADER_H),
            Size::new(W as u32, (H - HEADER_H) as u32),
        ),
        BG,
    );
}

/// Block until the finger comes off, so one press cannot act twice.
pub fn wait_release<SPI, IRQ>(touch: &mut Xpt2046<SPI, IRQ>, delay: &mut Delay)
where
    SPI: embedded_hal::spi::SpiDevice,
    IRQ: embedded_hal::digital::InputPin,
{
    while touch.is_touched() {
        delay.delay_millis(10);
    }
    delay.delay_millis(60);
}
