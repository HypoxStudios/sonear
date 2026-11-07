//! Picks an accent colour from a cover's pixels.

use gpui_kit::{Hsla, hsla};

use crate::theme;

const BINS: usize = 24;
const MIN_LIGHTNESS: f32 = 0.12;
const MAX_LIGHTNESS: f32 = 0.92;
const MIN_SATURATION: f32 = 0.20;
/// Share of the pixels that must be colourful for the cover to count.
const MIN_COLOURFUL_SHARE: usize = 20;
const SAMPLE_SIDE: u32 = 32;

/// A colour as hue in degrees, saturation and lightness in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    pub hue: f32,
    pub saturation: f32,
    pub lightness: f32,
}

impl Accent {
    pub fn hsla(self) -> Hsla {
        hsla(self.hue / 360., self.saturation, self.lightness, 1.)
    }

    /// The same hue pulled into the saturation and lightness ranges that read well.
    pub fn clamped(self) -> Self {
        Self {
            hue: self.hue,
            saturation: self
                .saturation
                .clamp(theme::ACCENT_SATURATION.0, theme::ACCENT_SATURATION.1),
            lightness: self
                .lightness
                .clamp(theme::ACCENT_LIGHTNESS.0, theme::ACCENT_LIGHTNESS.1),
        }
    }
}

fn to_hsl((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.,
        f32::from(g) / 255.,
        f32::from(b) / 255.,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.;
    let delta = max - min;

    if delta <= f32::EPSILON {
        return (0., 0., lightness);
    }

    let saturation = delta / (1. - (2. * lightness - 1.).abs());
    let hue = if max == r {
        ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        (b - r) / delta + 2.
    } else {
        (r - g) / delta + 4.
    };

    (hue * 60., saturation, lightness)
}

#[derive(Default, Clone, Copy)]
struct Bin {
    weight: f32,
    count: usize,
    hue: f32,
    saturation: f32,
    lightness: f32,
}

/// The dominant colourful hue of `pixels`, clamped for readability. Near-black,
/// near-white and grey pixels are ignored; `None` when too few are left.
pub fn accent_from_pixels(pixels: impl IntoIterator<Item = (u8, u8, u8)>) -> Option<Accent> {
    let mut bins = [Bin::default(); BINS];
    let mut total = 0;
    let mut colourful = 0;

    for pixel in pixels {
        total += 1;

        let (hue, saturation, lightness) = to_hsl(pixel);

        if !(MIN_LIGHTNESS..=MAX_LIGHTNESS).contains(&lightness) || saturation < MIN_SATURATION {
            continue;
        }

        let bin = &mut bins[((hue / 360. * BINS as f32) as usize).min(BINS - 1)];

        bin.weight += saturation;
        bin.count += 1;
        bin.hue += hue;
        bin.saturation += saturation;
        bin.lightness += lightness;
        colourful += 1;
    }

    if colourful == 0 || colourful * MIN_COLOURFUL_SHARE < total {
        return None;
    }

    let best = bins
        .iter()
        .filter(|bin| bin.count > 0)
        .max_by(|a, b| a.weight.total_cmp(&b.weight))?;
    let count = best.count as f32;

    Some(
        Accent {
            hue: best.hue / count,
            saturation: best.saturation / count,
            lightness: best.lightness / count,
        }
        .clamped(),
    )
}

/// Decodes a cover and finds its accent on a small copy of it.
pub fn accent_from_image(bytes: &[u8]) -> Option<Accent> {
    let image = image::load_from_memory(bytes).ok()?;
    let small = image.thumbnail(SAMPLE_SIDE, SAMPLE_SIDE).to_rgb8();

    accent_from_pixels(small.pixels().map(|pixel| (pixel[0], pixel[1], pixel[2])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(color: (u8, u8, u8), count: usize) -> Vec<(u8, u8, u8)> {
        vec![color; count]
    }

    fn luminance(color: Hsla) -> f32 {
        let rgba = gpui_kit::Rgba::from(color);
        let linear = |c: f32| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };

        0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (a, b) = (luminance(a), luminance(b));

        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn red_gives_a_red_hue() {
        let accent = accent_from_pixels(solid((220, 30, 30), 100)).unwrap();

        assert!(accent.hue < 10. || accent.hue > 350.);
        assert!(accent.saturation >= theme::ACCENT_SATURATION.0);
        assert!(accent.lightness >= theme::ACCENT_LIGHTNESS.0);
    }

    #[test]
    fn grey_black_and_white_give_none() {
        assert_eq!(accent_from_pixels(solid((128, 128, 128), 100)), None);
        assert_eq!(accent_from_pixels(solid((0, 0, 0), 100)), None);
        assert_eq!(accent_from_pixels(solid((255, 255, 255), 100)), None);
        assert_eq!(accent_from_pixels(Vec::new()), None);
    }

    #[test]
    fn a_few_stray_colourful_pixels_are_not_enough() {
        let mut pixels = solid((90, 90, 90), 200);

        pixels.extend(solid((220, 30, 30), 3));

        assert_eq!(accent_from_pixels(pixels), None);
    }

    #[test]
    fn the_dominant_hue_wins() {
        let mut pixels = solid((30, 60, 220), 60);

        pixels.extend(solid((220, 30, 30), 20));
        pixels.extend(solid((100, 100, 100), 20));

        let accent = accent_from_pixels(pixels).unwrap();

        assert!((200. ..260.).contains(&accent.hue), "{}", accent.hue);
    }

    #[test]
    fn decodes_a_png() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let image = image::RgbImage::from_pixel(64, 64, image::Rgb([30, 200, 60]));

        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();

        let accent = accent_from_image(buffer.get_ref()).unwrap();

        assert!((100. ..140.).contains(&accent.hue), "{}", accent.hue);
        assert_eq!(accent_from_image(b"not an image"), None);
    }

    #[test]
    fn every_hue_is_readable_on_the_panel_and_under_dark_text() {
        for degrees in 0..360 {
            for (saturation, lightness) in [(0., 0.), (1., 1.), (0.5, 0.75)] {
                let accent = Accent {
                    hue: degrees as f32,
                    saturation,
                    lightness,
                }
                .clamped()
                .hsla();

                assert!(contrast(accent, theme::panel()) >= 4.5, "hue {degrees}");
                assert!(contrast(accent, theme::on_accent()) >= 4.5, "hue {degrees}");
            }
        }
    }

    #[cfg(feature = "demo")]
    #[test]
    fn demo_covers_give_varied_accents() {
        let mut hues = Vec::new();

        for (path, bytes) in crate::assets::DEMO_COVERS {
            let accent = accent_from_image(bytes).unwrap_or_else(|| panic!("no accent for {path}"));

            hues.push(accent.hue);
        }

        let mut distinct: Vec<f32> = Vec::new();

        for hue in hues {
            let apart = |other: &f32| {
                let gap = (hue - other).abs();

                gap.min(360. - gap) >= 25.
            };

            if distinct.iter().all(apart) {
                distinct.push(hue);
            }
        }

        assert!(distinct.len() >= 6, "{distinct:?}");
    }
}
