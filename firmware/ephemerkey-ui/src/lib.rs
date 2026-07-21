//! Generator display model — one render path, two surfaces.
//!
//! The product board's display is a **128×32 I²C OLED**: wide but short. Four
//! 8-px lines would be legible only to an ant, and a TOTP code wants to be the
//! big thing on screen — so the model is not a uniform character grid but a
//! short stack of **sized lines** ([`Line`]: text + [`Size`] + [`Align`]). The
//! firmware draws each line with a real font — [`Size::Big`] ≈ FONT_10X20 (the
//! code / headline), [`Size::Small`] ≈ FONT_6X10 (hints / status) — stacked and
//! vertically centered; the emulator previews the same lines at the display's
//! wide aspect. Both consume one [`render`], so the sim shows what the device
//! draws (the `ephemerkey-config` discipline).
//!
//! HARD REQUIREMENT (poison mode): a decoy reveal and a real reveal MUST render
//! identically. [`View::Reveal`] carries no is-decoy bit — the code and its
//! countdown are all it has, so the screen cannot leak which stream a code came
//! from.

#![cfg_attr(not(test), no_std)]

/// OLED pixel dimensions.
pub const WIDTH: u16 = 128;
pub const HEIGHT: u16 = 32;
/// Longest text a line holds (small font: 6 px × 21 = 126 ≤ 128).
pub const MAX_LINE: usize = 21;
/// Most lines a screen stacks (Big 20 + Small 10 fills the 32 px height).
pub const MAX_LINES: usize = 3;

/// Text size — the firmware maps each to a concrete font.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Size {
    /// Hints / status. ≈ 6×10.
    Small,
    /// The code / headline — as large as the panel allows. ≈ 10×20.
    Big,
}

