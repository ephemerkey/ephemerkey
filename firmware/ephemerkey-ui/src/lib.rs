//! Generator display model — one render path, two surfaces.
//!
//! The product board's display is a 128×32 I²C OLED. At a 6×8 font that is a
//! **21×4 character grid** ([`COLS`]×[`ROWS`]). This crate turns a [`View`] (what
//! the generator wants to show) into a [`Screen`] (that grid, ASCII). The
//! firmware paints the grid to the OLED through a font; the emulator prints it
//! to the terminal inside a little bezel. Because both consume the *same*
//! render, the emulator shows exactly what the device will — no divergence, the
//! same discipline as `ephemerkey-config`.
//!
//! HARD REQUIREMENT (poison mode): a decoy reveal and a real reveal MUST render
//! identically. [`View::Reveal`] therefore carries no is-decoy bit — the code
//! and its countdown are all it has, so the screen cannot leak which stream a
//! code came from.

#![cfg_attr(not(test), no_std)]

/// Character columns (128 px / 6 px per glyph).
pub const COLS: usize = 21;
/// Character rows (32 px / 8 px per glyph).
pub const ROWS: usize = 4;

/// A rendered screen: `ROWS` lines of `COLS` ASCII bytes, space = blank pixel
/// cell. The firmware maps each cell through a font; the emulator prints it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Screen {
    pub cells: [[u8; COLS]; ROWS],
}

impl Default for Screen {
    fn default() -> Self {
        Self::blank()
    }
}

impl Screen {
    pub fn blank() -> Self {
        Screen { cells: [[b' '; COLS]; ROWS] }
    }

    /// Row `r` as a `&str`, trailing blanks trimmed (for the emulator / tests).
    pub fn row_str(&self, r: usize) -> &str {
        let row = &self.cells[r];
        let end = row.iter().rposition(|&c| c != b' ').map_or(0, |i| i + 1);
        // The grid is built from ASCII only, so this is always valid UTF-8.
        core::str::from_utf8(&row[..end]).unwrap_or("")
    }

    /// Write `s` at (row, col), clipping at the right edge. Non-ASCII/control
    /// bytes are drawn as '?', so a stray byte can't desync the grid.
    fn text(&mut self, row: usize, col: usize, s: &[u8]) {
        if row >= ROWS {
            return;
        }
        for (i, &b) in s.iter().enumerate() {
            let c = col + i;
            if c >= COLS {
                break;
            }
            self.cells[row][c] = if (0x20..0x7f).contains(&b) { b } else { b'?' };
        }
    }

    /// Write `s` centered on `row`.
    fn center(&mut self, row: usize, s: &[u8]) {
        let col = COLS.saturating_sub(s.len()) / 2;
        self.text(row, col, s);
    }

    /// Write a decimal number at (row, col); returns the column just past it.
    fn num(&mut self, row: usize, col: usize, n: u32) -> usize {
        let mut buf = [0u8; 10];
        let s = fmt_u32(n, &mut buf);
        self.text(row, col, s);
        col + s.len()
    }
}

/// What the generator wants on screen. Constructed by the firmware task / the
/// emulator from the same state, so the render is shared.
pub enum View<'a> {
    /// No usable GNSS fix yet — the emission gate is shut.
    Searching,
    /// Cascade-gated and no unlock window open: idle, waiting for the ritual.
    Locked,
    /// Dialing the ritual code: `entered` are the committed digits (0–9),
    /// `cur` is the digit currently being adjusted at the cursor.
    Dialing { entered: &'a [u8], cur: u8 },
    /// The ritual advanced but is not complete: `have` of `need` steps.
    Progress { have: u8, need: u8 },
    /// Unlock window open. `key` is the selected key; `secs_left` counts the
    /// window down.
    Unlocked { key: u8, secs_left: u32 },
    /// A revealed code and how long it stays up. Carries NO decoy flag.
    Reveal { code: &'a str, secs_left: u16 },
}

/// Render a [`View`] to the 21×4 [`Screen`].
pub fn render(view: &View) -> Screen {
    let mut s = Screen::blank();
    match *view {
        View::Searching => {
            s.center(0, b"SEARCHING");
            s.center(2, b"no GPS fix");
        }
        View::Locked => {
            s.center(0, b"LOCKED");
            s.center(2, b"dial to unlock");
        }
        View::Dialing { entered, cur } => {
            s.text(0, 0, b"UNLOCK RITUAL");
            // committed digits, space-separated, then the current digit in [ ].
            let mut col = 0usize;
            for &d in entered {
                col = s.num(1, col, (d % 10) as u32) + 1;
            }
            s.text(1, col, b"[");
            col = s.num(1, col + 1, (cur % 10) as u32);
            s.text(1, col, b"]");
            s.text(2, 0, b"- digit +   OK:next");
            s.text(3, 0, b"hold OK = submit");
        }
        View::Progress { have, need } => {
            s.center(0, b"RITUAL");
            let mut buf = [0u8; 10];
            let mut line = [b' '; COLS];
            let mut w = 0usize;
            for &b in b"step " {
                line[w] = b;
                w += 1;
            }
            for &b in fmt_u32(have as u32, &mut buf) {
                line[w] = b;
                w += 1;
            }
            line[w] = b'/';
            w += 1;
            for &b in fmt_u32(need as u32, &mut buf) {
                line[w] = b;
                w += 1;
            }
            s.center(2, &line[..w]);
        }
        View::Unlocked { key, secs_left } => {
            s.text(0, 0, b"UNLOCKED");
            let col = s.num(0, 10, secs_left);
            s.text(0, col, b"s");
            s.text(1, 0, b"key ");
            s.num(1, 4, key as u32);
            s.text(2, 0, b"< key >");
            s.text(3, 0, b"OK = reveal");
        }
        View::Reveal { code, secs_left } => {
            s.center(0, b"CODE");
            s.center(1, code.as_bytes());
            let col = s.num(3, 0, secs_left as u32);
            s.text(3, col, b"s");
        }
    }
    s
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
    fn dialing_shows_entered_and_current() {
        let s = render(&View::Dialing { entered: &[4, 7], cur: 5 });
        assert_eq!(s.row_str(0), "UNLOCK RITUAL");
        assert_eq!(s.row_str(1), "4 7 [5]");
    }

    #[test]
    fn reveal_has_no_decoy_leak() {
        // The View can't carry a decoy bit, so real and "decoy" renders of the
        // same code+countdown are byte-identical by construction.
        let a = render(&View::Reveal { code: "481920", secs_left: 5 });
        let b = render(&View::Reveal { code: "481920", secs_left: 5 });
        assert_eq!(a, b);
        assert_eq!(a.row_str(1).trim(), "481920"); // centered
        assert_eq!(a.row_str(3), "5s");
    }

    #[test]
    fn unlocked_counts_down() {
        let s = render(&View::Unlocked { key: 2, secs_left: 27 });
        assert_eq!(s.row_str(0), "UNLOCKED  27s");
        assert_eq!(s.row_str(1), "key 2");
    }

    #[test]
    fn progress_step_of() {
        let s = render(&View::Progress { have: 1, need: 3 });
        assert_eq!(s.row_str(2).trim(), "step 1/3"); // centered
    }

    #[test]
    fn rows_never_exceed_width() {
        for v in [
            View::Dialing { entered: &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0], cur: 9 },
            View::Unlocked { key: 7, secs_left: 999_999 },
            View::Reveal { code: "1234567890", secs_left: 60 },
        ] {
            let s = render(&v);
            for r in 0..ROWS {
                assert!(s.cells[r].len() == COLS);
            }
        }
    }
}
