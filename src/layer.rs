use std::collections::{HashMap, HashSet};
use std::ops::{Add, Range};
use std::sync::Arc;

use gerber_types::{
    Aperture, ApertureDefinition, ApertureMacro, Command, Coordinates, DCode, ExtendedCode, FunctionCode, GCode,
    ImageRotation, MacroContent, MacroDecimal, Operation, VariableDefinition,
};
use gerber_types::{ApertureBlock, Circle, InterpolationMode, QuadrantMode, StepAndRepeat};
use log::{debug, error, info, trace, warn};
use nalgebra::{Point2, Vector2};

use super::expressions::{
    evaluate_expression, macro_boolean_to_bool, macro_decimal_pair_to_f64, macro_decimal_to_f64, macro_integer_to_u32,
    ExpressionEvaluationError, MacroContext,
};
use super::spacial::deduplicate::DedupEpsilon;
use super::{geometry, GerberImageTransform, ToVector};
use crate::geometry::BoundingBox;
use crate::geometry::PolygonMesh;
use crate::types::{Exposure, Winding};

/// FUTURE if the rendering is always real-time, then caching the points at the time the primitives are created would have
///        a performance benefit. e.g. `GerberArcPrimitive::generate_points` and similar methods.

#[derive(Clone, Debug)]
pub struct GerberLayer {
    /// Storing the commands, soon we'll want to tag the primitives with the `Command` used to build them.
    #[allow(unused)]
    commands: Vec<Command>,
    gerber_primitives: Vec<GerberPrimitive>,
    bounding_box: BoundingBox,

    image_transform: GerberImageTransform,
}

impl GerberLayer {
    fn build_image_transform(commands: &Vec<Command>) -> GerberImageTransform {
        let mut transform = GerberImageTransform::default();

        for cmd in commands.iter() {
            match cmd {
                Command::ExtendedCode(ExtendedCode::AxisSelect(axis_select)) => {
                    transform.axis_select = *axis_select;
                }
                Command::ExtendedCode(ExtendedCode::ScaleImage(image_scaling)) => {
                    transform.scale[0] = image_scaling.a;
                    transform.scale[1] = image_scaling.b;
                }
                Command::ExtendedCode(ExtendedCode::OffsetImage(image_offset)) => {
                    transform.offset[0] = image_offset.a;
                    transform.offset[1] = image_offset.b;
                }
                Command::ExtendedCode(ExtendedCode::RotateImage(image_rotation)) => {
                    let degrees: f64 = match image_rotation {
                        ImageRotation::None => 0.0,
                        ImageRotation::CCW_90 => 90.0,
                        ImageRotation::CCW_180 => 180.0,
                        ImageRotation::CCW_270 => 270.0,
                    };

                    transform.rotation = degrees.to_radians();
                }
                Command::ExtendedCode(ExtendedCode::MirrorImage(image_mirroring)) => {
                    transform.mirroring = *image_mirroring;
                }

                _ => {}
            }
        }

        transform
    }
}

impl GerberLayer {
    pub fn new(commands: Vec<Command>) -> Self {
        let gerber_primitives = GerberLayer::build_primitives(&commands);
        let bounding_box = GerberLayer::calculate_bounding_box(&gerber_primitives);
        let image_transform = GerberLayer::build_image_transform(&commands);

        Self {
            commands,
            gerber_primitives,
            bounding_box,
            image_transform,
        }
    }

    /// It's possible to have a gerber file with no primitives
    pub fn is_empty(&self) -> bool {
        self.bounding_box.is_empty()
    }

    pub fn bounding_box(&self) -> &BoundingBox {
        &self.bounding_box
    }

    /// Return the bounding box if the gerber file resulted in primitives which need drawing.
    pub fn try_bounding_box(&self) -> Option<&BoundingBox> {
        match self.is_empty() {
            true => None,
            false => Some(&self.bounding_box),
        }
    }

    #[allow(unused)]
    pub(crate) fn primitives(&self) -> &[GerberPrimitive] {
        &self.gerber_primitives
    }

    pub fn image_transform(&self) -> &GerberImageTransform {
        &self.image_transform
    }
}

trait WithBoundingBox {
    fn bounding_box(&self) -> BoundingBox;
}

impl WithBoundingBox for CircleGerberPrimitive {
    fn bounding_box(&self) -> BoundingBox {
        let Self {
            center,
            diameter,
            ..
        } = self;
        let radius = diameter / 2.0;
        BoundingBox {
            min: Point2::new(center.x - radius, center.y - radius),
            max: Point2::new(center.x + radius, center.y + radius),
        }
    }
}

impl WithBoundingBox for ArcGerberPrimitive {
    fn bounding_box(&self) -> BoundingBox {
        let Self {
            center,
            width,
            ..
        } = self;
        let half_width = width / 2.0;

        let points = self.generate_points();
        let mut bbox = BoundingBox::default();

        for point in points {
            // TODO this could be improved by using a tangent of the arc at each point and
            //      using a vector, of length `half_width`, pointing away from the arc origin, to calculate the
            //      real outer point.

            let center_point = center + point.to_vector();
            let (x, y) = (center_point.x, center_point.y);
            // Use an axis aligned SQUARE of the stroke width at the point to calculate the bounding box
            // For now this approximation is sufficient for current purposes.
            let stroke_bbox = BoundingBox {
                min: Point2::new(x - half_width, y - half_width),
                max: Point2::new(x + half_width, y + half_width),
            };

            // Update bounding box using the stroke bbox
            bbox.expand(&stroke_bbox);
        }

        bbox
    }
}

impl WithBoundingBox for RectangleGerberPrimitive {
    fn bounding_box(&self) -> BoundingBox {
        let Self {
            origin,
            width,
            height,
            ..
        } = self;
        BoundingBox {
            min: Point2::new(origin.x, origin.y),
            max: Point2::new(origin.x + width, origin.y + height),
        }
    }
}

impl WithBoundingBox for LineGerberPrimitive {
    fn bounding_box(&self) -> BoundingBox {
        let Self {
            start,
            end,
            width,
            ..
        } = self;

        let radius = width / 2.0;
        let mut bbox = BoundingBox {
            min: Point2::new(start.x - radius, start.y - radius),
            max: Point2::new(start.x + radius, start.y + radius),
        };
        let end_bbox = BoundingBox {
            min: Point2::new(end.x - radius, end.y - radius),
            max: Point2::new(end.x + radius, end.y + radius),
        };
        bbox.expand(&end_bbox);

        bbox
    }
}

impl WithBoundingBox for PolygonGerberPrimitive {
    fn bounding_box(&self) -> BoundingBox {
        let Self {
            center,
            geometry,
            ..
        } = self;

        let center: Vector2<f64> = center.coords;

        let points = geometry
            .relative_vertices
            .iter()
            .map(|position| position.add(center))
            .collect::<Vec<_>>();

        BoundingBox::from_points(&points)
    }
}

impl GerberLayer {
    fn update_position(current_pos: &mut Point2<f64>, coords: &Option<Coordinates>, offset: Vector2<f64>) {
        let Some(coords) = coords else { return };

        let (x, y) = (
            coords
                .x
                .map(|value| value.into())
                .map(|value: f64| value + offset.x)
                .unwrap_or(current_pos.x),
            coords
                .y
                .map(|value| value.into())
                .map(|value: f64| value + offset.y)
                .unwrap_or(current_pos.y),
        );

        *current_pos = Point2::new(x, y);
    }

    fn calculate_bounding_box(primitives: &Vec<GerberPrimitive>) -> BoundingBox {
        let mut bbox = BoundingBox::default();

        for primitive in primitives {
            let primitive_bbox = match primitive {
                GerberPrimitive::Circle(primitive) => primitive.bounding_box(),
                GerberPrimitive::Arc(primitive) => primitive.bounding_box(),
                GerberPrimitive::Rectangle(primitive) => primitive.bounding_box(),
                GerberPrimitive::Line(primitive) => primitive.bounding_box(),
                GerberPrimitive::Polygon(primitive) => primitive.bounding_box(),
            };
            bbox.expand(&primitive_bbox);
        }

        trace!("layer bbox: {:?}", bbox);

        bbox
    }