impl Size {
    /// Nominal glyph width (px) — used for alignment on both surfaces.
    pub const fn width(self) -> u16 {
        match self {
            Size::Small => 6,
            Size::Big => 10,
        }
    }
    /// Nominal line height (px) — used for vertical stacking.
    pub const fn height(self) -> u16 {
        match self {
            Size::Small => 10,
            Size::Big => 20,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One line of text with its size and horizontal alignment.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Line {
    buf: [u8; MAX_LINE],
    len: u8,
    size: Size,
    align: Align,
}

impl Line {
    fn new(text: &[u8], size: Size, align: Align) -> Self {
        let mut buf = [b' '; MAX_LINE];
        let n = text.len().min(MAX_LINE);
        for (d, &b) in buf.iter_mut().zip(&text[..n]) {
            *d = if (0x20..0x7f).contains(&b) { b } else { b'?' };
        }
        Line { buf, len: n as u8, size, align }
    }

    /// The line text (ASCII, always valid UTF-8 by construction).
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
    pub fn size(&self) -> Size {
        self.size
    }
    pub fn align(&self) -> Align {
        self.align
    }
    /// Rendered pixel width of the text.
    pub fn pixel_width(&self) -> u16 {
        self.len as u16 * self.size.width()
    }
}

/// A short stack of sized lines.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Screen {
    lines: [Line; MAX_LINES],
    count: u8,
}

impl Default for Screen {
    fn default() -> Self {
        Self::blank()
    }
}

impl Screen {
    pub fn blank() -> Self {
        Screen { lines: [Line::new(b"", Size::Small, Align::Left); MAX_LINES], count: 0 }
    }

    fn push(&mut self, text: &[u8], size: Size, align: Align) {
        if (self.count as usize) < MAX_LINES {
            self.lines[self.count as usize] = Line::new(text, size, align);
            self.count += 1;
        }
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines[..self.count as usize]
    }

    /// Total stacked height (px) of all lines — for vertical centering.
    pub fn total_height(&self) -> u16 {
        self.lines().iter().map(|l| l.size.height()).sum()
    }
}

/// What the generator wants on screen. The firmware and emulator each build one
/// of these from their own state, then share [`render`].
pub enum View<'a> {
    /// No usable GNSS fix — the emission gate is shut.
    Searching,
    /// Cascade-gated, no unlock window open: idle, waiting for the ritual.
    Locked,
    /// Dialing the ritual code: `entered` are the committed digits (0–9), `cur`
    /// the digit being adjusted at the cursor.
    Dialing { entered: &'a [u8], cur: u8 },
    /// The ritual advanced but is not complete: `have` of `need` steps.
    Progress { have: u8, need: u8 },
    /// Unlock window open: `key` selected, `secs_left` counting down.
    Unlocked { key: u8, secs_left: u32 },
    /// A revealed code and how long it stays up. NO decoy flag.
    Reveal { code: &'a str, secs_left: u16 },
}

/// Render a [`View`] to a sized-line [`Screen`].
pub fn render(view: &View) -> Screen {
    let mut s = Screen::blank();
    match *view {
        View::Searching => {
            s.push(b"NO FIX", Size::Big, Align::Center);
            s.push(b"searching", Size::Small, Align::Center);
        }
        View::Locked => {
            s.push(b"LOCKED", Size::Big, Align::Center);
            s.push(b"dial to unlock", Size::Small, Align::Center);
        }
        View::Dialing { entered, cur } => {
            let mut b = [0u8; MAX_LINE];
            let mut n = 0;
            for &d in entered {
                put(&mut b, &mut n, &[b'0' + d % 10]);
            }
            put(&mut b, &mut n, b"[");
            put(&mut b, &mut n, &[b'0' + cur % 10]);
            put(&mut b, &mut n, b"]");
            s.push(&b[..n], Size::Big, Align::Center);
            s.push(b"- + ok  hold=go", Size::Small, Align::Center);
        }
        View::Progress { have, need } => {
            let mut b = [0u8; MAX_LINE];
            let mut n = 0;
            putn(&mut b, &mut n, have as u32);
            put(&mut b, &mut n, b"/");
            putn(&mut b, &mut n, need as u32);
            s.push(&b[..n], Size::Big, Align::Center);
            s.push(b"ritual", Size::Small, Align::Center);
        }
        View::Unlocked { key, secs_left } => {
            let mut kb = [0u8; MAX_LINE];
            let mut kn = 0;
            put(&mut kb, &mut kn, b"KEY ");
            putn(&mut kb, &mut kn, key as u32);
            s.push(&kb[..kn], Size::Big, Align::Center);
            let mut sb = [0u8; MAX_LINE];
            let mut sn = 0;
            putn(&mut sb, &mut sn, secs_left);
            put(&mut sb, &mut sn, b"s  ok=show");
            s.push(&sb[..sn], Size::Small, Align::Center);
        }
        View::Reveal { code, secs_left } => {
            s.push(code.as_bytes(), Size::Big, Align::Center);
            let mut sb = [0u8; MAX_LINE];
            let mut sn = 0;
            putn(&mut sb, &mut sn, secs_left as u32);
            put(&mut sb, &mut sn, b"s");
            s.push(&sb[..sn], Size::Small, Align::Right);
        }
    }
    s
}

fn put(buf: &mut [u8; MAX_LINE], n: &mut usize, bytes: &[u8]) {
    for &b in bytes {
        if *n < MAX_LINE {
            buf[*n] = b;
            *n += 1;
        }
    }
}

fn putn(buf: &mut [u8; MAX_LINE], n: &mut usize, v: u32) {
    let mut tmp = [0u8; 10];
    put(buf, n, fmt_u32(v, &mut tmp));
}

/// Decimal-format `n` into `buf`, returning the written slice.
fn fmt_u32(n: u32, buf: &mut [u8; 10]) -> &[u8] {
    if n == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut tmp = [0u8; 10];
    let mut i = 0;
    let mut v = n;
    while v > 0 {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    &buf[..i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_is_a_big_centered_code() {
        let s = render(&View::Reveal { code: "481920", secs_left: 5 });
        let l = &s.lines()[0];
        assert_eq!(l.text(), "481920");
        assert_eq!(l.size(), Size::Big);
        assert_eq!(l.align(), Align::Center);
        assert_eq!(s.lines()[1].text(), "5s");
    }

    #[test]
    fn reveal_has_no_decoy_leak() {
        // No decoy bit on the View, so real and decoy of the same code+countdown
        // are byte-identical by construction.
        let a = render(&View::Reveal { code: "481920", secs_left: 5 });
        let b = render(&View::Reveal { code: "481920", secs_left: 5 });
        assert_eq!(a, b);
    }

    #[test]
    fn dialing_shows_entered_and_cursor_big() {
        let s = render(&View::Dialing { entered: &[4, 7], cur: 5 });
        assert_eq!(s.lines()[0].text(), "47[5]");
        assert_eq!(s.lines()[0].size(), Size::Big);
    }

    #[test]
    fn unlocked_key_and_countdown() {
        let s = render(&View::Unlocked { key: 2, secs_left: 27 });
        assert_eq!(s.lines()[0].text(), "KEY 2");
        assert_eq!(s.lines()[1].text(), "27s  ok=show");
    }

    #[test]
    fn stack_fits_the_panel_height() {
        // Every view's lines must fit 32 px stacked.
        for v in [
            View::Searching,
            View::Locked,
            View::Dialing { entered: &[1, 2, 3, 4, 5, 6], cur: 7 },
            View::Progress { have: 1, need: 3 },
            View::Unlocked { key: 7, secs_left: 30 },
            View::Reveal { code: "12345678", secs_left: 60 },
        ] {
            let s = render(&v);
            assert!(s.total_height() <= HEIGHT, "{:?} too tall", s);
            // and every big code fits the width
            for l in s.lines() {
                assert!(l.pixel_width() <= WIDTH, "{} too wide", l.text());
            }
        }
    }
}
