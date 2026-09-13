#![no_std]

extern crate alloc;

pub mod apps;
pub mod ble;
pub mod calib;
pub mod menu;
pub mod touch;
pub mod ui;

/// Pin assignments for the ESP32-2432S028R, better known as the
/// "Cheap Yellow Display" (CYD).
///
/// These are recorded as constants for documentation; esp-hal addresses pins
/// as typed peripheral fields (`peripherals.GPIO14`), so they cannot be looked
/// up by number. Every value here was confirmed on real hardware.
pub mod pins {
    // ILI9341 panel — SPI2 (HSPI).
    pub const LCD_SCLK: u8 = 14;
    pub const LCD_MOSI: u8 = 13;
    pub const LCD_MISO: u8 = 12;
    pub const LCD_CS: u8 = 15;
    pub const LCD_DC: u8 = 2;
    pub const LCD_BL: u8 = 21;

    // XPT2046 touch — SPI3 (VSPI). Deliberately a *separate* bus from the
    // panel on this board, which is what breaks most shared-bus examples.
    pub const TOUCH_SCLK: u8 = 25;
    pub const TOUCH_MOSI: u8 = 32;
    pub const TOUCH_MISO: u8 = 39; // input-only pin
    pub const TOUCH_CS: u8 = 33;
    pub const TOUCH_IRQ: u8 = 36; // input-only pin, active low

    // On-board RGB LED, common anode (drive LOW to light).
    // NOTE: the red channel is dead on this particular unit.
    pub const LED_RED: u8 = 4;
    pub const LED_GREEN: u8 = 16;
    pub const LED_BLUE: u8 = 17;

    // Audio: GPIO 26 is DAC2 and feeds the on-board amp and speaker header.
    // (DAC1 is GPIO 25, which this board uses for the touch clock.)
    pub const SPEAKER: u8 = 26;

    // Analog inputs left free by the other buses.
    pub const LDR: u8 = 34; // documented LDR pin; unpopulated on this unit
    pub const AUX_ADC: u8 = 35; // unconnected, free for an external sensor
}
