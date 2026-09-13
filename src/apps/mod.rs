//! The individual screens reachable from the main menu.
//!
//! Each app runs its own loop and returns when the user presses BACK, at
//! which point the menu redraws. Hardware that needs concrete esp-hal types
//! (the LEDs, the ADC) is reached through closures supplied by `main`, which
//! keeps this module free of peripheral plumbing.

pub mod analog;
pub mod blescan;
pub mod leds;
pub mod paint;
pub mod touchdiag;
pub mod wifi;
