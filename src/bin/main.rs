//! CYD Kitchen Sink — a menu-driven tour of the ESP32-2432S028R's hardware,
//! in no_std Rust on esp-hal.
//!
//! Brings up both SPI buses (ILI9341 panel on SPI2, XPT2046 touch on SPI3),
//! the ADC, the RGB LED and the radio, then dispatches to one screen at a
//! time from a menu.

#![no_std]
#![no_main]
#![deny(clippy::mem_forget, reason = "esp_hal types may hold DMA buffers")]

extern crate alloc;

use cyd_rust::menu::Choice;
use cyd_rust::touch::Xpt2046;
use cyd_rust::{apps, calib, menu};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace as _;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::Config as WifiConfig;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ILI9341Rgb565;
use mipidsi::options::{ColorOrder, Orientation};
use mipidsi::Builder;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);

    println!();
    println!("=== CYD Kitchen Sink ===");

    let out = OutputConfig::default();
    let _backlight = Output::new(peripherals.GPIO21, Level::High, out);

    // --- Panel: ILI9341 on SPI2 (HSPI) ------------------------------------
    let lcd_spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .expect("spi2")
    .with_sck(peripherals.GPIO14)
    .with_mosi(peripherals.GPIO13)
    .with_miso(peripherals.GPIO12);

    let lcd_dev = ExclusiveDevice::new(
        lcd_spi,
        Output::new(peripherals.GPIO15, Level::High, out),
        Delay::new(),
    )
    .expect("lcd device");

    let mut lcd_buf = [0u8; 512];
    let di = SpiInterface::new(
        lcd_dev,
        Output::new(peripherals.GPIO2, Level::Low, out),
        &mut lcd_buf,
    );

    let mut delay = Delay::new();
    let mut display = Builder::new(ILI9341Rgb565, di)
        .display_size(240, 320)
        // Verified on hardware: this panel is BGR, and its MADCTL MX bit is
        // inverted relative to mipidsi's default.
        .color_order(ColorOrder::Bgr)
        .orientation(Orientation::new().flip_horizontal())
        .init(&mut delay)
        .expect("display init");

    // --- Touch: XPT2046 on SPI3 (VSPI) ------------------------------------
    let touch_spi = Spi::new(
        peripherals.SPI3,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(2))
            .with_mode(Mode::_0),
    )
    .expect("spi3")
    .with_sck(peripherals.GPIO25)
    .with_mosi(peripherals.GPIO32)
    .with_miso(peripherals.GPIO39);

    let touch_dev = ExclusiveDevice::new(
        touch_spi,
        Output::new(peripherals.GPIO33, Level::High, out),
        Delay::new(),
    )
    .expect("touch device");

    // GPIO 36 has no internal pull resistor; the board supplies its own
    // pull-up on PENIRQ, which measures clean.
    let mut touch = Xpt2046::new(
        touch_dev,
        Input::new(
            peripherals.GPIO36,
            InputConfig::default().with_pull(Pull::Up),
        ),
    );

    // --- LEDs: common anode, so LOW lights -------------------------------
    let mut leds = [
        Output::new(peripherals.GPIO4, Level::High, out),
        Output::new(peripherals.GPIO16, Level::High, out),
        Output::new(peripherals.GPIO17, Level::High, out),
    ];

    // --- ADC --------------------------------------------------------------
    let mut adc_cfg = AdcConfig::new();
    let mut pin34 = adc_cfg.enable_pin(peripherals.GPIO34, Attenuation::_11dB);
    let mut pin35 = adc_cfg.enable_pin(peripherals.GPIO35, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_cfg);

    // --- Radio: Wi-Fi and BLE share the scheduler -------------------------
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let (mut wifi, mut interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default()).expect("wifi init");
    // Station mode with no SSID: enough to scan, never associates.
    wifi.set_config(&WifiConfig::Station(StationConfig::default()))
        .expect("wifi config");

    let mut ble = BleConnector::new(peripherals.BT, Default::default()).expect("ble init");

    let mut cal = calib::run(&mut display, &mut touch, &mut delay);
    println!(
        "calibrated, axes {}",
        if cal.is_swapped() { "swapped" } else { "direct" }
    );

    loop {
        match menu::run(&mut display, &mut touch, &mut delay, cal) {
            Choice::Paint => apps::paint::run(&mut display, &mut touch, &mut delay, cal),
            Choice::WifiScan => {
                apps::wifi::run(&mut display, &mut touch, &mut delay, cal, &mut wifi)
            }
            Choice::BleScan => {
                apps::blescan::run(&mut display, &mut touch, &mut delay, cal, &mut ble)
            }
            Choice::Flock => apps::flock::run(
                &mut display,
                &mut touch,
                &mut delay,
                cal,
                &mut wifi,
                &mut interfaces.sniffer,
            ),
            Choice::TouchDiag => apps::touchdiag::run(&mut display, &mut touch, &mut delay, cal),
            Choice::Analog => {
                // The ADC needs concrete esp-hal types, so the app reaches it
                // through a closure rather than owning the peripheral.
                let mut read = || {
                    let a: u16 = nb::block!(adc.read_oneshot(&mut pin34)).unwrap_or(0);
                    let b: u16 = nb::block!(adc.read_oneshot(&mut pin35)).unwrap_or(0);
                    (a, b)
                };
                apps::analog::run(&mut display, &mut touch, &mut delay, cal, &mut read)
            }
            Choice::Leds => {
                let mut set = |i: usize, on: bool| {
                    leds[i].set_level(if on { Level::Low } else { Level::High });
                };
                apps::leds::run(&mut display, &mut touch, &mut delay, cal, &mut set)
            }
            Choice::Calibrate => {
                cal = calib::run(&mut display, &mut touch, &mut delay);
            }
        }
    }
}
