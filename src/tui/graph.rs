// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style, Stylize},
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};

use crate::app::App;
use crate::effects::equalizer::{BiquadCoeffs, biquad_coefficients, biquad_magnitude_db};
use crate::pipeline::SAMPLE_RATE;

/// Number of frequency points to evaluate for the response curve.
const RESPONSE_POINTS: usize = 200;

/// Generate `n` log-spaced frequency points from `start` to `end` Hz.
#[allow(
    clippy::cast_precision_loss,
    reason = "n is small (200), no precision issue"
)]
fn log_frequencies(start: f32, end: f32, n: usize) -> Vec<f32> {
    let ratio = (end / start).powf(1.0 / (n.saturating_sub(1)) as f32);
    let mut freqs = Vec::with_capacity(n);
    let mut f = start;
    for _ in 0..n {
        freqs.push(f);
        f *= ratio;
    }
    freqs
}

/// Compute the combined magnitude response (dB) of all filters at a given frequency.
fn combined_response_db(coeffs: &[BiquadCoeffs], freq_hz: f32, sample_rate: f32) -> f64 {
    coeffs
        .iter()
        .map(|c| f64::from(biquad_magnitude_db(c, freq_hz, sample_rate)))
        .sum()
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let bands = &app.eq.bands;
    let bypass = app.eq.bypass;
    let preamp = app.preamp;

    // Build the response curve
    let curve: Vec<(f64, f64)> = if bands.is_empty() || bypass {
        // Flat 0 dB line when no bands or bypassed
        vec![(20.0f64.log10(), 0.0), (20000.0f64.log10(), 0.0)]
    } else {
        let coeffs: Vec<BiquadCoeffs> = bands
            .iter()
            .map(|b| biquad_coefficients(b, SAMPLE_RATE))
            .collect();

        let freqs = log_frequencies(20.0, 20000.0, RESPONSE_POINTS);
        freqs
            .iter()
            .map(|&f| {
                let x = f64::from(f).log10();
                let y = combined_response_db(&coeffs, f, SAMPLE_RATE);
                (x, y)
            })
            .collect()
    };

    // 0 dB reference line
    let ref_line: [(f64, f64); 2] = [(20.0f64.log10(), 0.0), (20000.0f64.log10(), 0.0)];

    // Determine curve styling
    let curve_dark = bypass || bands.is_empty();
    let curve_style = if curve_dark {
        Style::default().dark_gray()
    } else {
        Style::default().cyan().bold()
    };

    let mut datasets = vec![
        Dataset::default()
            .name("0 dB Ref")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().dark_gray().not_bold())
            .data(&ref_line),
    ];

    datasets.push(
        Dataset::default()
            .name(if bypass { "Bypassed" } else { "Response" })
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(curve_style)
            .data(&curve),
    );

    // Build the block title
    let mut title = String::from(" Frequency Response ");
    if !bands.is_empty() {
        let _ = std::fmt::write(&mut title, format_args!("| Preamp: {preamp:.1} dB "));
    }
    if bypass {
        title.push_str("[BYPASSED] ");
    }

    let x_axis = Axis::default()
        .title("Frequency (Hz)".white())
        .style(Style::default().gray())
        .bounds([20.0f64.log10(), 20000.0f64.log10()])
        .labels(["20", "100", "1k", "10k", "20k"]);

    let y_axis = Axis::default()
        .title("Gain (dB)".white())
        .style(Style::default().gray())
        .bounds([-24.0, 24.0])
        .labels(["-24", "-12", "0", "+12", "+24"]);

    let border_style = if bypass {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(title.as_str())
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .x_axis(x_axis)
        .y_axis(y_axis);

    frame.render_widget(chart, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_frequencies_range() {
        let freqs = log_frequencies(20.0, 20000.0, 200);
        assert_eq!(freqs.len(), 200);
        assert!((freqs[0] - 20.0).abs() < 0.01);
        assert!((freqs[199] - 20000.0).abs() < 1.0);
        // Verify log-spacing: each step should be a constant ratio > 1
        let ratio = freqs[1] / freqs[0];
        for i in 2..freqs.len() {
            let r = freqs[i] / freqs[i - 1];
            assert!((r - ratio).abs() < 0.001, "non-uniform log spacing at {i}");
        }
    }

    #[test]
    fn log_frequencies_single_point() {
        let freqs = log_frequencies(100.0, 100.0, 1);
        assert_eq!(freqs.len(), 1);
        assert!((freqs[0] - 100.0).abs() < 0.01);
    }

    #[test]
    fn combined_response_empty_coeffs() {
        assert!((combined_response_db(&[], 1000.0, 48000.0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn combined_response_peak_at_center() {
        let band = crate::state::EqBand {
            frequency: 1000.0,
            gain: 6.0,
            q: 1.0,
            filter_type: crate::state::FilterType::Peak,
        };
        let coeffs = vec![biquad_coefficients(&band, 48000.0)];
        // At the center frequency, a peaking filter with +6 dB gain
        // should be close to +6 dB
        let db = combined_response_db(&coeffs, 1000.0, 48000.0);
        assert!(
            (db - 6.0).abs() < 0.5,
            "expected ~6 dB at center, got {db:.3}"
        );
    }

    #[test]
    fn combined_response_far_from_center() {
        let band = crate::state::EqBand {
            frequency: 1000.0,
            gain: 12.0,
            q: 5.0,
            filter_type: crate::state::FilterType::Peak,
        };
        let coeffs = vec![biquad_coefficients(&band, 48000.0)];
        // Far from the center frequency, gain should approach 0 dB
        let db = combined_response_db(&coeffs, 100.0, 48000.0);
        assert!(
            db.abs() < 1.0,
            "expected near 0 dB far from center, got {db:.3}"
        );
    }

    #[test]
    fn response_far_from_center_very_low_freq() {
        // Regression test for catastrophic cancellation at low frequencies.
        // A peaking filter at 1 kHz should be ~0 dB at 20 Hz.
        let band = crate::state::EqBand {
            frequency: 1000.0,
            gain: 6.0,
            q: 1.0,
            filter_type: crate::state::FilterType::Peak,
        };
        let coeffs = vec![biquad_coefficients(&band, 48000.0)];
        let db = combined_response_db(&coeffs, 20.0, 48000.0);
        assert!(
            db.abs() < 0.1,
            "expected ~0 dB at 20 Hz for 1 kHz PK filter, got {db:.6}"
        );
    }

    #[test]
    fn multiple_bands_response_far_from_centers() {
        // Several bands with their centres at ≥ 1 kHz should be flat at 20-100 Hz
        let bands = [
            crate::state::EqBand {
                frequency: 2000.0,
                gain: 8.0,
                q: 2.0,
                filter_type: crate::state::FilterType::Peak,
            },
            crate::state::EqBand {
                frequency: 5000.0,
                gain: -3.0,
                q: 1.5,
                filter_type: crate::state::FilterType::Peak,
            },
            crate::state::EqBand {
                frequency: 10000.0,
                gain: 2.0,
                q: 0.7,
                filter_type: crate::state::FilterType::HighShelf,
            },
        ];
        let coeffs: Vec<_> = bands
            .iter()
            .map(|b| biquad_coefficients(b, 48000.0))
            .collect();

        let db_20 = combined_response_db(&coeffs, 20.0, 48000.0);
        let db_50 = combined_response_db(&coeffs, 50.0, 48000.0);
        let db_100 = combined_response_db(&coeffs, 100.0, 48000.0);

        assert!(db_20.abs() < 0.2, "20 Hz: expected ~0, got {db_20:.6}");
        assert!(db_50.abs() < 0.2, "50 Hz: expected ~0, got {db_50:.6}");
        assert!(db_100.abs() < 0.2, "100 Hz: expected ~0, got {db_100:.6}");
    }
}
