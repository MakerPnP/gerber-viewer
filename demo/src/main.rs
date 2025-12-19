use std::collections::HashMap;
use std::io::BufReader;
use std::time::Instant;
use eframe::emath::Rect;
use eframe::epaint::Color32;
use egui::{Frame, ViewportBuilder};
use nalgebra::{Point2, Vector2, Vector3};
use gerber_viewer::gerber_parser::parse;
use gerber_viewer::{draw_arrow, draw_crosshair, draw_marker, draw_outline, GerberLayer, GerberRenderer, RenderConfiguration, ToPosition, UiState, ViewState};
use gerber_viewer::BoundingBox;
use gerber_viewer::GerberTransform;

#[derive(Clone, Copy, Debug)]
struct Settings {
    use_unique_shape_colors: bool,
    use_vertex_numbering: bool,
    use_shape_numbering: bool,
    use_shape_bboxes: bool,
    zoom_factor: f32,
    rotation_speed_deg_per_sec: f32,
    initial_rotation: f32,
    mirroring: [bool; 2],

    /// for mirroring and rotation
    center_offset: Vector2<f64>,

    /// in EDA tools like DipTrace, a gerber offset can be specified when exporting gerbers, e.g. 10,5.
    /// use negative offsets here to relocate the gerber back to 0,0, e.g. -10, -5
    design_offset: Vector2<f64>,

    /// scaling is more important when rendering multiple layers where each layer needs a different scaling.
    default_scale: f64,

    /// radius of the markers, in gerber coordinates
    marker_radius: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            use_unique_shape_colors: true,
            use_vertex_numbering: false,
            use_shape_numbering: false,
            use_shape_bboxes: false,
            zoom_factor: 1.0,
            rotation_speed_deg_per_sec: 0.0,
            initial_rotation: 0.0_f32.to_radians(),
            mirroring: [false, false],
            center_offset: Vector2::new(0.0, 0.0),
            design_offset: Vector2::new(0.0, 0.0),
            default_scale: 1.0,

            // radius of the markers, in gerber coordinates (assuming Millimeters here)
            marker_radius: 2.5,
        }
    }
}

impl Settings {
    fn primary_demo_settings() -> Self {
        Self {
            use_unique_shape_colors: true,
            zoom_factor: 0.50,
            rotation_speed_deg_per_sec: 45.0,
            initial_rotation: 45.0_f32.to_radians(),
            center_offset: Vector2::new(15.0, 20.0),
            design_offset: Vector2::new(-5.0, -10.0),

            // we use a scale greater than 1.0 to ensure that scaling is applied correctly.
            // this has no effect on the view in this demo app, since the view is scaled to the gerber content.
            // scaling is more important when rendering multiple layers where each layer needs a different scaling.
            // this can be useful if one gerber layer is in MM and the other is in inches.
            default_scale: 2.0,

            ..Default::default()
        }
    }

    #[allow(dead_code)]
    fn local_file_settings() -> Self {
        Self {
            center_offset: Vector2::new(14.75, 6.0),
            design_offset: Vector2::new(-10.0, -10.0),

            ..Default::default()
        }
    }
}

struct GerberViewerInstance {
    settings: Settings,

    gerber_layer: GerberLayer,
    renderer_configuration: RenderConfiguration,
    view_state: ViewState,
    ui_state: UiState,
    needs_view_fitting: bool,
    transform: GerberTransform,
}

