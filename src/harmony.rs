use crate::params::{KeyRoot, ScaleMode, VoicingStyle};
use std::sync::atomic::{AtomicI8, AtomicU64, Ordering};

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];

#[derive(Debug, Clone, Copy)]
pub struct VoicedNote {
    pub note: u8,
    pub channel: u8,
    pub is_bass: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct VoicedChord {
    pub notes: [VoicedNote; 8],
    pub len: usize,
    pub name_bytes: [u8; 16],
    pub name_len: usize,
}

impl VoicedChord {
    pub fn slice(&self) -> &[VoicedNote] {
        &self.notes[..self.len]
    }
}

/// Lock-free state shared between the audio thread and egui editor.
pub struct SharedUiState {
    chord_name_lo: AtomicU64,
    chord_name_hi: AtomicU64,
    chord_name_len: AtomicU64,
    active_out_lo: AtomicU64,
    active_out_hi: AtomicU64,
    active_in_lo: AtomicU64,
    active_in_hi: AtomicU64,
    /// Audition note triggered from the UI mini-piano (-1 = none, 0..=127 = note on,
    /// encoded as `note` for trigger, and `audition_off` for release).
    pub ui_audition_on: AtomicI8,
    pub ui_audition_off: AtomicI8,
}

impl Default for SharedUiState {
    fn default() -> Self {
        Self {
            chord_name_lo: AtomicU64::new(0),
            chord_name_hi: AtomicU64::new(0),
            chord_name_len: AtomicU64::new(0),
            active_out_lo: AtomicU64::new(0),
            active_out_hi: AtomicU64::new(0),
            active_in_lo: AtomicU64::new(0),
            active_in_hi: AtomicU64::new(0),
            ui_audition_on: AtomicI8::new(-1),
            ui_audition_off: AtomicI8::new(-1),
        }
    }
}

impl SharedUiState {
    pub fn set_chord_name(&self, bytes: &[u8; 16], len: usize) {
        let mut lo_bytes = [0u8; 8];
        let mut hi_bytes = [0u8; 8];
        lo_bytes.copy_from_slice(&bytes[0..8]);
        hi_bytes.copy_from_slice(&bytes[8..16]);
        self.chord_name_lo
            .store(u64::from_le_bytes(lo_bytes), Ordering::Relaxed);
        self.chord_name_hi
            .store(u64::from_le_bytes(hi_bytes), Ordering::Relaxed);
        self.chord_name_len.store(len as u64, Ordering::Relaxed);
    }

    pub fn get_chord_name(&self) -> String {
        let len = (self.chord_name_len.load(Ordering::Relaxed) as usize).min(16);
        if len == 0 {
            return String::from("READY");
        }
        let lo = self.chord_name_lo.load(Ordering::Relaxed).to_le_bytes();
        let hi = self.chord_name_hi.load(Ordering::Relaxed).to_le_bytes();
        let mut buf = [0u8; 16];
        buf[0..8].copy_from_slice(&lo);
        buf[8..16].copy_from_slice(&hi);
        String::from_utf8_lossy(&buf[..len]).into_owned()
    }

    pub fn set_active_masks(&self, in_mask: u128, out_mask: u128) {
        self.active_in_lo.store(in_mask as u64, Ordering::Relaxed);
        self.active_in_hi
            .store((in_mask >> 64) as u64, Ordering::Relaxed);
        self.active_out_lo
            .store(out_mask as u64, Ordering::Relaxed);
        self.active_out_hi
            .store((out_mask >> 64) as u64, Ordering::Relaxed);
    }

    pub fn get_active_masks(&self) -> (u128, u128) {
        let in_lo = self.active_in_lo.load(Ordering::Relaxed) as u128;
        let in_hi = (self.active_in_hi.load(Ordering::Relaxed) as u128) << 64;
        let out_lo = self.active_out_lo.load(Ordering::Relaxed) as u128;
        let out_hi = (self.active_out_hi.load(Ordering::Relaxed) as u128) << 64;
        (in_lo | in_hi, out_lo | out_hi)
    }
}

fn scale_intervals(mode: ScaleMode) -> Option<&'static [u8; 7]> {
    match mode {
        ScaleMode::Dorian => Some(&[0, 2, 3, 5, 7, 9, 10]),
        ScaleMode::HarmonicMinor => Some(&[0, 2, 3, 5, 7, 8, 11]),
        ScaleMode::NaturalMinor => Some(&[0, 2, 3, 5, 7, 8, 10]),
        ScaleMode::Major => Some(&[0, 2, 4, 5, 7, 9, 11]),
        ScaleMode::Chromatic => None,
    }
}

