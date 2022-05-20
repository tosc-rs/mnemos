#![no_std]
#![no_main]

use core::ops::DerefMut;
use minijam::{
    scale::{
        Note, Pitch, Semitones, DIMINISHED_TRIAD_INTERVALS, MAJOR_PENTATONIC_INTERVALS,
        MAJOR_TRIAD_INTERVALS, MINOR_TRIAD_INTERVALS, NATURAL_MAJOR_INTERVALS,
        NATURAL_MINOR_INTERVALS,
    },
    tones::{ToneKind, Operator, OperatorKind, Tone},
    Track,
};
use userspace::common::porcelain::{
    pcm_sink as pcm,
    time,
    system,
};
use rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

type MjVec<T, const N: usize> = heapless::Vec<T, N>;
// type MjVec<T, const N: usize> = Vec<T>;

const CHUNK_SZ: usize = 512;

pub struct MetaTrack<const N: usize> {
    voice: ToneKind,
    refrain: MjVec<Option<Note>, N>,
    track: Track<N>,
    min_chance: u32,
    max_chance: u32,
    chance: u32,
    length: Length,
    notes: u32,
    operator: Operator,
}

pub enum Length {
    Eighth,
    Quarter,
    Half,
    Whole,
}

impl Length {
    fn qlen_to_notes(&self, qlen: u32) -> u32 {
        match self {
            Length::Eighth => qlen / 2,
            Length::Quarter => qlen,
            Length::Half => qlen * 2,
            Length::Whole => qlen * 4,
        }
    }
}

impl<const N: usize> MetaTrack<N> {
    pub fn new<R: RngCore>(sample_rate: u32, length: Length, notes: u32, rng: &mut R) -> Self {
        let mut me = MetaTrack {
            voice: ToneKind::Square,
            refrain: MjVec::new(),
            track: Track::new(sample_rate),
            min_chance: 0x2000_0000,
            max_chance: 0xC000_0000,
            chance: 0,
            length,
            notes,
            operator: Operator { kind: OperatorKind::None },
        };

        me.gen_voice(rng);
        me.gen_chance(rng);
        me.gen_operator(rng);

        me
    }

    pub fn gen_voice<R: RngCore>(&mut self, rng: &mut R) {
        self.voice = match rng.next_u32() % 3 {
            0 => ToneKind::Sine,
            1 => ToneKind::Square,
            _ => ToneKind::Saw,
        };
    }

    pub fn gen_operator<R: RngCore>(&mut self, rng: &mut R) {
        let wobble = rng.next_u32() % 64;
        if wobble < 32 {
            self.operator = Operator { kind: OperatorKind::AmplitudeLfo(Tone::new_sine(wobble as f32, 44100)) };
        } else {
            self.operator = Operator { kind: OperatorKind::None };
        }
    }

    pub fn gen_chance<R: RngCore>(&mut self, rng: &mut R) {
        self.chance = rng.next_u32().min(self.max_chance).max(self.min_chance);
    }

    // Regenerate the note, but keep the same octave and existing pattern
    pub fn remap_refrain<R: RngCore>(&mut self, rng: &mut R, scale: &[Semitones], key: Pitch) {
        self.refrain.iter_mut().for_each(|n| {
            if let Some(old_note) = n.as_mut() {
                let offset = scale[rng.next_u32() as usize % scale.len()];
                let note = Note {
                    pitch: key,
                    octave: old_note.octave,
                };
                let note = note + offset;
                *old_note = note;
            }
        })
    }

    pub fn gen_refrain<R: RngCore>(&mut self, rng: &mut R, scale: &[Semitones], key: Pitch) {
        self.refrain.clear();
        let to_gen = rng.next_u32() % (self.notes as u32 + 1);
        for _ in 0..to_gen {
            let chance = rng.next_u32();
            if chance < self.chance {
                let oct = match rng.next_u32() % 3 {
                    0 => 2,
                    1 => 3,
                    _ => 4,
                };

                let offset = scale[rng.next_u32() as usize % scale.len()];
                let note = Note {
                    pitch: key,
                    octave: oct,
                };
                let note = note + offset;
                self.refrain.push(Some(note)).ok();
            } else {
                self.refrain.push(None).ok();
            }
        }
    }