impl GerberViewerInstance {
    fn new(demo: &Demo) -> Self {
        // take a copy of the settings, so that we can modify them without affecting the original.
        let settings = demo.initial_settings.clone();

        let gerber_layer = Self::build_layer(&demo.source);
        
        //
        // setup a renderer
        //
        let renderer_config = RenderConfiguration {
            use_unique_shape_colors: settings.use_unique_shape_colors,
            use_shape_numbering: settings.use_shape_numbering,
            use_shape_bboxes: settings.use_shape_bboxes,
            use_vertex_numbering: settings.use_vertex_numbering,

            // use the default for any remaining options, doing this makes adding options easier in the future.
            .. RenderConfiguration::default()
        };

        let origin = settings.center_offset - settings.design_offset;

        let transform = GerberTransform {
            rotation: settings.initial_rotation,
            mirroring: settings.mirroring.into(),
            origin,
            offset: settings.design_offset,
            scale: settings.default_scale,
            ..GerberTransform::default()
        };

        Self {
            settings,
            gerber_layer,
            renderer_configuration: renderer_config,
            view_state: Default::default(),
            ui_state: Default::default(),
            needs_view_fitting: true,
            transform,
        }
    }
    
    fn build_layer(source: &str) -> GerberLayer {
        //
        // parse the gerber file
        //
        let reader = BufReader::new(source.as_bytes());
        let doc = parse(reader).unwrap();

        //
        // build a layer
        //
        let commands = doc.into_commands();
        GerberLayer::new(commands)
    }

    fn reparse(&mut self, source: &str) {
        let gerber_layer = Self::build_layer(source);
        self.gerber_layer = gerber_layer;
        self.needs_view_fitting = true;
    }