    fn build_primitives(commands: &[Command]) -> Vec<GerberPrimitive> {
        #[derive(Debug)]
        struct StepRepeatState {
            initial_position: Point2<f64>,
            start_index: usize,

            repeat_x: u32,
            repeat_y: u32,
            distance_x: f64,
            distance_y: f64,

            x_index: u32,
            y_index: u32,
        }

        let mut macro_definitions: HashMap<String, &ApertureMacro> = HashMap::default();

        // First pass: collect aperture macros
        for cmd in commands.iter() {
            if let Command::ExtendedCode(ExtendedCode::ApertureMacro(macro_def)) = cmd {
                macro_definitions.insert(macro_def.name.clone(), macro_def);
            }
        }

        // Second pass - collect aperture definitions, build their primitives (using supplied args)

        #[derive(Debug, Clone)]
        struct BlockAperture {
            code: i32,
            range: Range<usize>,
        }

        #[derive(Debug)]
        enum LocalApertureKind {
            Standard(ApertureKind),
            Block(BlockAperture),
        }

        let mut apertures: HashMap<i32, LocalApertureKind> = HashMap::default();

        // entries are pushed onto the stack as AB 'open' commands are found
        // popped off the stack and stored in the aperture definitions when a corresponding AB 'close' command is encountered.

        let mut aperture_block_discovery_stack: Vec<ApertureBlockDiscovery> = Vec::new();
        #[derive(Debug, Clone)]
        struct ApertureBlockDiscovery {
            code: i32,
            start: usize,
            // the end is unknown until the corresponding AB 'close' command is encountered
        }

        for (index, command) in commands.iter().enumerate() {
            match command {
                Command::ExtendedCode(ExtendedCode::ApertureBlock(ApertureBlock::Open {
                    code,
                })) => {
                    let discovery = ApertureBlockDiscovery {
                        code: *code,
                        start: index,
                    };
                    trace!("aperture block discovery started. discovery: {:?}", discovery);

                    aperture_block_discovery_stack.push(discovery);
                }
                Command::ExtendedCode(ExtendedCode::ApertureBlock(ApertureBlock::Close)) => {
                    if let Some(discovery) = aperture_block_discovery_stack.last_mut() {
                        let block = BlockAperture {
                            code: discovery.code,
                            // +1 and -1 to exclude the AB 'open/close' commands themselves
                            range: Range {
                                start: discovery.start + 1,
                                end: index - 1,
                            },
                        };
                        trace!("aperture block discovery completed. block: {:?}", block);
                        apertures.insert(discovery.code, LocalApertureKind::Block(block));
                        aperture_block_discovery_stack.pop();
                    } else {
                        error!("Aperture block close without matching open");
                    }
                }
                Command::ExtendedCode(ExtendedCode::ApertureDefinition(ApertureDefinition {
                    code,
                    aperture,
                })) => match aperture {
                    Aperture::Macro(macro_name, args) => {
                        // Handle macro-based apertures

                        if let Some(macro_def) = macro_definitions.get(macro_name) {
                            //
                            // build a unique name based on the macro name and args
                            //
                            let macro_name_and_args = match args {
                                None => macro_name,
                                Some(args) => {
                                    let args_str = args
                                        .iter()
                                        .map(|arg| {
                                            let meh = match arg {
                                                MacroDecimal::Value(value) => value.to_string(),
                                                MacroDecimal::Variable(variable) => format!("${}", variable),
                                                MacroDecimal::Expression(expression) => expression.clone(),
                                            };

                                            meh
                                        })
                                        .collect::<Vec<_>>()
                                        .join("X");

                                    &format!("{}_{}", macro_name, args_str)
                                }
                            };
                            debug!("macro_name_and_args: {}", macro_name_and_args);

                            let mut macro_context = MacroContext::default();

                            //
                            // populate the macro_context from the args.
                            //
                            if let Some(args) = args {
                                for (index, arg) in args.iter().enumerate() {
                                    let arg_number = (index + 1) as u32;

                                    match arg {
                                        MacroDecimal::Value(value) => {
                                            macro_context
                                                .put(arg_number, *value)
                                                .inspect_err(|error| {
                                                    error!("Error setting variable {}: {}", arg_number, error);
                                                })
                                                .ok();
                                        }
                                        MacroDecimal::Variable(variable) => {
                                            macro_context
                                                .put(arg_number, macro_context.get(variable))
                                                .inspect_err(|error| {
                                                    error!("Error setting variable {}: {}", arg_number, error);
                                                })
                                                .ok();
                                        }
                                        MacroDecimal::Expression(expression) => {
                                            evaluate_expression(&expression, &macro_context)
                                                .map(|value| {
                                                    macro_context
                                                        .put(arg_number, value)
                                                        .inspect_err(|error| {
                                                            error!("Error setting variable {}: {}", arg_number, error);
                                                        })
                                                        .ok();
                                                })
                                                .inspect_err(|error| {
                                                    error!("Error evaluating expression {}: {}", expression, error);
                                                })
                                                .ok();
                                        }
                                    }
                                }
                            }

                            trace!("initial macro_context: {:?}", macro_context);

                            let mut primitive_defs = vec![];

                            for content in &macro_def.content {
                                trace!("macro_content: {:?}", content);

                                fn process_content(
                                    content: &MacroContent,
                                    macro_context: &mut MacroContext,
                                ) -> Result<Option<GerberPrimitive>, ExpressionEvaluationError>
                                {
                                    match content {
                                        MacroContent::Circle(circle) => {
                                            let diameter = macro_decimal_to_f64(&circle.diameter, macro_context)?;
                                            let (center_x, center_y) =
                                                macro_decimal_pair_to_f64(&circle.center, macro_context)?;

                                            // Get rotation angle and convert to radians
                                            let rotation_radians = if let Some(angle) = &circle.angle {
                                                macro_decimal_to_f64(angle, macro_context)? * std::f64::consts::PI
                                                    / 180.0
                                            } else {
                                                0.0
                                            };

                                            // Apply rotation to center coordinates around macro origin (0,0)
                                            let (sin_theta, cos_theta) = rotation_radians.sin_cos();
                                            let rotated_x = center_x * cos_theta - center_y * sin_theta;
                                            let rotated_y = center_x * sin_theta + center_y * cos_theta;

                                            Ok(Some(GerberPrimitive::Circle(CircleGerberPrimitive {
                                                center: Point2::new(rotated_x, rotated_y),
                                                diameter,
                                                exposure: macro_boolean_to_bool(&circle.exposure, macro_context)?
                                                    .into(),
                                            })))
                                        }
                                        MacroContent::VectorLine(vector_line) => {
                                            // Get parameters
                                            let (start_x, start_y) =
                                                macro_decimal_pair_to_f64(&vector_line.start, macro_context)?;
                                            let (end_x, end_y) =
                                                macro_decimal_pair_to_f64(&vector_line.end, macro_context)?;
                                            let width = macro_decimal_to_f64(&vector_line.width, macro_context)?;
                                            let rotation_angle =
                                                macro_decimal_to_f64(&vector_line.angle, macro_context)?;
                                            let rotation_radians = rotation_angle.to_radians();
                                            let (sin_theta, cos_theta) = rotation_radians.sin_cos();

                                            // Rotate start and end points
                                            let rotated_start_x = start_x * cos_theta - start_y * sin_theta;
                                            let rotated_start_y = start_x * sin_theta + start_y * cos_theta;
                                            let rotated_end_x = end_x * cos_theta - end_y * sin_theta;
                                            let rotated_end_y = end_x * sin_theta + end_y * cos_theta;

                                            // Calculate direction vector
                                            let dx = rotated_end_x - rotated_start_x;
                                            let dy = rotated_end_y - rotated_start_y;
                                            let length = (dx * dx + dy * dy).sqrt();

                                            if length == 0.0 {
                                                return Ok(None);
                                            }

                                            // Calculate perpendicular direction
                                            let ux = dx / length;
                                            let uy = dy / length;
                                            let perp_x = -uy;
                                            let perp_y = ux;

                                            // Calculate width offsets
                                            let half_width = width / 2.0;
                                            let hw_perp_x = perp_x * half_width;
                                            let hw_perp_y = perp_y * half_width;

                                            // Calculate corners in absolute coordinates
                                            let corners = [
                                                (rotated_start_x - hw_perp_x, rotated_start_y - hw_perp_y),
                                                (rotated_start_x + hw_perp_x, rotated_start_y + hw_perp_y),
                                                (rotated_end_x + hw_perp_x, rotated_end_y + hw_perp_y),
                                                (rotated_end_x - hw_perp_x, rotated_end_y - hw_perp_y),
                                            ];

                                            // Calculate center point
                                            let center_x = (rotated_start_x + rotated_end_x) / 2.0;
                                            let center_y = (rotated_start_y + rotated_end_y) / 2.0;

                                            // Convert to relative vertices
                                            let vertices = corners
                                                .iter()
                                                .map(|&(x, y)| Point2::new(x - center_x, y - center_y))
                                                .collect();

                                            Ok(Some(GerberPrimitive::new_polygon(GerberPolygon {
                                                center: Point2::new(center_x, center_y),
                                                vertices,
                                                exposure: macro_boolean_to_bool(&vector_line.exposure, macro_context)?
                                                    .into(),
                                            })))
                                        }
                                        MacroContent::CenterLine(center_line) => {
                                            // Get parameters
                                            let (center_x, center_y) =
                                                macro_decimal_pair_to_f64(&center_line.center, macro_context)?;
                                            let (length, width) =
                                                macro_decimal_pair_to_f64(&center_line.dimensions, macro_context)?;
                                            let rotation_angle =
                                                macro_decimal_to_f64(&center_line.angle, macro_context)?;
                                            let rotation_radians = rotation_angle.to_radians();
                                            let (sin_theta, cos_theta) = rotation_radians.sin_cos();

                                            // Calculate half dimensions
                                            let half_length = length / 2.0;
                                            let half_width = width / 2.0;

                                            // Define unrotated vertices relative to center
                                            let unrotated_vertices = [
                                                Point2::new(half_length, half_width),
                                                Point2::new(-half_length, half_width),
                                                Point2::new(-half_length, -half_width),
                                                Point2::new(half_length, -half_width),
                                            ];

                                            // Rotate each vertex relative to the center
                                            let vertices = unrotated_vertices
                                                .iter()
                                                .map(|pos| {
                                                    let x = pos.x * cos_theta - pos.y * sin_theta;
                                                    let y = pos.x * sin_theta + pos.y * cos_theta;
                                                    Point2::new(x, y)
                                                })
                                                .collect();

                                            Ok(Some(GerberPrimitive::new_polygon(GerberPolygon {
                                                center: Point2::new(center_x, center_y),
                                                vertices,
                                                exposure: macro_boolean_to_bool(&center_line.exposure, macro_context)?
                                                    .into(),
                                            })))
                                        }
                                        MacroContent::Outline(outline) => {
                                            // Need at least 3 points to form a polygon
                                            if outline.points.len() < 3 {
                                                warn!("Outline with less than 3 points. outline: {:?}", outline);
                                                return Ok(None);
                                            }

                                            // Get vertices - points are already relative to (0,0)
                                            let mut vertices: Vec<Point2<f64>> = outline
                                                .points
                                                .iter()
                                                .filter_map(|point| {
                                                    macro_decimal_pair_to_f64(point, macro_context)
                                                        .map(|(x, y)| Point2::new(x, y))
                                                        .inspect_err(|err| {
                                                            error!("Error building vertex: {}", err);
                                                        })
                                                        .ok()
                                                })
                                                .collect::<Vec<_>>();

                                            // Get rotation angle and convert to radians
                                            let rotation_degrees = macro_decimal_to_f64(&outline.angle, macro_context)?;
                                            let rotation_radians = rotation_degrees * std::f64::consts::PI / 180.0;

                                            // If there's rotation, apply it to all vertices around (0,0)
                                            if rotation_radians != 0.0 {
                                                let (sin_theta, cos_theta) = rotation_radians.sin_cos();
                                                vertices = vertices
                                                    .into_iter()
                                                    .map(|position| {
                                                        let rotated_x = position.x * cos_theta - position.y * sin_theta;
                                                        let rotated_y = position.x * sin_theta + position.y * cos_theta;
                                                        Point2::new(rotated_x, rotated_y)
                                                    })
                                                    .collect();
                                            }

                                            Ok(Some(GerberPrimitive::new_polygon(GerberPolygon {
                                                center: Point2::new(0.0, 0.0), // The flash operation will move this to final position
                                                vertices,
                                                exposure: macro_boolean_to_bool(&outline.exposure, macro_context)?
                                                    .into(),
                                            })))
                                        }
                                        MacroContent::Polygon(polygon) => {
                                            let center = macro_decimal_pair_to_f64(&polygon.center, macro_context)?;

                                            let vertices_count =
                                                macro_integer_to_u32(&polygon.vertices, macro_context)? as usize;
                                            let diameter = macro_decimal_to_f64(&polygon.diameter, macro_context)?;
                                            let rotation_degrees = macro_decimal_to_f64(&polygon.angle, macro_context)?;
                                            let rotation_radians = rotation_degrees * std::f64::consts::PI / 180.0;

                                            // First generate vertices around (0,0)
                                            let radius = diameter / 2.0;
                                            let mut vertices = Vec::with_capacity(vertices_count);
                                            for i in 0..vertices_count {
                                                let angle =
                                                    (2.0 * std::f64::consts::PI * i as f64) / vertices_count as f64;
                                                let x = radius * angle.cos();
                                                let y = radius * angle.sin();

                                                // Apply rotation around macro origin (0,0)
                                                let (sin_theta, cos_theta) = rotation_radians.sin_cos();
                                                let rotated_x = x * cos_theta - y * sin_theta;
                                                let rotated_y = x * sin_theta + y * cos_theta;

                                                vertices.push(Point2::new(rotated_x, rotated_y));
                                            }

                                            // Rotate center point around macro origin
                                            let (sin_theta, cos_theta) = rotation_radians.sin_cos();
                                            let rotated_center_x = center.0 * cos_theta - center.1 * sin_theta;
                                            let rotated_center_y = center.0 * sin_theta + center.1 * cos_theta;

                                            Ok(Some(GerberPrimitive::new_polygon(GerberPolygon {
                                                center: Point2::new(rotated_center_x, rotated_center_y),
                                                vertices,
                                                exposure: macro_boolean_to_bool(&polygon.exposure, macro_context)?
                                                    .into(),
                                            })))
                                        }
                                        MacroContent::Moire(_) => {
                                            error!("Moire not supported");
                                            Ok(None)
                                        }
                                        MacroContent::Thermal(_) => {
                                            error!("Moire not supported");
                                            Ok(None)
                                        }
                                        MacroContent::VariableDefinition(VariableDefinition {
                                            number,
                                            expression,
                                        }) => {
                                            let result = evaluate_expression(&expression, macro_context);
                                            match result {
                                                Ok(value) => {
                                                    macro_context
                                                        .put(*number, value)
                                                        .inspect_err(|error| {
                                                            error!("Error setting variable {}: {}", number, error);
                                                        })
                                                        .ok();
                                                }
                                                Err(cause) => {
                                                    error!("Error evaluating expression {}: {}", expression, cause);
                                                }
                                            };
                                            Ok(None)
                                        }
                                        MacroContent::Comment(_) => {
                                            // Nothing to do
                                            Ok(None)
                                        }
                                    }
                                }

                                let result = process_content(content, &mut macro_context);
                                match result {
                                    Err(cause) => {
                                        error!("Error processing macro content: {:?}, cause: {}", content, cause);
                                    }
                                    Ok(Some(primitive)) => primitive_defs.push(primitive),
                                    Ok(None) => {}
                                }
                            }
                            trace!("final macro_context: {:?}", macro_context);

                            trace!("primitive_defs: {:?}", primitive_defs);

                            apertures.insert(*code, LocalApertureKind::Standard(ApertureKind::Macro(primitive_defs)));
                        } else {
                            error!(
                                "Aperture definition references unknown macro. macro_name: {}",
                                macro_name
                            );
                        }
                    }
                    _ => {
                        apertures.insert(
                            *code,
                            LocalApertureKind::Standard(ApertureKind::Standard(aperture.clone())),
                        );
                    }
                },
                _ => {}
            }
        }
        info!("macros: {:?}", macro_definitions.len());

        debug!("aperture codes: {:?}", apertures.keys());
        info!("apertures: {:?}", apertures.len());

        // Third pass: collect all primitives, handle regions, aperture-block replay and step-repeat blocks

        let mut layer_primitives = Vec::new();
        let mut current_pos = Point2::new(0.0, 0.0);

        let mut current_aperture = None;
        let mut interpolation_mode = InterpolationMode::Linear;
        let mut quadrant_mode = QuadrantMode::Single;

        // also record aperture selection errors
        let mut aperture_selection_errors: HashSet<i32> = HashSet::new();

        // regions are a special case - they are defined by aperture codes
        let mut current_region_vertices: Vec<Point2<f64>> = Vec::new();
        let mut in_region = false;

        let mut index = 0;

        // set to some when the first step-repeat block is encountered
        let mut step_repeat_state: Option<StepRepeatState> = None;
        // not using an option here to keep the math simple
        let mut step_repeat_offset: Vector2<f64> = Vector2::new(0.0, 0.0);

        #[derive(Debug, Clone)]
        struct ApertureBlockReplayState<'a> {
            block: &'a BlockAperture,
            initial_position: Point2<f64>,
            initial_index: usize,
            initial_offset: Vector2<f64>,
            initial_interpolation_mode: InterpolationMode,
            initial_quadrant_mode: QuadrantMode,
        }

