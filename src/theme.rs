use eframe::egui::Color32;

use crate::config::{ColorTheme, GraphicsQuality};

/// Linear RGB triplet `[r, g, b]` in `0.0..=1.0+` passed to the GLSL orb shader.
pub type RgbF = [f32; 3];

/// Complete palette for a single theme preset (shader linear RGB + crisp sRGB UI accents).
#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub short_name: &'static str,
    pub display_name: &'static str,

    // Swatch dots for the Settings / Setup theme picker (sRGB)
    pub swatches: [Color32; 3],

    // UI accent colors (sRGB)
    pub ui_primary: Color32,
    pub ui_secondary: Color32,
    pub ui_speaking: Color32,
    pub ui_thinking: Color32,
    pub ui_bg_tint: Color32,

    // Shader state palettes in linear RGB: (primary, secondary, accent)
    pub listen: (RgbF, RgbF, RgbF),
    pub speak: (RgbF, RgbF, RgbF),
    pub think: (RgbF, RgbF, RgbF),
    pub sleep: (RgbF, RgbF, RgbF),

    // Subtle atmospheric backdrop tint (linear RGB)
    pub bg_center: RgbF,
    pub bg_edge: RgbF,

    // > 0.0 enables smooth rainbow / iridescent spectrum shifting
    pub rainbow: f32,
}

