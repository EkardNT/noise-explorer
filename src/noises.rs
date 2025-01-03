use egui::{Align, ImageSource, Layout, Vec2};
use noise::{NoiseFn, Perlin};
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

pub struct DynNoise(Box<dyn NoiseFn<f64, 2> + Send + 'static>);

impl DynNoise {
    pub fn new(noise_fn: impl NoiseFn<f64, 2> + Send + 'static) -> Self {
        Self(Box::new(noise_fn))
    }
}

impl NoiseFn<f64, 2> for DynNoise {
    fn get(&self, point: [f64; 2]) -> f64 {
        self.0.get(point)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum NoiseClassification {
    Source,
    Combinator
}

#[derive(Debug, Eq, PartialEq, VariantArray, Clone, Copy, Serialize, Deserialize)]
pub enum NoiseType {
    // Sources
    Checkerboard,
    Perlin,
    Constant,

    // Combinators
    Blend,
    Max,
    Min,
}

impl NoiseType {
    pub fn all() -> impl Iterator<Item = &'static NoiseType> {
        NoiseType::VARIANTS.iter()
    }

    pub fn combinators() -> impl Iterator<Item = &'static NoiseType> {
        Self::all().filter(|n| n.classification() == NoiseClassification::Combinator)
    }

    pub fn sources() -> impl Iterator<Item = &'static NoiseType> {
        Self::all().filter(|n| n.classification() == NoiseClassification::Source)
    }

    pub const fn name(&self) -> &'static str {
        use NoiseType::*;
        match self {
            Perlin => "Perlin",
            Max => "Maximum",
            Min => "Minimum",
            Blend => "Blend",
            Checkerboard => "Checkerboard",
            Constant => "Constant",
        }
    }

    pub fn lowercase_name(&self) -> &'static str {
        use NoiseType::*;
        match self {
            Perlin => "perlin",
            Max => "maximum",
            Min => "minimum",
            Blend => "blend",
            Checkerboard => "checkerboard",
            Constant => "constant",
        }
    }

    pub fn classification(&self) -> NoiseClassification {
        use NoiseType::*;
        match self {
            Perlin | Checkerboard | Constant => NoiseClassification::Source,
            Max | Min | Blend => NoiseClassification::Combinator,
        }
    }

    pub fn config(&self) -> NoiseConfig {
        use NoiseType::*;
        match self {
            Perlin => NoiseConfig::Perlin { 
                seed: 12345
            },
            Constant => NoiseConfig::Constant {
                value: 0.5
            },
            _ => NoiseConfig::Empty
        }
    }

    pub fn input_count(&self) -> usize {
        use NoiseType::*;
        match self {
            Checkerboard | Perlin | Constant => 0,
            Blend => 3,
            Max | Min => 2,
        }
    }

    pub fn show_input(&self, input_index: usize, ui: &mut egui::Ui, scale: f32) {
        use NoiseType::*;
        match self {
            Checkerboard | Perlin | Constant => panic!("No input expected"),
            Blend => match input_index {
                0 => ui.label("A"),
                1 => ui.label("B"),
                2 => ui.label("Control"),
                _ => panic!("Unexpected input pin index")
            },
            Max | Min => match input_index {
                0 => ui.label("A"),
                1 => ui.label("B"),
                _ => panic!("Unexpected input pin index")
            },
        };
    }

    pub fn show_header(&self, config: &mut NoiseConfig, ui: &mut egui::Ui, scale: f32) -> HeaderResponse {
        ui.set_height(16.0 * scale);
        ui.set_min_width(128.0 * scale);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(self.name()).selectable(false));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(" x ").clicked() {
                HeaderResponse::Remove
            } else {
                HeaderResponse::None
            }
        }).inner
    }

    pub fn show_body(&self, config: &mut NoiseConfig, ui: &mut egui::Ui, scale: f32) -> bool {
        use NoiseConfig::*;
        match config {
            Empty => false,
            Perlin { seed } => ui.add(egui::Slider::new(seed, 0 ..= std::u32::MAX)).changed(),
            Constant { value } => ui.add(egui::Slider::new(value, 0.0 ..= 1.0)).changed(),
        }
    }
}


#[derive(Serialize, Deserialize)]
pub enum NoiseConfig {
    Empty,
    Perlin {
        seed: u32
    },
    Constant {
        value: f64
    }
}

pub enum HeaderResponse {
    Remove,
    Changed,
    None
}