        let mut aperture_block_replay_stack: Vec<ApertureBlockReplayState> = vec![];
        let mut aperture_block_offset: Vector2<f64> = Vector2::new(0.0, 0.0);

        loop {
            trace!("aperture_block_replay_stack: {:?}", aperture_block_replay_stack);
            if let Some(state) = aperture_block_replay_stack.last_mut() {
                if index > state.block.range.end {
                    trace!("completed aperture block replay");

                    // The gerber spec says: "After an AB statemen[t] the graphics state remains as it is at the end of
                    // the AB definition, except for the current point, which is undefined. (Gerber has no stack of
                    // graphics states.)"
                    // but let's be consistent by resetting the position to the position when the block we started.
                    // We could just not do this, which might be more 'compliant', but inconsistent.

                    current_pos = state.initial_position;
                    interpolation_mode = state.initial_interpolation_mode;
                    quadrant_mode = state.initial_quadrant_mode;

                    // furthermore, the statement in the spec "Gerber has no stack of graphics states" is misleading,
                    // since we have to reset the current aperture and restore the offset, both of which require
                    // a 'stack of graphic states'.
                    aperture_block_offset = state.initial_offset;
                    // restore the current aperture to this one, since it may be re-used by the next flash command
                    // before another Dxx code is encountered.
                    current_aperture = apertures.get(&state.block.code);

                    // skip the same command, otherwise we'd repeat forever
                    index = state.initial_index + 1;
                    aperture_block_replay_stack.pop();

                    // in the case of nested blocks, we need to check again to see if we're ending the outer block, so
                    // we `continue` here.
                    continue;
                }
            }

            trace!(
                "index: {}, current_position: {}, step_repeat_offset: ({},{}), aperture_block_offset: ({},{})",
                index,
                current_pos,
                step_repeat_offset.x,
                step_repeat_offset.y,
                aperture_block_offset.x,
                aperture_block_offset.y
            );
            let Some(cmd) = commands.get(index) else { break };

            match cmd {
                Command::ExtendedCode(ExtendedCode::ApertureBlock(ApertureBlock::Open {
                    code,
                })) => {
                    // We can get here on an outer block in the case of nested blocked
                    if !aperture_block_replay_stack.is_empty() {
                        trace!("AB (open) during replay");
                    } else {
                        // we're waiting for a block aperture to be selected
                    }

                    // we already discovered the block, get the corresponding block then
                    // jump the the command after it.
                    let block = apertures.get(code).unwrap();
                    if let LocalApertureKind::Block(block) = block {
                        // +1 for the AB close itself, +1 again so we start on the command after it.
                        index = block.range.end + 2;
                        trace!("AB (open), skipping to: {:?}", index);
                        continue;
                    } else {
                        error!("AB (open) not using an aperture block definition");
                    }
                }
                Command::ExtendedCode(ExtendedCode::ApertureBlock(ApertureBlock::Close)) => {
                    // this shouldn't happen, since the block range should cause this to be skipped
                    // when the AP (open) is processed
                    error!("AB (close) encountered during 3rd pass");

                    if !aperture_block_replay_stack.is_empty() {
                        trace!("AB (close) during replay");
                    } else {
                        // we're waiting for a block aperture to be selected
                    }
                    unreachable!()
                }
                Command::ExtendedCode(ExtendedCode::StepAndRepeat(StepAndRepeat::Open {
                    repeat_x,
                    repeat_y,
                    distance_x,
                    distance_y,
                })) => {
                    if !aperture_block_replay_stack.is_empty() {
                        trace!("SR (open) during AB replay");
                    } else {
                        if step_repeat_state.is_some() {
                            error!("Step repeat open without matching close");
                        } else {
                            let state = StepRepeatState {
                                initial_position: current_pos,
                                repeat_x: *repeat_x,
                                repeat_y: *repeat_y,
                                distance_x: *distance_x,
                                distance_y: *distance_y,
                                start_index: index + 1,
                                x_index: 0,
                                y_index: 0,
                            };
                            trace!("Step-and-repeat open, state: {:?}", state);
                            step_repeat_state = Some(state);
                        }
                    }
                }
                Command::ExtendedCode(ExtendedCode::StepAndRepeat(StepAndRepeat::Close)) => {
                    if !aperture_block_replay_stack.is_empty() {
                        trace!("SR (close) during AB replay");
                    } else {
                        if let Some(state) = &mut step_repeat_state {
                            let mut complete = false;
                            state.y_index += 1;
                            if state.y_index >= state.repeat_y {
                                state.y_index = 0;

                                state.x_index += 1;
                                if state.x_index >= state.repeat_x {
                                    complete = true;
                                }
                            }

                            // The gerber spec says "The current point is undefined after an SR statement."
                            // but let's be consistent by resetting the position to the position when the
                            // block we started, for commands AFTER the step-repeat and for commands
                            // in the next step-repeat iteration.
                            // We could just not do this, which might be more 'compliant', but inconsistent.
                            current_pos = state.initial_position;

                            if complete {
                                trace!("Step-and-repeat close");
                                step_repeat_offset = Vector2::new(0.0, 0.0);
                                step_repeat_state = None;
                            } else {
                                step_repeat_offset = Vector2::new(
                                    state.distance_x * state.x_index as f64,
                                    state.distance_y * state.y_index as f64,
                                );

                                trace!(
                                    "Step-and-repeat continue, state: {:?}, current_position: {:?}",
                                    state,
                                    current_pos
                                );

                                index = state.start_index;
                                continue;
                            }
                        } else {
                            error!("Step repeat close without matching open");
                        }
                    }
                }
                Command::FunctionCode(FunctionCode::GCode(GCode::InterpolationMode(mode))) => {
                    interpolation_mode = *mode;
                }
                Command::FunctionCode(FunctionCode::GCode(GCode::QuadrantMode(mode))) => {
                    quadrant_mode = *mode;
                }
                Command::FunctionCode(FunctionCode::GCode(GCode::RegionMode(enabled))) => {
                    if *enabled {
                        // G36 - Begin Region
                        Self::region_begin(&mut current_region_vertices, &mut in_region);
                    } else {
                        // G37 - End Region
                        Self::region_finalize(&mut layer_primitives, &mut current_region_vertices, &mut in_region);
                    }
                }

                Command::FunctionCode(FunctionCode::DCode(DCode::SelectAperture(code))) => {
                    current_aperture = apertures.get(&code);
                    if current_aperture.is_none() {
                        aperture_selection_errors.insert(*code);
                    }
                }
                Command::FunctionCode(FunctionCode::DCode(DCode::Operation(operation))) => {
                    match operation {
                        Operation::Move(coords) => {
                            let mut end = current_pos;
                            Self::update_position(&mut end, coords, step_repeat_offset + aperture_block_offset);
                            if in_region {
                                // In a region, a move operation starts a new path segment
                                // However, we may not have any segments yet, i.e. G36 immediately followed by D02
                                if !current_region_vertices.is_empty() {
                                    // If we have vertices, close the current segment
                                    current_region_vertices.push(*current_region_vertices.first().unwrap());

                                    Self::region_finalize(
                                        &mut layer_primitives,
                                        &mut current_region_vertices,
                                        &mut in_region,
                                    );

                                    // Now start a new segment
                                    Self::region_begin(&mut current_region_vertices, &mut in_region);
                                    current_region_vertices.push(end);
                                } else {
                                    current_region_vertices.push(end);
                                }
                            }
                            current_pos = end;
                        }
                        Operation::Interpolate(coords, offset) => {
                            let mut end = current_pos;
                            Self::update_position(&mut end, coords, step_repeat_offset + aperture_block_offset);
                            if in_region {
                                // Add vertex to the current region
                                current_region_vertices.push(end);
                            } else {
                                match current_aperture {
                                    // 2024.05 - 2.3 "Graphical objects"
                                    // "The solid circle standard aperture is the only aperture allowed for creating draw or arc objects.
                                    // Other standard apertures or macro apertures that fortuitously have a circular shape are not
                                    // allowed."
                                    Some(LocalApertureKind::Standard(ApertureKind::Standard(Aperture::Circle(
                                        circle,
                                    )))) => {
                                        // get the stroke width with the aperture definition
                                        let stroke_width = circle.diameter;

                                        match interpolation_mode {
                                            InterpolationMode::Linear => {
                                                layer_primitives.push(GerberPrimitive::Line(LineGerberPrimitive {
                                                    start: current_pos,
                                                    end,
                                                    width: stroke_width,
                                                    exposure: Exposure::Add,
                                                }));
                                            }
                                            InterpolationMode::ClockwiseCircular
                                            | InterpolationMode::CounterclockwiseCircular => {
                                                // Handle circular interpolation
                                                if let Some(offset) = offset {
                                                    // Get I and J offsets (relative to current position)
                                                    let offset_i = offset
                                                        .x
                                                        .map(|x| x.into())
                                                        .unwrap_or(0.0);
                                                    let offset_j = offset
                                                        .y
                                                        .map(|y| y.into())
                                                        .unwrap_or(0.0);

                                                    // Calculate center of the arc
                                                    let center_x = current_pos.x + offset_i;
                                                    let center_y = current_pos.y + offset_j;
                                                    let center = Point2::new(center_x, center_y);

                                                    // Calculate radius (distance from current position to center)
                                                    let radius = ((offset_i * offset_i) + (offset_j * offset_j)).sqrt();

                                                    // Calculate start angle (from center to current position)
                                                    let start_angle =
                                                        (current_pos.y - center.y).atan2(current_pos.x - center.x);

                                                    // Calculate end angle (from center to target position)
                                                    let end_angle = (end.y - center.y).atan2(end.x - center.x);

                                                    // Calculate sweep angle based on interpolation mode
                                                    let mut sweep_angle = match interpolation_mode {
                                                        InterpolationMode::ClockwiseCircular => {
                                                            if end_angle > start_angle {
                                                                end_angle - start_angle - 2.0 * std::f64::consts::PI
                                                            } else {
                                                                end_angle - start_angle
                                                            }
                                                        }
                                                        InterpolationMode::CounterclockwiseCircular => {
                                                            if end_angle < start_angle {
                                                                end_angle - start_angle + 2.0 * std::f64::consts::PI
                                                            } else {
                                                                end_angle - start_angle
                                                            }
                                                        }
                                                        _ => 0.0, // Should never happen
                                                    };

                                                    // Adjust for single/multi quadrant mode
                                                    if let QuadrantMode::Single = quadrant_mode {
                                                        // In single quadrant mode, sweep angle is always <= 90°
                                                        if sweep_angle.abs() > std::f64::consts::PI / 2.0 {
                                                            if sweep_angle > 0.0 {
                                                                sweep_angle = std::f64::consts::PI / 2.0;
                                                            } else {
                                                                sweep_angle = -std::f64::consts::PI / 2.0;
                                                            }
                                                        }
                                                    }

                                                    let arc_primitive = ArcGerberPrimitive {
                                                        center,
                                                        radius,
                                                        width: stroke_width,
                                                        start_angle,
                                                        sweep_angle,
                                                        exposure: Exposure::Add,
                                                    };

                                                    if arc_primitive.is_full_circle() {
                                                        // add the arc primitive
                                                        layer_primitives.push(GerberPrimitive::Arc(arc_primitive));
                                                    } else {
                                                        let points = arc_primitive.generate_points();

                                                        // draw a circle primitive at the start
                                                        let start_point = points.first().unwrap();
                                                        layer_primitives.push(GerberPrimitive::Circle(
                                                            CircleGerberPrimitive {
                                                                center: start_point + center.to_vector(),
                                                                diameter: stroke_width,
                                                                exposure: Exposure::Add,
                                                            },
                                                        ));

                                                        layer_primitives.push(GerberPrimitive::Arc(arc_primitive));

                                                        // draw a circle primitive at the end
                                                        let end_point = points.last().unwrap();
                                                        layer_primitives.push(GerberPrimitive::Circle(
                                                            CircleGerberPrimitive {
                                                                center: end_point + center.to_vector(),
                                                                diameter: stroke_width,
                                                                exposure: Exposure::Add,
                                                            },
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some(aperture) => {
                                        warn!("Unsupported aperture for plotting. aperture: {:?}", aperture);
                                    }
                                    None => {
                                        error!("No aperture selected for plotting");
                                    }
                                }
                            }
                            current_pos = end;
                        }
                        Operation::Flash(coords, ..) => {
                            if in_region {
                                warn!("Flash operation found within region - ignoring");
                            } else {
                                Self::update_position(
                                    &mut current_pos,
                                    coords,
                                    step_repeat_offset + aperture_block_offset,
                                );

                                if let Some(aperture) = current_aperture {
                                    match aperture {
                                        LocalApertureKind::Standard(ApertureKind::Macro(macro_primitives)) => {
                                            for primitive in macro_primitives {
                                                let mut primitive = primitive.clone();
                                                // Update the primitive's position based on flash coordinates
                                                match &mut primitive {
                                                    GerberPrimitive::Polygon(PolygonGerberPrimitive {
                                                        center,
                                                        ..
                                                    }) => {
                                                        *center += Vector2::new(current_pos.x, current_pos.y);
                                                    }
                                                    GerberPrimitive::Circle(CircleGerberPrimitive {
                                                        center, ..
                                                    }) => {
                                                        *center += Vector2::new(current_pos.x, current_pos.y);
                                                    }
                                                    GerberPrimitive::Arc(ArcGerberPrimitive {
                                                        center, ..
                                                    }) => {
                                                        *center += Vector2::new(current_pos.x, current_pos.y);
                                                    }
                                                    GerberPrimitive::Rectangle(RectangleGerberPrimitive {
                                                        origin,
                                                        ..
                                                    }) => {
                                                        *origin += Vector2::new(current_pos.x, current_pos.y);
                                                    }
                                                    GerberPrimitive::Line(LineGerberPrimitive {
                                                        start,
                                                        end,
                                                        ..
                                                    }) => {
                                                        *start += Vector2::new(current_pos.x, current_pos.y);
                                                        *end += Vector2::new(current_pos.x, current_pos.y);
                                                    }
                                                }
                                                trace!("flashing macro primitive: {:?}", primitive);
                                                layer_primitives.push(primitive);
                                            }
                                        }
                                        LocalApertureKind::Standard(ApertureKind::Standard(aperture)) => {
                                            match aperture {
                                                Aperture::Circle(Circle {
                                                    diameter,
                                                    hole_diameter,
                                                }) => {
                                                    let primitive = if let Some(hole_diameter) = hole_diameter {
                                                        let outer_radius = diameter / 2.0;
                                                        let inner_radius = hole_diameter / 2.0;

                                                        // Mid radius should be the center of where we want our stroke
                                                        let mid_radius = (outer_radius + inner_radius) / 2.0;

                                                        // For StrokeKind::Middle, width should be exactly (outer_radius - inner_radius)
                                                        let width = outer_radius - inner_radius;

                                                        GerberPrimitive::Arc(ArcGerberPrimitive {
                                                            center: current_pos,
                                                            radius: mid_radius,
                                                            width,
                                                            start_angle: 0.0,
                                                            sweep_angle: 2.0 * std::f64::consts::PI, // Full circle, clockwise
                                                            exposure: Exposure::Add,
                                                        })
                                                    } else {
                                                        GerberPrimitive::Circle(CircleGerberPrimitive {
                                                            center: current_pos,
                                                            diameter: *diameter,
                                                            exposure: Exposure::Add,
                                                        })
                                                    };

                                                    layer_primitives.push(primitive);
                                                }

                                                Aperture::Rectangle(rect) => {
                                                    layer_primitives.push(GerberPrimitive::Rectangle(
                                                        RectangleGerberPrimitive {
                                                            origin: Point2::new(
                                                                current_pos.x - rect.x / 2.0,
                                                                current_pos.y - rect.y / 2.0,
                                                            ),
                                                            width: rect.x,
                                                            height: rect.y,
                                                            exposure: Exposure::Add,
                                                        },
                                                    ));
                                                }
                                                Aperture::Polygon(polygon) => {
                                                    let radius = polygon.diameter / 2.0;
                                                    let vertices_count = polygon.vertices as usize;
                                                    let mut vertices = Vec::with_capacity(vertices_count);

                                                    // For standard aperture polygon, we need to generate vertices
                                                    // starting at angle 0 and moving counterclockwise
                                                    for i in 0..vertices_count {
                                                        let angle = (2.0 * std::f64::consts::PI * i as f64)
                                                            / vertices_count as f64;
                                                        let x = radius * angle.cos();
                                                        let y = radius * angle.sin();

                                                        // Apply rotation if specified
                                                        let final_position = if let Some(rotation) = polygon.rotation {
                                                            let rot_rad = rotation * std::f64::consts::PI / 180.0;
                                                            let (sin_rot, cos_rot) = rot_rad.sin_cos();
                                                            Point2::new(
                                                                x * cos_rot - y * sin_rot,
                                                                x * sin_rot + y * cos_rot,
                                                            )
                                                        } else {
                                                            Point2::new(x, y)
                                                        };

                                                        vertices.push(final_position);
                                                    }

                                                    layer_primitives.push(GerberPrimitive::new_polygon(
                                                        GerberPolygon {
                                                            center: current_pos,
                                                            vertices,
                                                            exposure: Exposure::Add,
                                                        },
                                                    ));
                                                }
                                                Aperture::Obround(rect) => {
                                                    // For an obround, we need to:
                                                    // 1. Create a rectangle for the center part
                                                    // 2. Add two circles (one at each end)
                                                    // The longer dimension determines which way the semicircles go

                                                    let (rect_width, rect_height, circle_centers) = if rect.x > rect.y {
                                                        // Horizontal obround
                                                        let rect_width = rect.x - rect.y; // Subtract circle diameter
                                                        let circle_offset = rect_width / 2.0;
                                                        (rect_width, rect.y, [
                                                            (circle_offset, 0.0),
                                                            (-circle_offset, 0.0),
                                                        ])
                                                    } else {
                                                        // Vertical obround
                                                        let rect_height = rect.y - rect.x; // Subtract circle diameter
                                                        let circle_offset = rect_height / 2.0;
                                                        (rect.x, rect_height, [
                                                            (0.0, circle_offset),
                                                            (0.0, -circle_offset),
                                                        ])
                                                    };

                                                    // Add the center rectangle
                                                    layer_primitives.push(GerberPrimitive::Rectangle(
                                                        RectangleGerberPrimitive {
                                                            origin: Point2::new(
                                                                current_pos.x - rect_width / 2.0,
                                                                current_pos.y - rect_height / 2.0,
                                                            ),
                                                            width: rect_width,
                                                            height: rect_height,
                                                            exposure: Exposure::Add,
                                                        },
                                                    ));

                                                    // Add the end circles
                                                    let circle_radius = rect.x.min(rect.y) / 2.0;
                                                    for (dx, dy) in circle_centers {
                                                        layer_primitives.push(GerberPrimitive::Circle(
                                                            CircleGerberPrimitive {
                                                                center: current_pos + Vector2::new(dx, dy),
                                                                diameter: circle_radius * 2.0,
                                                                exposure: Exposure::Add,
                                                            },
                                                        ));
                                                    }
                                                }
                                                Aperture::Macro(code, _args) => {
                                                    // if the aperture referred to a macro, and the macro was supported, it will have been handled by the `ApertureKind::Macro` handling.
                                                    warn!("Unsupported macro aperture: {:?}, code: {}", aperture, code);
                                                }
                                            }
                                        }
                                        LocalApertureKind::Block(block) => {
                                            trace!("flashing block aperture: {:?}", block);

                                            let state = ApertureBlockReplayState {
                                                block,
                                                initial_position: current_pos,
                                                initial_index: index,
                                                initial_offset: aperture_block_offset,
                                                initial_interpolation_mode: interpolation_mode,
                                                initial_quadrant_mode: quadrant_mode,
                                            };
                                            aperture_block_replay_stack.push(state);

                                            aperture_block_offset = current_pos.to_vector();
                                            index = block.range.start;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            index += 1;
        }

        if aperture_selection_errors.len() > 0 {
            error!(
                "Selecting some apertures failed; Check gerber file content and parser errors. aperture_codes: {:?}",
                aperture_selection_errors
            );
        }

        info!("layer_primitives: {:?}", layer_primitives.len());
        trace!("layer_primitives: {:?}", layer_primitives);

        layer_primitives
    }

    fn region_begin(current_region_vertices: &mut Vec<Point2<f64>>, in_region: &mut bool) {
        *in_region = true;
        current_region_vertices.clear();
    }

    fn region_finalize(
        layer_primitives: &mut Vec<GerberPrimitive>,
        current_region_vertices: &mut Vec<Point2<f64>>,
        in_region: &mut bool,
    ) {
        if !*in_region {
            return;
        }
        *in_region = false;

        // SPEC-ISSUE: closed-vs-unclosed-regions - EasyEDA v6.5.48 does not close regions properly
        if current_region_vertices.len() >= 2 {
            let first = current_region_vertices.first().unwrap();
            let last = current_region_vertices.last().unwrap();
            if first != last {
                // TODO add some context to the error message, e.g. command indexes of the start and end of the region
                warn!("Unclosed region detected");
            } else {
                // `GerberPolygon` expects an un-closed polygon vertices, so REMOVE the last coordinate from the vertices
                current_region_vertices.pop();
            }
        }

        trace!("current_region_vertices: {:?}", current_region_vertices);

        if current_region_vertices.len() < 3 {
            return;
        }

        // Find bounding box
        let min_x = current_region_vertices
            .iter()
            .map(|position| position.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = current_region_vertices
            .iter()
            .map(|position| position.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = current_region_vertices
            .iter()
            .map(|position| position.y)
            .fold(f64::INFINITY, f64::min);
        let max_y = current_region_vertices
            .iter()
            .map(|position| position.y)
            .fold(f64::NEG_INFINITY, f64::max);

        // Calculate center from bounding box
        let center_x = (min_x + max_x) / 2.0;
        let center_y = (min_y + max_y) / 2.0;

        let center = Vector2::new(center_x, center_y);

        // Make vertices relative to center
        let relative_vertices: Vec<Point2<f64>> = current_region_vertices
            .iter()
            .map(|position| *position - center)
            .collect();

        let polygon = GerberPrimitive::new_polygon(GerberPolygon {
            center: Point2::new(center_x, center_y),
            vertices: relative_vertices,
            exposure: Exposure::Add,
        });
        layer_primitives.push(polygon);
    }
}

#[derive(Debug)]
enum ApertureKind {
    Standard(Aperture),
    Macro(Vec<GerberPrimitive>),
}

#[derive(Debug, Clone)]
pub(crate) enum GerberPrimitive {
    Circle(CircleGerberPrimitive),
    Rectangle(RectangleGerberPrimitive),
    Line(LineGerberPrimitive),
    Arc(ArcGerberPrimitive),
    Polygon(PolygonGerberPrimitive),
}

#[derive(Debug, Clone)]
pub(crate) struct CircleGerberPrimitive {
    pub center: Point2<f64>,
    pub diameter: f64,
    pub exposure: Exposure,
}

#[derive(Debug, Clone)]
pub(crate) struct RectangleGerberPrimitive {
    pub origin: Point2<f64>,
    pub width: f64,
    pub height: f64,
    pub exposure: Exposure,
}

#[derive(Debug, Clone)]
pub(crate) struct LineGerberPrimitive {
    pub start: Point2<f64>,
    pub end: Point2<f64>,
    pub width: f64,
    pub exposure: Exposure,
}

#[derive(Debug, Clone)]
pub(crate) struct PolygonGerberPrimitive {
    pub center: Point2<f64>,
    pub exposure: Exposure,
    pub geometry: Arc<PolygonGeometry>,
}

#[derive(Debug, Clone)]
pub(crate) struct ArcGerberPrimitive {
    pub center: Point2<f64>,
    pub radius: f64,
    pub width: f64,
    pub start_angle: f64, // in radians
    pub sweep_angle: f64, // in radians, positive = clockwise
    pub exposure: Exposure,
}

impl ArcGerberPrimitive {
    /// Spec 4.7.2 "When start point and end point coincide the result is a full 360° arc"
    ///
    /// However, we to avoid being to strict due to rounding errors.
    pub fn is_full_circle(&self) -> bool {
        // A full circle in Gerber is either:
        // 1. Sweep angle is exactly 0 (special Gerber convention)
        // 2. Sweep angle is exactly 2π (360 degrees)
        const EPSILON: f64 = 1e-10;

        // Check for zero sweep (Gerber convention for full circle)
        if self.sweep_angle.abs() < EPSILON {
            return true;
        }

        // Check for 2π sweep (360 degrees)
        let normalized_sweep = (self.sweep_angle.abs() - 2.0 * std::f64::consts::PI).abs();
        if normalized_sweep < EPSILON {
            return true;
        }

        false
    }

    pub fn generate_points(&self) -> Vec<Point2<f64>> {
        let Self {
            radius,
            start_angle,
            sweep_angle,
            ..
        } = self;

        // Check if this is a full circle
        let is_full_circle = self.is_full_circle();

        let steps = if is_full_circle { 33 } else { 32 };

        let effective_sweep = if is_full_circle {
            2.0 * std::f64::consts::PI
        } else {
            *sweep_angle
        };

        // Calculate the absolute sweep for determining the step size
        let abs_sweep = effective_sweep.abs();
        let angle_step = abs_sweep / (steps - 1) as f64;

        // Generate points along the outer radius
        let mut points = Vec::with_capacity(steps);
        for i in 0..steps {
            // Adjust the angle based on sweep direction
            let angle = if effective_sweep >= 0.0 {
                start_angle + angle_step * i as f64
            } else {
                start_angle - angle_step * i as f64
            };

            let x = *radius * angle.cos();
            let y = *radius * angle.sin();

            points.push(Point2::new(x, y));
        }

        // Ensure exact closure for full circles
        if is_full_circle {
            points[steps - 1] = points[0];
        }

        points
    }
}

#[derive(Debug, Clone)]
pub struct PolygonGeometry {
    pub relative_vertices: Vec<Point2<f64>>, // Relative to center
    pub tessellation: Option<PolygonMesh>,   // Precomputed tessellation data
    pub is_convex: bool,
}

#[derive(Debug)]
pub struct GerberPolygon {
    center: Point2<f64>,
    /// Relative to center
    vertices: Vec<Point2<f64>>,
    exposure: Exposure,
}

impl GerberPolygon {
    /// Checks if a polygon is convex by verifying that all cross products
    /// between consecutive edges have the same sign
    pub fn is_convex(&self) -> bool {
        geometry::is_convex(&self.vertices)
    }
}

impl GerberPrimitive {
    fn new_polygon(polygon: GerberPolygon) -> Self {
        trace!("new_polygon: {:?}", polygon);
        let is_convex = polygon.is_convex();
        let mut relative_vertices = polygon.vertices;

        // Calculate and fix winding order
        let winding = Winding::from_vertices(&relative_vertices);
        if matches!(winding, Winding::Clockwise) {
            relative_vertices.reverse();
        }

        // Deduplicate adjacent vertices with geometric tolerance
        let epsilon = 1e-6; // 1 nanometer in mm units
        let relative_vertices = relative_vertices.dedup_with_epsilon(epsilon);

        // Precompute tessellation for concave polygons
        let tessellation = if !is_convex {
            Some(geometry::tessellate_polygon(&relative_vertices))
        } else {
            None
        };

        let polygon = GerberPrimitive::Polygon(PolygonGerberPrimitive {
            center: polygon.center,
            exposure: polygon.exposure,
            geometry: Arc::new(PolygonGeometry {
                relative_vertices,
                tessellation,
                is_convex,
            }),
        });

        trace!("polygon: {:?}", polygon);

        polygon
    }
}

#[cfg(test)]
mod circular_plotting_tests {
    use std::convert::TryFrom;
    use std::f64::consts::{FRAC_PI_2, PI};

    use gerber_types::{
        Command, CoordinateFormat, CoordinateNumber, CoordinateOffset, Coordinates, DCode, GCode, InterpolationMode,
        Operation, Unit,
    };

    use super::*;
    use crate::layer::{GerberLayer, GerberPrimitive};
    use crate::testing::dump_gerber_source;

    #[test]
    fn test_rounded_rectangle_outline() {
        // Given
        env_logger::init();

        // and
        let corner_radius: f64 = 5.0; // mm
        let line_width: f64 = 0.1; // mm

        let format = CoordinateFormat::new(3, 5);

        let mut commands: Vec<Command> = Vec::new();

        // Set unit to millimeters
        commands.push(Command::ExtendedCode(ExtendedCode::Unit(Unit::Millimeters)));

        // Define circle aperture for outline
        commands.push(Command::ExtendedCode(ExtendedCode::ApertureDefinition(
            ApertureDefinition::new(12, Aperture::Circle(Circle::new(line_width))),
        )));

        // Format codes
        commands.push(Command::ExtendedCode(ExtendedCode::CoordinateFormat(format)));
        commands.push(GCode::InterpolationMode(InterpolationMode::Linear).into());

        // Start at top-left corner
        commands.push(
            DCode::Operation(Operation::Move(Some(Coordinates::new(
                CoordinateNumber::try_from(5.0).unwrap(),
                CoordinateNumber::try_from(15.0).unwrap(),
                format,
            ))))
            .into(),
        );

        // Select aperture
        commands.push(DCode::SelectAperture(12).into());

        // Draw top-left corner arc (counterclockwise)
        commands.push(GCode::InterpolationMode(InterpolationMode::CounterclockwiseCircular).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(0.0).unwrap(),
                    CoordinateNumber::try_from(10.0).unwrap(),
                    format,
                )),
                Some(CoordinateOffset::new(
                    CoordinateNumber::try_from(0.0).unwrap(),
                    CoordinateNumber::try_from(-5.0).unwrap(),
                    format,
                )),
            ))
            .into(),
        );

        // Linear interpolation for left side
        commands.push(GCode::InterpolationMode(InterpolationMode::Linear).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(0.0).unwrap(),
                    CoordinateNumber::try_from(5.0).unwrap(),
                    format,
                )),
                None,
            ))
            .into(),
        );

        // Bottom-left corner arc (counterclockwise)
        commands.push(GCode::InterpolationMode(InterpolationMode::CounterclockwiseCircular).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(5.0).unwrap(),
                    CoordinateNumber::try_from(0.0).unwrap(),
                    format,
                )),
                Some(CoordinateOffset::new(
                    CoordinateNumber::try_from(5.0).unwrap(),
                    CoordinateNumber::try_from(0.0).unwrap(),
                    format,
                )),
            ))
            .into(),
        );

        // Linear interpolation for bottom side
        commands.push(GCode::InterpolationMode(InterpolationMode::Linear).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(15.0).unwrap(),
                    CoordinateNumber::try_from(0.0).unwrap(),
                    format,
                )),
                None,
            ))
            .into(),
        );

        // Bottom-right corner arc (counterclockwise)
        commands.push(GCode::InterpolationMode(InterpolationMode::CounterclockwiseCircular).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(20.0).unwrap(),
                    CoordinateNumber::try_from(5.0).unwrap(),
                    format,
                )),
                Some(CoordinateOffset::new(
                    CoordinateNumber::try_from(0.0).unwrap(),
                    CoordinateNumber::try_from(5.0).unwrap(),
                    format,
                )),
            ))
            .into(),
        );

        // Linear interpolation for right side
        commands.push(GCode::InterpolationMode(InterpolationMode::Linear).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(20.0).unwrap(),
                    CoordinateNumber::try_from(10.0).unwrap(),
                    format,
                )),
                None,
            ))
            .into(),
        );

        // Top-right corner arc (counterclockwise)
        commands.push(GCode::InterpolationMode(InterpolationMode::CounterclockwiseCircular).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(15.0).unwrap(),
                    CoordinateNumber::try_from(15.0).unwrap(),
                    format,
                )),
                Some(CoordinateOffset::new(
                    CoordinateNumber::try_from(-5.0).unwrap(),
                    CoordinateNumber::try_from(0.0).unwrap(),
                    format,
                )),
            ))
            .into(),
        );

        // Linear interpolation for top side (back to start)
        commands.push(GCode::InterpolationMode(InterpolationMode::Linear).into());
        commands.push(
            DCode::Operation(Operation::Interpolate(
                Some(Coordinates::new(
                    CoordinateNumber::try_from(5.0).unwrap(),
                    CoordinateNumber::try_from(15.0).unwrap(),
                    format,
                )),
                None,
            ))
            .into(),
        );

        // and
        dump_gerber_source(&commands);

        // When
        let gerber_layer = GerberLayer::new(commands);
        let primitives = gerber_layer.primitives();
        println!("primitives");
        primitives
            .iter()
            .for_each(|primitive| println!("{:?}", primitive));

        // Then
        // Verify primitives count - should have 4 lines and 4 arcs and 8 circles
        assert_eq!(primitives.len(), 16);

        // Verify that we have the required groups
        for i in 0..16 {
            match i % 4 {
                0 => assert!(
                    matches!(primitives[i], GerberPrimitive::Circle { .. }),
                    "Expected Circle at index {}",
                    i
                ),
                1 => assert!(
                    matches!(primitives[i], GerberPrimitive::Arc { .. }),
                    "Expected Arc at index {}",
                    i
                ),
                2 => assert!(
                    matches!(primitives[i], GerberPrimitive::Circle { .. }),
                    "Expected Circle at index {}",
                    i
                ),
                3 => assert!(
                    matches!(primitives[i], GerberPrimitive::Line { .. }),
                    "Expected Line at index {}",
                    i
                ),
                _ => unreachable!(),
            }
        }

        // Define the expected positions for centers and radii first
        let expected_centers = [(5.0, 10.0), (5.0, 5.0), (15.0, 5.0), (15.0, 10.0)];

        // Collect all arcs for property testing
        let arcs: Vec<_> = primitives
            .iter()
            .cloned()
            .enumerate()
            .filter_map(|(i, p)| {
                if let GerberPrimitive::Arc(arc) = p {
                    Some((i, arc))
                } else {
                    None
                }
            })
            .collect();

        // Verify we have exactly 4 arcs
        assert_eq!(arcs.len(), 4, "Expected exactly 4 arcs");

        // Property 1: All sweep angles should be PI/2
        for (
            i,
            (
                arc_index,
                ArcGerberPrimitive {
                    sweep_angle, ..
                },
            ),
        ) in arcs.iter().enumerate()
        {
            assert!(
                (sweep_angle - FRAC_PI_2).abs() < f64::EPSILON,
                "Arc {} at index {} has sweep angle {} which is not PI/2 (expected {})",
                i,
                arc_index,
                sweep_angle,
                FRAC_PI_2
            );
        }

        // Property 2: All radii should be equal to corner_radius
        for (
            i,
            (
                arc_index,
                ArcGerberPrimitive {
                    radius, ..
                },
            ),
        ) in arcs.iter().enumerate()
        {
            assert_eq!(
                *radius, corner_radius,
                "Arc {} at index {} has radius {} which is not equal to corner_radius {}",
                i, arc_index, radius, corner_radius
            );
        }

        // Property 3: All line widths should be equal to line_width
        for (
            i,
            (
                arc_index,
                ArcGerberPrimitive {
                    width, ..
                },
            ),
        ) in arcs.iter().enumerate()
        {
            assert_eq!(
                *width, line_width,
                "Arc {} at index {} has width {} which is not equal to line_width {}",
                i, arc_index, width, line_width
            );
        }

        // Property 4: All arcs should have Add exposure
        for (
            i,
            (
                arc_index,
                ArcGerberPrimitive {
                    exposure, ..
                },
            ),
        ) in arcs.iter().enumerate()
        {
            assert!(
                matches!(*exposure, Exposure::Add),
                "Arc {} at index {} has exposure {:?} which is not Add",
                i,
                arc_index,
                exposure
            );
        }

        // Property 5: Centers should match expected positions
        for (
            center_index,
            (
                arc_index,
                ArcGerberPrimitive {
                    center, ..
                },
            ),
        ) in arcs.iter().enumerate()
        {
            let expected_center = expected_centers[center_index];
            let arc_center = (center.x, center.y);
            assert_eq!(
                arc_center, expected_center,
                "Arc {} at index {} has center {:?} which is not equal to expected {:?}",
                center_index, arc_index, arc_center, expected_center
            );
        }

        // Display start angles for each arc to document the pattern
        println!("Arc start angles (in radians):");
        for (
            i,
            (
                arc_index,
                ArcGerberPrimitive {
                    start_angle, ..
                },
            ),
        ) in arcs.iter().enumerate()
        {
            // Convert to degrees for more readable output
            let degrees = start_angle.to_degrees();
            println!(
                "Arc {}, index: {}, start_angle = {} rad ({}°)",
                i, arc_index, start_angle, degrees
            );
        }

        // Optionally, verify the specific pattern of start angles that was observed
        // This is kept separate as it's more of a documentation of the observed pattern
        // rather than an enforced property of the API
        let expected_start_angles = [FRAC_PI_2, PI, -FRAC_PI_2, 0.0];
        let angle_names = ["PI", "-PI/2", "0", "PI/2"]; // For better error messages

        for (
            idx,
            (
                (
                    arc_idx,
                    ArcGerberPrimitive {
                        start_angle, ..
                    },
                ),
                angle_name,
            ),
        ) in arcs
            .iter()
            .zip(angle_names.iter())
            .enumerate()
        {
            assert!(
                (start_angle - expected_start_angles[idx]).abs() < f64::EPSILON,
                "Arc at index {} has start_angle {} which doesn't match expected {} ({})",
                arc_idx,
                start_angle,
                angle_name,
                expected_start_angles[idx]
            );
        }
    }
}

