# Changelog

This project follows semantic versioning.

Possible log types:

- `[added]` for new features.
- `[changed]` for changes in existing functionality.
- `[deprecated]` for once-stable features removed in upcoming releases.
- `[removed]` for deprecated features removed in this release.
- `[fixed]` for any bug fixes.
- `[security]` to invite users to upgrade in case of vulnerabilities.

### v0.2.0 (2025-07-10)

- [added] Support for legacy/deprecated gerber commands: MI, SF, OF, IR, and AS.
  This is a breaking change because you must use the `GerberLayer::image_transform` method.
  Refer to the commit that updated `demo/src/main.rs` and make the corresponding changes to your app.
- [changed] bumped gerber-types and gerber-parser dependencies, the former added new types which may result
  in compilation errors due to additional enum variants.

### v0.1.0 (2025-06-30)

Initial release.
