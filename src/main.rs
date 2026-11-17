#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use nih_plug::prelude::*;
use phantom_voicer::PhantomVoicer;

fn main() {
    nih_export_standalone::<PhantomVoicer>();
}
