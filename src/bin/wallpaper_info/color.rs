use std::collections::HashMap;

pub const COLOR_RESET: &str = "\x1b[0m";

#[derive(Debug, Clone, Copy)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn as_fg(self) -> String {
        format!("\x1b[38;2;{};{};{}m", self.r, self.g, self.b)
    }

    pub fn as_bg(self) -> String {
        format!("\x1b[48;2;{};{};{}m", self.r, self.g, self.b)
    }

    fn luminance(&self) -> f64 {
        0.299 * self.r as f64 / 255.0
            + 0.587 * self.g as f64 / 255.0
            + 0.114 * self.b as f64 / 255.0
    }

    fn saturation(&self) -> f64 {
        let max = self.r.max(self.g).max(self.b) as f64;
        let min = self.r.min(self.g).min(self.b) as f64;
        if max == 0.0 {
            0.0
        } else {
            (max - min) / max
        }
    }

    pub fn lighten(&self, factor: f64) -> Rgb {
        Rgb {
            r: (self.r as f64 + (255.0 - self.r as f64) * factor) as u8,
            g: (self.g as f64 + (255.0 - self.g as f64) * factor) as u8,
            b: (self.b as f64 + (255.0 - self.b as f64) * factor) as u8,
        }
    }

    pub fn darken(&self, factor: f64) -> Rgb {
        Rgb {
            r: (self.r as f64 * (1.0 - factor)) as u8,
            g: (self.g as f64 * (1.0 - factor)) as u8,
            b: (self.b as f64 * (1.0 - factor)) as u8,
        }
    }

    pub fn muted(&self) -> Rgb {
        let gray = (self.r as u32 + self.g as u32 + self.b as u32) / 3;
        Rgb {
            r: ((self.r as u32 + gray) / 2) as u8,
            g: ((self.g as u32 + gray) / 2) as u8,
            b: ((self.b as u32 + gray) / 2) as u8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColorPalette {
    pub accent: Rgb,
    pub secondary: Rgb,
    pub background: Rgb,
    pub text: Rgb,
    pub dim: Rgb,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self {
            accent: Rgb {
                r: 255,
                g: 170,
                b: 100,
            },
            secondary: Rgb {
                r: 100,
                g: 160,
                b: 220,
            },
            background: Rgb {
                r: 20,
                g: 25,
                b: 35,
            },
            text: Rgb {
                r: 220,
                g: 225,
                b: 230,
            },
            dim: Rgb {
                r: 120,
                g: 125,
                b: 135,
            },
        }
    }
}

pub fn extract_palette(image: &image::DynamicImage) -> ColorPalette {
    let small = image.resize(64, 64, image::imageops::FilterType::Nearest);
    let rgb_image = small.to_rgb8();

    let mut color_counts: HashMap<(u8, u8, u8), u32> = HashMap::new();
    for pixel in rgb_image.pixels() {
        let key = (pixel[0] / 16 * 16, pixel[1] / 16 * 16, pixel[2] / 16 * 16);
        *color_counts.entry(key).or_insert(0) += 1;
    }

    let mut colors: Vec<((u8, u8, u8), u32)> = color_counts.into_iter().collect();
    colors.sort_by(|a, b| b.1.cmp(&a.1));

    let accent = colors
        .iter()
        .take(20)
        .filter_map(|((r, g, b), count)| {
            let rgb = Rgb {
                r: *r,
                g: *g,
                b: *b,
            };
            let lum = rgb.luminance();
            if lum > 0.15 && lum < 0.85 {
                Some((rgb, rgb.saturation() * (*count as f64).sqrt()))
            } else {
                None
            }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(rgb, _)| rgb)
        .unwrap_or(Rgb {
            r: 255,
            g: 170,
            b: 100,
        });

    let secondary = colors
        .iter()
        .take(20)
        .filter_map(|((r, g, b), _)| {
            let rgb = Rgb {
                r: *r,
                g: *g,
                b: *b,
            };
            let diff = (accent.r as i32 - rgb.r as i32).abs()
                + (accent.g as i32 - rgb.g as i32).abs()
                + (accent.b as i32 - rgb.b as i32).abs();
            if diff > 100 && rgb.saturation() > 0.2 && rgb.luminance() > 0.15 {
                Some(rgb)
            } else {
                None
            }
        })
        .next()
        .unwrap_or(accent);

    let background = colors
        .iter()
        .find(|((r, g, b), _)| {
            Rgb {
                r: *r,
                g: *g,
                b: *b,
            }
            .luminance()
                < 0.3
        })
        .map(|((r, g, b), _)| {
            Rgb {
                r: *r,
                g: *g,
                b: *b,
            }
            .darken(0.6)
        })
        .unwrap_or(Rgb {
            r: 15,
            g: 20,
            b: 30,
        });

    ColorPalette {
        accent: if accent.luminance() < 0.3 {
            accent.lighten(0.4)
        } else {
            accent
        },
        secondary: if secondary.luminance() < 0.3 {
            secondary.lighten(0.3)
        } else {
            secondary.muted()
        },
        background,
        text: Rgb {
            r: 230,
            g: 235,
            b: 240,
        },
        dim: Rgb {
            r: 140,
            g: 145,
            b: 155,
        },
    }
}
