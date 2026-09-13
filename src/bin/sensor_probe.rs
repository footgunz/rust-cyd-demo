//! Scans the CYD's candidate analog inputs.
//!
//! GPIO 34 is the documented spot for the board's LDR (ADC1 channel 6);
//! GPIO 35 (channel 7) is the other ADC1 pin left free by the display and
//! touch buses, so both are sampled to see which one actually responds.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let mut cfg = AdcConfig::new();
    let mut p34 = cfg.enable_pin(peripherals.GPIO34, Attenuation::_11dB);
    let mut p35 = cfg.enable_pin(peripherals.GPIO35, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, cfg);

    let delay = Delay::new();
    println!();
    println!("=== analog scan: GPIO34, GPIO35 ===");

    loop {
        let a: u16 = nb::block!(adc.read_oneshot(&mut p34)).unwrap_or(0);
        let b: u16 = nb::block!(adc.read_oneshot(&mut p35)).unwrap_or(0);
        println!("gpio34={a:>4}  gpio35={b:>4}");
        delay.delay_millis(250);
    }
}
