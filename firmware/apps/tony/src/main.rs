#![no_std]
#![no_main]

use core::{sync::atomic::{Ordering, AtomicU32, fence, AtomicUsize}, arch::asm, ops::DerefMut};

use userspace::common::porcelain::{
    pcm_sink as pcm,
    time,
    system,
};

use rand_core::{self, SeedableRng, RngCore};
use rand_chacha::{self, ChaCha8Rng};

mod scale;

const CHUNK_SZ: usize = 512;

#[no_mangle]
pub fn entry() -> ! {
    pcm::enable().ok();

    let mut seed = [0u8; 32];
    system::rand_fill(&mut seed).unwrap();

    let mut rng = ChaCha8Rng::from_seed(seed);

    let mut samps_a: usize = 0;
    let mut samps_b: usize = 0;
    let mut cur_offset_a = 0;
    let mut cur_offset_b = 0;
    let mut incr_a = new_freq_incr(rng.next_u32() as usize, 3);
    let mut incr_b = new_freq_incr(rng.next_u32() as usize, 4);

    let mut buf_a = [0i16; CHUNK_SZ];
    let mut buf_b = [0i16; CHUNK_SZ];
    fill_sample_buf(&mut buf_a, incr_a, &mut cur_offset_a);
    fill_sample_buf(&mut buf_b, incr_b, &mut cur_offset_b);

    loop {
        if let Ok(mut samples) = pcm::alloc_samples(CHUNK_SZ) {
            samples.deref_mut().chunks_exact_mut(4).zip(buf_a.iter().zip(buf_b.iter())).for_each(|(ch, (a, b))| {
                let val = (*a >> 1).wrapping_add(*b >> 1);

                let leb = val.to_le_bytes();
                ch[0] = leb[0];
                ch[1] = leb[1];
                ch[2] = leb[0];
                ch[3] = leb[1];
            });

            samples.send();
            samps_a = samps_a.saturating_sub(CHUNK_SZ);
            samps_b = samps_b.saturating_sub(CHUNK_SZ);
            fill_sample_buf(&mut buf_a, incr_a, &mut cur_offset_a);
            fill_sample_square(&mut buf_b, incr_b, &mut cur_offset_b);
            // samples.deref_mut().iter_mut().for_each(|b| *b = 0);
            // samples.send();
        } else {
            time::sleep_micros(5000).ok();
        }

        if samps_a == 0 {
            samps_a = 44100 / ((rng.next_u32() as usize % 4) + 1);
            incr_a = new_freq_incr(rng.next_u32() as usize, 3);
        }
        if samps_b == 0 {
            samps_b = (88200 / 3);
            incr_b = new_freq_incr(rng.next_u32() as usize, 4);
        }
    }
}


#[inline(always)]
pub fn fill_sample_square(data: &mut [i16], incr: i32, cur_offset: &mut i32) {
    data.iter_mut().for_each(|ch| {
        if *cur_offset > 0 {
            *ch = i16::MAX;
        } else {
            *ch = i16::MIN;
        }
        *cur_offset = cur_offset.wrapping_add(incr);
    });
}

#[inline(always)]
pub fn fill_sample_saw(data: &mut [i16], incr: i32, cur_offset: &mut i32) {
    data.iter_mut().for_each(|ch| {
        *ch = ((*cur_offset) >> 16) as i16;
        *cur_offset = cur_offset.wrapping_add(incr);
    });
}

#[inline(always)]
pub fn fill_sample_buf(data: &mut [i16], incr: i32, cur_offset: &mut i32) {
    data.iter_mut().for_each(|ch| {
        let val = (*cur_offset) as u32;
        let idx_now = ((val >> 24) & 0xFF) as u8;
        let idx_nxt = idx_now.wrapping_add(1);
        let base_val = SINE_TABLE[idx_now as usize] as i32;
        let next_val = SINE_TABLE[idx_nxt as usize] as i32;

        // Distance to next value - perform 256 slot linear interpolation
        let off = ((val >> 16) & 0xFF) as i32; // 0..=255
        let cur_weight = base_val.wrapping_mul(256i32.wrapping_sub(off));
        let nxt_weight = next_val.wrapping_mul(off);
        let ttl_weight = cur_weight.wrapping_add(nxt_weight);
        let ttl_val = ttl_weight >> 8; // div 256
        let ttl_val = ttl_val as i16;

        // Set the linearly interpolated value
        *ch = ttl_val;

        *cur_offset = cur_offset.wrapping_add(incr);
    });
}

fn new_freq_incr(it: usize, oct: u8) -> i32 {
    let cur_scale = scale::BLUES_MAJOR_PENTATONIC_INTERVALS;
    let semi = cur_scale[it % cur_scale.len()];
    let cur_note = scale::Note { pitch: scale::Pitch::ASharp, octave: oct };
    let freq = (cur_note + semi).freq_f32();

    let samp_per_cyc: f32 = 44100.0 / freq; // 141.7
    let fincr = 256.0 / samp_per_cyc; // 1.81
    let incr = (((1 << 24) as f32) * fincr) as i32;
    incr
}

const SINE_TABLE: [i16; 256] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767, 32757, 32728, 32678,
    32609, 32521, 32412, 32285, 32137, 31971, 31785, 31580, 31356, 31113, 30852, 30571, 30273,
    29956, 29621, 29268, 28898, 28510, 28105, 27683, 27245, 26790, 26319, 25832, 25329, 24811,
    24279, 23731, 23170, 22594, 22005, 21403, 20787, 20159, 19519, 18868, 18204, 17530, 16846,
    16151, 15446, 14732, 14010, 13279, 12539, 11793, 11039, 10278, 9512, 8739, 7962, 7179, 6393,
    5602, 4808, 4011, 3212, 2410, 1608, 804, 0, -804, -1608, -2410, -3212, -4011, -4808, -5602,
    -6393, -7179, -7962, -8739, -9512, -10278, -11039, -11793, -12539, -13279, -14010, -14732,
    -15446, -16151, -16846, -17530, -18204, -18868, -19519, -20159, -20787, -21403, -22005, -22594,
    -23170, -23731, -24279, -24811, -25329, -25832, -26319, -26790, -27245, -27683, -28105, -28510,
    -28898, -29268, -29621, -29956, -30273, -30571, -30852, -31113, -31356, -31580, -31785, -31971,
    -32137, -32285, -32412, -32521, -32609, -32678, -32728, -32757, -32767, -32757, -32728, -32678,
    -32609, -32521, -32412, -32285, -32137, -31971, -31785, -31580, -31356, -31113, -30852, -30571,
    -30273, -29956, -29621, -29268, -28898, -28510, -28105, -27683, -27245, -26790, -26319, -25832,
    -25329, -24811, -24279, -23731, -23170, -22594, -22005, -21403, -20787, -20159, -19519, -18868,
    -18204, -17530, -16846, -16151, -15446, -14732, -14010, -13279, -12539, -11793, -11039, -10278,
    -9512, -8739, -7962, -7179, -6393, -5602, -4808, -4011, -3212, -2410, -1608, -804,
];
