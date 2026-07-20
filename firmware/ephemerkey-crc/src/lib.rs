//! Shared checksums for ephemerkey. One implementation so the wire uses (the
//! config-transfer checksum in `ephemerkey-provision`, mirrored by the emulator
//! device) and the flash-journal record CRC (`ephemerkey-store`) can never
//! drift apart. no_std, no-alloc, zero dependencies.

#![no_std]

/// CRC-32/ISO-HDLC (a.k.a. "CRC-32", zlib/PNG): reflected, poly `0xEDB88320`,
/// init/xorout `0xFFFFFFFF`. Bytewise reference implementation.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(crc32(b""), 0x0000_0000);
        // The canonical "123456789" check value for CRC-32/ISO-HDLC.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
