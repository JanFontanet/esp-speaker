use core::f32::consts::PI;
use micromath::F32Ext;

const TABLE_SIZE: usize = 256;

/// Pre-computed sine lookup table (i16 samples)
pub struct SineGenerator {
    table: [i16; TABLE_SIZE],
    phase: usize,
    phase_inc: usize,
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
        // phase_inc = how many table steps per sample
        let phase_inc = (TABLE_SIZE as u32 * frequency / sample_rate) as usize;

        Self {
            table,
            phase: 0,
            phase_inc,
        }
    }

    /// Fill buffer with interleaved stereo samples (L, R, L, R, ...)
    pub fn fill(&mut self, buf: &mut [i16]) {
        let mut i = 0;
        while i + 1 < buf.len() {
            let sample = self.table[self.phase % TABLE_SIZE];
            buf[i] = sample; // left
            buf[i + 1] = sample; // right
            self.phase = (self.phase + self.phase_inc) % TABLE_SIZE;
            i += 2;
        }
    }
}