#[cfg(test)]
mod circle_aperture_tests {
    use std::f64::consts::PI;

    use gerber_types::{
        Aperture, ApertureDefinition, Circle, Command, CoordinateFormat, CoordinateNumber, Coordinates, DCode,
        ExtendedCode, FunctionCode, Operation, Unit,
    };
    use nalgebra::Point2;

    use crate::testing::dump_gerber_source;
    use crate::types::Exposure;
    use crate::ArcGerberPrimitive;
    use crate::{GerberLayer, GerberPrimitive};

    #[test]
    fn test_circle_with_hole_rendering() {
        // Given: A circle aperture with a hole
        let outer_diameter = 2.5;
        let hole_diameter = 0.5;
        let center = Point2::new(0.0_f64, 0.0_f64);

        // Create an aperture definition that would be parsed from the Gerber file
        let aperture = Aperture::Circle(Circle {
            diameter: outer_diameter,
            hole_diameter: Some(hole_diameter),
        });

        let format = CoordinateFormat::new(2, 4);

        // Create commands that would define and use this aperture
        let commands = vec![
            // Set unit to millimeters
            Command::ExtendedCode(ExtendedCode::Unit(Unit::Millimeters)),
            Command::ExtendedCode(ExtendedCode::ApertureDefinition(ApertureDefinition::new(11, aperture))),
            Command::FunctionCode(FunctionCode::DCode(DCode::SelectAperture(11))),
            Command::FunctionCode(FunctionCode::DCode(DCode::Operation(Operation::Flash(Some(
                Coordinates::new(
                    CoordinateNumber::try_from(center.x).unwrap(),
                    CoordinateNumber::try_from(center.y).unwrap(),
                    format,
                ),
            ))))),
        ];

        // and
        dump_gerber_source(&commands);

        // When
        let layer = GerberLayer::new(commands);
        let primitives = layer.primitives();

        // Then
        assert_eq!(primitives.len(), 1);

        match &primitives[0] {
            GerberPrimitive::Arc(ArcGerberPrimitive {
                center: c,
                radius,
                width,
                start_angle,
                sweep_angle,
                exposure,
            }) => {
                assert_eq!(*c, center);

                // For correct rendering with StrokeKind::Middle
                // The radius should be midway between outer and inner radius
                let expected_radius = (outer_diameter / 2.0 + hole_diameter / 2.0) / 2.0;

                assert!(
                    (radius - expected_radius).abs() < f64::EPSILON,
                    "Radius should be midway between outer and inner radii ({}), got {}",
                    expected_radius,
                    radius
                );

                // Width should be the difference between outer and inner radius
                let expected_width = outer_diameter / 2.0 - hole_diameter / 2.0;
                assert!(
                    (width - expected_width).abs() < f64::EPSILON,
                    "Width should equal the difference between outer and inner radii ({}), got {}",
                    expected_width,
                    width
                );

                assert_eq!(*start_angle, 0.0);
                assert!(
                    (sweep_angle.abs() - 2.0 * PI).abs() < f64::EPSILON,
                    "Sweep angle should be 2π radians (full circle)"
                );
                assert_eq!(*exposure, Exposure::Add);
            }
            _ => panic!("Expected an Arc primitive for circle with hole"),
        }
    }
}