    pub fn fill_track<R: RngCore>(
        &mut self,
        rng: &mut R,
        bpm: u32,
        scale: &[Semitones],
        key: Pitch,
    ) {
        let full_length = (self.track.sample_rate * 60) / bpm;
        let full_length = self.length.qlen_to_notes(full_length);
        let note_length = (full_length * 9) / 10;
        let mut cur = 0;

        let v1_len = self.refrain.len();
        let v1_rnd = self.notes - (v1_len as u32);

        for n in self.refrain.iter() {
            if let Some(note) = n {
                self.track
                    .add_note(self.voice, *note, cur, cur + note_length)
                    .unwrap();
            }
            cur += full_length;
        }

        for _ in 0..v1_rnd {
            let chance = rng.next_u32();
            if chance < self.chance {
                let oct = match rng.next_u32() % 3 {
                    0 => 2,
                    1 => 3,
                    _ => 4,
                };

                let scale_len = scale.len();
                let offset = scale[rng.next_u32() as usize % scale_len];
                let note = Note {
                    pitch: key,
                    octave: oct,
                };
                let note = note + offset;

                // TODO: Extend last one? Add a rest?
                self.track
                    .add_note(self.voice, note, cur, cur + note_length)
                    .unwrap();
            }
            cur += full_length;
        }
    }

    pub fn clear(&mut self) {
        self.refrain.clear();
        self.track.reset();
    }
}

struct MetaChorus<const N: usize, const M: usize> {
    tracks: [MetaTrack<N>; M],
}

impl<const N: usize, const M: usize> MetaChorus<N, M> {
    pub fn set_min_chances(&mut self, min_chance: u32) {
        self.tracks
            .iter_mut()
            .for_each(|t| t.min_chance = min_chance);
    }

    pub fn set_max_chances(&mut self, max_chance: u32) {
        self.tracks
            .iter_mut()
            .for_each(|t| t.max_chance = max_chance);
    }

    pub fn fill_tracks(&mut self, bpm: u32) {
        self.tracks.iter_mut().for_each(|t| {
            let full_length = (t.track.sample_rate * 60) / bpm;
            let full_length = t.length.qlen_to_notes(full_length);
            let note_length = (full_length * 9) / 10;
            let mut cur = 0;

            for n in t.refrain.iter() {
                if let Some(note) = n {
                    t.track
                        .add_note(t.voice, *note, cur, cur + note_length)
                        .unwrap();
                }
                cur += full_length;
            }
        })
    }

    pub fn gen_refrain<R: RngCore>(
        &mut self,
        rng: &mut R,
        chords: &[(Semitones, &[Semitones])],
        key: Pitch,
    ) {
        let ch_note = Note {
            pitch: key,
            octave: 3,
        };
        self.tracks.iter_mut().for_each(|v| v.refrain.clear());
        assert_eq!(M, 3, "TODO - Index assumptions");

        let chance = self.tracks[0].chance;

        fill_chord2(
            &mut self.tracks,
            rng,
            // TODO
            chance,
            ch_note,
            &[chords[0]],
        );

        for _ in 0..5 {
            fill_chord2(
                &mut self.tracks,
                rng,
                // TODO
                chance,
                ch_note,
                chords,
            );
        }

        fill_chord2(
            &mut self.tracks,
            rng,
            // TODO
            chance,
            ch_note,
            &[chords[3], chords[4]],
        );

        fill_chord2(
            &mut self.tracks,
            rng,
            // TODO
            chance,
            ch_note,
            &[chords[0]],
        );
    }
}

