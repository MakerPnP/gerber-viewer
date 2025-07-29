# Changelog

This project follows semantic versioning.

Possible log types:

- `[added]` for new features.
- `[changed]` for changes in existing functionality.
- `[deprecated]` for once-stable features removed in upcoming releases.
- `[removed]` for deprecated features removed in this release.
- `[fixed]` for any bug fixes.
- `[security]` to invite users to upgrade in case of vulnerabilities.

### v0.5.0 (unreleased)

- [changed] Change the `GerberViewer` API (Migration: use `new` instead of `default`, just move some of the arguments from `paint_layer` to `new`)
- [added] Added `gerber_to_screen_coordinates`. This allows you to draw on top or below the layer at the correct screen coordinates.
  The original use-case of this is to allow panel-unit numbers to be drawn on top of a panel outline gerber.

### v0.4.4 (2025-07-25)

- [changed] Handle viewport relocation.
- [changed] Demo app now shows one window for each file.
- [changed] Bump gerber-parser.
- [added] Added feature that allows the user to edit the source for each demo and see the changes.

### v0.4.3 (2025-07-21)

- [fixed] Fix missing scale from circles.
- [fixed] Fix incorrect scaling on axis-aligned rectangles when an offset was used. 

### v0.4.2 (2025-07-21)

- [fixed] Fix not applying transform to axis-aligned rectangles. 

### v0.4.1 (2025-07-17)

- [changed] Add support for unclosed regions, as found in some EasyEDA generated files. SPEC-ISSUE: closed-vs-unclosed-regions
- [changed] Using rust 2024 edition.

### v0.4.0 (2025-07-14)

- [changed] Support egui 0.32.0
- [changed] Minimum rust version is now 1.85.0.

### v0.3.0 (2025-07-11)

- [changed] Bump gerber-types and gerber-parser.

### v0.2.0 (2025-07-10)

- [added] Support for legacy/deprecated gerber commands: MI, SF, OF, IR, and AS.
  This is a breaking change because you must use the `GerberLayer::image_transform` method.
  Refer to the commit that updated `demo/src/main.rs` and make the corresponding changes to your app.
- [changed] bumped gerber-types and gerber-parser dependencies, the former added new types which may result
  in compilation errors due to additional enum variants.

### v0.1.0 (2025-06-30)

Initial release.
