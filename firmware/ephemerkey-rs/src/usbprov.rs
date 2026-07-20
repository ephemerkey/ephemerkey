//! USB FS CDC-ACM provisioning transport. Enumerates only when the device
//! boots in provisioning mode (button held — see `main`), so it is never
//! silently writable. Raw CDC bytes are pumped into the shared
//! `Provisioner::feed`; the framed responses it emits go back out the same
//! pipe. The engine is the same one `ekemu serial` runs, so the console's
//! `/push` flow drives real hardware unchanged.

use embassy_futures::join::join;
use embassy_stm32::usb::Driver;
use embassy_stm32::{peripherals, Peri};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use ephemerkey_frame::MAX_PAYLOAD;
use ephemerkey_store::StoredIdentity;

use crate::provision::{provisioner, DeviceJournal, DeviceProvisioner};
use crate::usbcdc::{cdc, CdcBuffers, PACKET};
use crate::Irqs;

/// Run the provisioning console over USB CDC. Owns the flash journal + identity
/// for the lifetime of provisioning mode.
#[embassy_executor::task]
pub async fn task(
    usb: Peri<'static, peripherals::USB>,
    dp: Peri<'static, peripherals::PA12>,
    dm: Peri<'static, peripherals::PA11>,
    aes: Peri<'static, peripherals::AES>,
    journal: DeviceJournal,
    id: StoredIdentity,
) {
    let driver = Driver::new(usb, Irqs, dp, dm);
    let mut buffers = CdcBuffers::new();
    let mut state = State::new();
    let (mut device, mut class) = cdc(driver, "ephemerkey provisioning", &mut buffers, &mut state);

    // The engine (~14 KiB, or ~18 KiB with the hw-aes scratch) lives here as a
    // task local — in the executor's task arena, never on a call stack.
    let mut prov = provisioner(id, journal, aes);

    let usb_fut = device.run();
    let pump_fut = async {
        loop {
            class.wait_connection().await;
            defmt::info!("provisioning: host connected");
            let _ = pump(&mut class, &mut prov).await;
            defmt::info!("provisioning: host disconnected");
        }
    };
    join(usb_fut, pump_fut).await;
}

/// Read CDC packets, feed the engine, write back its framed responses. A single
/// provisioning frame may span several USB packets — the engine's parser is
/// incremental, so feeding each packet as it arrives is correct.
async fn pump<'d>(
    class: &mut CdcAcmClass<'d, Driver<'d, peripherals::USB>>,
    prov: &mut DeviceProvisioner,
) -> Result<(), EndpointError> {
    let mut rx = [0u8; PACKET];
    loop {
        let n = class.read_packet(&mut rx).await?;

        // feed() may emit up to two response frames; buffer them, then chunk
        // out to the 64-byte USB max packet size.
        let mut out: heapless::Vec<u8, { 2 * (MAX_PAYLOAD + 8) }> = heapless::Vec::new();
        prov.feed(&rx[..n], |resp| {
            let _ = out.extend_from_slice(resp);
        });
        if out.is_empty() {
            continue;
        }
        for chunk in out.chunks(PACKET) {
            class.write_packet(chunk).await?;
        }
        // A response that is an exact multiple of the packet size needs a
        // zero-length packet to signal end-of-transfer to the host.
        if out.len().is_multiple_of(PACKET) {
            class.write_packet(&[]).await?;
        }
    }
}
