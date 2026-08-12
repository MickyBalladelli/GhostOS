//! Kernel CSPRNG seeded only by trusted processor entropy instructions.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const MAX_REQUEST_BYTES: usize = 4096;

static KEY: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static BLOCK_COUNTER: AtomicU64 = AtomicU64::new(0);
static READY: AtomicBool = AtomicBool::new(false);

pub fn initialize() {
    let Some(mut seed) = platform::seed() else {
        return
    };
    for (slot, value) in KEY.iter().zip(seed) {
        slot.store(value, Ordering::Relaxed)
    }
    seed.fill(0);
    BLOCK_COUNTER.store(0, Ordering::Relaxed);
    READY.store(true, Ordering::Release)
}

pub fn ready() -> bool {
    READY.load(Ordering::Acquire)
}

pub fn fill(output: &mut [u8]) -> bool {
    if output.is_empty() || output.len() > MAX_REQUEST_BYTES || !ready() {
        return false
    }
    let key = load_key();
    let blocks = output.len().div_ceil(64) as u64;
    let Ok(first_counter) = BLOCK_COUNTER.fetch_update(
        Ordering::AcqRel,
        Ordering::Acquire,
        |counter| counter.checked_add(blocks),
    ) else {
        return false
    };
    for (index, chunk) in output.chunks_mut(64).enumerate() {
        let block = chacha20_block(key, first_counter + index as u64);
        chunk.copy_from_slice(&block[..chunk.len()])
    }
    true
}

fn load_key() -> [u32; 8] {
    let mut key = [0; 8];
    for (index, value) in KEY.iter().enumerate() {
        let bytes = value.load(Ordering::Relaxed).to_le_bytes();
        key[index * 2] = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        key[index * 2 + 1] = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
    }
    key
}

fn chacha20_block(key: [u32; 8], counter: u64) -> [u8; 64] {
    let mut state = [
        0x6170_7865,
        0x3320_646e,
        0x7962_2d32,
        0x6b20_6574,
        key[0],
        key[1],
        key[2],
        key[3],
        key[4],
        key[5],
        key[6],
        key[7],
        counter as u32,
        (counter >> 32) as u32,
        key[0] ^ key[4],
        key[3] ^ key[7],
    ];
    let original = state;
    for _ in 0..10 {
        quarter_round(&mut state, 0, 4, 8, 12);
        quarter_round(&mut state, 1, 5, 9, 13);
        quarter_round(&mut state, 2, 6, 10, 14);
        quarter_round(&mut state, 3, 7, 11, 15);
        quarter_round(&mut state, 0, 5, 10, 15);
        quarter_round(&mut state, 1, 6, 11, 12);
        quarter_round(&mut state, 2, 7, 8, 13);
        quarter_round(&mut state, 3, 4, 9, 14)
    }
    let mut output = [0; 64];
    for (index, word) in state.iter().enumerate() {
        output[index * 4..index * 4 + 4]
            .copy_from_slice(&word.wrapping_add(original[index]).to_le_bytes())
    }
    output
}

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(7)
}

#[cfg(target_arch = "x86_64")]
mod platform {
    use core::arch::{asm, x86_64::__cpuid, x86_64::__cpuid_count};

    pub fn seed() -> Option<[u64; 4]> {
        let leaf_one = __cpuid(1);
        let leaf_seven = __cpuid_count(7, 0);
        let has_rdrand = leaf_one.ecx & (1 << 30) != 0;
        let has_rdseed = leaf_seven.ebx & (1 << 18) != 0;
        if !has_rdrand && !has_rdseed {
            return None
        }
        let mut seed = [0; 4];
        for (index, word) in seed.iter_mut().enumerate() {
            let source = if has_rdseed {
                retry(rdseed).or_else(|| has_rdrand.then(|| retry(rdrand)).flatten())
            } else {
                retry(rdrand)
            }?;
            *word = avalanche(source ^ timestamp().rotate_left(index as u32 * 11))
        }
        Some(seed)
    }

    fn retry(read: fn() -> Option<u64>) -> Option<u64> {
        for _ in 0..64 {
            if let Some(value) = read() {
                return Some(value)
            }
            core::hint::spin_loop()
        }
        None
    }

    fn rdseed() -> Option<u64> {
        let value: u64;
        let ok: u8;
        unsafe {
            asm!(
                "rdseed {value}",
                "setc {ok}",
                value = out(reg) value,
                ok = out(reg_byte) ok,
                options(nomem, nostack),
            )
        }
        (ok != 0).then_some(value)
    }

    fn rdrand() -> Option<u64> {
        let value: u64;
        let ok: u8;
        unsafe {
            asm!(
                "rdrand {value}",
                "setc {ok}",
                value = out(reg) value,
                ok = out(reg_byte) ok,
                options(nomem, nostack),
            )
        }
        (ok != 0).then_some(value)
    }

    fn timestamp() -> u64 {
        let low: u32;
        let high: u32;
        unsafe {
            asm!(
                "rdtsc",
                out("eax") low,
                out("edx") high,
                options(nomem, nostack),
            )
        }
        u64::from(low) | (u64::from(high) << 32)
    }

    fn avalanche(mut value: u64) -> u64 {
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value ^= value >> 27;
        value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod platform {
    pub fn seed() -> Option<[u64; 4]> {
        None
    }
}
