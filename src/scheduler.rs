use crate::harmony::{build_voiced_chord, SharedUiState, VoicedNote};
use crate::params::{KeyRoot, PhantomParams, ScaleMode, StrumDir, VoicingStyle};
use crate::synth::PhantomSynth;
use nih_plug::prelude::*;

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];

const MAX_PENDING: usize = 256;

#[derive(Debug, Clone, Copy)]
pub struct PendingNoteOn {
    pub active: bool,
    pub src_note: u8,
    pub note: u8,
    pub channel: u8,
    pub velocity: f32,
    pub is_bass: bool,
    pub samples_left: u32,
}

impl Default for PendingNoteOn {
    fn default() -> Self {
        Self {
            active: false,
            src_note: 0,
            note: 0,
            channel: 0,
            velocity: 0.0,
            is_bass: false,
            samples_left: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActiveChordSlot {
    pub active: bool,
    pub notes: [VoicedNote; 8],
    pub fired: [bool; 8],
    pub len: usize,
}

impl Default for ActiveChordSlot {
    fn default() -> Self {
        Self {
            active: false,
            notes: [VoicedNote {
                note: 0,
                channel: 0,
                is_bass: false,
            }; 8],
            fired: [false; 8],
            len: 0,
        }
    }
}

pub struct ChordScheduler {
    pending: [PendingNoteOn; MAX_PENDING],
    /// Keyed by `[channel (0..16)][src_note (0..128)]`? To keep memory tiny,
    /// `[ActiveChordSlot; 128]` tracks input notes (if multi-channel same note, we check channel too,
    /// or use 128 slots keyed by note since FL Piano Roll uses channel 0 for notes).
    active_chords: [ActiveChordSlot; 128],
    in_mask: u128,
    out_ref_counts: [u8; 128],
    rng_state: u64,
    alternate_flip: bool,
    sample_rate: f32,
}

impl Default for ChordScheduler {
    fn default() -> Self {
        Self {
            pending: [PendingNoteOn::default(); MAX_PENDING],
            active_chords: [ActiveChordSlot::default(); 128],
            in_mask: 0,
            out_ref_counts: [0; 128],
            rng_state: 0x5EED_F00D_CAFE_BEEF,
            alternate_flip: false,
            sample_rate: 44100.0,
        }
    }
}

impl ChordScheduler {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
    }

    pub fn reset(&mut self) {
        self.pending = [PendingNoteOn::default(); MAX_PENDING];
        self.active_chords = [ActiveChordSlot::default(); 128];
        self.in_mask = 0;
        self.out_ref_counts = [0; 128];
        self.alternate_flip = false;
    }

    #[inline]
    fn next_rand_signed(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        // Map to [-1.0, 1.0]
        ((x & 0xFFFF) as f32 / 32767.5) - 1.0
    }

    #[inline]
    fn out_mask(&self) -> u128 {
        let mut mask = 0u128;
        for (note, &cnt) in self.out_ref_counts.iter().enumerate() {
            if cnt > 0 {
                mask |= 1u128 << note;
            }
        }
        mask
    }

    fn sync_ui(&self, ui_state: &SharedUiState) {
        ui_state.set_active_masks(self.in_mask, self.out_mask());
    }

    pub fn trigger_note_on<P: Plugin>(
        &mut self,
        src_note: u8,
        src_channel: u8,
        velocity: f32,
        timing: u32,
        params: &PhantomParams,
        synth: &mut PhantomSynth,
        ui_state: &SharedUiState,
        context: &mut impl ProcessContext<P>,
    ) {
        // Release any previous active chord for this input key first (prevents stuck notes)
        self.trigger_note_off(src_note, src_channel, timing, synth, ui_state, context);

        if !params.chord_assist.value() {
            // Single-note instrument mode (default): play exact note without generating chords
            let patch = params.sound_patch.value();
            let glide_ms = params.glide_ms.value();

            let mut name_buf = [0u8; 16];
            let pc = (src_note % 12) as usize;
            let oct = (src_note as i32 / 12) - 1;
            let n_bytes = NOTE_NAMES[pc].as_bytes();
            let mut nlen = 0usize;
            for &b in n_bytes {
                name_buf[nlen] = b;
                nlen += 1;
            }
            if oct < 0 {
                name_buf[nlen] = b'-';
                nlen += 1;
                name_buf[nlen] = b'1';
                nlen += 1;
            } else if oct >= 10 {
                name_buf[nlen] = b'0' + (oct / 10) as u8;
                nlen += 1;
                name_buf[nlen] = b'0' + (oct % 10) as u8;
                nlen += 1;
            } else {
                name_buf[nlen] = b'0' + oct as u8;
                nlen += 1;
            }
            ui_state.set_chord_name(&name_buf, nlen);
            self.in_mask |= 1u128 << (src_note as usize);

            let mut slot = ActiveChordSlot::default();
            slot.active = true;
            slot.notes[0] = VoicedNote {
                note: src_note,
                channel: src_channel,
                is_bass: src_note < 48,
            };
            slot.fired[0] = true;
            slot.len = 1;
            self.active_chords[src_note as usize] = slot;

            self.fire_single_note_on(slot.notes[0], velocity, timing, patch, glide_ms, synth, context);
            self.sync_ui(ui_state);
            return;
        }

        let key_root: KeyRoot = params.key_root.value();
        let scale_mode: ScaleMode = params.scale_mode.value();
        let style: VoicingStyle = params.voicing_style.value();
        let voice_lead = params.voice_lead.value();
        let center_note = params.center_note.value() as u8;
        let extensions = params.extensions.value();
        let bass_split = params.bass_split.value();
        let bass_ch = params.bass_channel.value() as u8;
        let patch = params.sound_patch.value();
        let glide_ms = params.glide_ms.value();

        let chord = build_voiced_chord(
            src_note,
            src_channel,
            key_root,
            scale_mode,
            style,
            voice_lead,
            center_note,
            extensions,
            bass_split,
            bass_ch,
        );

        ui_state.set_chord_name(&chord.name_bytes, chord.name_len);
        self.in_mask |= 1u128 << (src_note as usize);

        let strum_ms = params.strum_ms.value();
        let strum_dir = params.strum_dir.value();
        let hum_vel = params.humanize_vel.value();

        let effective_strum_ms = match strum_dir {
            StrumDir::Tight => 0.0,
            _ => strum_ms,
        };
        let step_samples = ((effective_strum_ms / 1000.0) * self.sample_rate).round() as u32;

        let reverse = match strum_dir {
            StrumDir::Down => true,
            StrumDir::Alternate => {
                self.alternate_flip = !self.alternate_flip;
                self.alternate_flip
            }
            _ => false,
        };

        let mut slot = ActiveChordSlot {
            active: true,
            notes: chord.notes,
            fired: [false; 8],
            len: chord.len,
        };

        // Count upper notes for reverse indexing (bass always fires at 0 delay so groove stays tight)
        let upper_count = chord.slice().iter().filter(|n| !n.is_bass).count();
        let mut upper_idx = 0usize;

        for (i, &vn) in chord.slice().iter().enumerate() {
            let delay = if vn.is_bass || step_samples == 0 || upper_count <= 1 {
                0
            } else {
                let order = if reverse {
                    upper_count - 1 - upper_idx
                } else {
                    upper_idx
                };
                upper_idx += 1;
                (order as u32) * step_samples
            };

            let jitter = self.next_rand_signed() * hum_vel;
            let vel = (velocity * (1.0 + jitter)).clamp(0.05, 1.0);

            if delay == 0 {
                self.fire_single_note_on(vn, vel, timing, patch, glide_ms, synth, context);
                slot.fired[i] = true;
            } else {
                self.enqueue_pending(PendingNoteOn {
                    active: true,
                    src_note,
                    note: vn.note,
                    channel: vn.channel,
                    velocity: vel,
                    is_bass: vn.is_bass,
                    samples_left: delay,
                });
            }
        }

        self.active_chords[src_note as usize] = slot;
        self.sync_ui(ui_state);
    }

    pub fn trigger_note_off<P: Plugin>(
        &mut self,
        src_note: u8,
        _src_channel: u8,
        timing: u32,
        synth: &mut PhantomSynth,
        ui_state: &SharedUiState,
        context: &mut impl ProcessContext<P>,
    ) {
        // Cancel any unfired pending strums for this src_note
        for p in &mut self.pending {
            if p.active && p.src_note == src_note {
                p.active = false;
            }
        }

        self.in_mask &= !(1u128 << (src_note as usize));

        let slot = &mut self.active_chords[src_note as usize];
        if slot.active {
            for i in 0..slot.len {
                if slot.fired[i] {
                    let vn = slot.notes[i];
                    let ref_cnt = &mut self.out_ref_counts[vn.note as usize];
                    *ref_cnt = ref_cnt.saturating_sub(1);

                    // Only send NoteOff if no other held chord is currently holding this pitch
                    if *ref_cnt == 0 {
                        synth.note_off(vn.note, vn.channel);
                        context.send_event(NoteEvent::NoteOff {
                            timing,
                            voice_id: None,
                            channel: vn.channel,
                            note: vn.note,
                            velocity: 0.0,
                        });
                    }
                }
            }
            slot.active = false;
            slot.len = 0;
        }

        self.sync_ui(ui_state);
    }

    pub fn tick_sample<P: Plugin>(
        &mut self,
        timing: u32,
        params: &PhantomParams,
        synth: &mut PhantomSynth,
        ui_state: &SharedUiState,
        context: &mut impl ProcessContext<P>,
    ) {
        let mut any_fired = false;
        let patch = params.sound_patch.value();
        let glide_ms = params.glide_ms.value();

        for p_idx in 0..MAX_PENDING {
            if !self.pending[p_idx].active {
                continue;
            }
            if self.pending[p_idx].samples_left > 0 {
                self.pending[p_idx].samples_left -= 1;
            }
            if self.pending[p_idx].samples_left == 0 {
                let p = self.pending[p_idx];
                self.pending[p_idx].active = false;

                if self.active_chords[p.src_note as usize].active {
                    let vn = VoicedNote {
                        note: p.note,
                        channel: p.channel,
                        is_bass: p.is_bass,
                    };
                    self.fire_single_note_on(vn, p.velocity, timing, patch, glide_ms, synth, context);
                    let slot = &mut self.active_chords[p.src_note as usize];
                    for i in 0..slot.len {
                        if !slot.fired[i]
                            && slot.notes[i].note == p.note
                            && slot.notes[i].channel == p.channel
                        {
                            slot.fired[i] = true;
                            break;
                        }
                    }
                    any_fired = true;
                }
            }
        }

        if any_fired {
            self.sync_ui(ui_state);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fire_single_note_on<P: Plugin>(
        &mut self,
        vn: VoicedNote,
        velocity: f32,
        timing: u32,
        patch: crate::params::SoundPatch,
        glide_ms: f32,
        synth: &mut PhantomSynth,
        context: &mut impl ProcessContext<P>,
    ) {
        let cnt = &mut self.out_ref_counts[vn.note as usize];
        // If already sounding from another overlapping chord, send quick NoteOff first
        if *cnt > 0 {
            context.send_event(NoteEvent::NoteOff {
                timing,
                voice_id: None,
                channel: vn.channel,
                note: vn.note,
                velocity: 0.0,
            });
        }
        *cnt = cnt.saturating_add(1);

        synth.note_on(vn.note, vn.channel, velocity, vn.is_bass, patch, glide_ms);
        context.send_event(NoteEvent::NoteOn {
            timing,
            voice_id: None,
            channel: vn.channel,
            note: vn.note,
            velocity,
        });
    }

    fn enqueue_pending(&mut self, item: PendingNoteOn) {
        for slot in &mut self.pending {
            if !slot.active {
                *slot = item;
                return;
            }
        }
    }
}
