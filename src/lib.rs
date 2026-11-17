use nih_plug::prelude::*;
use std::sync::{atomic::Ordering, Arc};

mod editor;
mod harmony;
mod params;
mod scheduler;
mod synth;
mod updater;

use harmony::SharedUiState;
use params::PhantomParams;
use scheduler::ChordScheduler;
use synth::PhantomSynth;
use updater::UpdaterHandle;

pub struct PhantomVoicer {
    params: Arc<PhantomParams>,
    ui_state: Arc<SharedUiState>,
    scheduler: ChordScheduler,
    synth: PhantomSynth,
    updater: UpdaterHandle,
}

impl Default for PhantomVoicer {
    fn default() -> Self {
        Self {
            params: Arc::new(PhantomParams::default()),
            ui_state: Arc::new(SharedUiState::default()),
            scheduler: ChordScheduler::default(),
            synth: PhantomSynth::default(),
            updater: UpdaterHandle::default(),
        }
    }
}

impl Plugin for PhantomVoicer {
    const NAME: &'static str = "Phantom Voicer";
    const VENDOR: &'static str = "Endio Audio";
    const URL: &'static str = "https://github.com";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.ui_state.clone(),
            self.updater.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.scheduler.set_sample_rate(buffer_config.sample_rate);
        self.synth.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.scheduler.reset();
        self.synth.reset();
        self.ui_state.set_active_masks(0, 0);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        if num_samples == 0 {
            return ProcessStatus::Normal;
        }

        // Handle mini-piano mouse auditions from the GUI
        let off_note = self.ui_state.ui_audition_off.swap(-1, Ordering::Relaxed);
        if off_note >= 0 {
            self.scheduler.trigger_note_off(
                off_note as u8,
                0,
                0,
                &mut self.synth,
                &self.ui_state,
                context,
            );
        }
        let on_note = self.ui_state.ui_audition_on.swap(-1, Ordering::Relaxed);
        if on_note >= 0 {
            self.scheduler.trigger_note_on(
                on_note as u8,
                0,
                0.85,
                0,
                &self.params,
                &mut self.synth,
                &self.ui_state,
                context,
            );
        }

        let preview_on = self.params.preview_enabled.value();
        let patch = self.params.sound_patch.value();
        let brightness = self.params.brightness.value();
        let attack_ms = self.params.attack_ms.value();
        let release_ms = self.params.release_ms.value();
        let glide_ms = self.params.glide_ms.value();
        let bark = self.params.rhodes_bark.value();
        let wah_amt = self.params.wah_amount.value();
        let trem_depth = self.params.tremolo_depth.value();
        let trem_rate = self.params.tremolo_rate_hz.value();

        let mut next_ev = context.next_event();

        for (sample_idx, channel_samples) in buffer.iter_samples().enumerate() {
            let s_u32 = sample_idx as u32;

            while let Some(ev) = next_ev.as_ref() {
                if ev.timing() > s_u32 {
                    break;
                }
                let ev = next_ev.take().unwrap();
                self.handle_incoming_event(ev, s_u32, context);
                next_ev = context.next_event();
            }

            self.scheduler.tick_sample(
                s_u32,
                &self.params,
                &mut self.synth,
                &self.ui_state,
                context,
            );

            let gain = self.params.preview_gain.smoothed.next();
            let (out_l, out_r) = if preview_on {
                self.synth.process_sample(
                    patch,
                    brightness,
                    attack_ms,
                    release_ms,
                    glide_ms,
                    bark,
                    wah_amt,
                    trem_depth,
                    trem_rate,
                    gain,
                )
            } else {
                let _ = self.synth.process_sample(
                    patch,
                    brightness,
                    attack_ms,
                    release_ms,
                    glide_ms,
                    bark,
                    wah_amt,
                    trem_depth,
                    trem_rate,
                    0.0,
                );
                (0.0, 0.0)
            };

            let mut ch_iter = channel_samples.into_iter();
            if let Some(l) = ch_iter.next() {
                *l = out_l;
            }
            if let Some(r) = ch_iter.next() {
                *r = out_r;
            }
        }

        // Flush any events timestamped at or past end of buffer
        let last_sample = (num_samples - 1) as u32;
        while let Some(ev) = next_ev {
            self.handle_incoming_event(ev, last_sample, context);
            next_ev = context.next_event();
        }

        ProcessStatus::KeepAlive
    }
}

impl PhantomVoicer {
    fn handle_incoming_event(
        &mut self,
        ev: PluginNoteEvent<Self>,
        timing: u32,
        context: &mut impl ProcessContext<Self>,
    ) {
        match ev {
            NoteEvent::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => {
                if velocity <= 0.0 {
                    self.scheduler.trigger_note_off(
                        note,
                        channel,
                        timing,
                        &mut self.synth,
                        &self.ui_state,
                        context,
                    );
                } else {
                    self.scheduler.trigger_note_on(
                        note,
                        channel,
                        velocity,
                        timing,
                        &self.params,
                        &mut self.synth,
                        &self.ui_state,
                        context,
                    );
                }
            }
            NoteEvent::NoteOff { channel, note, .. }
            | NoteEvent::Choke { channel, note, .. } => {
                self.scheduler.trigger_note_off(
                    note,
                    channel,
                    timing,
                    &mut self.synth,
                    &self.ui_state,
                    context,
                );
            }
            NoteEvent::MidiCC { cc, value, .. } => {
                // CC 123 = All Notes Off
                if cc == 123 && value == 0.0 {
                    self.scheduler.reset();
                    self.synth.all_notes_off();
                    self.ui_state.set_active_masks(0, 0);
                }
                context.send_event(ev);
            }
            NoteEvent::MidiPitchBend { value, .. } => {
                self.synth.set_pitch_bend(value);
                context.send_event(ev);
            }
            NoteEvent::MidiChannelPressure { .. } => {
                context.send_event(ev);
            }
            _ => {}
        }
    }
}

impl ClapPlugin for PhantomVoicer {
    const CLAP_ID: &'static str = "com.endio-audio.phantom-voicer";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Persona 5 inspired acid-jazz chord & stab voicer with Rhodes preview");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::NoteEffect,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for PhantomVoicer {
    const VST3_CLASS_ID: [u8; 16] = *b"Ph4nt0mV01c3rP5!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Tools,
    ];
}

nih_export_clap!(PhantomVoicer);
nih_export_vst3!(PhantomVoicer);