#[cfg(test)]
mod bounding_box_arc_tests {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    use rstest::rstest;

    use super::*;

    // Helper function to create a test arc
    fn create_arc_primitive(
        center_x: f64,
        center_y: f64,
        radius: f64,
        width: f64,
        start_angle: f64,
        sweep_angle: f64,
    ) -> GerberPrimitive {
        GerberPrimitive::Arc(ArcGerberPrimitive {
            center: Point2::new(center_x, center_y),
            radius,
            width,
            start_angle,
            sweep_angle,
            exposure: Exposure::Add,
        })
    }

    // This test is more result-orientated, requires no use of sin/cos/tan/PI/etc.
    #[test]
    pub fn test_full_circle() {
        // given
        let arc_primitive = ArcGerberPrimitive {
            center: Default::default(),
            radius: 100.0,
            width: 1.0,
            start_angle: 0.0_f64.to_radians(),
            sweep_angle: 0.0_f64.to_radians(),
            exposure: Exposure::Add,
        };

        // when
        let bbox = arc_primitive.bounding_box();

        // then
        println!("bbox: {:?}", bbox);
        // should be the same with as the diameter of the circle + half of the stroke width.
        assert_eq!(bbox.min, Point2::new(-100.5, -100.5));
        assert_eq!(bbox.max, Point2::new(100.5, 100.5));
    }