/// Quantize a MIDI note to the nearest degree in the active scale.
/// Returns (quantized_midi_note, degree_index 0..7).
fn quantize_to_scale(note: u8, root: KeyRoot, intervals: &[u8; 7]) -> (u8, usize) {
    let key_pc = root.semitone() as i32;
    let rel = (note as i32) - key_pc;
    let oct = rel.div_euclid(12);
    let pc_rel = rel.rem_euclid(12) as u8;

    let mut best_idx = 0usize;
    let mut best_dist = 99i32;
    let mut best_semi = intervals[0] as i32;

    for (idx, &iv) in intervals.iter().enumerate() {
        let d = (pc_rel as i32) - (iv as i32);
        let abs_d = d.abs();
        if abs_d < best_dist {
            best_dist = abs_d;
            best_idx = idx;
            best_semi = iv as i32;
        }
    }

    // Check wrapped octave-up interval (12 + intervals[0]) in case pc_rel is 11 and intervals[0] is 0
    let wrap_dist = ((pc_rel as i32) - 12 - (intervals[0] as i32)).abs();
    if wrap_dist < best_dist {
        let snapped = (key_pc + (oct + 1) * 12 + intervals[0] as i32).clamp(0, 127) as u8;
        return (snapped, 0);
    }

    let snapped = (key_pc + oct * 12 + best_semi).clamp(0, 127) as u8;
    (snapped, best_idx)
}

#[derive(Clone, Copy)]
struct ChordTemplate {
    intervals: &'static [u8],
    suffix_triad: &'static str,
    suffix_7: &'static str,
    suffix_9: &'static str,
    suffix_full: &'static str,
}

const MIN9: ChordTemplate = ChordTemplate {
    intervals: &[0, 3, 7, 10, 14],
    suffix_triad: "m",
    suffix_7: "m7",
    suffix_9: "m9",
    suffix_full: "m9",
};

const MIN11: ChordTemplate = ChordTemplate {
    intervals: &[0, 3, 7, 10, 14, 17],
    suffix_triad: "m",
    suffix_7: "m7",
    suffix_9: "m9",
    suffix_full: "m11",
};

const MIN_MAJ9: ChordTemplate = ChordTemplate {
    intervals: &[0, 3, 7, 11, 14],
    suffix_triad: "m",
    suffix_7: "mMaj7",
    suffix_9: "mMaj9",
    suffix_full: "mMaj9",
};

const MAJ9: ChordTemplate = ChordTemplate {
    intervals: &[0, 4, 7, 11, 14],
    suffix_triad: "",
    suffix_7: "maj7",
    suffix_9: "maj9",
    suffix_full: "maj9",
};

const MAJ9_SHARP11: ChordTemplate = ChordTemplate {
    intervals: &[0, 4, 7, 11, 14, 18],
    suffix_triad: "",
    suffix_7: "maj7",
    suffix_9: "maj9",
    suffix_full: "maj9#11",
};

const MAJ7_SHARP5: ChordTemplate = ChordTemplate {
    intervals: &[0, 4, 8, 11, 14],
    suffix_triad: "+",
    suffix_7: "maj7#5",
    suffix_9: "maj9#5",
    suffix_full: "maj9#5",
};

const DOM13: ChordTemplate = ChordTemplate {
    intervals: &[0, 4, 7, 10, 14, 21],
    suffix_triad: "",
    suffix_7: "7",
    suffix_9: "9",
    suffix_full: "13",
};

const DOM7_SHARP9: ChordTemplate = ChordTemplate {
    intervals: &[0, 4, 7, 10, 15],
    suffix_triad: "",
    suffix_7: "7",
    suffix_9: "7#9",
    suffix_full: "7#9",
};

const DOM7_FLAT9: ChordTemplate = ChordTemplate {
    intervals: &[0, 4, 7, 10, 13],
    suffix_triad: "",
    suffix_7: "7",
    suffix_9: "7b9",
    suffix_full: "7b9",
};

const SUS9: ChordTemplate = ChordTemplate {
    intervals: &[0, 5, 7, 10, 14],
    suffix_triad: "sus4",
    suffix_7: "7sus4",
    suffix_9: "9sus4",
    suffix_full: "9sus4",
};

const HALF_DIM9: ChordTemplate = ChordTemplate {
    intervals: &[0, 3, 6, 10, 14],
    suffix_triad: "dim",
    suffix_7: "m7b5",
    suffix_9: "m9b5",
    suffix_full: "m9b5",
};

const DIM7: ChordTemplate = ChordTemplate {
    intervals: &[0, 3, 6, 9],
    suffix_triad: "dim",
    suffix_7: "dim7",
    suffix_9: "dim7",
    suffix_full: "dim7",
};

