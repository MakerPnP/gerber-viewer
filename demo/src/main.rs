use std::io::BufReader;

use eframe::emath::Rect;
use eframe::epaint::Color32;
use egui::ViewportBuilder;
use nalgebra::{Point2, Vector2, Vector3};
use gerber_viewer::gerber_parser::parse;
use gerber_viewer::{draw_arrow, draw_crosshair, draw_marker, draw_outline, GerberLayer, GerberRenderer, RenderConfiguration, ToPosition, UiState, ViewState};
use gerber_viewer::BoundingBox;
use gerber_viewer::GerberTransform;

const ENABLE_UNIQUE_SHAPE_COLORS: bool = true;
const ENABLE_VERTEX_NUMBERING: bool = false;
const ENABLE_SHAPE_NUMBERING: bool = false;
const ZOOM_FACTOR: f32 = 0.50;
const ROTATION_SPEED_DEG_PER_SEC: f32 = 45.0;
const INITIAL_ROTATION: f32 = 45.0_f32.to_radians();
const MIRRORING: [bool; 2] = [false, false];

// for mirroring and rotation
//const CENTER_OFFSET: Vector2<f64> = Vector2::new(0.0, 0.0);
const CENTER_OFFSET: Vector2<f64> = Vector2::new(15.0, 20.0);
//const CENTER_OFFSET: Vector2<f64> = Vector2::new(14.75, 6.0);

// in EDA tools like DipTrace, a gerber offset can be specified when exporting gerbers, e.g. 10,5.
// use negative offsets here to relocate the gerber back to 0,0, e.g. -10, -5
//const DESIGN_OFFSET: Vector2<f64> = Vector2::new(0.0, 0.0);
const DESIGN_OFFSET: Vector2<f64> = Vector2::new(-5.0, -10.0);
//const DESIGN_OFFSET: Vector2<f64> = Vector2::new(-10.0, -10.0);

// radius of the markers, in gerber coordinates
const MARKER_RADIUS: f32 = 2.5;

struct DemoApp {
    gerber_layer: GerberLayer,
    renderer_configuration: RenderConfiguration,
    view_state: ViewState,
    ui_state: UiState,
    needs_view_fitting: bool,
    transform: GerberTransform,
    last_frame_time: std::time::Instant,
}