impl ThemePalette {
    pub fn for_theme(theme: ColorTheme) -> Self {
        match theme {
            ColorTheme::Obsidian => Self {
                short_name: "Obsidian",
                display_name: "Neon Obsidian",
                swatches: [
                    Color32::from_rgb(148, 108, 255),
                    Color32::from_rgb(38, 224, 245),
                    Color32::from_rgb(64, 242, 184),
                ],
                ui_primary: Color32::from_rgb(148, 108, 255),
                ui_secondary: Color32::from_rgb(38, 224, 245),
                ui_speaking: Color32::from_rgb(64, 242, 184),
                ui_thinking: Color32::from_rgb(255, 162, 72),
                ui_bg_tint: Color32::from_rgb(7, 6, 16),
                listen: ([0.42, 0.18, 0.98], [0.06, 0.84, 0.98], [0.84, 0.24, 0.90]),
                speak: ([0.04, 0.82, 0.96], [0.14, 0.98, 0.74], [0.54, 0.28, 1.00]),
                think: ([0.64, 0.16, 0.96], [1.00, 0.52, 0.16], [0.98, 0.22, 0.62]),
                sleep: ([0.20, 0.14, 0.48], [0.12, 0.36, 0.56], [0.36, 0.24, 0.62]),
                bg_center: [0.016, 0.012, 0.036],
                bg_edge: [0.005, 0.004, 0.014],
                rainbow: 0.0,
            },
            ColorTheme::AuroraPrism => Self {
                short_name: "Aurora",
                display_name: "Aurora Prism",
                swatches: [
                    Color32::from_rgb(255, 78, 205),
                    Color32::from_rgb(38, 232, 255),
                    Color32::from_rgb(255, 214, 70),
                ],
                ui_primary: Color32::from_rgb(232, 110, 255),
                ui_secondary: Color32::from_rgb(46, 234, 255),
                ui_speaking: Color32::from_rgb(84, 252, 178),
                ui_thinking: Color32::from_rgb(255, 198, 66),
                ui_bg_tint: Color32::from_rgb(8, 6, 18),
                listen: ([0.95, 0.18, 0.78], [0.08, 0.88, 1.00], [0.98, 0.78, 0.14]),
                speak: ([0.08, 0.92, 0.98], [0.22, 1.00, 0.56], [0.98, 0.30, 0.86]),
                think: ([0.98, 0.32, 0.74], [1.00, 0.74, 0.14], [0.24, 0.86, 1.00]),
                sleep: ([0.34, 0.16, 0.52], [0.14, 0.42, 0.56], [0.54, 0.30, 0.42]),
                bg_center: [0.018, 0.012, 0.038],
                bg_edge: [0.006, 0.004, 0.014],
                rainbow: 1.0,
            },
            ColorTheme::CyberSakura => Self {
                short_name: "Sakura",
                display_name: "Cyber Sakura",
                swatches: [
                    Color32::from_rgb(255, 92, 172),
                    Color32::from_rgb(206, 98, 255),
                    Color32::from_rgb(255, 176, 218),
                ],
                ui_primary: Color32::from_rgb(255, 96, 176),
                ui_secondary: Color32::from_rgb(212, 118, 255),
                ui_speaking: Color32::from_rgb(255, 168, 216),
                ui_thinking: Color32::from_rgb(255, 164, 110),
                ui_bg_tint: Color32::from_rgb(16, 7, 16),
                listen: ([0.98, 0.20, 0.58], [0.76, 0.24, 0.98], [1.00, 0.62, 0.82]),
                speak: ([1.00, 0.32, 0.68], [0.98, 0.68, 0.88], [0.24, 0.84, 0.98]),
                think: ([0.88, 0.18, 0.72], [1.00, 0.54, 0.32], [1.00, 0.38, 0.68]),
                sleep: ([0.44, 0.14, 0.32], [0.32, 0.16, 0.46], [0.54, 0.26, 0.44]),
                bg_center: [0.032, 0.011, 0.028],
                bg_edge: [0.012, 0.004, 0.011],
                rainbow: 0.0,
            },
            ColorTheme::SolarFlare => Self {
                short_name: "Solar",
                display_name: "Solar Flare",
                swatches: [
                    Color32::from_rgb(255, 114, 46),
                    Color32::from_rgb(255, 206, 72),
                    Color32::from_rgb(255, 58, 92),
                ],
                ui_primary: Color32::from_rgb(255, 122, 54),
                ui_secondary: Color32::from_rgb(255, 204, 76),
                ui_speaking: Color32::from_rgb(255, 224, 112),
                ui_thinking: Color32::from_rgb(255, 76, 98),
                ui_bg_tint: Color32::from_rgb(16, 9, 7),
                listen: ([1.00, 0.32, 0.08], [1.00, 0.74, 0.14], [0.96, 0.14, 0.34]),
                speak: ([1.00, 0.56, 0.12], [1.00, 0.88, 0.30], [1.00, 0.26, 0.24]),
                think: ([0.96, 0.16, 0.28], [1.00, 0.58, 0.10], [1.00, 0.82, 0.24]),
                sleep: ([0.46, 0.18, 0.08], [0.42, 0.28, 0.10], [0.46, 0.14, 0.20]),
                bg_center: [0.034, 0.016, 0.010],
                bg_edge: [0.012, 0.005, 0.004],
                rainbow: 0.0,
            },
            ColorTheme::EmeraldPulse => Self {
                short_name: "Emerald",
                display_name: "Emerald Pulse",
                swatches: [
                    Color32::from_rgb(32, 240, 152),
                    Color32::from_rgb(24, 214, 232),
                    Color32::from_rgb(164, 255, 72),
                ],
                ui_primary: Color32::from_rgb(36, 238, 152),
                ui_secondary: Color32::from_rgb(32, 218, 236),
                ui_speaking: Color32::from_rgb(154, 255, 96),
                ui_thinking: Color32::from_rgb(255, 206, 68),
                ui_bg_tint: Color32::from_rgb(6, 15, 12),
                listen: ([0.06, 0.88, 0.46], [0.05, 0.78, 0.90], [0.52, 0.98, 0.18]),
                speak: ([0.08, 0.96, 0.62], [0.54, 1.00, 0.26], [0.06, 0.84, 0.94]),
                think: ([0.14, 0.82, 0.48], [0.96, 0.82, 0.16], [0.08, 0.88, 0.78]),
                sleep: ([0.08, 0.38, 0.26], [0.08, 0.34, 0.42], [0.22, 0.44, 0.18]),
                bg_center: [0.009, 0.028, 0.022],
                bg_edge: [0.003, 0.010, 0.009],
                rainbow: 0.0,
            },
            ColorTheme::ArcticFrost => Self {
                short_name: "Arctic",
                display_name: "Arctic Frost",
                swatches: [
                    Color32::from_rgb(66, 174, 255),
                    Color32::from_rgb(132, 244, 255),
                    Color32::from_rgb(164, 174, 255),
                ],
                ui_primary: Color32::from_rgb(76, 178, 255),
                ui_secondary: Color32::from_rgb(138, 242, 255),
                ui_speaking: Color32::from_rgb(196, 248, 255),
                ui_thinking: Color32::from_rgb(156, 152, 255),
                ui_bg_tint: Color32::from_rgb(6, 11, 20),
                listen: ([0.14, 0.56, 1.00], [0.42, 0.92, 1.00], [0.56, 0.62, 1.00]),
                speak: ([0.34, 0.86, 1.00], [0.76, 0.98, 1.00], [0.24, 0.64, 1.00]),
                think: ([0.38, 0.48, 1.00], [0.44, 0.92, 1.00], [0.74, 0.82, 1.00]),
                sleep: ([0.12, 0.28, 0.52], [0.20, 0.42, 0.56], [0.26, 0.30, 0.54]),
                bg_center: [0.010, 0.020, 0.038],
                bg_edge: [0.004, 0.007, 0.015],
                rainbow: 0.0,
            },
            ColorTheme::CrimsonPhantom => Self {
                short_name: "Crimson",
                display_name: "Crimson Phantom",
                swatches: [
                    Color32::from_rgb(255, 56, 94),
                    Color32::from_rgb(164, 58, 255),
                    Color32::from_rgb(255, 134, 118),
                ],
                ui_primary: Color32::from_rgb(255, 62, 98),
                ui_secondary: Color32::from_rgb(176, 78, 255),
                ui_speaking: Color32::from_rgb(255, 128, 136),
                ui_thinking: Color32::from_rgb(255, 174, 68),
                ui_bg_tint: Color32::from_rgb(16, 6, 11),
                listen: ([0.96, 0.10, 0.28], [0.56, 0.14, 0.96], [1.00, 0.38, 0.42]),
                speak: ([1.00, 0.18, 0.36], [1.00, 0.48, 0.32], [0.76, 0.16, 0.94]),
                think: ([0.88, 0.10, 0.42], [1.00, 0.56, 0.14], [1.00, 0.22, 0.54]),
                sleep: ([0.44, 0.10, 0.20], [0.28, 0.12, 0.42], [0.48, 0.18, 0.26]),
                bg_center: [0.032, 0.009, 0.018],
                bg_edge: [0.012, 0.003, 0.007],
                rainbow: 0.0,
            },
            ColorTheme::Vaporwave => Self {
                short_name: "Vapor",
                display_name: "Vaporwave Neon",
                swatches: [
                    Color32::from_rgb(255, 66, 206),
                    Color32::from_rgb(16, 238, 255),
                    Color32::from_rgb(255, 224, 88),
                ],
                ui_primary: Color32::from_rgb(255, 74, 210),
                ui_secondary: Color32::from_rgb(24, 238, 255),
                ui_speaking: Color32::from_rgb(255, 226, 92),
                ui_thinking: Color32::from_rgb(142, 92, 255),
                ui_bg_tint: Color32::from_rgb(12, 6, 20),
                listen: ([0.98, 0.16, 0.78], [0.04, 0.90, 1.00], [0.52, 0.22, 1.00]),
                speak: ([0.04, 0.92, 1.00], [1.00, 0.84, 0.20], [1.00, 0.22, 0.78]),
                think: ([0.58, 0.18, 1.00], [1.00, 0.22, 0.72], [0.06, 0.90, 0.98]),
                sleep: ([0.42, 0.14, 0.46], [0.10, 0.36, 0.52], [0.34, 0.18, 0.52]),
                bg_center: [0.026, 0.010, 0.042],
                bg_edge: [0.009, 0.004, 0.016],
                rainbow: 0.25,
            },
        }
    }