    // Test for full circles (behavior orientated)
    #[rstest]
    #[case(0.0, 0.0, 100.0, 0.0, 0.0)] // start = 0, sweep = 0 (special case for full circle)
    #[case(0.0, 0.0, 100.0, 0.0, 2.0 * PI)] // start = 0, sweep = 2π
    #[case(10.0, 5.0, 100.0, 0.0, 0.0)] // start = 0, sweep = 0 (special case for full circle)
    #[case(10.0, 5.0, 100.0, 0.0, 2.0 * PI)] // start = 0, sweep = 2π
    fn test_full_circle_bounds(
        #[case] center_y: f64,
        #[case] center_x: f64,
        #[case] radius: f64,
        #[case] start_angle: f64,
        #[case] sweep_angle: f64,
    ) {
        // Setup

        let width = 0.5;

        let arc = create_arc_primitive(center_x, center_y, radius, width, start_angle, sweep_angle);
        let primitives = vec![arc];

        let bbox = GerberLayer::calculate_bounding_box(&primitives);

        // For a full circle, the bounds should be center +/- (radius + half_width)
        let half_width = width / 2.0;

        // Verify the bounding box is approximately correct
        assert!(
            (bbox.min.x - (center_x - radius - half_width)).abs() < 1.0,
            "min.x should be approximately {}, got {}",
            center_x - radius - half_width,
            bbox.min.x
        );
        assert!(
            (bbox.min.y - (center_y - radius - half_width)).abs() < 1.0,
            "min.y should be approximately {}, got {}",
            center_y - radius - half_width,
            bbox.min.y
        );
        assert!(
            (bbox.max.x - (center_x + radius + half_width)).abs() < 1.0,
            "max.x should be approximately {}, got {}",
            center_x + radius + half_width,
            bbox.max.x
        );
        assert!(
            (bbox.max.y - (center_y + radius + half_width)).abs() < 1.0,
            "max.y should be approximately {}, got {}",
            center_y + radius + half_width,
            bbox.max.y
        );
    }

