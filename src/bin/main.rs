#![no_std]
#![no_main]
#![deny(clippy::mem_forget, reason = "esp_hal types may hold DMA buffers")]

use embedded_graphics::mono_font::{ascii::FONT_10X20, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use esp_println::println;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ILI9341Rgb565;
use mipidsi::options::{ColorOrder, Orientation};
use mipidsi::Builder;

esp_bootloader_esp_idf::esp_app_desc!();

// ESP32-2432S028R ("CYD") display bus — panel is on HSPI/SPI2.
const LCD_SCLK: u8 = 14;
const LCD_MOSI: u8 = 13;
const LCD_MISO: u8 = 12;
const LCD_CS: u8 = 15;
const LCD_DC: u8 = 2;
const LCD_BL: u8 = 21;

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    println!();
    println!("=== CYD ILI9341 bring-up ===");
    println!(
        "SPI2: sclk={LCD_SCLK} mosi={LCD_MOSI} miso={LCD_MISO} cs={LCD_CS} dc={LCD_DC} bl={LCD_BL}"
    );

    let io = OutputConfig::default();
    // Backlight on first, so a blank panel is distinguishable from a dark one.
    let _backlight = Output::new(peripherals.GPIO21, Level::High, io);
    let cs = Output::new(peripherals.GPIO15, Level::High, io);
    let dc = Output::new(peripherals.GPIO2, Level::Low, io);

    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .expect("spi2")
    .with_sck(peripherals.GPIO14)
    .with_mosi(peripherals.GPIO13)
    .with_miso(peripherals.GPIO12);

    let delay = Delay::new();
    let spi_dev = ExclusiveDevice::new(spi, cs, delay).expect("exclusive device");

    let mut dma_buf = [0u8; 512];
    let di = SpiInterface::new(spi_dev, dc, &mut dma_buf);

    let mut init_delay = Delay::new();
    println!("initialising panel...");
    let mut display = Builder::new(ILI9341Rgb565, di)
        .display_size(240, 320)
        // CYD panels are wired BGR; Rgb here shows red/blue swapped.
        .color_order(ColorOrder::Bgr)
        // This panel's MADCTL MX bit is inverted vs mipidsi's default, so
        // without this the image renders left-right mirrored.
        .orientation(Orientation::new().flip_horizontal())
        .init(&mut init_delay)
        .expect("display init");
    println!("panel initialised");

    display.clear(Rgb565::BLACK).expect("clear");

    // Labelled bars: confirms panel, colour order and orientation in one shot.
    let bars: [(Rgb565, &str); 4] = [
        (Rgb565::RED, "RED"),
        (Rgb565::GREEN, "GREEN"),
        (Rgb565::BLUE, "BLUE"),
        (Rgb565::WHITE, "WHITE"),
    ];
    let bar_h = 40;
    for (i, (colour, _)) in bars.iter().enumerate() {
        Rectangle::new(Point::new(0, (i as i32) * bar_h), Size::new(240, bar_h as u32))
            .into_styled(PrimitiveStyle::with_fill(*colour))
            .draw(&mut display)
            .expect("bar");
    }

    let text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    Text::new("CYD + Rust", Point::new(10, 200), text_style)
        .draw(&mut display)
        .expect("text");
    Text::new("esp-hal no_std", Point::new(10, 230), text_style)
        .draw(&mut display)
        .expect("text");
    // Corner marker: identifies which physical corner is origin (0,0).
    Rectangle::new(Point::new(0, 0), Size::new(20, 20))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
        .draw(&mut display)
        .expect("marker");

    println!("draw complete - check the panel");

    loop {
        init_delay.delay_millis(1000);
    }
}
