use std::sync::Arc;

use egui::Painter;
use egui::epaint::emath::Align2;
use egui::epaint::{
    Color32, ColorMode, FontId, Mesh, PathShape, PathStroke, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, Vertex,
};
use nalgebra::{Matrix3, Point2, Vector2};

use crate::geometry::{GerberTransform, Matrix3Pos2Ext, Matrix3TransformExt};
use crate::layer::GerberPrimitive;
use crate::{
    ArcGerberPrimitive, CircleGerberPrimitive, LineGerberPrimitive, Matrix3ScalingExt, PolygonGerberPrimitive,
    RectangleGerberPrimitive, WithBoundingBox,
};
use crate::{GerberLayer, ViewState, color};

macro_rules! draw_bbox {
    ($primitive:ident, $configuration:ident, $painter:ident, $color:ident, $view:ident, $transform_matrix:ident) => {
        #[cfg(feature = "egui")]
        if $configuration.use_shape_bboxes {
            let bbox_rect: Rect = $primitive.bounding_box().into();

            let center = bbox_rect.center();
            let screen_center = Pos2::new(center.x, -center.y);
            let hw = bbox_rect.width() / 2.0;
            let hh = bbox_rect.height() / 2.0;

            // Define corners in local space (centered)
            let corners = [
                Pos2::new(-hw, -hh),
                Pos2::new(hw, -hh),
                Pos2::new(hw, hh),
                Pos2::new(-hw, hh),
            ];

            let points: Vec<Pos2> = corners
                .iter()
                .map(|corner| {
                    ($view.translation
                        + $transform_matrix.transform_pos2(screen_center + (*corner).to_vec2()) * $view.scale)
                        .to_pos2()
                })
                .collect();

            $painter.add(Shape::Path(PathShape {
                points,
                closed: true,
                fill: Color32::TRANSPARENT,
                stroke: PathStroke {
                    width: 1.0,
                    color: ColorMode::Solid($color),
                    kind: StrokeKind::Middle,
                },
            }));
        }
    };
}

#[derive(Debug, Clone)]
pub struct RenderConfiguration {
    /// Gives each shape a unique color.
    pub use_unique_shape_colors: bool,
    /// Draws the shape number in the center of the shape.
    pub use_shape_numbering: bool,
    /// Draws the vertex number at the start of each line.
    pub use_vertex_numbering: bool,
    /// Draws a bounding box for each shape,
    pub use_shape_bboxes: bool,
}

impl Default for RenderConfiguration {
    fn default() -> Self {
        Self {
            use_unique_shape_colors: false,
            use_shape_numbering: false,
            use_vertex_numbering: false,
            use_shape_bboxes: false,
        }
    }
}

pub struct GerberRenderer<'a> {
    configuration: &'a RenderConfiguration,
    view: ViewState,

    layer: &'a GerberLayer,

    transform_matrix: Matrix3<f64>,
    transform_scaling: Vector2<f64>,
}

impl<'a> GerberRenderer<'a> {
    pub fn new(
        configuration: &'a RenderConfiguration,
        view: ViewState,
        transform: &GerberTransform,
        layer: &'a GerberLayer,
    ) -> Self {
        let render_transform_matrix = transform.to_matrix();
        let image_transform_matrix = layer.image_transform().to_matrix();

        let transform_matrix = image_transform_matrix * render_transform_matrix;

        // computing the transform_scaling from the matrix is expensive, so we cache it
        let transform_scaling = transform_matrix.get_scaling_factors();

        Self {
            configuration,
            view,
            layer,
            transform_matrix,
            transform_scaling,
        }
    }

    /// converts gerber to screen coordinates, using the renderer transforms.
    /// coordinates are in gerber units.
    pub fn gerber_to_screen_coordinates(&self, position: &Point2<f64>) -> Pos2 {
        let position = Pos2::new(position.x as f32, -(position.y as f32));

        (self.view.translation
            + self
                .transform_matrix
                .transform_pos2(position)
                * self.view.scale)
            .to_pos2()
    }