struct Conductor<R: RngCore, const N: usize, const M: usize> {
    rng: R,
    lead_1: MetaTrack<N>,
    lead_2: MetaTrack<N>,
    chorus: MetaChorus<N, M>,
    key: Pitch,
    bpm: u32,
    scale: &'static [Semitones],
    chords: &'static [(Semitones, &'static [Semitones])],
    is_major: bool,
}

impl<R: RngCore, const N: usize, const M: usize> Conductor<R, N, M> {
    pub fn mutate(&mut self) {
        match self.rng.next_u32() % 18 {
            0 => {
                self.lead_1.gen_voice(&mut self.rng);
            }
            1 => {
                self.lead_2.gen_voice(&mut self.rng);
            }
            2 => {
                self.chorus
                    .tracks
                    .iter_mut()
                    .for_each(|t| t.gen_voice(&mut self.rng));
            }
            3 => {
                self.pick_scale();
            }
            4 => {
                self.lead_1.gen_chance(&mut self.rng);
            }
            5 => {
                self.lead_2.gen_chance(&mut self.rng);
            }
            6 => {
                self.chorus
                    .tracks
                    .iter_mut()
                    .for_each(|t| t.gen_chance(&mut self.rng));
            }
            7 => {
                self.bpm = 120 + self.rng.next_u32() % 64;
            }
            8 => {
                let old_major = self.is_major;

                if (self.rng.next_u32() & 0b1) == 0 {
                    self.chords = MAJOR_CHORDS;
                    self.is_major = true;
                } else {
                    self.chords = MINOR_CHORDS;
                    self.is_major = false;
                }

                // TODO: If we change minor/major, regenerate everything so we aren't
                // playing off. I could probably be smarter and transpose the progressions
                // rather than regenerate everything
                if old_major != self.is_major {
                    self.pick_scale();
                    self.lead_1
                        .remap_refrain(&mut self.rng, self.scale, self.key);
                    self.lead_2
                        .remap_refrain(&mut self.rng, self.scale, self.key);
                    // TODO: remap_refrain?
                    self.chorus
                        .gen_refrain(&mut self.rng, self.chords, self.key);
                }
            }
            9 => {
                self.gen_lead_1_refrain();
            }
            10 => {
                self.gen_lead_2_refrain();
            }
            11 => {
                self.chorus
                    .gen_refrain(&mut self.rng, self.chords, self.key);
            }
            12 => {
                self.lead_1.gen_operator(&mut self.rng);
            }
            13 => {
                self.lead_2.gen_operator(&mut self.rng);
            }
            14 => {
                self.chorus.tracks[0].gen_operator(&mut self.rng);
            }
            15 => {
                self.chorus.tracks[1].gen_operator(&mut self.rng);
            }
            16 => {
                self.chorus.tracks[2].gen_operator(&mut self.rng);
            }
            _ => {
                self.pick_key();
                self.lead_1
                    .remap_refrain(&mut self.rng, self.scale, self.key);
                self.lead_2
                    .remap_refrain(&mut self.rng, self.scale, self.key);
                // TODO: remap_refrain?
                self.chorus
                    .gen_refrain(&mut self.rng, self.chords, self.key);
            }
        }
    }

    pub fn gen_lead_1_refrain(&mut self) {
        self.lead_1.gen_refrain(&mut self.rng, self.scale, self.key);
    }

    pub fn gen_lead_2_refrain(&mut self) {
        self.lead_2.gen_refrain(&mut self.rng, self.scale, self.key);
    }

    pub fn gen_chorus_refrain(&mut self) {
        self.chorus
            .gen_refrain(&mut self.rng, self.chords, self.key);
    }

    pub fn pick_scale(&mut self) {
        let scales = if self.is_major {
            MAJOR_SCALES
        } else {
            MINOR_SCALES
        };
        self.scale = scales[self.rng.next_u32() as usize % scales.len()];
    }