fn pick_template(mode: ScaleMode, degree: usize, style: VoicingStyle) -> ChordTemplate {
    if style == VoicingStyle::DimPassing {
        return DIM7;
    }

    match mode {
        ScaleMode::Chromatic => match style {
            VoicingStyle::AcidJazz => MIN9,
            VoicingStyle::NeoSoul => MIN11,
            VoicingStyle::FunkStab => DOM7_SHARP9,
            VoicingStyle::DimPassing => DIM7,
        },
        ScaleMode::Dorian => match (degree, style) {
            (0, VoicingStyle::NeoSoul) => MIN11,
            (0, _) => MIN9,
            (1, VoicingStyle::NeoSoul) => MIN11,
            (1, _) => MIN9,
            (2, VoicingStyle::NeoSoul) => MAJ9_SHARP11,
            (2, _) => MAJ9,
            (3, VoicingStyle::FunkStab) => DOM7_SHARP9,
            (3, VoicingStyle::NeoSoul) => SUS9,
            (3, _) => DOM13,
            (4, _) => MIN9,
            (5, _) => HALF_DIM9,
            (6, _) => MAJ9,
            _ => MIN9,
        },
        ScaleMode::HarmonicMinor => match (degree, style) {
            (0, VoicingStyle::FunkStab) => MIN9,
            (0, _) => MIN_MAJ9,
            (1, _) => HALF_DIM9,
            (2, _) => MAJ7_SHARP5,
            (3, VoicingStyle::NeoSoul) => MIN11,
            (3, _) => MIN9,
            (4, VoicingStyle::FunkStab) => DOM7_SHARP9,
            (4, _) => DOM7_FLAT9,
            (5, _) => MAJ9,
            (6, _) => DIM7,
            _ => MIN_MAJ9,
        },
        ScaleMode::NaturalMinor => match (degree, style) {
            (0, VoicingStyle::NeoSoul) => MIN11,
            (0, _) => MIN9,
            (1, _) => HALF_DIM9,
            (2, _) => MAJ9,
            (3, VoicingStyle::NeoSoul) => MIN11,
            (3, _) => MIN9,
            (4, VoicingStyle::FunkStab) => DOM7_SHARP9,
            (4, _) => MIN9,
            (5, _) => MAJ9,
            (6, VoicingStyle::FunkStab) => DOM7_SHARP9,
            (6, _) => DOM13,
            _ => MIN9,
        },
        ScaleMode::Major => match (degree, style) {
            (0, VoicingStyle::NeoSoul) => MAJ9_SHARP11,
            (0, _) => MAJ9,
            (1, VoicingStyle::NeoSoul) => MIN11,
            (1, _) => MIN9,
            (2, _) => MIN9,
            (3, _) => MAJ9_SHARP11,
            (4, VoicingStyle::FunkStab) => DOM7_SHARP9,
            (4, VoicingStyle::NeoSoul) => SUS9,
            (4, _) => DOM13,
            (5, VoicingStyle::NeoSoul) => MIN11,
            (5, _) => MIN9,
            (6, _) => HALF_DIM9,
            _ => MAJ9,
        },
    }
}

