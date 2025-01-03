use std::{any::Any, collections::{HashMap, HashSet}, hint::black_box, sync::{atomic::{AtomicUsize, Ordering}, mpsc::{Receiver, RecvError, Sender}, Arc}};

use datazoo::Bimultimap;
use egui::{include_image, Align, Color32, ImageSource, Layout, Pos2, RichText, Ui, Vec2};
use egui_snarl::{ui::{BackgroundPattern, Grid, PinInfo, SnarlStyle, SnarlViewer, WireStyle}, NodeId, Snarl};
use noise::{utils::{NoiseFnWrapper, PlaneMapBuilder}, NoiseFn};
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

use crate::noises::{self, DynNoise, NoiseClassification, NoiseConfig, NoiseType};

slotmap::new_key_type! {
    pub struct NodeSlotKey;
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize, Default)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
struct PersistableApp {
    node_type_filter: String,
    node_type_filter_lowercase: String,
    node_graph: Snarl<GraphNode>,
    node_graph_style: SnarlStyle,
    slot_to_node: SlotMap<NodeSlotKey, NodeId>,
}

pub struct NoiseExplorerApp {
    node_type_filter: String,
    node_type_filter_lowercase: String,
    node_graph: Snarl<GraphNode>,
    node_graph_style: SnarlStyle,
    changed_nodes: HashSet<NodeSlotKey>,
    recalculate_sender: std::sync::mpsc::Sender<RecalculateRequest>,
    recalculate_receiver: std::sync::mpsc::Receiver<RecalculateResult>,
    slot_to_node: SlotMap<NodeSlotKey, NodeId>,
}

impl NoiseExplorerApp {
    fn default(ctx: egui::Context) -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let (response_tx, response_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("Recalculator".to_string())
            .spawn(move || {
                recalculator_thread(request_rx, response_tx, ctx);
            })
            .expect("Failed to spawn recalculator thread");

        Self {
            node_type_filter: "".to_string(),
            node_type_filter_lowercase: "".to_string(),
            node_graph: Snarl::new(),
            node_graph_style: SnarlStyle {
                bg_pattern: Some(BackgroundPattern::Grid(Grid::new(
                    Vec2::new(50.0, 50.0),
                    std::f32::consts::PI / 4.0,
                ))),
                collapsible: Some(false),
                header_drag_space: Some(Vec2::ZERO),
                wire_frame_size: Some(100.0),
                wire_width: Some(3.0),
                ..Default::default()
            },
            changed_nodes: HashSet::new(),
            recalculate_sender: request_tx,
            recalculate_receiver: response_rx,
            slot_to_node: SlotMap::with_key(),
        }
    }
}