    #[profiling::function]
    pub fn paint_layer(&self, painter: &egui::Painter, base_color: Color32) {
        for (index, primitive) in self
            .layer
            .primitives()
            .iter()
            .enumerate()
        {
            let color = match self
                .configuration
                .use_unique_shape_colors
            {
                true => color::generate_pastel_color(index as u64),
                false => base_color,
            };

            let shape_number = match self.configuration.use_shape_numbering {
                true => Some(index),
                false => None,
            };

            match primitive {
                GerberPrimitive::Circle(circle) => circle.render(
                    painter,
                    &self.view,
                    &self.transform_matrix,
                    &self.transform_scaling,
                    color,
                    shape_number,
                    self.configuration,
                ),
                GerberPrimitive::Rectangle(rect) => rect.render(
                    painter,
                    &self.view,
                    &self.transform_matrix,
                    &self.transform_scaling,
                    color,
                    shape_number,
                    self.configuration,
                ),
                GerberPrimitive::Line(line) => line.render(
                    painter,
                    &self.view,
                    &self.transform_matrix,
                    &self.transform_scaling,
                    color,
                    shape_number,
                    self.configuration,
                ),
                GerberPrimitive::Arc(arc) => arc.render(
                    painter,
                    &self.view,
                    &self.transform_matrix,
                    &self.transform_scaling,
                    color,
                    shape_number,
                    self.configuration,
                ),
                GerberPrimitive::Polygon(polygon) => polygon.render(
                    painter,
                    &self.view,
                    &self.transform_matrix,
                    &self.transform_scaling,
                    color,
                    shape_number,
                    self.configuration,
                ),
            }
        }
    }
}

trait Renderable {
    fn render(
        &self,
        painter: &Painter,
        view: &ViewState,
        transform_matrix: &Matrix3<f64>,
        transform_scaling: &Vector2<f64>,
        color: Color32,
        shape_number: Option<usize>,
        configuration: &RenderConfiguration,
    );
}

impl Renderable for CircleGerberPrimitive {
    #[cfg_attr(feature = "profile-renderables", profiling::function)]
    fn render(
        &self,
        painter: &Painter,
        view: &ViewState,
        transform_matrix: &Matrix3<f64>,
        transform_scaling: &Vector2<f64>,
        color: Color32,
        shape_number: Option<usize>,
        configuration: &RenderConfiguration,
    ) {
        let Self {
            center,
            diameter,
            exposure,
        } = self;

        let color = exposure.to_color(&color);

        let screen_center = Pos2::new(center.x as f32, -(center.y as f32));

        let center = view.translation.to_pos2() + transform_matrix.transform_pos2(screen_center) * view.scale;

        let diameter = *diameter * transform_scaling.x;

        let radius = (diameter as f32 / 2.0) * view.scale;
        #[cfg(feature = "egui")]
        painter.circle(center, radius, color, Stroke::NONE);

        draw_bbox!(self, configuration, painter, color, view, transform_matrix);
        draw_shape_number(
            painter,
            view,
            transform_matrix,
            ShapeNumberPosition::Transformed(center),
            shape_number,
        );
    }
}

