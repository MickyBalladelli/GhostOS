//! PS/2 mouse packet collection for the boot console and user input service.

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

static PACKET: [AtomicU8; 3] = [const { AtomicU8::new(0) }; 3];
static PACKET_INDEX: AtomicU8 = AtomicU8::new(0);
static SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseState {
    pub buttons: u8,
    pub delta_x: i16,
    pub delta_y: i16,
    pub sequence: u32,
}

pub(crate) fn ingest(byte: u8) {
    let index = PACKET_INDEX.load(Ordering::Relaxed) as usize;
    if index == 0 && byte & 0x08 == 0 {
        return
    }
    PACKET[index].store(byte, Ordering::Relaxed);
    if index == 2 {
        PACKET_INDEX.store(0, Ordering::Release);
        SEQUENCE.fetch_add(1, Ordering::Release);
    } else {
        PACKET_INDEX.store((index + 1) as u8, Ordering::Release)
    }
}

pub fn state() -> MouseState {
    loop {
        let before = SEQUENCE.load(Ordering::Acquire);
        let flags = PACKET[0].load(Ordering::Relaxed);
        let x = PACKET[1].load(Ordering::Relaxed);
        let y = PACKET[2].load(Ordering::Relaxed);
        let after = SEQUENCE.load(Ordering::Acquire);
        if before == after {
            let delta_x = if flags & 0x10 != 0 {
                i16::from(x) - 256
            } else {
                i16::from(x)
            };
            let raw_y = if flags & 0x20 != 0 {
                i16::from(y) - 256
            } else {
                i16::from(y)
            };
            return MouseState {
                buttons: flags & 0x07,
                delta_x,
                delta_y: -raw_y,
                sequence: after,
            }
        }
    }
}