pub fn build_voiced_chord(
    in_note: u8,
    in_channel: u8,
    key_root: KeyRoot,
    scale_mode: ScaleMode,
    style: VoicingStyle,
    voice_lead: bool,
    center_note: u8,
    extensions: f32,
    bass_split: bool,
    bass_channel_1based: u8,
) -> VoicedChord {
    let (chord_root_midi, degree) = match scale_intervals(scale_mode) {
        Some(ivs) => quantize_to_scale(in_note, key_root, ivs),
        None => (in_note, 0),
    };

    let template = pick_template(scale_mode, degree, style);

    // Determine how many intervals to include based on `extensions` (0.0..=1.0)
    let (max_tones, suffix) = if extensions < 0.25 {
        (3, template.suffix_triad)
    } else if extensions < 0.55 {
        (4, template.suffix_7)
    } else if extensions < 0.80 {
        (5, template.suffix_9)
    } else {
        (template.intervals.len(), template.suffix_full)
    };
    let tone_count = max_tones.min(template.intervals.len());

    let mut out = VoicedChord {
        notes: [VoicedNote {
            note: 0,
            channel: in_channel,
            is_bass: false,
        }; 8],
        len: 0,
        name_bytes: [0u8; 16],
        name_len: 0,
    };

    // Format chord name e.g. "Dm9" into fixed buffer
    let root_pc = (chord_root_midi % 12) as usize;
    let root_str = NOTE_NAMES[root_pc].as_bytes();
    let suffix_str = suffix.as_bytes();
    let mut name_pos = 0usize;
    for &b in root_str {
        if name_pos < 16 {
            out.name_bytes[name_pos] = b;
            name_pos += 1;
        }
    }
    for &b in suffix_str {
        if name_pos < 16 {
            out.name_bytes[name_pos] = b;
            name_pos += 1;
        }
    }
    out.name_len = name_pos;

    let mut upper_notes = [0u8; 7];
    let mut upper_len = 0usize;

    if voice_lead {
        let center = center_note as i32;
        for (idx, &iv) in template.intervals[..tone_count].iter().enumerate() {
            // In AcidJazz / NeoSoul with bass split, drop the duplicated root above 3rd
            // only if we have >= 4 notes so rootless Rhodes voicings sound super clean!
            if bass_split && idx == 0 && tone_count >= 4 {
                continue;
            }
            let pc = ((chord_root_midi as i32) + (iv as i32)).rem_euclid(12);
            // Find octave placement closest to center_note (within [center - 6 .. center + 7])
            let mut best_midi = (center - (center.rem_euclid(12)) + pc).clamp(24, 108);
            while best_midi < center - 6 {
                best_midi += 12;
            }
            while best_midi > center + 7 {
                best_midi -= 12;
            }
            let midi = best_midi.clamp(0, 127) as u8;
            if !upper_notes[..upper_len].contains(&midi) && upper_len < 7 {
                upper_notes[upper_len] = midi;
                upper_len += 1;
            }
        }
        // Sort ascending so strumming rolls cleanly low -> high
        upper_notes[..upper_len].sort_unstable();
    } else {
        for &iv in &template.intervals[..tone_count] {
            let midi = ((chord_root_midi as i32) + (iv as i32)).clamp(0, 127) as u8;
            if !upper_notes[..upper_len].contains(&midi) && upper_len < 7 {
                upper_notes[upper_len] = midi;
                upper_len += 1;
            }
        }
    }

    if bass_split {
        // Place bass note in C2..B2 (MIDI 36..=47)
        let bass_midi = (36 + (chord_root_midi % 12)).clamp(0, 127);
        let bass_ch = bass_channel_1based.saturating_sub(1).min(15);
        out.notes[out.len] = VoicedNote {
            note: bass_midi,
            channel: bass_ch,
            is_bass: true,
        };
        out.len += 1;
    }

    for &midi in &upper_notes[..upper_len] {
        if out.len < 8 {
            out.notes[out.len] = VoicedNote {
                note: midi,
                channel: in_channel,
                is_bass: false,
            };
            out.len += 1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d_dorian_white_keys_produce_persona_chords() {
        // Pressing D4 (62) in D Dorian AcidJazz should give Dm9
        let chord = build_voiced_chord(
            62,
            0,
            KeyRoot::D,
            ScaleMode::Dorian,
            VoicingStyle::AcidJazz,
            true,
            60,
            0.85,
            true,
            2,
        );
        let name = std::str::from_utf8(&chord.name_bytes[..chord.name_len]).unwrap();
        assert_eq!(name, "Dm9");
        assert!(chord.len >= 5); // bass + rootless 3,5,7,9

        // Pressing G4 (67) in D Dorian AcidJazz should give G13
        let g_chord = build_voiced_chord(
            67,
            0,
            KeyRoot::D,
            ScaleMode::Dorian,
            VoicingStyle::AcidJazz,
            true,
            60,
            0.9,
            true,
            2,
        );
        let g_name = std::str::from_utf8(&g_chord.name_bytes[..g_chord.name_len]).unwrap();
        assert_eq!(g_name, "G13");
    }

    #[test]
    fn harmonic_minor_boss_chords_and_voice_leading() {
        // E Harmonic Minor degree 4 (B) with FunkStab should yield B7#9
        let v_chord = build_voiced_chord(
            59,
            0,
            KeyRoot::E,
            ScaleMode::HarmonicMinor,
            VoicingStyle::FunkStab,
            true,
            60,
            0.9,
            true,
            2,
        );
        let name = std::str::from_utf8(&v_chord.name_bytes[..v_chord.name_len]).unwrap();
        assert_eq!(name, "B7#9");

        // All upper notes should stay tightly clustered around center_note=60
        for vn in v_chord.slice() {
            if vn.is_bass {
                assert!((36..=47).contains(&vn.note));
                assert_eq!(vn.channel, 1); // 1-based channel 2 -> 0-based channel 1
            } else {
                assert!((54..=67).contains(&vn.note));
                assert_eq!(vn.channel, 0);
            }
        }
    }
}
