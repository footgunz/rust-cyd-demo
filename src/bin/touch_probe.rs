//! Dumps raw XPT2046 channels and the PENIRQ line over serial.
//!
//! Note that GPIO 34-39 on the ESP32 have no internal pull resistors, so
//! whether PENIRQ idles high depends entirely on the board's own pull-up.
//! This tells you what it actually does.

#![no_std]
#![no_main]

use cyd_rust::touch::Xpt2046;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let out = OutputConfig::default();

    let spi = Spi::new(
        peripherals.SPI3,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(2))
            .with_mode(Mode::_0),
    )
    .expect("spi3")
    .with_sck(peripherals.GPIO25)
    .with_mosi(peripherals.GPIO32)
    .with_miso(peripherals.GPIO39);

    let dev = ExclusiveDevice::new(
        spi,
        Output::new(peripherals.GPIO33, Level::High, out),
        Delay::new(),
    )
    .expect("touch device");

    let irq = Input::new(
        peripherals.GPIO36,
        InputConfig::default().with_pull(Pull::Up),
    );
    let mut touch = Xpt2046::new(dev, irq);
    let delay = Delay::new();

    println!();
    println!("=== XPT2046 probe: irq_low, x, y, z1, z2 ===");

    loop {
        let (irq_low, x, y, z1, z2) = touch.probe();
        println!("irq_low={irq_low} x={x:>4} y={y:>4} z1={z1:>4} z2={z2:>4}");
        delay.delay_millis(250);
    }
}
