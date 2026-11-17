use crate::params::SoundPatch;
use std::f32::consts::{PI, TAU};

const MAX_VOICES: usize = 24;

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
    age_samples: u64,
    env: f32,
    pluck_env: f32,
    filter_ic1: f32,
    filter_ic2: f32,
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
            age_samples: 0,
            env: 0.0,
            pluck_env: 0.0,
            filter_ic1: 0.0,
            filter_ic2: 0.0,
            internal_id: 0,
        }
    }
}

pub struct PhantomSynth {
    sample_rate: f32,
    voices: [SynthVoice; MAX_VOICES],
    next_id: u64,
    last_freq: Option<f32>,
    pitch_bend_semis: f32,
    tremolo_phase: f32,
    chorus_phase: f32,
    // Simple stereo delay line for warm chorus/ensemble width
    delay_l: [f32; 2048],
    delay_r: [f32; 2048],
    delay_idx: usize,
    // Auto-wah state
    wah_env: f32,
    svf_ic1_l: f32,
    svf_ic2_l: f32,
    svf_ic1_r: f32,
    svf_ic2_r: f32,
}

impl Default for PhantomSynth {
    fn default() -> Self {
        Self {
            sample_rate: 44100.0,
            voices: [SynthVoice::default(); MAX_VOICES],
            next_id: 1,
            last_freq: None,
            pitch_bend_semis: 0.0,
            tremolo_phase: 0.0,
            chorus_phase: 0.0,
            delay_l: [0.0; 2048],
            delay_r: [0.0; 2048],
            delay_idx: 0,
            wah_env: 0.0,
            svf_ic1_l: 0.0,
            svf_ic2_l: 0.0,
            svf_ic1_r: 0.0,
            svf_ic2_r: 0.0,
        }
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

/// PolyBLEP antialiasing residual for phase `t` in [0, 1) and phase increment `dt`.
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
    let raw = 2.0 * t - 1.0;
    raw - poly_blep(t, dt)
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
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
    }

    pub fn reset(&mut self) {
        self.voices = [SynthVoice::default(); MAX_VOICES];
        self.last_freq = None;
        self.pitch_bend_semis = 0.0;
        self.tremolo_phase = 0.0;
        self.chorus_phase = 0.0;
        self.delay_l = [0.0; 2048];
        self.delay_r = [0.0; 2048];
        self.delay_idx = 0;
        self.wah_env = 0.0;
        self.svf_ic1_l = 0.0;
        self.svf_ic2_l = 0.0;
        self.svf_ic1_r = 0.0;
        self.svf_ic2_r = 0.0;
    }