impl DemoApp {
    pub fn new() -> Self {
        let demo_str = include_str!("../assets/demo.gbr").as_bytes();
        //let demo_str = include_str!("../assets/mirroring-rotation-scaling.gbr").as_bytes();
        //let demo_str = include_str!("../assets/aperture-block-simple.gbr").as_bytes();
        //let demo_str = include_str!("../assets/aperture-block-nested.gbr").as_bytes();
        //let demo_str = include_str!("../assets/aperture-block-reference.gbr").as_bytes();
        //let demo_str = include_str!("../assets/vector-font.gbr").as_bytes();
        //let demo_str = include_str!("../assets/diptrace-outline-test-1/BoardOutline.gbr").as_bytes();
        //let demo_str = include_str!("../assets/diptrace-font-test-1/TopAssembly.gbr").as_bytes();
        //let demo_str = include_str!("../assets/diptrace-region-test-1.gbr").as_bytes();
        //let demo_str = include_str!("../assets/rectangles.gbr").as_bytes();
        //let demo_str = include_str!("../assets/region-non-overlapping-contours.gbr").as_bytes();
        //let demo_str = include_str!("../assets/easyeda-unclosed-region-test-1.gbr").as_bytes();
        //let demo_str = include_str!("../assets/arcs.gbr").as_bytes();
        //let demo_str = include_str!("../assets/arcs-90.gbr").as_bytes();
        //let demo_str = include_str!("../assets/macro-centerline.gbr").as_bytes();
        //let demo_str = include_str!("../assets/macro-vectorline.gbr").as_bytes();
        //let demo_str = include_str!("../assets/macro-rounded-rectangle.gbr").as_bytes();
        //let demo_str = include_str!("../assets/macro-polygons.gbr").as_bytes();
        //let demo_str = include_str!("../assets/macro-polygons-concave.gbr").as_bytes();
        //let demo_str = include_str!("../assets/step-repeat.gbr").as_bytes();

        //let demo_str = include_str!(r#"D:\Users\Hydra\Documents\DipTrace\Projects\SPRacingRXN1\Export\SPRacingRXN1-RevB-20240507-1510_gerberx2\TopSilk.gbr"#).as_bytes();


        let reader = BufReader::new(demo_str);

        let doc = parse(reader).unwrap();
        let commands = doc.into_commands();

        let gerber_layer = GerberLayer::new(commands);

        let renderer_config = RenderConfiguration {
            use_unique_shape_colors: ENABLE_UNIQUE_SHAPE_COLORS,
            use_shape_numbering: ENABLE_SHAPE_NUMBERING,
            use_vertex_numbering: ENABLE_VERTEX_NUMBERING,

            // use the default for any remaining options, doing this makes adding options easier in the future.
            .. RenderConfiguration::default()
        };

        let origin = CENTER_OFFSET - DESIGN_OFFSET;

        let transform = GerberTransform {
            rotation: INITIAL_ROTATION,
            mirroring: MIRRORING.into(),
            origin,
            offset: DESIGN_OFFSET,
            ..GerberTransform::default()
        };

        Self {
            gerber_layer,
            renderer_configuration: renderer_config,
            view_state: Default::default(),
            ui_state: Default::default(),
            needs_view_fitting: true,
            transform,
            last_frame_time: std::time::Instant::now(),
        }
    }

    fn fit_view(&mut self, viewport: Rect) {
        let bbox = self.gerber_layer.bounding_box();
        self.view_state.fit_view(viewport, bbox, ZOOM_FACTOR);
        self.needs_view_fitting = false;
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        //
        // Animate the gerber view by rotating it.
        //
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;

        let rotation_increment = ROTATION_SPEED_DEG_PER_SEC.to_radians() * delta;
        self.transform.rotation += rotation_increment;

        if ROTATION_SPEED_DEG_PER_SEC > 0.0 {
            // force the UI to refresh every frame for a smooth animation
            ctx.request_repaint();
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

        //
        // Build a UI
        //

        egui::TopBottomPanel::top("top_panel")
            .resizable(true)
            .show(ctx, |ui| {
            ui.heading("Gerber Viewer Demo");
            ui.label("by Dominic Clifton (2025)");

            ui.label("Pan by using left-mouse button + drag, zoom using scroll wheel.");

            ui.label(format!("coords: {:?}", self.ui_state.cursor_gerber_coords));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
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

                GerberRenderer::default().paint_layer(
                    &painter,
                    self.view_state,
                    &self.gerber_layer,
                    Color32::WHITE,
                    &self.renderer_configuration,
                    &self.transform,
                );
                
                // if you want to display multiple layers, call `paint_layer` for each layer. 

                draw_outline(&painter, bbox_vertices_screen, Color32::RED);
                draw_outline(&painter, outline_vertices_screen, Color32::GREEN);

                let screen_radius = MARKER_RADIUS * self.view_state.scale;

                let design_offset_screen_position = self.view_state.gerber_to_screen_coords(DESIGN_OFFSET.to_position());
                draw_arrow(&painter, design_offset_screen_position, self.ui_state.origin_screen_pos, Color32::ORANGE);
                draw_marker(&painter, design_offset_screen_position, Color32::ORANGE, Color32::YELLOW, screen_radius);

                let design_origin_screen_position = self.view_state.gerber_to_screen_coords((CENTER_OFFSET - DESIGN_OFFSET).to_position());
                draw_marker(&painter, design_origin_screen_position, Color32::PURPLE, Color32::MAGENTA, screen_radius);
            });
        });
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
