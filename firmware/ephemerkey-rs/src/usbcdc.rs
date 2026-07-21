//! Shared USB CDC-ACM device setup for the two consoles that use it — the
//! provisioning transport ([`crate::usbprov`]) and the lock code console
//! ([`crate::lockconsole`]). They differ only in the product string; the
//! VID/PID, power, packet size, and the embassy `Builder` dance are identical.

use embassy_stm32::peripherals;
use embassy_stm32::usb::Driver;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::{Builder, Config, UsbDevice};

/// USB FS bulk/interrupt max packet size for CDC-ACM.
pub const PACKET: usize = 64;

type UsbDriver<'d> = Driver<'d, peripherals::USB>;

/// Descriptor + control buffers the USB device borrows for its whole lifetime.
/// Declare one as a task local and pass it to [`cdc`].
pub struct CdcBuffers {
    config_descriptor: [u8; 128],
    bos_descriptor: [u8; 32],
    control_buf: [u8; 64],
    msos_descriptor: [u8; 0],
}

impl CdcBuffers {
    pub const fn new() -> Self {
        Self {
            config_descriptor: [0; 128],
            bos_descriptor: [0; 32],
            control_buf: [0; 64],
            msos_descriptor: [],
        }
    }
}

impl Default for CdcBuffers {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the CDC-ACM device + class. `buffers` and `state` are borrowed for the
/// device's lifetime, so both must outlive the returned pair — declare them as
/// task locals immediately before the call.
pub fn cdc<'d>(
    driver: UsbDriver<'d>,
    product: &'static str,
    buffers: &'d mut CdcBuffers,
    state: &'d mut State<'d>,
) -> (UsbDevice<'d, UsbDriver<'d>>, CdcAcmClass<'d, UsbDriver<'d>>) {
    // TODO: replace the placeholder VID/PID before any public release. 0x1209
    // is pid.codes (community/test space); 0x0001 is its "in development" PID.
    let mut config = Config::new(0x1209, 0x0001);
    config.manufacturer = Some("ephemerkey");
    config.product = Some(product);
    config.max_power = 100;
    config.max_packet_size_0 = PACKET as u8;

    let mut builder = Builder::new(
        driver,
        config,
        &mut buffers.config_descriptor,
        &mut buffers.bos_descriptor,
        &mut buffers.msos_descriptor,
        &mut buffers.control_buf,
    );
    let class = CdcAcmClass::new(&mut builder, state, PACKET as u16);
    let device = builder.build();
    (device, class)
}