    pub fn set_pitch_bend(&mut self, norm_value: f32) {
        // nih-plug MidiPitchBend is normalized 0.0..=1.0 (0.5 = center)
        self.pitch_bend_semis = (norm_value - 0.5) * 4.0; // +/- 2 semitones
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

        // SlapBass is monophonic when glide > 0 for authentic bass slides
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
            age_samples: 0,
            env: if keep_phase { prev.env.max(0.25) } else { 0.0 },
            pluck_env: 1.0,
            filter_ic1: if keep_phase { prev.filter_ic1 } else { 0.0 },
            filter_ic2: if keep_phase { prev.filter_ic2 } else { 0.0 },
            internal_id: id,
        };
    }

    pub fn note_off(&mut self, note: u8, channel: u8) {
        for v in &mut self.voices {
            if v.active && !v.releasing && v.note == note && v.channel == channel {
                v.releasing = true;
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.releasing = true;
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
    ) -> (f32, f32) {
        let sr = self.sample_rate;
        let atk_s = (attack_ms / 1000.0).max(0.0015);
        let rel_s = (release_ms / 1000.0).max(0.015);
        let attack_coeff = 1.0 - (-1.0 / (atk_s * sr)).exp();
        let release_coeff = (-1.0 / (rel_s * sr)).exp();
        let hold_decay = match patch {
            SoundPatch::VelvetRhodes => (-1.0 / (2.6 * sr)).exp(),
            SoundPatch::SlapBass => (-1.0 / (1.4 * sr)).exp(),
            SoundPatch::PhantomBrass => 1.0, // sustained horns
            SoundPatch::WahClav => (-1.0 / (0.85 * sr)).exp(),
        };
        let pluck_decay = match patch {
            SoundPatch::VelvetRhodes => (-1.0 / (0.065 * sr)).exp(),
            SoundPatch::SlapBass => (-1.0 / (0.085 * sr)).exp(),
            SoundPatch::PhantomBrass => (-1.0 / (0.140 * sr)).exp(),
            SoundPatch::WahClav => (-1.0 / (0.095 * sr)).exp(),
        };
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
                if v.env < 0.0002 {
                    v.active = false;
                    continue;
                }
            } else {
                // Smooth rise toward 1.0 during attack, then natural decay if applicable
                if v.age_samples < (atk_s * 3.0 * sr) as u64 {
                    v.env += (1.0 - v.env) * attack_coeff;
                } else {
                    v.env *= hold_decay;
                }
            }

            v.pluck_env *= pluck_decay;
            v.age_samples += 1;

            v.freq += (v.target_freq - v.freq) * glide_coeff;
            let f_eff = (v.freq * bend_ratio).clamp(20.0, sr * 0.45);
            let dt = f_eff / sr;

            // Normalized phases [0, 1)
            v.phase = (v.phase + dt).fract();

            let (sig_l, sig_r) = match patch {
                SoundPatch::VelvetRhodes => {
                    let ph = v.phase * TAU;
                    let tine_ratio = if f_eff > 900.0 {
                        3.0
                    } else if v.is_bass {
                        2.0
                    } else {
                        7.0
                    };
                    v.tine_phase = (v.tine_phase + dt * tine_ratio).fract();

                    let bark_idx = (0.3 + bark * 1.4 + brightness * 0.45)
                        * v.velocity
                        * (0.35 + 0.65 * v.env);
                    let body_mod = (ph * 2.0).sin() * bark_idx * if v.is_bass { 0.45 } else { 0.72 };
                    let body = (ph + body_mod).sin();
                    let tine_amt = if v.is_bass {
                        0.06
                    } else {
                        (0.12 + 0.16 * brightness) * v.velocity
                    };
                    let tine = (v.tine_phase * TAU).sin() * v.pluck_env * tine_amt;

                    let s = (body * 0.74 + tine) * v.env * v.velocity * 0.28;
                    let pan = ((v.note as f32 - 60.0) / 36.0).clamp(-0.55, 0.55);
                    (s * (1.0 - pan * 0.45), s * (1.0 + pan * 0.45))
                }
                SoundPatch::SlapBass => {
                    // Punchy slap transient + sub sine + filtered midrange saw/square
                    let sub = (v.phase * TAU).sin();
                    let body = blep_saw(v.phase, dt) * 0.65 + blep_pulse(v.phase, dt, 0.35) * 0.35;
                    let slap_pop = (v.phase * TAU * 3.0).sin() * v.pluck_env * (0.35 + 0.5 * bark);

                    let raw = sub * 0.62 + body * 0.52 + slap_pop;
                    let cutoff = (160.0
                        + brightness * 1400.0
                        + v.pluck_env * (1800.0 + 2400.0 * v.velocity * brightness))
                        .min(sr * 0.42);
                    let filtered = voice_lpf(raw, cutoff, 0.58, sr, &mut v.filter_ic1, &mut v.filter_ic2);
                    let s = filtered * v.env * v.velocity * 0.44;
                    (s, s)
                }
                SoundPatch::PhantomBrass => {
                    // 3 detuned PolyBLEP saws with subtle pitch scoop on attack & brassy filter sweep
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
                SoundPatch::WahClav => {
                    // Thin funky pulse clavinet wave (18% pulse width + octave harmonic)
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

        // 1) Tube / Amp Bark Saturation
        let drive = 1.0 + bark * 2.6;
        left = fast_tanh(left * drive) / (1.0 + bark * 0.55);
        right = fast_tanh(right * drive) / (1.0 + bark * 0.55);

        // 2) Envelope-Following Auto-Wah (Cytomic SVF Bandpass/Lowpass blend)
        if wah_amount > 0.001 {
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

            left = left * (1.0 - wah_amount) + wah_l * wah_amount;
            right = right * (1.0 - wah_amount) + wah_r * wah_amount;
        }

        // 3) Stereo Chorus (for Brass/Clav) + Suitcase Tremolo (for Rhodes)
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

        self.tremolo_phase = (self.tremolo_phase + TAU * tremolo_rate_hz / sr).rem_euclid(TAU);
        if patch == SoundPatch::VelvetRhodes {
            let lfo = self.tremolo_phase.sin();
            let trem_l = 1.0 - tremolo_depth * 0.45 * (0.5 + 0.5 * lfo);
            let trem_r = 1.0 - tremolo_depth * 0.45 * (0.5 - 0.5 * lfo);
            left *= trem_l;
            right *= trem_r;
        }

        (left * gain, right * gain)
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
        for patch in [
            SoundPatch::VelvetRhodes,
            SoundPatch::SlapBass,
            SoundPatch::PhantomBrass,
            SoundPatch::WahClav,
        ] {
            let mut synth = PhantomSynth::default();
            synth.set_sample_rate(44100.0);
            synth.note_on(62, 0, 0.9, false, patch, 0.0);
            let mut peak = 0.0f32;
            for _ in 0..2048 {
                let (l, r) = synth.process_sample(patch, 0.7, 4.0, 250.0, 0.0, 0.35, 0.25, 0.35, 3.8, 0.7);
                assert!(l.is_finite() && r.is_finite());
                peak = peak.max(l.abs()).max(r.abs());
            }
            assert!(peak > 0.02 && peak < 1.2, "patch {:?} peak={}", patch, peak);
        }
    }
}
