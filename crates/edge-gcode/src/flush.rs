//! What a printer's own G-code flushes of each filament, as the
//! reference's G-code processor reads it from the file it has just
//! written and counts it into the filament each print uses
//! (GCodeProcessor.cpp `update_flush_per_filament`): on an Elegoo
//! printer each `M6211` purge (ElegooGCodeProcessorHelper.cpp
//! `process_elegoo_M6211`), and on a printer whose change G-code marks
//! its purge between `; FLUSH_START` and `; FLUSH_END` (Bambu Lab's)
//! what is pushed out there with the head standing still
//! (GCodeProcessor.cpp `process_G1`, `m_flushing`).
//!
//! Each purge is the new filament's. The processor's code would first
//! book what the filament put away left in the nozzle (`nozzle_volume`)
//! to that one, but when it times a slice it never reads the nozzle's
//! volume (only its `apply_config` for a file opened in the viewer does,
//! GCodeProcessor.cpp 1009-1018; the slice's, 879, does not), so in what
//! it writes into the file and its result every purge is the new
//! filament's. That is what is counted here, so the figures are the
//! ones the reference gives for the same print.

use std::f64::consts::PI;

use crate::settings::PrintSettings;

#[derive(Clone, Debug, Default)]
pub(crate) struct Flushed {
    /// mm³ flushed of each filament.
    volumes: Vec<f64>,
    /// The filament in use, as the file has selected it; the reference's
    /// processor starts with the first.
    current: usize,
    /// Between `; FLUSH_START` and `; FLUSH_END`.
    flushing: bool,
}

impl Flushed {
    pub(crate) fn new(filaments: usize) -> Self {
        Self {
            volumes: vec![0.0; filaments],
            ..Self::default()
        }
    }

    /// mm³ flushed of `filament`.
    pub(crate) fn volume(&self, filament: usize) -> f64 {
        self.volumes.get(filament).copied().unwrap_or(0.0)
    }

    fn add(&mut self, filament: usize, volume: f32) {
        if let Some(sum) = self.volumes.get_mut(filament) {
            *sum += f64::from(volume);
        }
    }

    /// The file selects `filament` (a `T` line, `process_T`).
    pub(crate) fn select(&mut self, filament: usize, all: &[PrintSettings]) {
        if filament < all.len() {
            self.current = filament;
        }
    }

    /// Reads a stretch of G-code the printer's templates wrote.
    pub(crate) fn read(&mut self, text: &str, all: &[PrintSettings]) {
        if all.is_empty() {
            return;
        }
        let elegoo = all[0].elegoo;
        for line in text.lines() {
            let (code, comment) = match line.split_once(';') {
                Some((code, comment)) => (code.trim(), Some(comment)),
                None => (line.trim(), None),
            };
            if let Some(comment) = comment {
                if comment.starts_with(" FLUSH_START") {
                    self.flushing = true;
                } else if comment.starts_with(" FLUSH_END") {
                    self.flushing = false;
                }
            }
            let Some(command) = code.split_whitespace().next() else {
                continue;
            };
            let value = |letter: char| {
                code.split_whitespace()
                    .skip(1)
                    .find_map(|word| word.strip_prefix(letter)?.parse::<f32>().ok())
            };
            if let Some(number) = command.strip_prefix('T') {
                // Bambu Lab's own T numbers above 254 are not changes.
                if let Ok(filament) = number.parse::<usize>()
                    && filament <= 254
                {
                    self.select(filament, all);
                }
            } else if command == "M6211" && elegoo {
                // `M6211 T<filament> L<length>`: change to the filament
                // and purge that length of it.
                if let (Some(length), Some(filament)) = (value('L'), value('T'))
                    && length > 0.0
                    && filament >= 0.0
                    && (filament.round() as usize) < all.len()
                {
                    let filament = filament.round() as usize;
                    self.select(filament, all);
                    self.add(filament, area(&all[filament]) * length);
                }
            } else if command == "G1" && self.flushing {
                // A push of filament with the head standing still
                // (`EMoveType::Unretract`).
                let moved = ['X', 'Y', 'Z'].iter().any(|axis| value(*axis).is_some());
                if let (Some(e), false) = (value('E'), moved)
                    && e > 0.0
                {
                    let current = self.current.min(all.len() - 1);
                    self.add(current, area(&all[current]) * e);
                }
            }
        }
    }
}

/// The filament's cross-section, in single precision as there.
fn area(settings: &PrintSettings) -> f32 {
    let radius = 0.5 * settings.filament_diameter as f32;
    PI as f32 * radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(elegoo: bool) -> Vec<PrintSettings> {
        let one = PrintSettings {
            elegoo,
            ..PrintSettings::default()
        };
        vec![one.clone(), one]
    }

    #[test]
    fn an_elegoo_purge_is_the_filament_s_it_loads() {
        let all = tools(true);
        let mut flushed = Flushed::new(2);
        let mm = |flushed: &Flushed, filament| flushed.volume(filament) / f64::from(area(&all[0]));
        // The start G-code loads filament 1 and purges 200 mm of it.
        flushed.read("M6211 A1 L200 T0 Q210 R210 S210\nM6211 T0 L0 M523", &all);
        assert!((mm(&flushed, 0) - 200.0).abs() < 1e-3);
        // Each change purges the filament it loads.
        flushed.read("M6211 T1 L52.1353 M523 N523 Q210 R240 S210\nT1", &all);
        flushed.read("M6211 T0 L28.4374 M523\nT0", &all);
        assert!((mm(&flushed, 1) - 52.1353).abs() < 1e-3);
        assert!((mm(&flushed, 0) - 228.4374).abs() < 1e-3);
        // Not on another make of printer.
        let mut other = Flushed::new(2);
        other.read("M6211 T1 L52 M523", &tools(false));
        assert_eq!(other.volume(1), 0.0);
    }

    #[test]
    fn a_marked_flush_is_what_is_pushed_out_standing_still() {
        let all = tools(false);
        let mut flushed = Flushed::new(2);
        let area = f64::from(area(&all[0]));
        flushed.read(
            "T1\n; FLUSH_START\nG1 E23.7\nG1 X10 E5\nG1 E-2 F1800\nG1 E30 F523\n; FLUSH_END\nG1 E50",
            &all,
        );
        // 53.7 mm, all filament 2's; nothing outside the marks or while
        // moving counts.
        assert_eq!(flushed.volume(0), 0.0);
        assert!((flushed.volume(1) - 53.7 * area).abs() < 1e-2);
    }
}
