# Dragon Release Governance

This document describes the release process and versioning policies for the Dragon programming language.

## Release Types

Dragon follows semantic versioning with three types of releases:

### Patch Releases (x.y.Z)
- Backward-compatible bug fixes
- No new features
- Released as needed to address issues

### Minor Releases (x.Y.0)
- Backward-compatible new features
- May include bug fixes
- Released on a regular schedule (approximately every 3 months)

### Major Releases (X.0.0)
- May include breaking changes
- May include new features
- Released when significant changes accumulate
- Breaking changes are documented in `spec/compatibility/breaking-changes.md`

## Release Schedule

- **Patch releases**: As needed
- **Minor releases**: Approximately every 3 months (March, June, September, December)
- **Major releases**: As significant changes warrant (typically every 12-24 months)

## Release Process

### Preparation
1. **Feature Freeze**: One week before release, no new features are merged (only bug fixes)
2. **Testing**: Comprehensive testing of release candidates
3. **Documentation**: Update release notes and documentation
4. **Version Bump**: Update version numbers in appropriate places

### Release Candidate (RC)
1. Create release candidate tag: `vX.Y.Z-rc1`
2. Publish to testing channels
3. Collect feedback and fix critical issues
4. Repeat with rc2, rc3, etc. as needed

### Final Release
1. Create final release tag: `vX.Y.Z`
2. Build and publish release artifacts
3. Publish release notes
4. Announce to community

## Version Number Updates

Version numbers are updated in the following places:
- `Cargo.toml` - Package version
- `src/main.rs` - Version string for `--version` flag
- Documentation references

## Release Artifacts

For each release, the following artifacts are published:
- Source code tarball (`dragon-x.y.z.tar.gz`)
- Windows executable (`dragon-x.y.z-windows.exe`)
- macOS executable (`dragon-x.y.z-macos`)
- Linux executable (`dragon-x.y.z-linux`)
- Documentation package

## Release Criteria

A release is ready when:
1. All critical bugs for the release milestone are fixed
2. Test suite passes on all supported platforms
3. Documentation is up to date
4. Release notes are written
5. Maintainers approve the release

## Hotfixes

For critical issues in a released version:
1. Create a hotfix branch from the release tag
2. Fix the issue
3. Test thoroughly
4. Release as a patch version (x.y.Z+1)
5. Merge hotfix back to main branch

## Pre-releases

For experimental or unstable features:
- Use pre-release identifiers: `x.y.z-alpha.1`, `x.y.z-beta.2`, `x.y.z-rc.1`
- Clearly communicate instability to users
- Not guaranteed to be compatible with other pre-releases

## Related Files
- `spec/compatibility/versioning.md` - Versioning guidelines
- `spec/compatibility/breaking-changes.md` - Documented breaking changes
- `governance/LANGUAGE_CHANGE_PROCESS.md` - Language change process
- `CHANGELOG.md` - Human-readable change log