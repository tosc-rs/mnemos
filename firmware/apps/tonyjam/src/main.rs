#![no_std]
#![no_main]

use core::ops::DerefMut;

use minijam::{Track, scale::{Pitch, Note, NATURAL_MAJOR_INTERVALS, MAJOR_TRIAD_INTERVALS, MINOR_TRIAD_INTERVALS}};
use userspace::common::porcelain::{
    pcm_sink as pcm,
    time,
    system,
};

use rand_core::{self, SeedableRng, RngCore};
use rand_chacha::{self, ChaCha8Rng};



const CHUNK_SZ: usize = 512;

#[no_mangle]
pub fn entry() -> ! {
    let mut seed = [0u8; 32];
    system::rand_fill(&mut seed).unwrap();

    let mut rng = ChaCha8Rng::from_seed(seed);
    let mut track_lead: Track<128> = Track::new(44100);
    let mut track_ch1: Track<128> = Track::new(44100);
    let mut track_ch2: Track<128> = Track::new(44100);
    let mut track_ch3: Track<128> = Track::new(44100);

    pcm::enable().ok();

    loop {
        track_lead.reset();
        track_ch1.reset();
        track_ch2.reset();
        track_ch3.reset();

        let pitch = match rng.next_u32() % 12 {
            0 => Pitch::C,
            1 => Pitch::CSharp,
            2 => Pitch::D,
            3 => Pitch::DSharp,
            4 => Pitch::E,
            5 => Pitch::F,
            6 => Pitch::FSharp,
            7 => Pitch::G,
            8 => Pitch::GSharp,
            9 => Pitch::A,
            10 => Pitch::ASharp,
            _ => Pitch::B,
        };

        let scales = &[
            minijam::scale::IONIAN_INTERVALS,
            // minijam::scale::DORIAN_INTERVALS,
            // minijam::scale::PHRYGIAN_INTERVALS,
            minijam::scale::LYDIAN_INTERVALS,
            minijam::scale::MIXOLYDIAN_INTERVALS,
            // minijam::scale::AEOLIAN_INTERVALS,
            // minijam::scale::LOCRIAN_INTERVALS,
            // minijam::scale::HARMONIC_MINOR_INTERVALS,
            // minijam::scale::MELODIC_MINOR_ASCENDING_INTERVALS,
            // minijam::scale::MELODIC_MINOR_DESCENDING_INTERVALS,
            minijam::scale::MAJOR_TRIAD_INTERVALS,
            // minijam::scale::MINOR_TRIAD_INTERVALS,
            // minijam::scale::DIMINISHED_TRIAD_INTERVALS,
            // minijam::scale::AUGMENTED_TRIAD_INTERVALS,
            minijam::scale::DOMINANT_7TH_TETRAD_INTERVALS,
            minijam::scale::MINOR_7TH_TETRAD_INTERVALS,
            minijam::scale::MAJOR_7TH_TETRAD_INTERVALS,
            // minijam::scale::MINOR_MAJOR_7TH_TETRAD_INTERVALS,
            // minijam::scale::AUGMENTED_7TH_TETRAD_INTERVALS,
            minijam::scale::AUGMENTED_MAJOR_7TH_TETRAD_INTERVALS,
            minijam::scale::DIMINISHED_7TH_TETRAD_INTERVALS,
            // minijam::scale::DIMINISHED_HALF_7TH_TETRAD_INTERVALS,
            minijam::scale::MAJOR_PENTATONIC_INTERVALS,
            minijam::scale::EGYPTIAN_PENTATONIC_INTERVALS,
            // minijam::scale::BLUES_MINOR_PENTATONIC_INTERVALS,
            minijam::scale::BLUES_MAJOR_PENTATONIC_INTERVALS,
            // minijam::scale::MINOR_PENTATONIC_INTERVALS,
        ];

        let scale = scales[rng.next_u32() as usize % scales.len()];

        let major_chords = &[
            (NATURAL_MAJOR_INTERVALS[0], MAJOR_TRIAD_INTERVALS), // I - Primary
            (NATURAL_MAJOR_INTERVALS[1], MINOR_TRIAD_INTERVALS), // II
            (NATURAL_MAJOR_INTERVALS[2], MINOR_TRIAD_INTERVALS), // III
            (NATURAL_MAJOR_INTERVALS[3], MAJOR_TRIAD_INTERVALS), // IV - Primary
            (NATURAL_MAJOR_INTERVALS[4], MAJOR_TRIAD_INTERVALS), // V - Primary
            (NATURAL_MAJOR_INTERVALS[5], MINOR_TRIAD_INTERVALS), // VI
        ];


        let wave = match rng.next_u32() % 3 {
            0 => minijam::tones::ToneKind::Sine,
            1 => minijam::tones::ToneKind::Square,
            _ => minijam::tones::ToneKind::Saw,
        };
        // let wave = minijam::tones::ToneKind::Sine;

        let bpm = 180;
        let full_length = (44100 * 60) / bpm;
        let note_length = (full_length * 9) / 10;
        let scale_len = scale.len();
        let mut cur = 0;

        for _ in 0..64 {
            let oct = match rng.next_u32() % 3 {
                0 => 2,
                1 => 3,
                _ => 4,
            };

            let offset = scale[rng.next_u32() as usize % scale_len];
            let note = Note {
                pitch,
                octave: oct,
            };
            let note = note + offset;

            track_lead.add_note(wave, note, cur, cur + note_length).unwrap();
            cur += full_length;
        }

        // Chords
        // First and Last should be I
        let chord_wave = match rng.next_u32() % 3 {
            0 => minijam::tones::ToneKind::Sine,
            1 => minijam::tones::ToneKind::Square,
            _ => minijam::tones::ToneKind::Saw,
        };
        let ch_full_length = ((44100 * 60) / bpm) * 4;
        let ch_note_length = (ch_full_length * 9) / 10;
        let mut ch_cur = 0;

        for (track, semi) in [&mut track_ch1, &mut track_ch2, &mut track_ch3].iter_mut().zip(major_chords[0].1.iter()) {
            let note = Note {
                pitch,
                octave: 3,
            };
            let note = note + major_chords[0].0 + *semi;
            track.add_note(chord_wave, note, ch_cur, ch_cur + ch_note_length).unwrap();
        }

        ch_cur += ch_full_length;

        for _ in 0..13 {
            let ch = major_chords[rng.next_u32() as usize % major_chords.len()];

            for (track, semi) in [&mut track_ch1, &mut track_ch2, &mut track_ch3].iter_mut().zip(ch.1.iter()) {
                let note = Note {
                    pitch,
                    octave: 3,
                };
                let note = note + ch.0 + *semi;
                track.add_note(chord_wave, note, ch_cur, ch_cur + ch_note_length).unwrap();
            }

            ch_cur += ch_full_length;
        }

        let ch = if (rng.next_u32() & 0b1) == 0 {
            major_chords[3]
        } else {
            major_chords[4]
        };

        for (track, semi) in [&mut track_ch1, &mut track_ch2, &mut track_ch3].iter_mut().zip(ch.1.iter()) {
            let note = Note {
                pitch,
                octave: 3,
            };
            let note = note + ch.0 + *semi;
            track.add_note(chord_wave, note, ch_cur, ch_cur + ch_note_length).unwrap();
        }

        ch_cur += ch_full_length;

        for (track, semi) in [&mut track_ch1, &mut track_ch2, &mut track_ch3].iter_mut().zip(major_chords[0].1.iter()) {
            let note = Note {
                pitch,
                octave: 3,
            };
            let note = note + major_chords[0].0 + *semi;
            track.add_note(chord_wave, note, ch_cur, ch_cur + ch_note_length).unwrap();
        }

        // End chords

        while ![&track_lead, &track_ch1, &track_ch2, &track_ch3].iter().all(|t| t.is_done()) {
            if let Ok(mut samples) = pcm::alloc_samples(CHUNK_SZ) {
                // TODO: unhax
                samples.iter_mut().for_each(|b| *b = 0);
                let samps = unsafe {
                    let buf: &mut [u8] = samples.deref_mut();
                    let ptr = buf.as_mut_ptr();
                    assert_eq!(ptr as usize % 2, 0);
                    assert_eq!(buf.len(), CHUNK_SZ * 4);
                    let samp_ref = core::slice::from_raw_parts_mut(ptr.cast::<minijam::StereoSample>(), CHUNK_SZ);
                    samp_ref
                };
                track_lead.fill_stereo_samples(samps, minijam::tones::Mix::Div4);
                track_ch1.fill_stereo_samples(samps, minijam::tones::Mix::Div8);
                track_ch2.fill_stereo_samples(samps, minijam::tones::Mix::Div8);
                track_ch3.fill_stereo_samples(samps, minijam::tones::Mix::Div8);
                samples.send();
            } else {
                time::sleep_micros(5000).ok();
            }
        }
    }
}
