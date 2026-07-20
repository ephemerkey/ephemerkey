//! SSD1306 128×32 OLED (I²C1 @ 0x3C): the generator's dial / reveal display.
//!
//! Renders the shared [`ephemerkey_ui::Screen`] — `embedded-graphics` supplies
//! the glyphs (FONT_5X8), `ephemerkey-ui` the 21×4 layout. Each cell is placed
//! on a 6-px pitch so the on-device geometry matches the grid the emulator
//! paints. The OLED is optional: [`Oled::new`] returns `None` if the panel
//! doesn't ACK, so a generator with no display attached still runs (logging).

use embassy_stm32::i2c::mode::Master;
use embassy_stm32::i2c::I2c;
use embassy_stm32::mode::Blocking;
use embedded_graphics::mono_font::{ascii::FONT_5X8, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use ephemerkey_ui::{Screen, COLS, ROWS};
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};

type Display<'d> = Ssd1306<
    I2CInterface<I2c<'d, Blocking, Master>>,
    DisplaySize128x32,
    ssd1306::mode::BufferedGraphicsMode<DisplaySize128x32>,
>;

pub struct Oled<'d> {
    dp: Display<'d>,
    style: MonoTextStyle<'static, BinaryColor>,
}

impl<'d> Oled<'d> {
    /// Bring up the panel. `None` if it doesn't initialize (not populated).
    pub fn new(i2c: I2c<'d, Blocking, Master>) -> Option<Self> {
        let iface = I2CDisplayInterface::new(i2c);
        let mut dp = Ssd1306::new(iface, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        dp.init().ok()?;
        let style = MonoTextStyle::new(&FONT_5X8, BinaryColor::On);
        Some(Self { dp, style })
    }

    /// Blit a rendered [`Screen`] to the panel. Cells are drawn on a 6-px pitch
    /// (row height 8), so the 21×4 grid maps to the full 128×32 face.
    pub fn show(&mut self, screen: &Screen) {
        let _ = self.dp.clear(BinaryColor::Off);
        for r in 0..ROWS {
            for c in 0..COLS {
                let ch = screen.cells[r][c];
                if ch == b' ' {
                    continue;
                }
                let s = [ch];
                if let Ok(g) = core::str::from_utf8(&s) {
                    let _ = Text::with_baseline(
                        g,
                        Point::new((c * 6) as i32, (r * 8) as i32),
                        self.style,
                        Baseline::Top,
                    )
                    .draw(&mut self.dp);
                }
            }
        }
        let _ = self.dp.flush();
    }
}