    // bbox should be the same as the stroke width, centered on the center
    #[rstest]
    #[case(0.0, 0.0, BoundingBox { min: Point2::new(-0.5, -0.5), max: Point2::new(0.5, 0.5)})]
    #[case(10.0, 10.0, BoundingBox { min: Point2::new(9.5, 9.5), max: Point2::new(10.5, 10.5)})]
    pub fn test_full_circle_zero_radius(
        #[case] center_y: f64,
        #[case] center_x: f64,
        #[case] expected_bbox: BoundingBox,
    ) {
        // given
        let arc_primitive = ArcGerberPrimitive {
            center: Point2::new(center_x, center_y),
            radius: 0.0,
            width: 1.0,
            start_angle: 0.0_f64.to_radians(),
            sweep_angle: 0.0_f64.to_radians(),
            exposure: Exposure::Add,
        };

        // when
        let bbox = arc_primitive.bounding_box();

        // then
        println!("bbox: {:?}, expected: {:?}", bbox, expected_bbox);
        assert_eq!(bbox.min, expected_bbox.min);
        assert_eq!(bbox.max, expected_bbox.max);
    }

    // Test for partial arcs
    #[rstest]
    #[case(0.0, FRAC_PI_2)] // 0° to 90°
    #[case(FRAC_PI_2, FRAC_PI_2)] // 90° to 180°
    #[case(PI, FRAC_PI_2)] // 180° to 270°
    #[case(PI + FRAC_PI_2, FRAC_PI_2)] // 270° to 360°
    fn test_quarter_arc_bounds(#[case] start_angle: f64, #[case] sweep_angle: f64) {
        // Setup
        let center_x = 5.0;
        let center_y = 5.0;
        let radius = 10.0;
        let width = 0.5;

        let arc = create_arc_primitive(center_x, center_y, radius, width, start_angle, sweep_angle);
        let primitives = vec![arc];

        // Execute
        let bbox = GerberLayer::calculate_bounding_box(&primitives);

        // Verify the bounding box contains the center point plus the arc
        let half_width = width / 2.0;
        let total_radius = radius + half_width;

        // The bounds shouldn't exceed center +/- (radius + half_width) in any direction
        assert!(bbox.min.x >= center_x - total_radius - 0.1);
        assert!(bbox.min.y >= center_y - total_radius - 0.1);
        assert!(bbox.max.x <= center_x + total_radius + 0.1);
        assert!(bbox.max.y <= center_y + total_radius + 0.1);

        // Verify the bounds contain the start and end points of the arc
        let start_x = center_x + radius * start_angle.cos();
        let start_y = center_y + radius * start_angle.sin();
        let end_x = center_x + radius * (start_angle + sweep_angle).cos();
        let end_y = center_y + radius * (start_angle + sweep_angle).sin();

        assert!(bbox.min.x <= start_x + 0.1);
        assert!(bbox.min.y <= start_y + 0.1);
        assert!(bbox.max.x >= start_x - 0.1);
        assert!(bbox.max.y >= start_y - 0.1);

        assert!(bbox.min.x <= end_x + 0.1);
        assert!(bbox.min.y <= end_y + 0.1);
        assert!(bbox.max.x >= end_x - 0.1);
        assert!(bbox.max.y >= end_y - 0.1);

        // The bounds should contain the center point, but only because they would naturally
        assert!(bbox.min.x <= center_x);
        assert!(bbox.min.y <= center_y);
        assert!(bbox.max.x >= center_x);
        assert!(bbox.max.y >= center_y);
    }

