mod color;
mod expressions;
mod geometry;
mod layer;
mod spacial;
mod types;

#[cfg(feature = "egui")]
mod renderer;

#[cfg(feature = "egui")]
mod drawing;

#[cfg(feature = "egui")]
mod ui;

pub use color::*;
#[cfg(feature = "egui")]
pub use drawing::*;
pub use geometry::*;
/// re-export 'gerber_parser' crate
#[cfg(feature = "parser")]
pub use gerber_parser;
/// re-export 'gerber_types' crate
#[cfg(feature = "types")]
pub use gerber_types;
pub use layer::*;
#[cfg(feature = "egui")]
pub use renderer::*;
pub use spacial::*;
pub use types::*;
#[cfg(feature = "egui")]
pub use ui::*;

#[cfg(feature = "testing")]
pub mod testing;
