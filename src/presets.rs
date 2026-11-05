//! Built-in sketches, compiled into the binary.

pub struct Preset {
    pub name: &'static str,
    /// Menu group: "monet", "paintings", "covers" or "fractals".
    pub group: &'static str,
    pub source: &'static str,
}

macro_rules! presets {
    ($($group:literal: $($name:literal),*;)*) => {
        pub const PRESETS: &[Preset] = &[$($(Preset {
            name: $name,
            group: $group,
            source: include_str!(concat!("../presets/", $name, ".monet")),
        },)*)*];
    };
}

presets! {
    "monet": "lilies", "poppies", "sunrise", "rouen", "impression";
    "paintings": "dawn", "riviera", "glints", "pool", "aurora", "silk", "lacquer", "atlas", "stained",
        "marble", "reeded", "waves", "bokeh", "pulse", "lens", "halftone", "ascii", "trails";
    "covers": "riso", "poster", "cobalt-sun", "vermilion", "riso-seahorse", "orbit", "ring", "swirl", "op",
        "atom", "starburst", "mosaic", "tunnel";
    "fractals": "mandelbrot", "seahorse", "dive", "morph", "julia", "kali", "tricorn", "newton";
    "": "blank", "shader";
}

pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_compiles_to_valid_wgsl() {
        for p in PRESETS {
            let prog =
                crate::chain::compile(p.source).unwrap_or_else(|e| panic!("{}: {}", p.name, e.msg));
            for pass in &prog.graph.passes {
                crate::engine::validate(&pass.source())
                    .unwrap_or_else(|e| panic!("{} {}: {e}", p.name, pass.label));
            }
        }
    }

    #[test]
    fn fractals_pan() {
        for p in PRESETS.iter().filter(|p| p.group == "fractals") {
            assert!(
                crate::chain::compile(p.source).unwrap().uses_view(),
                "{}",
                p.name
            );
        }
    }
}
