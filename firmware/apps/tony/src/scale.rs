use core::ops::Add;

pub const PITCHES_PER_OCTAVE: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub enum Pitch {
    C,
    CSharp,
    D,
    DSharp,
    E,
    F,
    FSharp,
    G,
    GSharp,
    A,
    ASharp,
    B,
}

impl Add<Semitones> for Note {
    type Output = Note;

    #[allow(clippy::suspicious_arithmetic_impl)]
    fn add(self, rhs: Semitones) -> Self::Output {
        let pitch: u8 = self.pitch.into();
        let pitch: u32 = pitch.into();
        let pitch = pitch.wrapping_add(rhs.0);

        let new_oct = self.octave + (pitch / PITCHES_PER_OCTAVE) as u8;
        let new_pitch = (pitch % PITCHES_PER_OCTAVE) as u8;
        Note {
            pitch: new_pitch.into(),
            octave: new_oct,
        }
    }
}

impl From<Pitch> for u8 {
    fn from(val: Pitch) -> Self {
        match val {
            Pitch::C => 0,
            Pitch::CSharp => 1,
            Pitch::D => 2,
            Pitch::DSharp => 3,
            Pitch::E => 4,
            Pitch::F => 5,
            Pitch::FSharp => 6,
            Pitch::G => 7,
            Pitch::GSharp => 8,
            Pitch::A => 9,
            Pitch::ASharp => 10,
            Pitch::B => 11,
        }
    }
}

impl From<u8> for Pitch {
    fn from(val: u8) -> Self {
        match val {
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
             11 => Pitch::B,
             _ => {
                debug_assert!(false, "what?");
                // lol
                Pitch::C
            }
        }
    }
}

impl Pitch {
    // Note: frequencies taken from
    // https://pages.mtu.edu/~suits/notefreqs.html
    pub const fn root_frequency(&self) -> f32 {
        match self {
            Pitch::C => 16.35160,
            Pitch::CSharp => 17.32391,
            Pitch::D => 18.35405,
            Pitch::DSharp => 19.44544,
            Pitch::E => 20.60172,
            Pitch::F => 21.82676,
            Pitch::FSharp => 23.12465,
            Pitch::G => 24.49971,
            Pitch::GSharp => 25.95654,
            Pitch::A => 27.50,
            Pitch::ASharp => 29.13524,
            Pitch::B => 30.86771,
        }
    }

    pub fn freq_with_octave(&self, octave: u8) -> f32 {
        let base = self.root_frequency();
        let mult = (1 << (octave as u32)) as f32;
        base * mult
    }
}

/// A note.
#[derive(Debug, Clone, Copy)]
pub struct Note {
    /// The pitch of the note (A, B, C#, etc).
    pub pitch: Pitch,
    /// The octave of the note in standard notation.
    pub octave: u8,
}

impl Note {
    pub fn freq_f32(&self) -> f32 {
        self.pitch.freq_with_octave(self.octave)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Semitones(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity_check_octave() {
        let tests = [
            (Pitch::C, 1, 32.70),
            (Pitch::C, 4, 261.63),
            (Pitch::C, 8, 4186.01),
            (Pitch::A, 1, 55.00),
            (Pitch::A, 3, 220.00),
            (Pitch::A, 7, 3520.00),
        ];

        for (note, octave, exp_freq) in tests {
            let freq = note.freq_with_octave(octave);
            f32_compare(freq, exp_freq, exp_freq * 0.001);
        }
    }

    #[test]
    fn sanity_check_semitone() {
        let tests = [
            (Pitch::C, Semitones(0), Pitch::C),
            (Pitch::C, Semitones(12), Pitch::C),
            (Pitch::C, Semitones(1), Pitch::CSharp),
            (Pitch::C, Semitones(3), Pitch::DSharp),
        ];

        for (note, semis, exp_note) in tests {
            let new_note = (note as u32) + semis.0;
            assert_eq!(Pitch::from((new_note as u8) % 12), exp_note);
        }
    }

    fn f32_compare(lhs: f32, rhs: f32, tol: f32) {
        let abs_diff = (rhs - lhs).abs();
        if abs_diff > tol.abs() {
            panic!(
                "Value out of tolerance! lhs: {} rhs: {} diff: {} tol: {}",
                lhs,
                rhs,
                abs_diff,
                tol,
            );
        }
    }
}

// --------------------------
// Diatonic Scale Sequences
//
// REF: https://en.wikipedia.org/wiki/Diatonic_scale#Theory
// --------------------------
// MAJOR
pub const IONIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),  // 1
    Semitones(2),
    Semitones(4),  // 3
    Semitones(5),
    Semitones(7),  // 5
    Semitones(9),
    Semitones(11), // 7
    Semitones(12),
];