impl Renderable for RectangleGerberPrimitive {
    #[cfg_attr(feature = "profile-renderables", profiling::function)]
    fn render(
        &self,
        painter: &Painter,
        view: &ViewState,
        transform_matrix: &Matrix3<f64>,
        transform_scaling: &Vector2<f64>,
        color: Color32,
        shape_number: Option<usize>,
        configuration: &RenderConfiguration,
    ) {
        let Self {
            origin,
            width,
            height,
            exposure,
        } = self;

        let color = exposure.to_color(&color);

        // Calculate center-based position
        let screen_center = Pos2::new(
            origin.x as f32 + *width as f32 / 2.0,     // Add half width to get center
            -(origin.y as f32 + *height as f32 / 2.0), // Flip Y and add half height
        );
        let center = (view.translation + transform_matrix.transform_pos2(screen_center) * view.scale).to_pos2();

        let is_axis_aligned = transform_matrix.is_axis_aligned();

        if is_axis_aligned {
            // Fast-path: axis-aligned rectangle (mirroring allowed, since mirroring across axis doesn't affect axis-alignment)
            // Determine if width/height should be swapped
            let mut width = *width as f32;
            let mut height = *height as f32;

            let should_swap = transform_matrix.is_90_or_270_rotation();
            if should_swap {
                std::mem::swap(&mut width, &mut height);
            }

            width *= transform_scaling.x as f32;
            height *= transform_scaling.y as f32;

            let size = Vec2::new(width, height) * view.scale;

            let top_left = center - size / 2.0; // Calculate top-left from center

            painter.rect(
                Rect::from_min_size(top_left, size),
                0.0,
                color,
                Stroke::NONE,
                StrokeKind::Middle,
            );
        } else {
            // Arbitrary rotation: draw as polygon
            let hw = *width as f32 / 2.0;
            let hh = *height as f32 / 2.0;

            // Define corners in local space (centered)
            let corners = [
                Pos2::new(-hw, -hh),
                Pos2::new(hw, -hh),
                Pos2::new(hw, hh),
                Pos2::new(-hw, hh),
            ];

            let screen_corners: Vec<Pos2> = corners
                .iter()
                .map(|corner| {
                    (view.translation
                        + transform_matrix.transform_pos2(screen_center + (*corner).to_vec2()) * view.scale)
                        .to_pos2()
                })
                .collect();

            painter.add(Shape::convex_polygon(screen_corners, color, Stroke::NONE));
        }

        draw_bbox!(self, configuration, painter, color, view, transform_matrix);
        draw_shape_number(
            painter,
            view,
            transform_matrix,
            ShapeNumberPosition::Transformed(center),
            shape_number,
        );
    }
}

impl Renderable for LineGerberPrimitive {
    #[cfg_attr(feature = "profile-renderables", profiling::function)]
    fn render(
        &self,
        painter: &Painter,
        view: &ViewState,
        transform_matrix: &Matrix3<f64>,
        _transform_scaling: &Vector2<f64>,
        color: Color32,
        shape_number: Option<usize>,
        configuration: &RenderConfiguration,
    ) {
        let Self {
            start,
            end,
            width,
            exposure,
        } = self;
        let color = exposure.to_color(&color);

        let start_position = Pos2::new(start.x as f32, -(start.y as f32));
        let end_position = Pos2::new(end.x as f32, -(end.y as f32));

        let transformed_start_position =
            (view.translation + transform_matrix.transform_pos2(start_position) * view.scale).to_pos2();
        let transformed_end_position =
            (view.translation + transform_matrix.transform_pos2(end_position) * view.scale).to_pos2();

        painter.line_segment(
            [transformed_start_position, transformed_end_position],
            Stroke::new((*width as f32) * view.scale, color),
        );
        // Draw circles at either end of the line.
        let radius = (*width as f32 / 2.0) * view.scale;
        painter.circle(transformed_start_position, radius, color, Stroke::NONE);
        painter.circle(transformed_end_position, radius, color, Stroke::NONE);

        draw_bbox!(self, configuration, painter, color, view, transform_matrix);

        if shape_number.is_some() {
            let screen_center = (transformed_start_position + transformed_end_position.to_vec2()) / 2.0;
            draw_shape_number(
                painter,
                view,
                transform_matrix,
                ShapeNumberPosition::Transformed(screen_center),
                shape_number,
            );
        }
    }
}

impl Renderable for ArcGerberPrimitive {
    #[cfg_attr(feature = "profile-renderables", profiling::function)]
    fn render(
        &self,
        painter: &Painter,
        view: &ViewState,
        transform_matrix: &Matrix3<f64>,
        _transform_scaling: &Vector2<f64>,
        color: Color32,
        shape_number: Option<usize>,
        configuration: &RenderConfiguration,
    ) {
        let Self {
            center,
            width,
            exposure,
            ..
        } = self;
        let color = exposure.to_color(&color);
        let screen_center = Pos2::new(center.x as f32, -(center.y as f32));

        let points = self
            .generate_points()
            .iter()
            .map(|p| {
                let local = Vec2::new(p.x as f32, -p.y as f32);
                let position =
                    (view.translation + transform_matrix.transform_pos2(screen_center + local) * view.scale).to_pos2();
                position
            })
            .collect::<Vec<_>>();

        let steps = points.len();

        let center_point = points[steps / 2];

        painter.add(Shape::Path(PathShape {
            points,
            closed: self.is_full_circle(),
            fill: Color32::TRANSPARENT,
            stroke: PathStroke {
                width: *width as f32 * view.scale,
                color: ColorMode::Solid(color),
                kind: StrokeKind::Middle,
            },
        }));

        draw_bbox!(self, configuration, painter, color, view, transform_matrix);

        // draw the shape number at the center of the arc, not at the origin of the arc, which for arcs with a
        // large radius but small sweep could be way off the screen.
        draw_shape_number(
            painter,
            view,
            transform_matrix,
            ShapeNumberPosition::Transformed(center_point),
            shape_number,
        );
    }
}

