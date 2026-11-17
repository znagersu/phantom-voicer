use crate::params::SoundPatch;
use std::f32::consts::{PI, TAU};

const MAX_VOICES: usize = 24;

#[derive(Debug, Clone, Copy)]
pub struct Mark1Settings {
    pub bass: f32,
    pub treble: f32,
    pub noise_level: f32,
    pub release_noise: f32,
    pub fx_pan: bool,
    pub fx_autowah: bool,
    pub fx_comp: bool,
    pub fx_chorus: bool,
    pub fx_dist: bool,
    pub fx_phaser: bool,
    pub fx_amp: bool,
    pub pan_intensity: f32,
    pub pan_speed: f32,
    pub pan_sync: bool,
    pub amp_drive: f32,
    pub amp_output: f32,
    pub tempo_bpm: f32,
}

impl Default for Mark1Settings {
    fn default() -> Self {
        Self {
            bass: 0.05,
            treble: 0.12,
            noise_level: 0.45,
            release_noise: 0.48,
            fx_pan: true,
            fx_autowah: false,
            fx_comp: false,
            fx_chorus: true,
            fx_dist: false,
            fx_phaser: false,
            fx_amp: true,
            pan_intensity: 0.45,
            pan_speed: 0.55,
            pan_sync: true,
            amp_drive: 0.35,
            amp_output: 0.58,
            tempo_bpm: 120.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    #[inline]
    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn high_pass(sr: f32, freq: f32, q: f32) -> Self {
        let w0 = TAU * (freq / sr).clamp(0.0001, 0.45);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cos_w0) * 0.5) / a0,
            b1: (-(1.0 + cos_w0)) / a0,
            b2: ((1.0 + cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn low_pass(sr: f32, freq: f32, q: f32) -> Self {
        let w0 = TAU * (freq / sr).clamp(0.0001, 0.45);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) * 0.5) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn peaking(sr: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = TAU * (freq / sr).clamp(0.0001, 0.45);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn low_shelf(sr: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = TAU * (freq / sr).clamp(0.0001, 0.45);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let sqrt_a = a.sqrt();
        let alpha = sin_w0 * 0.5 * ((a + 1.0 / a) * (1.0 / 0.707 - 1.0) + 2.0).max(0.1).sqrt();
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha;
        Self {
            b0: (a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha)) / a0,
            b1: (2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0)) / a0,
            b2: (a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha)) / a0,
            a1: (-2.0 * ((a - 1.0) + (a + 1.0) * cos_w0)) / a0,
            a2: ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn high_shelf(sr: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = TAU * (freq / sr).clamp(0.0001, 0.45);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let sqrt_a = a.sqrt();
        let alpha = sin_w0 * 0.5 * ((a + 1.0 / a) * (1.0 / 0.707 - 1.0) + 2.0).max(0.1).sqrt();
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha;
        Self {
            b0: (a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha)) / a0,
            b1: (-2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0)) / a0,
            b2: (a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha)) / a0,
            a1: (2.0 * ((a - 1.0) - (a + 1.0) * cos_w0)) / a0,
            a2: ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SynthVoice {
    active: bool,
    releasing: bool,
    note: u8,
    channel: u8,
    is_bass: bool,
    velocity: f32,
    freq: f32,
    target_freq: f32,
    phase: f32,
    phase_b: f32,
    phase_c: f32,
    tine_phase: f32,
    tine_phase2: f32,
    age_samples: u64,
    env: f32,
    pluck_env: f32,
    hammer_env: f32,
    release_thump_env: f32,
    harm_env: [f32; 6],
    filter_ic1: f32,
    filter_ic2: f32,
    rng_state: u32,
    internal_id: u64,
}

impl Default for SynthVoice {
    fn default() -> Self {
        Self {
            active: false,
            releasing: false,
            note: 0,
            channel: 0,
            is_bass: false,
            velocity: 0.0,
            freq: 440.0,
            target_freq: 440.0,
            phase: 0.0,
            phase_b: 0.0,
            phase_c: 0.0,
            tine_phase: 0.0,
            tine_phase2: 0.0,
            age_samples: 0,
            env: 0.0,
            pluck_env: 0.0,
            hammer_env: 0.0,
            release_thump_env: 0.0,
            harm_env: [0.0; 6],
            filter_ic1: 0.0,
            filter_ic2: 0.0,
            rng_state: 0x1234_5678,
            internal_id: 0,
        }
    }
}

impl SynthVoice {
    #[inline]
    fn next_noise(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        ((x & 0xFFFF) as f32 / 32767.5) - 1.0
    }
}

pub struct PhantomSynth {
    sample_rate: f32,
    voices: [SynthVoice; MAX_VOICES],
    next_id: u64,
    last_freq: Option<f32>,
    pitch_bend_semis: f32,
    pancake_phase: f32,
    chorus_phase: f32,
    phaser_phase: f32,
    delay_l: [f32; 2048],
    delay_r: [f32; 2048],
    delay_idx: usize,
    // Trilian Clean Fender Jazz Bass DI EQ & Optical Comp
    bass_hpf: Biquad,
    bass_low: Biquad,
    bass_box_cut: Biquad,
    bass_growl: Biquad,
    bass_air: Biquad,
    bass_comp_env: f32,
    // Scarbee Mark I Amp Cabinet + Tone Stack + FX state
    mk1_cab_body_l: Biquad,
    mk1_cab_body_r: Biquad,
    mk1_cab_scoop_l: Biquad,
    mk1_cab_scoop_r: Biquad,
    mk1_cab_lpf_l: Biquad,
    mk1_cab_lpf_r: Biquad,
    mk1_bass_l: Biquad,
    mk1_bass_r: Biquad,
    mk1_treble_l: Biquad,
    mk1_treble_r: Biquad,
    last_mk1_bass: f32,
    last_mk1_treble: f32,
    mk1_comp_env: f32,
    // 4-stage stereo allpass phaser state
    phaser_ap_l: [f32; 4],
    phaser_ap_r: [f32; 4],
    phaser_fb_l: f32,
    phaser_fb_r: f32,
    // Auto-wah state
    wah_env: f32,
    svf_ic1_l: f32,
    svf_ic2_l: f32,
    svf_ic1_r: f32,
    svf_ic2_r: f32,
}

impl Default for PhantomSynth {
    fn default() -> Self {
        let mut s = Self {
            sample_rate: 44100.0,
            voices: [SynthVoice::default(); MAX_VOICES],
            next_id: 1,
            last_freq: None,
            pitch_bend_semis: 0.0,
            pancake_phase: 0.0,
            chorus_phase: 0.0,
            phaser_phase: 0.0,
            delay_l: [0.0; 2048],
            delay_r: [0.0; 2048],
            delay_idx: 0,
            bass_hpf: Biquad::default(),
            bass_low: Biquad::default(),
            bass_box_cut: Biquad::default(),
            bass_growl: Biquad::default(),
            bass_air: Biquad::default(),
            bass_comp_env: 0.0,
            mk1_cab_body_l: Biquad::default(),
            mk1_cab_body_r: Biquad::default(),
            mk1_cab_scoop_l: Biquad::default(),
            mk1_cab_scoop_r: Biquad::default(),
            mk1_cab_lpf_l: Biquad::default(),
            mk1_cab_lpf_r: Biquad::default(),
            mk1_bass_l: Biquad::default(),
            mk1_bass_r: Biquad::default(),
            mk1_treble_l: Biquad::default(),
            mk1_treble_r: Biquad::default(),
            last_mk1_bass: 999.0,
            last_mk1_treble: 999.0,
            mk1_comp_env: 0.0,
            phaser_ap_l: [0.0; 4],
            phaser_ap_r: [0.0; 4],
            phaser_fb_l: 0.0,
            phaser_fb_r: 0.0,
            wah_env: 0.0,
            svf_ic1_l: 0.0,
            svf_ic2_l: 0.0,
            svf_ic1_r: 0.0,
            svf_ic2_r: 0.0,
        };
        s.rebuild_eq(44100.0);
        s.update_mk1_tone(0.05, 0.12);
        s
    }
}

#[inline]
fn midi_to_hz(note: u8) -> f32 {
    440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0)
}

#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[inline]
fn blep_saw(t: f32, dt: f32) -> f32 {
    (2.0 * t - 1.0) - poly_blep(t, dt)
}

#[inline]
fn blep_pulse(t: f32, dt: f32, width: f32) -> f32 {
    let raw = if t < width { 1.0 } else { -1.0 };
    let mut out = raw + poly_blep(t, dt);
    let t2 = (t - width).rem_euclid(1.0);
    out -= poly_blep(t2, dt);
    out
}

impl PhantomSynth {
    fn rebuild_eq(&mut self, sr: f32) {
        self.bass_hpf = Biquad::high_pass(sr, 36.0, 0.707);
        self.bass_low = Biquad::low_shelf(sr, 92.0, 1.8);
        self.bass_box_cut = Biquad::peaking(sr, 285.0, 1.4, -2.5);
        self.bass_growl = Biquad::peaking(sr, 820.0, 1.1, 2.4);
        self.bass_air = Biquad::high_shelf(sr, 2800.0, 2.0);

        // Scarbee Mark I Fender Twin / 2x12 open-back cabinet voicing
        self.mk1_cab_body_l = Biquad::peaking(sr, 165.0, 0.8, 2.6);
        self.mk1_cab_body_r = Biquad::peaking(sr, 165.0, 0.8, 2.6);
        self.mk1_cab_scoop_l = Biquad::peaking(sr, 430.0, 1.1, -1.6);
        self.mk1_cab_scoop_r = Biquad::peaking(sr, 430.0, 1.1, -1.6);
        self.mk1_cab_lpf_l = Biquad::low_pass(sr, 4700.0, 0.72);
        self.mk1_cab_lpf_r = Biquad::low_pass(sr, 4700.0, 0.72);
        self.last_mk1_bass = 999.0;
        self.last_mk1_treble = 999.0;
    }