fn recalculator_thread(request_rx: Receiver<RecalculateRequest>, response_tx: Sender<RecalculateResult>, ctx: egui::Context) {
    loop {
        let Ok(request) = request_rx.recv() else { break };

        if request.config_version.load(Ordering::SeqCst) != request.new_version {
            // This request has been superseded, skip it.
            continue;
        }
        
        let mut image_colors = Vec::with_capacity(request.texture_width * request.texture_height);
        let mut noise_min = std::f64::MAX;
        let mut noise_max = std::f64::MIN;
        for y in 0..request.texture_height {
            for x in 0..request.texture_width {
                let noise_val = request.noise_fn.get([
                    x as f64 / request.texture_width as f64 * request.noise_width,
                    y as f64 / request.texture_height as f64 * request.noise_height
                ]);
                if noise_val < noise_min {
                    noise_min = noise_val;
                }
                if noise_val > noise_max {
                    noise_max = noise_val;
                }
                // This assumes the noise is in the range [-1, 1].
                let noise_u8 = ((noise_val * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
                image_colors.push(egui::Color32::from_gray(noise_u8));
            }
        }

        // TODO: somehow actually convert this into a texture egui can display.
        std::hint::black_box(image_colors);

        if response_tx.send(RecalculateResult {
            node_id: request.node_id,
            new_version: request.new_version,
            noise_max,
            noise_min,
            texture: ()
        }).is_ok() {
            ctx.request_repaint();
        };
    }
}

impl NoiseExplorerApp {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let default = Self::default(cc.egui_ctx.clone());

        if std::env::var("FRESH").ok().map(|val| val == "true").unwrap_or(false) {
            return default;
        }

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        // if let Some(storage) = cc.storage {
        //     let persistable: PersistableApp = eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
        //     return Self {
        //         node_type_filter: persistable.node_type_filter,
        //         node_type_filter_lowercase: persistable.node_type_filter_lowercase,
        //         node_graph: persistable.node_graph,
        //         node_graph_style: persistable.node_graph_style,
        //         slot_to_node: persistable.slot_to_node,
        //         ..default
        //     };
        // }

        default
    }
}

impl eframe::App for NoiseExplorerApp {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // eframe::set_value(storage, eframe::APP_KEY, &PersistableApp {
        //     node_type_filter: std::mem::take(&mut self.node_type_filter),
        //     node_type_filter_lowercase: std::mem::take(&mut self.node_type_filter_lowercase),
        //     node_graph: std::mem::take(&mut self.node_graph),
        //     node_graph_style: std::mem::take(&mut self.node_graph_style),
        //     slot_to_node: std::mem::take(&mut self.slot_to_node),
        // });
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(response) = self.recalculate_receiver.try_recv() {
            // If None, node was deleted in the mean time.
            let Some(&node_id) = self.slot_to_node.get(response.node_id) else { continue };
            let node = self.node_graph.get_node_mut(node_id).expect("Didn't find node");
            if node.config_version.load(Ordering::SeqCst) == response.new_version {
                node.data_version = response.new_version;
                node.noise_range = Some((response.noise_min, response.noise_max));
                // TODO: set texture from response
            }
        }

        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                egui::widgets::global_theme_preference_buttons(ui);
                ui.separator();
                ui.add(egui::github_link_file!(
                    "https://github.com/EkardNT/noise-explorer/blob/main/",
                    "Source code."
                ));
                powered_by_egui_and_eframe(ui);
                egui::warn_if_debug_build(ui);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut node_graph = std::mem::take(&mut self.node_graph);
            let mut viewer = GraphNodeViewer {
                node_type_filter: &mut self.node_type_filter,
                node_type_filter_lowercase: &mut self.node_type_filter_lowercase,
                clear_graph: false,
                changed_nodes: &mut self.changed_nodes,
                slot_to_node: &mut self.slot_to_node,
            };
            node_graph.show(&mut viewer, &self.node_graph_style, "noise_graph", ui);
            if !viewer.clear_graph {
                self.node_graph = node_graph;
            }


            if !self.changed_nodes.is_empty() {
                // Build the set of dirty nodes by iterating over the changed nodes and adding both them and their linked dependencies.
                let connections: Bimultimap<NodeSlotKey, NodeSlotKey> = self.node_graph
                    .wires()
                    .flat_map(|(out_pin, in_pin)| {
                        let Some(out_node) = self.node_graph.get_node(out_pin.node) else { return None.into_iter() };
                        let Some(in_node) = self.node_graph.get_node(in_pin.node) else { return None.into_iter() };
                        Some((in_node.node_id_key, out_node.node_id_key)).into_iter()
                    })
                    .collect();
                fn add_dirty_tree(
                        node: NodeSlotKey,
                        connections: &Bimultimap<NodeSlotKey, NodeSlotKey>,
                        to: &mut HashSet<NodeSlotKey>) {
                    if !to.insert(node) {
                        return;
                    }
                    for &dependent_node in connections.get(&node) {
                        add_dirty_tree(dependent_node, connections, to);
                    }
                }
    
                let mut dirty_nodes: HashSet<NodeSlotKey> = HashSet::new();
                for changed_node in self.changed_nodes.drain() {
                    add_dirty_tree(changed_node, &connections, &mut dirty_nodes);
                }
                for dirty_node in dirty_nodes.drain() {
                    let &node_id = self.slot_to_node.get(dirty_node).expect("Didn't find node");
                    let node = self.node_graph.get_node_mut(node_id).expect("Didn't find node in graph");
                    let new_version = node.config_version.fetch_add(1, Ordering::SeqCst) + 1;
                    let _ = self.recalculate_sender.send(RecalculateRequest {
                        node_id: dirty_node,
                        new_version: new_version,
                        config_version: Arc::clone(&node.config_version),
                        noise_fn: DynNoise::new(noise::Constant::new(0.5)),
                        texture_height: 256,
                        texture_width: 256,
                        noise_width: 1.0,
                        noise_height: 1.0
                    });
                }
            }
            

            
        });
    }
}