    pub fn pick_key(&mut self) {
        self.key = match self.rng.next_u32() % 12 {
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
    }

    pub fn clear(&mut self) {
        self.lead_1.track.reset();
        self.lead_2.track.reset();
        self.chorus.tracks.iter_mut().for_each(|t| t.track.reset());
    }

    pub fn fill_tracks(&mut self) {
        self.lead_1
            .fill_track(&mut self.rng, self.bpm, self.scale, self.key);
        self.lead_2
            .fill_track(&mut self.rng, self.bpm, self.scale, self.key);
        self.chorus.fill_tracks(self.bpm);
    }

    pub fn is_done(&self) -> bool {
        self.lead_1.track.is_done()
            && self.lead_2.track.is_done()
            && self.chorus.tracks.iter().all(|t| t.track.is_done())
    }
}

#[no_mangle]
pub fn entry() -> ! {
    let mut seed = [0u8; 32];
    system::rand_fill(&mut seed).unwrap();

    let mut rng = ChaCha8Rng::from_seed(seed);

    let mut conductor: Conductor<_, 128, 3> = Conductor {
        lead_1: MetaTrack::new(44100, Length::Quarter, 32, &mut rng),
        lead_2: MetaTrack::new(44100, Length::Eighth, 64, &mut rng),
        chorus: MetaChorus {
            tracks: [
                MetaTrack::new(44100, Length::Whole, 8, &mut rng),
                MetaTrack::new(44100, Length::Whole, 8, &mut rng),
                MetaTrack::new(44100, Length::Whole, 8, &mut rng),
            ],
        },
        rng,
        key: Pitch::C,
        bpm: 120,
        scale: MAJOR_PENTATONIC_INTERVALS,
        chords: MAJOR_CHORDS,
        is_major: true,
    };

    conductor.pick_scale();
    conductor.chorus.set_min_chances(0x6000_0000);
    conductor.chorus.set_max_chances(0xC000_0000);

    // Seed stuff
    conductor.gen_lead_1_refrain();
    conductor.gen_lead_2_refrain();
    conductor.gen_chorus_refrain();

    pcm::enable().ok();

    loop {
        conductor.clear();
        conductor.mutate();
        conductor.fill_tracks();
        // End chords

        while !conductor.is_done() {
            if let Ok(mut samples) = pcm::alloc_samples(CHUNK_SZ) {
                let samps = unsafe {
                    let buf: &mut [u8] = samples.deref_mut();
                    let ptr = buf.as_mut_ptr();
                    assert_eq!(ptr as usize % 2, 0);
                    assert_eq!(buf.len(), CHUNK_SZ * 4);
                    let samp_ref = core::slice::from_raw_parts_mut(ptr.cast::<minijam::StereoSample>(), CHUNK_SZ);
                    samp_ref
                };

                conductor
                    .lead_1
                    .track
                    .fill_stereo_samples(samps, minijam::tones::Mix::Div4, &mut conductor.lead_1.operator);
                conductor
                    .lead_2
                    .track
                    .fill_stereo_samples(samps, minijam::tones::Mix::Div4, &mut conductor.lead_2.operator);
                conductor.chorus.tracks[0]
                    .track
                    .fill_stereo_samples(samps, minijam::tones::Mix::Div8, &mut conductor.chorus.tracks[0].operator);
                conductor.chorus.tracks[1]
                    .track
                    .fill_stereo_samples(samps, minijam::tones::Mix::Div8, &mut conductor.chorus.tracks[1].operator);
                conductor.chorus.tracks[2]
                    .track
                    .fill_stereo_samples(samps, minijam::tones::Mix::Div8, &mut conductor.chorus.tracks[2].operator);

                samples.send();
            } else {
                time::sleep_micros(5000).ok();
            }
        }

    }
}

fn fill_chord2<R: RngCore, const N: usize>(
    tracks: &mut [MetaTrack<N>],
    rng: &mut R,
    prob: u32,
    note: Note,
    chords: &[(Semitones, &[Semitones])],
) {
    let ch = if chords.len() > 1 {
        chords[rng.next_u32() as usize % chords.len()]
    } else {
        chords[0]
    };

    for (track, semi) in tracks.iter_mut().zip(ch.1.iter()) {
        let chance = rng.next_u32();
        if chance < prob {
            let note = note + ch.0 + *semi;
            track.refrain.push(Some(note)).ok();
        } else {
            track.refrain.push(None).ok();
        }
    }
}

const MAJOR_SCALES: &[&[Semitones]] = &[
    minijam::scale::IONIAN_INTERVALS,
    minijam::scale::LYDIAN_INTERVALS,
    minijam::scale::MIXOLYDIAN_INTERVALS,
    minijam::scale::MAJOR_TRIAD_INTERVALS,
    minijam::scale::DOMINANT_7TH_TETRAD_INTERVALS,
    minijam::scale::MAJOR_7TH_TETRAD_INTERVALS,
    minijam::scale::AUGMENTED_MAJOR_7TH_TETRAD_INTERVALS,
    minijam::scale::DIMINISHED_7TH_TETRAD_INTERVALS,
    minijam::scale::MAJOR_PENTATONIC_INTERVALS,
    minijam::scale::EGYPTIAN_PENTATONIC_INTERVALS,
    minijam::scale::BLUES_MAJOR_PENTATONIC_INTERVALS,
];

const MINOR_SCALES: &[&[Semitones]] = &[
    minijam::scale::DORIAN_INTERVALS,
    minijam::scale::PHRYGIAN_INTERVALS,
    minijam::scale::AEOLIAN_INTERVALS,
    minijam::scale::LOCRIAN_INTERVALS,
    minijam::scale::HARMONIC_MINOR_INTERVALS,
    minijam::scale::MELODIC_MINOR_ASCENDING_INTERVALS,
    minijam::scale::MELODIC_MINOR_DESCENDING_INTERVALS,
    minijam::scale::MINOR_TRIAD_INTERVALS,
    minijam::scale::DIMINISHED_TRIAD_INTERVALS,
    minijam::scale::AUGMENTED_TRIAD_INTERVALS,
    minijam::scale::MINOR_7TH_TETRAD_INTERVALS,
    minijam::scale::MINOR_MAJOR_7TH_TETRAD_INTERVALS,
    minijam::scale::AUGMENTED_7TH_TETRAD_INTERVALS,
    minijam::scale::DIMINISHED_HALF_7TH_TETRAD_INTERVALS,
    minijam::scale::BLUES_MINOR_PENTATONIC_INTERVALS,
    minijam::scale::MINOR_PENTATONIC_INTERVALS,
];

const MAJOR_CHORDS: &[(Semitones, &[Semitones])] = &[
    (NATURAL_MAJOR_INTERVALS[0], MAJOR_TRIAD_INTERVALS), // I - Primary
    (NATURAL_MAJOR_INTERVALS[1], MINOR_TRIAD_INTERVALS), // II
    (NATURAL_MAJOR_INTERVALS[2], MINOR_TRIAD_INTERVALS), // III
    (NATURAL_MAJOR_INTERVALS[3], MAJOR_TRIAD_INTERVALS), // IV - Primary
    (NATURAL_MAJOR_INTERVALS[4], MAJOR_TRIAD_INTERVALS), // V - Primary
    (NATURAL_MAJOR_INTERVALS[5], MINOR_TRIAD_INTERVALS), // VI
];

const MINOR_CHORDS: &[(Semitones, &[Semitones])] = &[
    (NATURAL_MINOR_INTERVALS[0], MINOR_TRIAD_INTERVALS), // I
    (NATURAL_MINOR_INTERVALS[1], DIMINISHED_TRIAD_INTERVALS), // II
    (NATURAL_MINOR_INTERVALS[2], MINOR_TRIAD_INTERVALS), // III
    (NATURAL_MINOR_INTERVALS[3], MINOR_TRIAD_INTERVALS), // IV
    (NATURAL_MINOR_INTERVALS[4], MAJOR_TRIAD_INTERVALS), // V
    (NATURAL_MINOR_INTERVALS[5], MINOR_TRIAD_INTERVALS), // VI
];