    fn update_mk1_tone(&mut self, bass: f32, treble: f32) {
        if (bass - self.last_mk1_bass).abs() > 0.005 {
            self.last_mk1_bass = bass;
            let db = bass.clamp(-1.0, 1.0) * 7.5;
            self.mk1_bass_l = Biquad::low_shelf(self.sample_rate, 115.0, db);
            self.mk1_bass_r = Biquad::low_shelf(self.sample_rate, 115.0, db);
        }
        if (treble - self.last_mk1_treble).abs() > 0.005 {
            self.last_mk1_treble = treble;
            let db = treble.clamp(-1.0, 1.0) * 7.5;
            self.mk1_treble_l = Biquad::high_shelf(self.sample_rate, 2700.0, db);
            self.mk1_treble_r = Biquad::high_shelf(self.sample_rate, 2700.0, db);
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
        self.rebuild_eq(self.sample_rate);
    }

    pub fn reset(&mut self) {
        self.voices = [SynthVoice::default(); MAX_VOICES];
        self.last_freq = None;
        self.pitch_bend_semis = 0.0;
        self.pancake_phase = 0.0;
        self.chorus_phase = 0.0;
        self.phaser_phase = 0.0;
        self.delay_l = [0.0; 2048];
        self.delay_r = [0.0; 2048];
        self.delay_idx = 0;
        self.bass_hpf.reset();
        self.bass_low.reset();
        self.bass_box_cut.reset();
        self.bass_growl.reset();
        self.bass_air.reset();
        self.bass_comp_env = 0.0;
        self.mk1_cab_body_l.reset();
        self.mk1_cab_body_r.reset();
        self.mk1_cab_scoop_l.reset();
        self.mk1_cab_scoop_r.reset();
        self.mk1_cab_lpf_l.reset();
        self.mk1_cab_lpf_r.reset();
        self.mk1_bass_l.reset();
        self.mk1_bass_r.reset();
        self.mk1_treble_l.reset();
        self.mk1_treble_r.reset();
        self.mk1_comp_env = 0.0;
        self.phaser_ap_l = [0.0; 4];
        self.phaser_ap_r = [0.0; 4];
        self.phaser_fb_l = 0.0;
        self.phaser_fb_r = 0.0;
        self.wah_env = 0.0;
        self.svf_ic1_l = 0.0;
        self.svf_ic2_l = 0.0;
        self.svf_ic1_r = 0.0;
        self.svf_ic2_r = 0.0;
    }

    pub fn set_pitch_bend(&mut self, norm_value: f32) {
        self.pitch_bend_semis = (norm_value - 0.5) * 4.0;
    }

    pub fn note_on(
        &mut self,
        note: u8,
        channel: u8,
        velocity: f32,
        is_bass: bool,
        patch: SoundPatch,
        glide_ms: f32,
    ) {
        let mut target_idx = None;
        let mut oldest_idx = 0usize;
        let mut oldest_id = u64::MAX;

        if patch == SoundPatch::SlapBass && glide_ms > 1.0 {
            for (i, v) in self.voices.iter().enumerate() {
                if v.active {
                    target_idx = Some(i);
                    break;
                }
            }
        }

        if target_idx.is_none() {
            for (i, v) in self.voices.iter().enumerate() {
                if !v.active {
                    target_idx = Some(i);
                    break;
                }
                if v.note == note && v.channel == channel {
                    target_idx = Some(i);
                    break;
                }
                if v.internal_id < oldest_id {
                    oldest_id = v.internal_id;
                    oldest_idx = i;
                }
            }
        }

        let idx = target_idx.unwrap_or(oldest_idx);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let target_freq = midi_to_hz(note);
        let start_freq = if glide_ms > 1.0 {
            self.last_freq.unwrap_or(target_freq)
        } else {
            target_freq
        };
        self.last_freq = Some(target_freq);

        let prev = self.voices[idx];
        let keep_phase = prev.active && patch == SoundPatch::SlapBass && glide_ms > 1.0;

        self.voices[idx] = SynthVoice {
            active: true,
            releasing: false,
            note,
            channel,
            is_bass,
            velocity: velocity.clamp(0.05, 1.0),
            freq: start_freq,
            target_freq,
            phase: if keep_phase { prev.phase } else { 0.0 },
            phase_b: if keep_phase { prev.phase_b } else { 0.17 },
            phase_c: if keep_phase { prev.phase_c } else { 0.53 },
            tine_phase: 0.0,
            tine_phase2: 0.0,
            age_samples: 0,
            env: if keep_phase { prev.env.max(0.25) } else { 0.0 },
            pluck_env: 1.0,
            hammer_env: 1.0,
            release_thump_env: 0.0,
            harm_env: [1.0; 6],
            filter_ic1: if keep_phase { prev.filter_ic1 } else { 0.0 },
            filter_ic2: if keep_phase { prev.filter_ic2 } else { 0.0 },
            rng_state: (id as u32).wrapping_mul(747796405).wrapping_add(2891336453) | 1,
            internal_id: id,
        };
    }

    pub fn note_off(&mut self, note: u8, channel: u8) {
        for v in &mut self.voices {
            if v.active && !v.releasing && v.note == note && v.channel == channel {
                v.releasing = true;
                // Trigger felt damper mechanical thud on key release
                v.release_thump_env = 1.0;
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active && !v.releasing {
                v.releasing = true;
                v.release_thump_env = 1.0;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_sample(
        &mut self,
        patch: SoundPatch,
        brightness: f32,
        attack_ms: f32,
        release_ms: f32,
        glide_ms: f32,
        bark: f32,
        wah_amount: f32,
        tremolo_depth: f32,
        tremolo_rate_hz: f32,
        gain: f32,
        mk1: &Mark1Settings,
    ) -> (f32, f32) {
        let sr = self.sample_rate;
        let atk_s = (attack_ms / 1000.0).max(0.0015);
        let rel_s = (release_ms / 1000.0).max(0.015);
        let attack_coeff = 1.0 - (-1.0 / (atk_s * sr)).exp();
        let release_coeff = (-1.0 / (rel_s * sr)).exp();

        let hold_decay = match patch {
            SoundPatch::VelvetRhodes => (-1.0 / (3.1 * sr)).exp(),
            SoundPatch::SlapBass => (-1.0 / (2.4 * sr)).exp(),
            SoundPatch::PhantomBrass => 1.0,
            SoundPatch::WahClav => (-1.0 / (0.85 * sr)).exp(),
        };
        let pluck_decay = match patch {
            SoundPatch::VelvetRhodes => (-1.0 / (0.068 * sr)).exp(),
            SoundPatch::SlapBass => (-1.0 / (0.018 * sr)).exp(),
            SoundPatch::PhantomBrass => (-1.0 / (0.140 * sr)).exp(),
            SoundPatch::WahClav => (-1.0 / (0.095 * sr)).exp(),
        };
        let hammer_decay = (-1.0 / (0.014 * sr)).exp();
        let rel_thump_decay = (-1.0 / (0.022 * sr)).exp();

        let harm_decays = [
            (-1.0 / (1.85 * sr)).exp(),
            (-1.0 / (1.05 * sr)).exp(),
            (-1.0 / (0.55 * sr)).exp(),
            (-1.0 / (0.30 * sr)).exp(),
            (-1.0 / (0.17 * sr)).exp(),
            (-1.0 / (0.10 * sr)).exp(),
        ];

        let glide_coeff = if glide_ms > 0.5 {
            1.0 - (-1.0 / ((glide_ms / 1000.0) * sr)).exp()
        } else {
            1.0
        };
        let bend_ratio = 2.0f32.powf(self.pitch_bend_semis / 12.0);

        let mut left = 0.0f32;
        let mut right = 0.0f32;

        for v in &mut self.voices {
            if !v.active {
                continue;
            }

            if v.releasing {
                v.env *= release_coeff;
                v.release_thump_env *= rel_thump_decay;
                if v.env < 0.0002 && v.release_thump_env < 0.001 {
                    v.active = false;
                    continue;
                }
            } else if v.age_samples < (atk_s * 3.0 * sr) as u64 {
                v.env += (1.0 - v.env) * attack_coeff;
            } else {
                v.env *= hold_decay;
            }

            v.pluck_env *= pluck_decay;
            v.hammer_env *= hammer_decay;
            for (he, &hd) in v.harm_env.iter_mut().zip(harm_decays.iter()) {
                *he *= hd;
            }
            v.age_samples += 1;

            v.freq += (v.target_freq - v.freq) * glide_coeff;
            let base_hz = (v.freq * bend_ratio).clamp(20.0, sr * 0.45);
            let f_eff = if patch == SoundPatch::SlapBass {
                base_hz * (1.0 + 0.018 * v.pluck_env * v.velocity)
            } else {
                base_hz
            };
            let dt = f_eff / sr;
            v.phase = (v.phase + dt).fract();

            let (sig_l, sig_r) = match patch {
                // =========================================================================
                // 1) SCARBEE MARK I — "PAN CAKE KORE" PHYSICAL MODEL
                // =========================================================================
                SoundPatch::VelvetRhodes => {
                    let ph = v.phase * TAU;
                    let nyquist_lim = sr * 0.40;

                    // Pure warm tonebar fundamental + velocity-proportional 2nd/3rd Rhodes bark
                    // (avoiding harsh FM sidebands so chords stay velvety and woody)
                    let bark_amt = (0.10 + bark * 0.45) * v.velocity.powf(1.4) * (0.35 + 0.65 * v.pluck_env);
                    let mut tonebar = ph.sin();
                    if f_eff * 2.0 < nyquist_lim {
                        tonebar += (ph * 2.0).sin() * (0.14 + bark_amt * 0.48);
                    }
                    if f_eff * 3.0 < nyquist_lim {
                        tonebar += (ph * 3.0).sin() * (0.04 + bark_amt * 0.24);
                    }
                    if f_eff * 4.0 < nyquist_lim {
                        tonebar += (ph * 4.0).sin() * (bark_amt * 0.10);
                    }

                    // Separate woody-bell Rhodes tine (4x + 7x damped quickly, never shrill)
                    v.tine_phase = (v.tine_phase + dt * 4.0).fract();
                    v.tine_phase2 = (v.tine_phase2 + dt * 7.0).fract();
                    let tine_lvl = if v.is_bass {
                        0.03
                    } else {
                        (0.08 + 0.16 * brightness) * (0.35 + 0.65 * v.velocity)
                    };
                    let mut tine = 0.0f32;
                    if f_eff * 4.0 < nyquist_lim {
                        tine += (v.tine_phase * TAU).sin() * v.pluck_env * tine_lvl * 0.75;
                    }
                    if f_eff * 7.0 < nyquist_lim {
                        tine += (v.tine_phase2 * TAU).sin() * (v.pluck_env * v.pluck_env) * tine_lvl * 0.55;
                    }

                    // Instrument Noise: hammer strike felt knock + key-off damper release thud
                    let raw_noise = v.next_noise();
                    let filt_noise = voice_lpf(
                        raw_noise,
                        280.0 + 260.0 * v.velocity,
                        0.68,
                        sr,
                        &mut v.filter_ic1,
                        &mut v.filter_ic2,
                    );
                    let strike_noise =
                        (filt_noise * 0.18 + (ph * 1.5).sin() * 0.06) * v.hammer_env * mk1.noise_level;
                    let rel_thump =
                        filt_noise * 0.28 * v.release_thump_env * mk1.release_noise;

                    let core = (tonebar * 0.62 + tine) * v.env + strike_noise + rel_thump;
                    let sample = core * v.velocity * 0.34;
                    let key_pan = ((v.note as f32 - 60.0) / 48.0).clamp(-0.25, 0.25);
                    (sample * (1.0 - key_pan * 0.3), sample * (1.0 + key_pan * 0.3))
                }

                // =========================================================================
                // 2) TRILIAN — "CLEAN FENDER - FULL RANGE" (Jazz Bass DI)
                // =========================================================================
                SoundPatch::SlapBass => {
                    let ph = v.phase * TAU;
                    const NECK_POS: f32 = 0.21;
                    const BRIDGE_POS: f32 = 0.095;
                    let nyquist_lim = sr * 0.42;

                    let w1 = 0.72 * (PI * NECK_POS).sin() + 0.48 * (PI * BRIDGE_POS).sin();
                    let mut sum = ph.sin() * w1 * 1.05;

                    let bright_scale = 0.35 + 0.85 * brightness;
                    let vel_tilt = 0.55 + 0.45 * v.velocity;
                    for idx in 0..6 {
                        let k = (idx + 2) as f32;
                        let hk_freq = f_eff * k;
                        if hk_freq >= nyquist_lim {
                            break;
                        }
                        let nyq_fade = ((nyquist_lim - hk_freq) / (nyquist_lim * 0.15)).clamp(0.0, 1.0);
                        let pickup_w = 0.65 * (k * PI * NECK_POS).sin().abs()
                            + 0.52 * (k * PI * BRIDGE_POS).sin().abs();
                        let string_roll = 1.0 / k.powf(1.12);
                        let amp = pickup_w * string_roll * v.harm_env[idx] * bright_scale * vel_tilt * nyq_fade;
                        sum += (ph * k).sin() * amp;
                    }

                    let finger_noise = voice_lpf(
                        v.next_noise(),
                        700.0 + 1600.0 * brightness * v.velocity,
                        0.65,
                        sr,
                        &mut v.filter_ic1,
                        &mut v.filter_ic2,
                    );
                    let pluck_thump =
                        (finger_noise * 0.22 + (ph * 2.0).sin() * 0.14) * v.pluck_env * v.velocity;

                    let bass_sig = (sum * 0.72 + pluck_thump) * v.env * v.velocity * 0.48;
                    (bass_sig, bass_sig)
                }

                // =========================================================================
                // 3) PHANTOM BRASS
                // =========================================================================
                SoundPatch::PhantomBrass => {
                    let scoop = 1.0 - v.pluck_env * 0.015;
                    let dt_a = (dt * scoop * 0.9945).min(0.45);
                    let dt_b = (dt * scoop).min(0.45);
                    let dt_c = (dt * scoop * 1.0055).min(0.45);
                    v.phase_b = (v.phase_b + dt_a).fract();
                    v.phase_c = (v.phase_c + dt_c).fract();

                    let s_mid = blep_saw(v.phase, dt_b);
                    let s_l = blep_saw(v.phase_b, dt_a);
                    let s_r = blep_saw(v.phase_c, dt_c);

                    let mono_raw = (s_mid * 0.5 + s_l * 0.35 + s_r * 0.35) * 0.7;
                    let cutoff = (260.0
                        + brightness * 2400.0
                        + (1.0 - v.pluck_env * 0.35) * v.env * 2800.0 * v.velocity)
                        .min(sr * 0.42);
                    let brass = voice_lpf(mono_raw, cutoff, 0.65, sr, &mut v.filter_ic1, &mut v.filter_ic2);
                    let spread = (s_l - s_r) * 0.12 * brightness;
                    let base = brass * v.env * v.velocity * 0.28;
                    (base + spread * v.env * 0.18, base - spread * v.env * 0.18)
                }

                // =========================================================================
                // 4) WAH CLAV / FUNK GUITAR
                // =========================================================================
                SoundPatch::WahClav => {
                    let pw = 0.18 + 0.08 * brightness;
                    let clav = blep_pulse(v.phase, dt, pw) * 0.75 + blep_saw(v.phase, dt) * 0.25;
                    let cutoff = (450.0 + brightness * 3600.0 + v.pluck_env * 3200.0).min(sr * 0.42);
                    let bright = voice_lpf(clav, cutoff, 0.55, sr, &mut v.filter_ic1, &mut v.filter_ic2);
                    let s = bright * v.env * v.velocity * 0.27;
                    let pan = ((v.note as f32 - 60.0) / 48.0).clamp(-0.35, 0.35);
                    (s * (1.0 - pan * 0.35), s * (1.0 + pan * 0.35))
                }
            };

            left += sig_l;
            right += sig_r;
        }

        // =================================================================================
        // POST-VOICE SIGNAL CHAIN: Scarbee Mark I Kore Rack vs. Clean Fender / Brass / Clav
        // =================================================================================
        if patch == SoundPatch::VelvetRhodes {
            self.update_mk1_tone(mk1.bass, mk1.treble);
            left = self.mk1_bass_l.process(left);
            right = self.mk1_bass_r.process(right);
            left = self.mk1_treble_l.process(left);
            right = self.mk1_treble_r.process(right);

            // 1) COMP. (optional)
            if mk1.fx_comp {
                let lvl = (left.abs() + right.abs()) * 0.5;
                let atk = 1.0 - (-1.0 / (0.012 * sr)).exp();
                let rel = 1.0 - (-1.0 / (0.160 * sr)).exp();
                if lvl > self.mk1_comp_env {
                    self.mk1_comp_env += (lvl - self.mk1_comp_env) * atk;
                } else {
                    self.mk1_comp_env += (lvl - self.mk1_comp_env) * rel;
                }
                let thresh = 0.18;
                let cg = if self.mk1_comp_env > thresh {
                    (thresh / self.mk1_comp_env).powf(0.55).clamp(0.35, 1.0)
                } else {
                    1.0
                };
                left *= cg * 1.28;
                right *= cg * 1.28;
            }

            // 2) DIST. (optional warm overdrive)
            if mk1.fx_dist || bark > 0.05 {
                let drv = 1.0 + if mk1.fx_dist { 2.4 } else { 0.0 } + bark * 1.4;
                left = fast_tanh(left * drv) / (1.0 + (drv - 1.0) * 0.42);
                right = fast_tanh(right * drv) / (1.0 + (drv - 1.0) * 0.42);
            }

            // 3) AUTOWAH (optional)
            let eff_wah = if mk1.fx_autowah { wah_amount.max(0.75) } else { wah_amount };
            if eff_wah > 0.001 {
                let (wl, wr) = self.process_autowah(left, right, brightness, sr);
                left = left * (1.0 - eff_wah) + wl * eff_wah;
                right = right * (1.0 - eff_wah) + wr * eff_wah;
            }

            // 4) PHASER (optional 4-stage analog allpass)
            self.phaser_phase = (self.phaser_phase + TAU * 0.52 / sr).rem_euclid(TAU);
            if mk1.fx_phaser {
                let (pl, pr) = self.process_phaser(left, right, sr);
                left = left * 0.55 + pl * 0.45;
                right = right * 0.55 + pr * 0.45;
            }

            // 5) AMP (ON in Pan Cake Kore! Tube preamp + Fender 2x12 cabinet body & speaker rolloff)
            if mk1.fx_amp {
                let drv = 1.0 + mk1.amp_drive * 1.65;
                left = fast_tanh((left + 0.04 * left * left) * drv);
                right = fast_tanh((right + 0.04 * right * right) * drv);

                left = self.mk1_cab_body_l.process(left);
                right = self.mk1_cab_body_r.process(right);
                left = self.mk1_cab_scoop_l.process(left);
                right = self.mk1_cab_scoop_r.process(right);
                left = self.mk1_cab_lpf_l.process(left);
                right = self.mk1_cab_lpf_r.process(right);

                let out_trim = 0.55 + mk1.amp_output * 0.85;
                left *= out_trim;
                right *= out_trim;
            }

            // 6) CHORUS (ON in Pan Cake Kore! Lush dual-tap BBD stereo chorus)
            self.chorus_phase = (self.chorus_phase + TAU * 0.52 / sr).rem_euclid(TAU);
            self.delay_l[self.delay_idx] = left;
            self.delay_r[self.delay_idx] = right;
            if mk1.fx_chorus {
                let mod1 = (0.0075 + 0.0028 * self.chorus_phase.sin()) * sr;
                let mod2 = (0.0112 + 0.0028 * self.chorus_phase.cos()) * sr;
                let wet_l = read_delay_linear(&self.delay_l, self.delay_idx, mod1);
                let wet_r = read_delay_linear(&self.delay_r, self.delay_idx, mod2);
                left = left * 0.72 + wet_r * 0.34;
                right = right * 0.72 + wet_l * 0.34;
            }
            self.delay_idx = (self.delay_idx + 1) & 2047;

            // 7) PAN (ON in Pan Cake Kore! Tempo-synced or free stereo auto-panner)
            let pan_hz = if mk1.pan_sync {
                let subdiv = match (mk1.pan_speed * 6.0).round() as usize {
                    0 => 0.5,
                    1 => 0.75,
                    2 => 1.0,
                    3 => 1.0,
                    4 => 1.5,
                    5 => 2.0,
                    _ => 4.0,
                };
                (mk1.tempo_bpm / 60.0) * subdiv
            } else {
                0.4 + mk1.pan_speed * 7.6
            };
            self.pancake_phase = (self.pancake_phase + TAU * pan_hz / sr).rem_euclid(TAU);
            if mk1.fx_pan && mk1.pan_intensity > 0.001 {
                let lfo = self.pancake_phase.sin();
                let pan_angle = (PI * 0.25) + lfo * (PI * 0.22 * mk1.pan_intensity.clamp(0.0, 1.0));
                left *= pan_angle.cos() * std::f32::consts::SQRT_2;
                right *= pan_angle.sin() * std::f32::consts::SQRT_2;
            }
        } else {
            // Clean Fender Bass / Phantom Brass / Wah Clav path
            if patch == SoundPatch::SlapBass {
                let mut m = (left + right) * 0.5;
                m = self.bass_hpf.process(m);
                m = self.bass_low.process(m);
                m = self.bass_box_cut.process(m);
                m = self.bass_growl.process(m);
                let air = self.bass_air.process(m);
                m = m * (1.0 - brightness * 0.65) + air * (brightness * 0.65);

                let abs_m = m.abs();
                let atk = 1.0 - (-1.0 / (0.025 * sr)).exp();
                let rel = 1.0 - (-1.0 / (0.180 * sr)).exp();
                if abs_m > self.bass_comp_env {
                    self.bass_comp_env += (abs_m - self.bass_comp_env) * atk;
                } else {
                    self.bass_comp_env += (abs_m - self.bass_comp_env) * rel;
                }
                let thresh = 0.22;
                let comp_gain = if self.bass_comp_env > thresh {
                    let over = self.bass_comp_env / thresh;
                    (1.0 / over.powf(0.55)).clamp(0.45, 1.0)
                } else {
                    1.0
                };
                m *= comp_gain * 1.12;
                left = m;
                right = m;
            }

            let drive = if patch == SoundPatch::SlapBass {
                1.0 + bark * 1.4
            } else {
                1.0 + bark * 2.5
            };
            left = fast_tanh(left * drive) / (1.0 + (drive - 1.0) * 0.35);
            right = fast_tanh(right * drive) / (1.0 + (drive - 1.0) * 0.35);

            if wah_amount > 0.001 {
                let (wl, wr) = self.process_autowah(left, right, brightness, sr);
                left = left * (1.0 - wah_amount) + wl * wah_amount;
                right = right * (1.0 - wah_amount) + wr * wah_amount;
            }

            self.chorus_phase = (self.chorus_phase + TAU * 0.75 / sr).rem_euclid(TAU);
            self.delay_l[self.delay_idx] = left;
            self.delay_r[self.delay_idx] = right;
            if matches!(patch, SoundPatch::PhantomBrass | SoundPatch::WahClav) && tremolo_depth > 0.01 {
                let mod_l = (0.008 + 0.003 * self.chorus_phase.sin()) * sr;
                let mod_r = (0.011 - 0.003 * self.chorus_phase.sin()) * sr;
                let wet_l = read_delay_linear(&self.delay_l, self.delay_idx, mod_l);
                let wet_r = read_delay_linear(&self.delay_r, self.delay_idx, mod_r);
                let mix = tremolo_depth * 0.42;
                left = left * (1.0 - mix * 0.5) + wet_r * mix;
                right = right * (1.0 - mix * 0.5) + wet_l * mix;
            }
            self.delay_idx = (self.delay_idx + 1) & 2047;
            self.pancake_phase = (self.pancake_phase + TAU * tremolo_rate_hz / sr).rem_euclid(TAU);
        }

        (left * gain, right * gain)
    }

    fn process_autowah(&mut self, left: f32, right: f32, brightness: f32, sr: f32) -> (f32, f32) {
        let level = (left.abs() + right.abs()) * 0.5;
        let wah_atk = 1.0 - (-1.0 / (0.008 * sr)).exp();
        let wah_rel = 1.0 - (-1.0 / (0.140 * sr)).exp();
        if level > self.wah_env {
            self.wah_env += (level - self.wah_env) * wah_atk;
        } else {
            self.wah_env += (level - self.wah_env) * wah_rel;
        }

        let sweep = (self.wah_env * 6.5).clamp(0.0, 1.0);
        let cutoff_hz = (280.0 + sweep * (2200.0 + brightness * 900.0)).min(sr * 0.42);
        let g = (PI * cutoff_hz / sr).tan();
        let k = 0.52;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3_l = left - self.svf_ic2_l;
        let v1_l = a1 * self.svf_ic1_l + a2 * v3_l;
        let v2_l = self.svf_ic2_l + a2 * self.svf_ic1_l + a3 * v3_l;
        self.svf_ic1_l = 2.0 * v1_l - self.svf_ic1_l;
        self.svf_ic2_l = 2.0 * v2_l - self.svf_ic2_l;
        let wah_l = v1_l * 1.15 + v2_l * 0.55;

        let v3_r = right - self.svf_ic2_r;
        let v1_r = a1 * self.svf_ic1_r + a2 * v3_r;
        let v2_r = self.svf_ic2_r + a2 * self.svf_ic1_r + a3 * v3_r;
        self.svf_ic1_r = 2.0 * v1_r - self.svf_ic1_r;
        self.svf_ic2_r = 2.0 * v2_r - self.svf_ic2_r;
        let wah_r = v1_r * 1.15 + v2_r * 0.55;

        (wah_l, wah_r)
    }

    fn process_phaser(&mut self, left: f32, right: f32, sr: f32) -> (f32, f32) {
        let lfo_l = 0.5 + 0.5 * self.phaser_phase.sin();
        let lfo_r = 0.5 + 0.5 * (self.phaser_phase + 0.8).sin();
        let freq_l = 320.0 + lfo_l * 1700.0;
        let freq_r = 320.0 + lfo_r * 1700.0;

        let ap_coeff_l = {
            let t = (PI * freq_l / sr).tan();
            (1.0 - t) / (1.0 + t)
        };
        let ap_coeff_r = {
            let t = (PI * freq_r / sr).tan();
            (1.0 - t) / (1.0 + t)
        };

        let mut xl = left + self.phaser_fb_l * 0.42;
        for st in &mut self.phaser_ap_l {
            let y = ap_coeff_l * xl + *st;
            *st = xl - ap_coeff_l * y;
            xl = y;
        }
        self.phaser_fb_l = xl.clamp(-1.5, 1.5);

        let mut xr = right + self.phaser_fb_r * 0.42;
        for st in &mut self.phaser_ap_r {
            let y = ap_coeff_r * xr + *st;
            *st = xr - ap_coeff_r * y;
            xr = y;
        }
        self.phaser_fb_r = xr.clamp(-1.5, 1.5);

        (xl, xr)
    }
}

#[inline]
fn voice_lpf(input: f32, cutoff_hz: f32, k: f32, sr: f32, ic1: &mut f32, ic2: &mut f32) -> f32 {
    let g = (PI * cutoff_hz / sr).tan();
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    let a3 = g * a2;
    let v3 = input - *ic2;
    let v1 = a1 * *ic1 + a2 * v3;
    let v2 = *ic2 + a2 * *ic1 + a3 * v3;
    *ic1 = 2.0 * v1 - *ic1;
    *ic2 = 2.0 * v2 - *ic2;
    v2
}

#[inline]
fn read_delay_linear(buf: &[f32; 2048], write_idx: usize, delay_samples: f32) -> f32 {
    let d = delay_samples.clamp(1.0, 2040.0);
    let i0 = d.floor() as usize;
    let frac = d - i0 as f32;
    let idx_a = (write_idx + 2048 - i0) & 2047;
    let idx_b = (write_idx + 2048 - (i0 + 1)) & 2047;
    buf[idx_a] * (1.0 - frac) + buf[idx_b] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_patches_produce_bounded_stereo_audio() {
        let mk1 = Mark1Settings::default();
        for patch in [
            SoundPatch::VelvetRhodes,
            SoundPatch::SlapBass,
            SoundPatch::PhantomBrass,
            SoundPatch::WahClav,
        ] {
            let mut synth = PhantomSynth::default();
            synth.set_sample_rate(44100.0);
            let note = if patch == SoundPatch::SlapBass { 40 } else { 62 };
            synth.note_on(note, 0, 0.9, false, patch, 0.0);
            let mut peak = 0.0f32;
            for _ in 0..2048 {
                let (l, r) = synth.process_sample(
                    patch, 0.7, 4.0, 250.0, 0.0, 0.30, 0.0, 0.60, 2.6, 0.7, &mk1,
                );
                assert!(l.is_finite() && r.is_finite());
                peak = peak.max(l.abs()).max(r.abs());
            }
            assert!(peak > 0.02 && peak < 1.2, "patch {:?} peak={}", patch, peak);
        }
    }

    #[test]
    fn scarbee_pancake_sweeps_stereo_field() {
        let mut synth = PhantomSynth::default();
        synth.set_sample_rate(44100.0);
        synth.note_on(60, 0, 0.85, false, SoundPatch::VelvetRhodes, 0.0);
        let mk1 = Mark1Settings {
            pan_intensity: 0.90,
            pan_sync: false,
            pan_speed: 0.27, // ~2.5 Hz
            ..Mark1Settings::default()
        };

        let mut sum_l_q1 = 0.0f32;
        let mut sum_r_q1 = 0.0f32;
        for _ in 0..8820 {
            let (l, r) = synth.process_sample(
                SoundPatch::VelvetRhodes,
                0.65,
                4.0,
                280.0,
                0.0,
                0.3,
                0.0,
                0.85,
                2.5,
                0.7,
                &mk1,
            );
            sum_l_q1 += l * l;
            sum_r_q1 += r * r;
        }

        let mut sum_l_q3 = 0.0f32;
        let mut sum_r_q3 = 0.0f32;
        for _ in 0..8820 {
            let (l, r) = synth.process_sample(
                SoundPatch::VelvetRhodes,
                0.65,
                4.0,
                280.0,
                0.0,
                0.3,
                0.0,
                0.85,
                2.5,
                0.7,
                &mk1,
            );
            sum_l_q3 += l * l;
            sum_r_q3 += r * r;
        }

        assert!(sum_r_q1 > sum_l_q1 * 1.25, "q1 L={} R={}", sum_l_q1, sum_r_q1);
        assert!(sum_l_q3 > sum_r_q3 * 1.25, "q3 L={} R={}", sum_l_q3, sum_r_q3);
    }
}