fn powered_by_egui_and_eframe(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label("Powered by ");
        ui.hyperlink_to("egui", "https://github.com/emilk/egui");
        ui.label(" and ");
        ui.hyperlink_to(
            "eframe",
            "https://github.com/emilk/egui/tree/master/crates/eframe",
        );
        ui.label(".");
    });
}

#[derive(Serialize, Deserialize)]
pub struct GraphNode {
    node_id_key: NodeSlotKey,
    noise_type: NoiseType,
    config: NoiseConfig,
    data_version: usize,
    noise_range: Option<(f64, f64)>,
    config_version: Arc<AtomicUsize>,
}

struct GraphNodeViewer<'app> {
    node_type_filter: &'app mut String,
    node_type_filter_lowercase: &'app mut String,
    clear_graph: bool,
    changed_nodes: &'app mut HashSet<NodeSlotKey>,
    slot_to_node: &'app mut SlotMap<NodeSlotKey, NodeId>,
}

impl<'app> GraphNodeViewer<'app> {
    fn add_noise_button(&mut self, ui: &mut Ui, noise_type: &'static NoiseType, node_graph: &mut Snarl<GraphNode>, pos: Pos2) {
        let response = ui.button(noise_type.name());
    
        if response.clicked() {
            self.slot_to_node.insert_with_key(|key| {
                let node_id = node_graph.insert_node(pos, GraphNode {
                    node_id_key: key,
                    noise_type: *noise_type,
                    config: noise_type.config(),
                    data_version: 0,
                    noise_range: None,
                    config_version: Arc::new(AtomicUsize::new(0)),
                });
                self.changed_nodes.insert(key);
                node_id
            });
            ui.close_menu();
        }
    }
}

impl<'app> SnarlViewer<GraphNode> for GraphNodeViewer<'app> {
    fn title(&mut self, _: &GraphNode) -> String {
        unimplemented!("Should not be called")
    }

    fn inputs(&mut self, node: &GraphNode) -> usize {
        node.noise_type.input_count()
    }

    fn show_input(&mut self, pin: &egui_snarl::InPin, ui: &mut egui::Ui, scale: f32, snarl: &mut Snarl<GraphNode>)
        -> egui_snarl::ui::PinInfo {
        if let Some(node) = snarl.get_node(pin.id.node) {
            node.noise_type.show_input(pin.id.input, ui, scale);
            PinInfo::circle()
        } else {
            PinInfo::triangle()
        }
    }

    fn outputs(&mut self, _: &GraphNode) -> usize {
        1
    }

    fn show_output(
        &mut self,
        _pin: &egui_snarl::OutPin,
        ui: &mut egui::Ui,
        _scale: f32,
        _snarl: &mut Snarl<GraphNode>,
    ) -> egui_snarl::ui::PinInfo {
        ui.label("Output");
        PinInfo::circle()
    }

    fn show_header(
            &mut self,
            node: NodeId,
            _inputs: &[egui_snarl::InPin],
            _outputs: &[egui_snarl::OutPin],
            ui: &mut Ui,
            scale: f32,
            snarl: &mut Snarl<GraphNode>,
        ) {
        if let Some(graph_node) = snarl.get_node_mut(node) {
            let node_key = graph_node.node_id_key;
            match graph_node.noise_type.show_header(&mut graph_node.config, ui, scale) {
                noises::HeaderResponse::Remove => {
                    snarl.remove_node(node);
                    self.slot_to_node.remove(node_key);
                }
                noises::HeaderResponse::Changed => {
                    self.changed_nodes.insert(graph_node.node_id_key);
                },
                noises::HeaderResponse::None => {
                    /* Nothing to do */
                },
            };
        }
    }

    fn has_body(&mut self, _node: &GraphNode) -> bool {
        true
    }