// MINOR
pub const DORIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(3),
    Semitones(5),
    Semitones(7),
    Semitones(9),
    Semitones(10),
    Semitones(12),
];

// MINOR
pub const PHRYGIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(1),
    Semitones(3),
    Semitones(5),
    Semitones(7),
    Semitones(8),
    Semitones(10),
    Semitones(12),
];

// MAJOR
pub const LYDIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(4),
    Semitones(6),
    Semitones(7),
    Semitones(9),
    Semitones(11),
    Semitones(12),
];

// MAJOR
pub const MIXOLYDIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(4),
    Semitones(5),
    Semitones(7),
    Semitones(9),
    Semitones(10),
    Semitones(12),
];

// MINOR
pub const AEOLIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(3),
    Semitones(5),
    Semitones(7),
    Semitones(8),
    Semitones(10),
    Semitones(12),
];

// MINOR-ISH
pub const LOCRIAN_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(1),
    Semitones(3),
    Semitones(5),
    Semitones(6),
    Semitones(8),
    Semitones(10),
    Semitones(12),
];

// --------------------------
// Other Scale Sequences
// --------------------------

pub const NATURAL_MAJOR_INTERVALS: &[Semitones] = IONIAN_INTERVALS;
pub const NATURAL_MINOR_INTERVALS: &[Semitones] = AEOLIAN_INTERVALS;

pub const HARMONIC_MINOR_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(3),
    Semitones(5),
    Semitones(7),
    Semitones(8),
    Semitones(11),
    Semitones(12),
];

pub const MELODIC_MINOR_ASCENDING_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(3),
    Semitones(5),
    Semitones(7),
    Semitones(9),
    Semitones(11),
    Semitones(12),
];

pub const MELODIC_MINOR_DESCENDING_INTERVALS: &[Semitones] = &[
    Semitones(12),
    Semitones(10),
    Semitones(8),
    Semitones(7),
    Semitones(5),
    Semitones(3),
    Semitones(2),
    Semitones(0),
];

// --------------------------
// Chord sequences
//
// NOTE:
//   https://www.bellandcomusic.com/chord-structure.html
//   https://www.bellandcomusic.com/music-chords.html
// --------------------------

// Triads

pub const MAJOR_TRIAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(4),
    Semitones(7),
];

pub const MINOR_TRIAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(7),
];

pub const DIMINISHED_TRIAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(6),
];

pub const AUGMENTED_TRIAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(4),
    Semitones(8),
];

// Tetrads

/// ex: C, E, G, ASharp
pub const DOMINANT_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(4),
    Semitones(7),
    Semitones(10),
];

/// ex: C, DSharp, G, ASharp
pub const MINOR_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(7),
    Semitones(10),
];

/// ex: C, E, G, B
pub const MAJOR_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(4),
    Semitones(7),
    Semitones(11),
];

/// ex: C, DSharp, G, B
pub const MINOR_MAJOR_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(7),
    Semitones(11),
];