    fn fit_view(&mut self, viewport: Rect) {
        let layer_bbox = self.gerber_layer.bounding_box();

        let image_transform_matrix = self.gerber_layer.image_transform().to_matrix();
        let layer_matrix = self.transform.to_matrix();

        let matrix = image_transform_matrix * layer_matrix;

        let layer_bbox = layer_bbox.apply_transform_matrix(&matrix);


        self.view_state.fit_view(viewport, &layer_bbox, self.settings.zoom_factor);
        self.needs_view_fitting = false;
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame_delta: f32) {
        egui::TopBottomPanel::bottom(ui.id().with("bottom_panel"))
            .show_inside(ui, |ui| {
                ui.label(format!("Coordinates: {:?}", self.ui_state.cursor_gerber_coords));
            });

        egui::CentralPanel::default()
            .frame(Frame::new())
            .show_inside(ui, |ui|{
                //
                // Animate the gerber view by rotating it.
                //

                let rotation_increment = self.settings.rotation_speed_deg_per_sec.to_radians() * frame_delta;
                self.transform.rotation += rotation_increment;

                if self.settings.rotation_speed_deg_per_sec > 0.0 {
                    // force the UI to refresh every frame for a smooth animation
                    ui.ctx().request_repaint();
                }

                //
                // Compute bounding box and outline
                //

                let bbox = self.gerber_layer.bounding_box();
                let image_transform_matrix = self.gerber_layer.image_transform().to_matrix();
                let render_transform_matrix = self.transform.to_matrix();

                let matrix = image_transform_matrix * render_transform_matrix;

                // Compute rotated outline (GREEN)
                let outline_vertices: Vec<_> = bbox
                    .vertices()
                    .into_iter()
                    .map(|v| {
                        // Convert to homogeneous coordinates
                        let point_vec = Vector3::new(v.x, v.y, 1.0);

                        let transformed = matrix * point_vec;
                        Point2::new(transformed.x, transformed.y)
                    })
                    .collect();

                // Compute transformed AABB (RED)
                let bbox = BoundingBox::from_points(&outline_vertices);

                // Convert to screen coords
                let bbox_vertices_screen = bbox.vertices().into_iter()
                    .map(|v| self.view_state.gerber_to_screen_coords(v))
                    .collect::<Vec<_>>();

                let outline_vertices_screen = outline_vertices.into_iter()
                    .map(|v| self.view_state.gerber_to_screen_coords(v))
                    .collect::<Vec<_>>();

                let response = ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::drag());
                let viewport = response.rect;

                if self.needs_view_fitting {
                    self.fit_view(viewport)
                }

                //
                // handle pan, drag and cursor position
                //
                self.ui_state.update(ui, &viewport, &response, &mut self.view_state);

                //
                // Show the gerber layer and other overlays
                //

                let painter = ui.painter().with_clip_rect(viewport);

                draw_crosshair(&painter, self.ui_state.origin_screen_pos, Color32::BLUE);
                draw_crosshair(&painter, self.ui_state.center_screen_pos, Color32::LIGHT_GRAY);

                GerberRenderer::new(
                    &self.renderer_configuration,
                    self.view_state,
                    &self.transform,
                    &self.gerber_layer,
                ).paint_layer(
                    &painter,
                    Color32::WHITE,
                );

                // if you want to display multiple layers, call `paint_layer` for each layer.

                draw_outline(&painter, bbox_vertices_screen, Color32::RED);
                draw_outline(&painter, outline_vertices_screen, Color32::GREEN);

                let screen_radius = self.settings.marker_radius * self.view_state.scale;

                let design_offset_screen_position = self.view_state.gerber_to_screen_coords(self.settings.design_offset.to_position());
                draw_arrow(&painter, design_offset_screen_position, self.ui_state.origin_screen_pos, Color32::ORANGE);
                draw_marker(&painter, design_offset_screen_position, Color32::ORANGE, Color32::YELLOW, screen_radius);

                let design_origin_screen_position = self.view_state.gerber_to_screen_coords((self.settings.center_offset - self.settings.design_offset).to_position());
                draw_marker(&painter, design_origin_screen_position, Color32::PURPLE, Color32::MAGENTA, screen_radius);
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DemoKind {
    Primary,
    MirroringRotationScaling,
    ApertureBlockSimple,
    ApertureBlockNested,
    ApertureBlockReference,
    VectorFont,
    DiptraceOutlineTest1,
    DiptraceFontTest1,
    DiptraceRegionTest1,
    Rectangles,
    RegionNonOverlappingContours,
    EasyEdaUnclosedRegionTest1,
    Arcs,
    MacroCenterLine,
    MacroVectorLine,
    MacroRoundedRectangle,
    MacroPolygons,
    MacroPolygonsConcave,
    StepRepeat,
    #[allow(dead_code)]
    LocalFile,
    Playground,
}

struct Demo {
    kind: DemoKind,
    name: &'static str,
    source: String,
    initial_settings: Settings,
    
    reparse_requested: bool,
}

impl Demo {
    pub fn new(kind: DemoKind, name: &'static str, initial_source: &'static str, initial_settings: Settings) -> Self {
        Self {
            kind,
            name,
            source: initial_source.to_string(),
            initial_settings,
            reparse_requested: false,
        }
    }
    
    pub fn request_reparse(&mut self) {
        self.reparse_requested = true;
    }
}

struct DemoApp {
    demos: Vec<Demo>,
    instances: HashMap<DemoKind, GerberViewerInstance>,

    last_frame_time: Instant,
    focussed_demo_kind: Option<DemoKind>,
}

impl DemoApp {

    pub fn new() -> Self {
        let demos = vec![
            Demo::new(DemoKind::Primary, "Primary demo", include_str!("../assets/demo.gbr"), Settings::primary_demo_settings()),
            Demo::new(DemoKind::Playground, "Playground", include_str!("../assets/playground.gbr"), Default::default()),
            Demo::new(DemoKind::ApertureBlockSimple, "Aperture Block - Simple", include_str!("../assets/aperture-block-simple.gbr"), Default::default()),
            Demo::new(DemoKind::ApertureBlockNested, "Aperture Block - Nested", include_str!("../assets/aperture-block-nested.gbr"), Default::default()),
            Demo::new(DemoKind::ApertureBlockReference, "Aperture Block - Reference", include_str!("../assets/aperture-block-reference.gbr"), Default::default()),
            Demo::new(DemoKind::VectorFont, "Vector Font", include_str!("../assets/vector-font.gbr"), Default::default()),
            Demo::new(DemoKind::Rectangles, "Rectangles", include_str!("../assets/rectangles.gbr"), Default::default()),
            Demo::new(DemoKind::RegionNonOverlappingContours, "Region - Non-overlapping Contours", include_str!("../assets/region-non-overlapping-contours.gbr"), Default::default()),
            Demo::new(DemoKind::Arcs, "Arcs", include_str!("../assets/arcs.gbr"), Default::default()),
            Demo::new(DemoKind::MacroCenterLine, "Macro - Center-line", include_str!("../assets/macro-centerline.gbr"), Default::default()),
            Demo::new(DemoKind::MacroVectorLine, "Macro - Vector-line", include_str!("../assets/macro-vectorline.gbr"), Default::default()),
            Demo::new(DemoKind::MacroRoundedRectangle, "Macro - Rounded Rectangle", include_str!("../assets/macro-rounded-rectangle.gbr"), Default::default()),
            Demo::new(DemoKind::MacroPolygons, "Macro - Polygons", include_str!("../assets/macro-polygons.gbr"), Default::default()),
            Demo::new(DemoKind::MacroPolygonsConcave, "Macro - Polygons (Concave)", include_str!("../assets/macro-polygons-concave.gbr"), Default::default()),
            Demo::new(DemoKind::StepRepeat, "Step Repeat", include_str!("../assets/step-repeat.gbr"), Default::default()),
            Demo::new(DemoKind::MirroringRotationScaling, "Mirroring rotation and scaling", include_str!("../assets/mirroring-rotation-scaling.gbr"), Default::default()),
            Demo::new(DemoKind::DiptraceOutlineTest1, "Diptrace - Outline Test 1", include_str!("../assets/diptrace-outline-test-1/BoardOutline.gbr"), Default::default()),
            Demo::new(DemoKind::DiptraceFontTest1, "Diptrace - Font Test 1", include_str!("../assets/diptrace-font-test-1/TopAssembly.gbr"), Default::default()),
            Demo::new(DemoKind::DiptraceRegionTest1, "Diptrace - Region Test 1", include_str!("../assets/diptrace-region-test-1.gbr"), Default::default()),
            Demo::new(DemoKind::EasyEdaUnclosedRegionTest1, "EasyEDA - Unclosed Region Test 1", include_str!("../assets/easyeda-unclosed-region-test-1.gbr"), Default::default()),
            Demo::new(DemoKind::LocalFile, "LocalFile", include_str!(r#"D:\Users\Hydra\Documents\DipTrace\Projects\SPRacingRXN1\Export\SPRacingRXN1-RevB-20240507-1510_gerberx2\TopSilk.gbr"#), Settings::local_file_settings()),
        ];
        
        let initial_demo_kind = DemoKind::Primary;
        
        let initial_demo = demos.iter().find(|demo| demo.kind == initial_demo_kind).unwrap();
        let instance = GerberViewerInstance::new(&initial_demo);

        let mut instances = HashMap::new();
        instances.insert(initial_demo_kind, instance);

        Self {
            demos,
            instances,
            last_frame_time: Instant::now(),
            focussed_demo_kind: None,
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        let now = Instant::now();
        let frame_delta = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        
        //
        // Build a UI
        //

        egui::TopBottomPanel::top("top_panel")
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.heading("Gerber Viewer Demo");
                    ui.label("by Dominic Clifton (2025)");
                });
        });

        egui::SidePanel::left("left_panel")
            .show(ctx, |ui| {
                ui.heading("Available demos");
                ui.separator();
                ui.vertical(|ui| {
                    for demo in &self.demos {
                        let mut is_open = self.instances.contains_key(&demo.kind);
                        if ui.toggle_value(&mut is_open, demo.name).changed() {
                            if is_open {
                                self.instances.insert(demo.kind, GerberViewerInstance::new(&demo));
                            } else {
                                self.instances.remove(&demo.kind);
                            }
                        }
                    }
                });
                ui.separator();
                if ui.button("Organize windows").clicked() {
                    ui.ctx().memory_mut(|mem| mem.reset_areas());
                }
                ui.separator();
                ui.label("Pan gerbers by using left-mouse button + drag, zoom using scroll wheel.");
            });

        if let Some(kind) = self.focussed_demo_kind {

            let demo = self.demos.iter_mut().find(|candidate|candidate.kind == kind).unwrap();

            egui::SidePanel::right("right_panel")
                .default_width(320.0)
                .min_width(200.0)
                .resizable(true)
                .show(ctx, |ui| {
                    egui::Sides::new()
                        .show(ui,|ui|{
                            ui.heading(demo.name.to_string());
                        }, |ui| {
                            let button = ui.button("Apply changes");
                            if button.hovered() {
                                button.show_tooltip_text("If there is a panic, please submit a bug report to the 'gerber-parser' repository.");
                            }

                            if button.clicked() {
                                demo.request_reparse();
                            }
                        });
                    
                    ui.separator();

                    egui::ScrollArea::both()
                        .max_width(ui.available_size_before_wrap().x)
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut demo.source)
                                    .desired_width(ui.available_size_before_wrap().x)
                            )
                        });
                });
        }

        let mut central_panel_rect = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                central_panel_rect = Some(ui.clip_rect());
            });
        });

        let central_panel_rect = central_panel_rect.unwrap();
        let mut close_list = vec![];
        for (kind, instance) in self.instances.iter_mut() {

            let title = self.demos.iter().find(|candidate| candidate.kind == *kind).unwrap().name;

            let mut open = true;
            egui::Window::new(title)
                .default_size([400.0, 400.0])
                .open(&mut open)
                .resizable(true)
                .scroll(true)
                .constrain_to(central_panel_rect)
                .show(ctx, |ui|{

                    let on_top = Some(ui.layer_id()) == ui.ctx().top_layer_id();

                    if on_top {
                        let changed = match self.focussed_demo_kind {
                            Some(focussed_demo_kind) => focussed_demo_kind != *kind,
                            None => true,
                        };

                        if changed {
                            self.focussed_demo_kind = Some(*kind);
                        }
                    }

                    instance.ui(ui, frame_delta);
                });
            
            if !open {
                close_list.push(*kind);
            }
        }
        
        // handle the windows that have been closed this frame
        for kind in close_list.into_iter() {
            self.instances.remove(&kind);
        }

        // handle no windows open
        if self.instances.is_empty() {
            self.focussed_demo_kind = None
        }

        // handle the window having been closed by the demo list or by it's close button
        if let Some(kind) = &self.focussed_demo_kind {
            if !self.instances.contains_key(kind) {
                self.focussed_demo_kind = None;
            }
        }
        
        // handle any reparse requests
        for demo in &mut self.demos {
            if demo.reparse_requested {
                demo.reparse_requested = false;
                if let Some(instance) = self.instances.get_mut(&demo.kind) {
                    instance.reparse(&demo.source);
                    ctx.request_repaint();
                }
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    init();
    eframe::run_native(
        "Gerber Viewer Demo (egui)",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::new(DemoApp::new()))),
    )
}

pub fn init() {
    env_logger::init(); // Log to stderr (optional).

    #[cfg(feature = "profile-with-puffin")]
    {
        start_puffin_server();
    }
}

#[cfg(feature = "profile-with-puffin")]
fn start_puffin_server() {
    use tracing::{error, info};

    profiling::puffin::set_scopes_on(true); // tell puffin to collect data

    match puffin_http::Server::new("127.0.0.1:8585") {
        Ok(puffin_server) => {
            info!("Run:  cargo install puffin_viewer && puffin_viewer --url 127.0.0.1:8585");

            std::process::Command::new("puffin_viewer")
                .arg("--url")
                .arg("127.0.0.1:8585")
                .spawn()
                .ok();

            // We can store the server if we want, but in this case we just want
            // it to keep running. Dropping it closes the server, so let's not drop it!
            #[allow(clippy::mem_forget)]
            std::mem::forget(puffin_server);
        }
        Err(err) => {
            error!("Failed to start puffin server: {err}");
        }
    };
}