    fn show_body(
            &mut self,
            node_id: NodeId,
            _inputs: &[egui_snarl::InPin],
            _outputs: &[egui_snarl::OutPin],
            ui: &mut Ui,
            scale: f32,
            snarl: &mut Snarl<GraphNode>,
        ) {
        let node = snarl.get_node_mut(node_id).unwrap();
        let changed = node.noise_type.show_body(&mut node.config, ui, scale);
        if changed {
            self.changed_nodes.insert(node.node_id_key);
        }
        static IMAGE: ImageSource<'static> = egui::include_image!("../assets/fbm.png");
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add(egui::Image::new(IMAGE.clone())
                .maintain_aspect_ratio(true)
                .fit_to_exact_size(Vec2::new(256.0, 256.0) * scale)
            );
            ui.horizontal(|ui| {
                ui.label(&format!("Data version: {}", node.data_version));
            });
            ui.horizontal(|ui| {
                ui.label(&format!("Config version: {}", node.config_version.load(Ordering::SeqCst)));
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                if let Some(range) = node.noise_range {
                    ui.label("Noise range: [");
                    ui.label(RichText::new(format!("{}", range.0)).color(if range.0 < -1.0 { Color32::RED } else { ui.style().visuals.text_color() }));
                    ui.label(", ");
                    ui.label(RichText::new(format!("{}", range.1)).color(if range.1 > 1.0 { Color32::RED } else { ui.style().visuals.text_color() }));
                    ui.label("]");
                }
            })
        });
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<GraphNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: Pos2, ui: &mut Ui, _scale: f32, snarl: &mut Snarl<GraphNode>) {
        ui.horizontal(|ui| {
            ui.label("Filter:");
            if ui.add(egui::TextEdit::singleline(self.node_type_filter)).changed() {
                *self.node_type_filter_lowercase = self.node_type_filter.to_lowercase();
            }
            if ui.button(" x ").clicked() {
                self.node_type_filter.clear();
            }
        });

        if self.node_type_filter.is_empty() {
            ui.menu_button("Sources", |ui| {
                for noise_type in NoiseType::sources() {
                    self.add_noise_button(ui, noise_type, snarl, pos);
                }
            });
            ui.menu_button("Combinators", |ui| {
                for noise_type in NoiseType::combinators() {
                    self.add_noise_button(ui, noise_type, snarl, pos);
                }
            });
        } else {
            let mut matches = 0;
            for noise_type in NoiseType::all() {
                if !noise_type.lowercase_name().contains(self.node_type_filter_lowercase as &_) {
                    continue;
                }
                matches += 1;
                self.add_noise_button(ui, noise_type, snarl, pos);
            }
            if matches == 0 {
                ui.centered_and_justified(|ui| {
                    ui.label("No matches");
                });
            }
        }

        ui.separator();
        if ui.button("Clear All").clicked() {
            self.clear_graph = true;
            ui.close_menu();
        }
    }

    fn connect(&mut self, from: &egui_snarl::OutPin, to: &egui_snarl::InPin, snarl: &mut Snarl<GraphNode>) {
        if from.id.node != to.id.node {
            snarl.connect(from.id, to.id);
            self.changed_nodes.insert(snarl.get_node(to.id.node).unwrap().node_id_key);
        }
    }

    fn disconnect(&mut self, from: &egui_snarl::OutPin, to: &egui_snarl::InPin, snarl: &mut Snarl<GraphNode>) {
        snarl.disconnect(from.id, to.id);
        if let Some(from_node) = snarl.get_node(from.id.node) {
            self.changed_nodes.insert(from_node.node_id_key);
        }
        if let Some(to_node) = snarl.get_node(to.id.node) {
            self.changed_nodes.insert(to_node.node_id_key);
        }
    }
}

struct RecalculateRequest {
    node_id: NodeSlotKey,
    new_version: usize,
    config_version: Arc<AtomicUsize>,
    texture_width: usize,
    texture_height: usize,
    noise_width: f64,
    noise_height: f64,
    noise_fn: DynNoise,
}

struct RecalculateResult {
    node_id: NodeSlotKey,
    new_version: usize,
    noise_max: f64,
    noise_min: f64,
    texture: () // TODO
}