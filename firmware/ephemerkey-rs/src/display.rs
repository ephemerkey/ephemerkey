//! SSD1306 128×32 OLED (I²C1 @ 0x3C): the generator's dial / reveal display.
//!
//! Renders the shared [`ephemerkey_ui::Screen`] — `embedded-graphics` supplies
//! the glyphs (FONT_5X8), `ephemerkey-ui` the 21×4 layout. Each cell is placed
//! on a 6-px pitch so the on-device geometry matches the grid the emulator
//! paints. The OLED is optional: [`Oled::new`] returns `None` if the panel
//! doesn't ACK, so a generator with no display attached still runs (logging).

use embedded_graphics::mono_font::{ascii::FONT_10X20, ascii::FONT_6X10, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use ephemerkey_ui::{Align, Screen, Size, HEIGHT, WIDTH};
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};

type Display = Ssd1306<
    I2CInterface<crate::I2c1Dev>,
    DisplaySize128x32,
    ssd1306::mode::BufferedGraphicsMode<DisplaySize128x32>,
>;

pub struct Oled {
    dp: Display,
    big: MonoTextStyle<'static, BinaryColor>,
    small: MonoTextStyle<'static, BinaryColor>,
}

impl Oled {
    /// Bring up the panel. `None` if it doesn't initialize (not populated).
    pub fn new(dev: crate::I2c1Dev) -> Option<Self> {
        let iface = I2CDisplayInterface::new(dev);
        let mut dp = Ssd1306::new(iface, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        dp.init().ok()?;
        Some(Self {
            dp,
            big: MonoTextStyle::new(&FONT_10X20, BinaryColor::On),
            small: MonoTextStyle::new(&FONT_6X10, BinaryColor::On),
        })
    }

    /// Blit a rendered [`Screen`]: stack its sized lines, vertically centered,
    /// each aligned horizontally — the code big, hints small.
    pub fn show(&mut self, screen: &Screen) {
        let _ = self.dp.clear(BinaryColor::Off);
        let mut y = ((HEIGHT as i32 - screen.total_height() as i32) / 2).max(0);
        for line in screen.lines() {
            let w = line.pixel_width() as i32;
            let x = match line.align() {
                Align::Left => 0,
                Align::Center => ((WIDTH as i32 - w) / 2).max(0),
                Align::Right => (WIDTH as i32 - w).max(0),
            };
            let style = match line.size() {
                Size::Big => self.big,
                Size::Small => self.small,
            };
            let _ = Text::with_baseline(line.text(), Point::new(x, y), style, Baseline::Top)
                .draw(&mut self.dp);
            y += line.size().height() as i32;
        }
        let _ = self.dp.flush();
    }
}