    // Test for negative sweeps (clockwise arcs)
    #[rstest]
    #[case(FRAC_PI_4, -FRAC_PI_4)] // Small negative sweep
    #[case(FRAC_PI_2, -FRAC_PI_2)] // Quarter negative sweep
    #[case(PI, -PI)] // Half negative sweep
    fn test_negative_sweep_arc_bounds(#[case] start_angle: f64, #[case] sweep_angle: f64) {
        // Setup
        let center_x = 5.0;
        let center_y = 5.0;
        let radius = 10.0;
        let width = 0.5;

        let arc = create_arc_primitive(center_x, center_y, radius, width, start_angle, sweep_angle);
        let primitives = vec![arc];

        // Execute
        let bbox = GerberLayer::calculate_bounding_box(&primitives);

        // Same verification as for positive sweeps
        let half_width = width / 2.0;
        let total_radius = radius + half_width;

        // The bounds shouldn't exceed center +/- (radius + half_width) in any direction
        assert!(bbox.min.x >= center_x - total_radius - 0.1);
        assert!(bbox.min.y >= center_y - total_radius - 0.1);
        assert!(bbox.max.x <= center_x + total_radius + 0.1);
        assert!(bbox.max.y <= center_y + total_radius + 0.1);

        // Verify the bounds contain the start and end points of the arc
        let start_x = center_x + radius * start_angle.cos();
        let start_y = center_y + radius * start_angle.sin();
        let end_x = center_x + radius * (start_angle + sweep_angle).cos();
        let end_y = center_y + radius * (start_angle + sweep_angle).sin();

        assert!(bbox.min.x <= start_x + 0.1);
        assert!(bbox.min.y <= start_y + 0.1);
        assert!(bbox.max.x >= start_x - 0.1);
        assert!(bbox.max.y >= start_y - 0.1);

        assert!(bbox.min.x <= end_x + 0.1);
        assert!(bbox.min.y <= end_y + 0.1);
        assert!(bbox.max.x >= end_x - 0.1);
        assert!(bbox.max.y >= end_y - 0.1);
    }

    // Test with offset center
    #[test]
    fn test_arc_offset_center() {
        // Test with a non-origin center
        let center_x = 15.0;
        let center_y = -10.0;
        let radius = 5.0;
        let width = 0.3;
        let start_angle = 0.0;
        let sweep_angle = FRAC_PI_2; // 90° sweep

        let arc = create_arc_primitive(center_x, center_y, radius, width, start_angle, sweep_angle);
        let primitives = vec![arc];

        let bbox = GerberLayer::calculate_bounding_box(&primitives);

        // Verify the bounds for offset center
        let half_width = width / 2.0;

        // The bounds must include at least the start and end points
        let start_x = center_x + radius * start_angle.cos();
        let start_y = center_y + radius * start_angle.sin();
        let end_x = center_x + radius * (start_angle + sweep_angle).cos();
        let end_y = center_y + radius * (start_angle + sweep_angle).sin();

        assert!(bbox.min.x <= start_x + 0.1);
        assert!(bbox.min.y <= start_y + 0.1);
        assert!(bbox.max.x >= start_x - 0.1);
        assert!(bbox.max.y >= start_y - 0.1);

        assert!(bbox.min.x <= end_x + 0.1);
        assert!(bbox.min.y <= end_y + 0.1);
        assert!(bbox.max.x >= end_x - 0.1);
        assert!(bbox.max.y >= end_y - 0.1);

        // For a 90° arc in the first quadrant, we expect:
        assert!(bbox.min.x >= center_x - half_width - 0.1); // min X should be near center
        assert!(bbox.min.y >= center_y - half_width - 0.1); // min Y should be near center
        assert!(bbox.max.x <= center_x + radius + half_width + 0.1); // max X should extend to right
        assert!(bbox.max.y <= center_y + radius + half_width + 0.1); // max Y should extend upward
    }
}
