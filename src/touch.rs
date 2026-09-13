//! Minimal XPT2046 resistive touch-controller driver.
//!
//! The XPT2046 is a 12-bit SAR ADC behind an SPI shift register. Each
//! measurement is a 3-byte transaction: one control byte out, then two bytes
//! back holding the result left-aligned in the top 12 bits.
//!
//! On the CYD this chip sits on its own SPI bus, independent of the panel.

use embedded_hal::digital::InputPin;
use embedded_hal::spi::SpiDevice;

/// Control bytes. Bit 7 starts a conversion; bits 6:4 select the channel.
const CMD_X: u8 = 0x90;
const CMD_Y: u8 = 0xD0;
const CMD_Z1: u8 = 0xB0;
const CMD_Z2: u8 = 0xC0;

/// Samples taken per axis. The median of these rejects the spikes that a
/// resistive panel produces as the contact settles.
const SAMPLES: usize = 7;

/// Minimum Z1 for a reading to count as a real touch.
///
/// Measured on this board: Z1 idles at 2 untouched and jumps to ~720 under a
/// normal fingertip, so this threshold sits far from both.
const Z1_TOUCH_FLOOR: u16 = 80;

/// One settled touch reading, in raw ADC counts (0..=4095).
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub x: u16,
    pub y: u16,
    /// Pressure proxy: rises as the contact area grows. Not ohms.
    pub z: u16,
}

pub struct Xpt2046<SPI, IRQ> {
    spi: SPI,
    irq: IRQ,
}

impl<SPI, IRQ> Xpt2046<SPI, IRQ>
where
    SPI: SpiDevice,
    IRQ: InputPin,
{
    pub fn new(spi: SPI, irq: IRQ) -> Self {
        Self { spi, irq }
    }

    /// The PENIRQ line is pulled low while the panel is being pressed. Cheaper
    /// and more reliable than inferring contact from the ADC.
    pub fn is_touched(&mut self) -> bool {
        self.irq.is_low().unwrap_or(false)
    }

    fn read_channel(&mut self, cmd: u8) -> u16 {
        let mut buf = [cmd, 0x00, 0x00];
        if self.spi.transfer_in_place(&mut buf).is_err() {
            return 0;
        }
        // Result is left-aligned across the two returned bytes.
        (((buf[1] as u16) << 8) | buf[2] as u16) >> 3
    }

    /// Raw, unfiltered read of every channel plus the IRQ line.
    /// Intended for the `touch_probe` diagnostic, not normal use.
    pub fn probe(&mut self) -> (bool, u16, u16, u16, u16) {
        let irq_low = self.is_touched();
        let y = self.read_channel(CMD_Y);
        let x = self.read_channel(CMD_X);
        let z1 = self.read_channel(CMD_Z1);
        let z2 = self.read_channel(CMD_Z2);
        (irq_low, x, y, z1, z2)
    }

    fn median(values: &mut [u16]) -> u16 {
        values.sort_unstable();
        values[values.len() / 2]
    }

    /// Take a filtered reading, or `None` if the panel is not being touched.
    ///
    /// The touch state is re-checked afterwards so a release part-way through
    /// sampling is discarded rather than reported as a wild coordinate.
    pub fn sample(&mut self) -> Option<Sample> {
        if !self.is_touched() {
            return None;
        }

        let mut xs = [0u16; SAMPLES];
        let mut ys = [0u16; SAMPLES];
        for i in 0..SAMPLES {
            ys[i] = self.read_channel(CMD_Y);
            xs[i] = self.read_channel(CMD_X);
        }
        let z1 = self.read_channel(CMD_Z1);
        let z2 = self.read_channel(CMD_Z2);

        // PENIRQ can glitch low on a bus transient; requiring real plate
        // pressure as well keeps phantom touches out.
        if !self.is_touched() || z1 < Z1_TOUCH_FLOOR {
            return None;
        }

        Some(Sample {
            x: Self::median(&mut xs),
            y: Self::median(&mut ys),
            z: z1.saturating_add(4095u16.saturating_sub(z2)) >> 1,
        })
    }
}

/// Maps raw ADC counts onto screen pixels.
///
/// Built from three touches rather than two: two diagonal points cannot tell
/// an axis swap from an axis inversion, and the touch panel's axes do not
/// always agree with the display's. Sampling a pure-X move and a pure-Y move
/// resolves swap, inversion and scale together.
#[derive(Debug, Clone, Copy)]
pub struct Calibration {
    swap_xy: bool,
    rx0: i32,
    rx1: i32,
    sx0: i32,
    sx1: i32,
    ry0: i32,
    ry1: i32,
    sy0: i32,
    sy1: i32,
}

impl Calibration {
    /// `p0`/`p1` differ only in screen X; `p0`/`p2` differ only in screen Y.
    pub fn from_points(
        raw0: (u16, u16),
        raw1: (u16, u16),
        raw2: (u16, u16),
        p0: (i32, i32),
        p1: (i32, i32),
        p2: (i32, i32),
    ) -> Self {
        // Moving along screen X: whichever raw axis moved more is the one
        // wired to screen X.
        let moved_on_raw_x = (raw1.0 as i32 - raw0.0 as i32).abs();
        let moved_on_raw_y = (raw1.1 as i32 - raw0.1 as i32).abs();
        let swap_xy = moved_on_raw_y > moved_on_raw_x;

        let (rx0, rx1) = if swap_xy {
            (raw0.1 as i32, raw1.1 as i32)
        } else {
            (raw0.0 as i32, raw1.0 as i32)
        };
        let (ry0, ry1) = if swap_xy {
            (raw0.0 as i32, raw2.0 as i32)
        } else {
            (raw0.1 as i32, raw2.1 as i32)
        };

        Self {
            swap_xy,
            rx0,
            rx1,
            sx0: p0.0,
            sx1: p1.0,
            ry0,
            ry1,
            sy0: p0.1,
            sy1: p2.1,
        }
    }

    /// True if the panel's axes are transposed relative to the display.
    pub fn is_swapped(&self) -> bool {
        self.swap_xy
    }

    pub fn map(&self, raw: (u16, u16)) -> (i32, i32) {
        let (along_x, along_y) = if self.swap_xy {
            (raw.1 as i32, raw.0 as i32)
        } else {
            (raw.0 as i32, raw.1 as i32)
        };

        // Spans stay signed: a negative span is simply an inverted axis.
        let span_x = match self.rx1 - self.rx0 {
            0 => 1,
            d => d,
        };
        let span_y = match self.ry1 - self.ry0 {
            0 => 1,
            d => d,
        };

        (
            (along_x - self.rx0) * (self.sx1 - self.sx0) / span_x + self.sx0,
            (along_y - self.ry0) * (self.sy1 - self.sy0) / span_y + self.sy0,
        )
    }
}
