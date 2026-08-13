# Dragon Language Versioning

## Status: Planned

Dragon follows semantic versioning for its language specification and standard library.

## Version Format

Versions follow the format: `MAJOR.MINOR.PATCH`

- **MAJOR**: Incremented for incompatible language changes
- **MINOR**: Incremented for backward-compatible language features
- **PATCH**: Incremented for backward-compatible bug fixes

## Examples
- `1.0.0` - Initial stable release
- `1.1.0` - Added new feature (backward compatible)
- `1.1.1` - Fixed bug (backward compatible)
- `2.0.0` - Incompatible changes that may require code updates

## Pre-release Versions
Pre-release versions may be denoted by appending a hyphen and identifiers:
- `1.0.0-alpha`
- `1.0.0-beta.2`
- `1.0.0-rc.1`

## Build Metadata
Build metadata can be appended after a plus sign:
- `1.0.0+20230314`
- `1.0.0+exp.sha.5114f85`

## Language vs Implementation Versioning
- **Language version**: The version of the Dragon language specification
- **Implementation version**: The version of the Dragon compiler/runtime
These may differ - an implementation may support multiple language versions.

## Related Files
- `spec/compatibility/compatibility-policy.md` - Compatibility guarantees
- `spec/compatibility/breaking-changes.md` - Documented breaking changes