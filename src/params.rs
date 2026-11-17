use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum SoundPatch {
    #[name = "Scarbee Pan Cake (EP)"]
    VelvetRhodes,
    #[name = "Clean Fender (Full Range)"]
    SlapBass,
    #[name = "Phantom Brass"]
    PhantomBrass,
    #[name = "Wah Clav / Guitar"]
    WahClav,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum KeyRoot {
    C,
    #[name = "C# / Db"]
    Cs,
    D,
    #[name = "D# / Eb"]
    Eb,
    E,
    F,
    #[name = "F# / Gb"]
    Fs,
    G,
    #[name = "G# / Ab"]
    Ab,
    A,
    #[name = "A# / Bb"]
    Bb,
    B,
}

impl KeyRoot {
    pub fn semitone(self) -> u8 {
        match self {
            Self::C => 0,
            Self::Cs => 1,
            Self::D => 2,
            Self::Eb => 3,
            Self::E => 4,
            Self::F => 5,
            Self::Fs => 6,
            Self::G => 7,
            Self::Ab => 8,
            Self::A => 9,
            Self::Bb => 10,
            Self::B => 11,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cs => "C#",
            Self::D => "D",
            Self::Eb => "Eb",
            Self::E => "E",
            Self::F => "F",
            Self::Fs => "F#",
            Self::G => "G",
            Self::Ab => "Ab",
            Self::A => "A",
            Self::Bb => "Bb",
            Self::B => "B",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ScaleMode {
    #[name = "Dorian (Last Surprise)"]
    Dorian,
    #[name = "Harmonic Minor (Boss)"]
    HarmonicMinor,
    #[name = "Natural Minor"]
    NaturalMinor,
    #[name = "Major (Tokyo Daylight)"]
    Major,
    #[name = "Chromatic (Free)"]
    Chromatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum VoicingStyle {
    #[name = "Acid Jazz (m9 / 13 / maj9)"]
    AcidJazz,
    #[name = "Neo-Soul (m11 / sus)"]
    NeoSoul,
    #[name = "Funk Stab (7#9 / 9)"]
    FunkStab,
    #[name = "Dim7 Passing"]
    DimPassing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum StrumDir {
    Up,
    Down,
    Alternate,
    Tight,
}

#[derive(Params)]
pub struct PhantomParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    // --- Sound Engine ---
    #[id = "sound_patch"]
    pub sound_patch: EnumParam<SoundPatch>,
    #[id = "brightness"]
    pub brightness: FloatParam,
    #[id = "attack_ms"]
    pub attack_ms: FloatParam,
    #[id = "release_ms"]
    pub release_ms: FloatParam,
    #[id = "glide_ms"]
    pub glide_ms: FloatParam,

    // --- Optional Harmony Assist (OFF by default: plays single notes) ---
    #[id = "chord_assist"]
    pub chord_assist: BoolParam,
    #[id = "key_root"]
    pub key_root: EnumParam<KeyRoot>,
    #[id = "scale_mode"]
    pub scale_mode: EnumParam<ScaleMode>,
    #[id = "voicing_style"]
    pub voicing_style: EnumParam<VoicingStyle>,
    #[id = "voice_lead"]
    pub voice_lead: BoolParam,
    #[id = "center_note"]
    pub center_note: IntParam,
    #[id = "extensions"]
    pub extensions: FloatParam,
    #[id = "bass_split"]
    pub bass_split: BoolParam,
    #[id = "bass_channel"]
    pub bass_channel: IntParam,

    // --- Feel & Humanize ---
    #[id = "strum_ms"]
    pub strum_ms: FloatParam,
    #[id = "strum_dir"]
    pub strum_dir: EnumParam<StrumDir>,
    #[id = "humanize_vel"]
    pub humanize_vel: FloatParam,

    // --- Built-in Rhodes Preview & FX ---
    #[id = "preview_enabled"]
    pub preview_enabled: BoolParam,
    #[id = "preview_gain"]
    pub preview_gain: FloatParam,
    #[id = "rhodes_bark"]
    pub rhodes_bark: FloatParam,
    #[id = "wah_amount"]
    pub wah_amount: FloatParam,
    #[id = "tremolo_depth"]
    pub tremolo_depth: FloatParam,
    #[id = "tremolo_rate"]
    pub tremolo_rate_hz: FloatParam,
}

impl Default for PhantomParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(860, 560),

            sound_patch: EnumParam::new("Sound", SoundPatch::VelvetRhodes),
            brightness: FloatParam::new(
                "Brightness",
                0.65,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            attack_ms: FloatParam::new(
                "Attack",
                4.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 250.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.5)
            .with_unit(" ms"),
            release_ms: FloatParam::new(
                "Release",
                280.0,
                FloatRange::Skewed {
                    min: 30.0,
                    max: 1500.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(1.0)
            .with_unit(" ms"),
            glide_ms: FloatParam::new(
                "Glide",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 200.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(1.0)
            .with_unit(" ms"),

            chord_assist: BoolParam::new("Chord Assist", false),
            key_root: EnumParam::new("Key", KeyRoot::D),
            scale_mode: EnumParam::new("Scale", ScaleMode::Dorian),
            voicing_style: EnumParam::new("Voicing", VoicingStyle::AcidJazz),
            voice_lead: BoolParam::new("Voice Lead", true),
            center_note: IntParam::new("Voicing Center", 60, IntRange::Linear { min: 48, max: 72 })
                .with_value_to_string(formatters::v2s_i32_note_formatter())
                .with_string_to_value(formatters::s2v_i32_note_formatter()),
            extensions: FloatParam::new(
                "Extensions",
                0.85,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            bass_split: BoolParam::new("Bass Split", true),
            bass_channel: IntParam::new("Bass MIDI Ch", 2, IntRange::Linear { min: 1, max: 16 }),

            strum_ms: FloatParam::new(
                "Strum",
                14.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 60.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.5)
            .with_unit(" ms"),
            strum_dir: EnumParam::new("Strum Dir", StrumDir::Up),
            humanize_vel: FloatParam::new(
                "Humanize Vel",
                0.15,
                FloatRange::Linear { min: 0.0, max: 0.5 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            preview_enabled: BoolParam::new("Rhodes Preview", true),
            preview_gain: FloatParam::new(
                "Preview Gain",
                util::db_to_gain(-6.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-36.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-36.0, 6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            rhodes_bark: FloatParam::new(
                "Bark / Growl",
                0.30,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            wah_amount: FloatParam::new(
                "Auto-Wah",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            tremolo_depth: FloatParam::new(
                "Pan Cake Depth",
                0.60,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            tremolo_rate_hz: FloatParam::new(
                "Pan Rate",
                2.6,
                FloatRange::Skewed {
                    min: 0.4,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.1)
            .with_unit(" Hz"),
        }
    }
}
