use core::f32::consts::PI;
use micromath::F32Ext;

const TABLE_SIZE: usize = 256;
/// `TABLE_SIZE == 2^TABLE_BITS`; the top `TABLE_BITS` of the phase accumulator
/// select the table entry.
const TABLE_BITS: u32 = 8;

/// Pre-computed sine lookup table driven by a fixed-point phase accumulator
/// (a numerically controlled oscillator).
///
/// The phase is a `u32` spanning a full `2^32` per cycle, so the frequency is
/// accurate for any pitch — the previous integer "table steps per sample"
/// truncated to zero for low frequencies (silence) and detuned the rest.
pub struct SineGenerator {
    table: [i16; TABLE_SIZE],
    phase: u32,
    phase_inc: u32,
}

impl SineGenerator {
    pub fn new(sample_rate: u32, frequency: u32, amplitude: f32) -> Self {
        let mut table = [0i16; TABLE_SIZE];
        for i in 0..TABLE_SIZE {
            let sample = (amplitude
                * i16::MAX as f32
                * (2.0 * PI * i as f32 / TABLE_SIZE as f32).sin()) as i16;
            table[i] = sample;
        }
        // Phase advance per sample: `frequency / sample_rate` of a full 2^32 cycle.
        let phase_inc = (((frequency as u64) << 32) / sample_rate as u64) as u32;

        Self {
            table,
            phase: 0,
            phase_inc,
        }
    }

    /// Return the current sample and advance the phase by one step.
    ///
    /// Uses linear interpolation between the two adjacent table entries (an
    /// 8-bit sub-step), which removes the "staircase" harmonics you'd otherwise
    /// hear as a buzzy edge on low notes.
    #[inline]
    pub fn sample(&mut self) -> i16 {
        // Top TABLE_BITS index the table (0..=255 for 256 entries).
        let index = (self.phase >> (32 - TABLE_BITS)) as usize;
        let next = (index + 1) % TABLE_SIZE;
        // Next 8 bits below the index are the fraction between the two entries.
        let frac = ((self.phase >> (32 - TABLE_BITS - 8)) & 0xFF) as i32;
        let a = self.table[index] as i32;
        let b = self.table[next] as i32;
        let interp = a + (((b - a) * frac) >> 8);
        self.phase = self.phase.wrapping_add(self.phase_inc);
        interp as i16
    }

    /// Fill buffer with interleaved stereo samples (L, R, L, R, ...)
    pub fn fill(&mut self, buf: &mut [i16]) {
        let mut i = 0;
        while i + 1 < buf.len() {
            let s = self.sample();
            buf[i] = s; // left
            buf[i + 1] = s; // right
            i += 2;
        }
    }
}