    /// Computes smoothly weighted linear-RGB `(primary, secondary, accent)` for the active state.
    pub fn blended_shader_colors(&self, weights: [f32; 4], muted: f32) -> (RgbF, RgbF, RgbF) {
        let blend3 = |a: RgbF, b: RgbF, c: RgbF, d: RgbF| -> RgbF {
            [
                a[0] * weights[0] + b[0] * weights[1] + c[0] * weights[2] + d[0] * weights[3],
                a[1] * weights[0] + b[1] * weights[1] + c[1] * weights[2] + d[1] * weights[3],
                a[2] * weights[0] + b[2] * weights[1] + c[2] * weights[2] + d[2] * weights[3],
            ]
        };

        let mut p = blend3(self.listen.0, self.speak.0, self.think.0, self.sleep.0);
        let mut s = blend3(self.listen.1, self.speak.1, self.think.1, self.sleep.1);
        let a = blend3(self.listen.2, self.speak.2, self.think.2, self.sleep.2);

        if muted > 0.001 {
            let mute_tint = [0.82_f32, 0.14, 0.34];
            let mute_sec = [0.42_f32, 0.22, 0.52];
            for i in 0..3 {
                let target_p = p[i] + (mute_tint[i] - p[i]) * 0.58;
                p[i] += (target_p - p[i]) * muted;
                s[i] += (mute_sec[i] - s[i]) * (muted * 0.65);
            }
        }

        (p, s, a)
    }