impl Renderable for PolygonGerberPrimitive {
    #[cfg_attr(feature = "profile-renderables", profiling::function)]
    fn render(
        &self,
        painter: &Painter,
        view: &ViewState,
        transform_matrix: &Matrix3<f64>,
        _transform_scaling: &Vector2<f64>,
        color: Color32,
        shape_number: Option<usize>,
        configuration: &RenderConfiguration,
    ) {
        let Self {
            center,
            exposure,
            geometry,
        } = self;
        let color = exposure.to_color(&color);

        let screen_center = Pos2::new(center.x as f32, -(center.y as f32));

        if geometry.is_convex {
            // Direct convex rendering
            let screen_vertices: Vec<Pos2> = geometry
                .relative_vertices
                .iter()
                .map(|v| {
                    let local = Vec2::new(v.x as f32, -v.y as f32);
                    let position = (view.translation
                        + transform_matrix.transform_pos2(screen_center + local) * view.scale)
                        .to_pos2();
                    position
                })
                .collect();

            painter.add(Shape::convex_polygon(screen_vertices, color, Stroke::NONE));
        } else if let Some(tess) = &geometry.tessellation {
            // Transform tessellated geometry
            let vertices: Vec<Vertex> = tess
                .vertices
                .iter()
                .map(|[x, y]| {
                    let local = Vec2::new(*x, -*y); // Flip Y just like convex path
                    let position = (view.translation
                        + transform_matrix.transform_pos2(screen_center + local) * view.scale)
                        .to_pos2();
                    Vertex {
                        pos: position,
                        uv: egui::epaint::WHITE_UV,
                        color,
                    }
                })
                .collect();

            painter.add(Shape::Mesh(Arc::new(Mesh {
                vertices,
                indices: tess.indices.clone(),
                texture_id: egui::TextureId::default(),
            })));
        }

        if configuration.use_vertex_numbering {
            let debug_vertices: Vec<Pos2> = geometry
                .relative_vertices
                .iter()
                .map(|v| {
                    let local = Vec2::new(v.x as f32, -v.y as f32);
                    let position = (view.translation
                        + transform_matrix.transform_pos2(screen_center + local) * view.scale)
                        .to_pos2();
                    position
                })
                .collect();

            for (i, pos) in debug_vertices.iter().enumerate() {
                painter.text(
                    *pos,
                    Align2::CENTER_CENTER,
                    format!("{}", i),
                    FontId::monospace(10.0),
                    Color32::RED,
                );
            }
        }

        draw_bbox!(self, configuration, painter, color, view, transform_matrix);

        draw_shape_number(
            painter,
            view,
            transform_matrix,
            ShapeNumberPosition::Untransformed(screen_center),
            shape_number,
        );
    }
}

fn draw_shape_number(
    painter: &Painter,
    view: &ViewState,
    transform_matrix: &Matrix3<f64>,
    position: ShapeNumberPosition,
    shape_number: Option<usize>,
) {
    let Some(shape_number) = shape_number else { return };

    let position = match position {
        ShapeNumberPosition::Transformed(position) => position,
        ShapeNumberPosition::Untransformed(position) => {
            (view.translation + transform_matrix.transform_pos2(position) * view.scale).to_pos2()
        }
    };
    painter.text(
        position,
        Align2::CENTER_CENTER,
        format!("{}", shape_number),
        FontId::monospace(16.0),
        Color32::GREEN,
    );
}

enum ShapeNumberPosition {
    Transformed(Pos2),
    Untransformed(Pos2),
}