/// ex: C, E, GSharp, ASharp
pub const AUGMENTED_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(4),
    Semitones(8),
    Semitones(10),
];

/// ex: C, E, GSharp, B
pub const AUGMENTED_MAJOR_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(4),
    Semitones(8),
    Semitones(11),
];

/// ex: C, DSharp, FSharp, A
pub const DIMINISHED_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(6),
    Semitones(9),
];

/// ex: C, DSharp, FSharp, ASharp
pub const DIMINISHED_HALF_7TH_TETRAD_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(6),
    Semitones(10),
];

// Pentatonics
//
// NOTE: https://en.wikipedia.org/wiki/Pentatonic_scale

/// ex: C, D, E, G, A, C
pub const MAJOR_PENTATONIC_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(4),
    Semitones(7),
    Semitones(9),
];

/// ex: C, D, F, G, ASharp, C
pub const EGYPTIAN_PENTATONIC_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(5),
    Semitones(7),
    Semitones(10),
];

/// ex: C, DSharp, F, GSharp, ASharp, C
pub const BLUES_MINOR_PENTATONIC_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(5),
    Semitones(8),
    Semitones(10),
];

/// ex: C, D, F, G, A, C
pub const BLUES_MAJOR_PENTATONIC_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(2),
    Semitones(5),
    Semitones(7),
    Semitones(9),
];

/// ex: C, DSharp, F, G, ASharp, C
pub const MINOR_PENTATONIC_INTERVALS: &[Semitones] = &[
    Semitones(0),
    Semitones(3),
    Semitones(5),
    Semitones(7),
    Semitones(10),
];


/*
# Chords

NOTE: https://www.musictheoryacademy.com/understanding-music/primary-chords/

## Primary Chords:

* Always I, IV, V
* Major Key:
  * Primary chords are all major triads
* Minor Key:
  * I and IV are minor triads, V is major triad

## Secondary Chords

* II, III, VI
* Major Key:
   * Secondary chords are all minor triads
* Minor Key:
  * III and VI are minor triads, II is a diminished triad
* Suggestions
  * Start and end with I (primary)
  * Use IV or V before the last I (all primary)
*/

pub const MAJOR_PRIMARY_CHORDS: &[(Semitones, &[Semitones])] = &[
    (NATURAL_MAJOR_INTERVALS[0], MAJOR_TRIAD_INTERVALS), // I
    (NATURAL_MAJOR_INTERVALS[3], MAJOR_TRIAD_INTERVALS), // IV
    (NATURAL_MAJOR_INTERVALS[4], MAJOR_TRIAD_INTERVALS), // V
];

pub const MINOR_PRIMARY_CHORDS: &[(Semitones, &[Semitones])] = &[
    (NATURAL_MINOR_INTERVALS[0], MINOR_TRIAD_INTERVALS), // I
    (NATURAL_MINOR_INTERVALS[3], MINOR_TRIAD_INTERVALS), // IV
    (NATURAL_MINOR_INTERVALS[4], MAJOR_TRIAD_INTERVALS), // V
];

pub const MAJOR_SECONDARY_CHORDS: &[(Semitones, &[Semitones])] = &[
    (NATURAL_MAJOR_INTERVALS[1], MINOR_TRIAD_INTERVALS), // II
    (NATURAL_MAJOR_INTERVALS[2], MINOR_TRIAD_INTERVALS), // III
    (NATURAL_MAJOR_INTERVALS[5], MINOR_TRIAD_INTERVALS), // VI
];

pub const MINOR_SECONDARY_CHORDS: &[(Semitones, &[Semitones])] = &[
    (NATURAL_MINOR_INTERVALS[1], DIMINISHED_TRIAD_INTERVALS), // II
    (NATURAL_MINOR_INTERVALS[2], MINOR_TRIAD_INTERVALS),      // III
    (NATURAL_MINOR_INTERVALS[5], MINOR_TRIAD_INTERVALS),      // VI
];