    /// HSV rainbow helper for animated UI spectrum bars when `rainbow > 0`.
    pub fn rainbow_srgb(hue: f32) -> Color32 {
        let h = hue.rem_euclid(1.0) * 6.0;
        let x = 1.0 - ((h % 2.0) - 1.0).abs();
        let (r, g, b) = match h as i32 {
            0 => (1.0, x, 0.18),
            1 => (x, 1.0, 0.18),
            2 => (0.18, 1.0, x),
            3 => (0.18, x, 1.0),
            4 => (x, 0.18, 1.0),
            _ => (1.0, 0.18, x),
        };
        Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }
}

pub fn quality_label(q: GraphicsQuality) -> &'static str {
    match q {
        GraphicsQuality::Low => "Low (Lite)",
        GraphicsQuality::Medium => "Medium",
        GraphicsQuality::High => "High (Ultra)",
    }
}

pub fn quality_description(q: GraphicsQuality) -> &'static str {
    match q {
        GraphicsQuality::Low => "Weak / integrated GPUs · 30 FPS sleek 3D glass orb (~15x lighter)",
        GraphicsQuality::Medium => "Standard laptops · 45 FPS 3D raymarched orb (~3-4x lighter)",
        GraphicsQuality::High => "Best visuals · 60 FPS full 64-step 3D Liquid Glass & plasma",
    }
}

pub fn quality_frame_interval_ms(q: GraphicsQuality) -> u64 {
    match q {
        GraphicsQuality::Low => 33,    // ~30 FPS
        GraphicsQuality::Medium => 22, // ~45 FPS
        GraphicsQuality::High => 16,   // ~60 FPS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_themes_provide_valid_palettes_and_blends() {
        for theme in ColorTheme::ALL {
            let pal = ThemePalette::for_theme(theme);
            assert!(!pal.short_name.is_empty());
            assert!(!pal.display_name.is_empty());
            let (p, s, a) = pal.blended_shader_colors([1.0, 0.0, 0.0, 0.0], 0.0);
            for ch in p.into_iter().chain(s).chain(a) {
                assert!((0.0..=1.0).contains(&ch));
            }
        }
    }

    #[test]
    fn quality_intervals_step_down_for_higher_tiers() {
        assert!(
            quality_frame_interval_ms(GraphicsQuality::Low)
                > quality_frame_interval_ms(GraphicsQuality::Medium)
        );
        assert!(
            quality_frame_interval_ms(GraphicsQuality::Medium)
                > quality_frame_interval_ms(GraphicsQuality::High)
        );
    }
}
