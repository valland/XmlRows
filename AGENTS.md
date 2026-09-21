# Repository instructions

These instructions apply to the entire repository.

## Product naming

- Use `XmlRows` for the user-facing product, application bundle, window title,
  documentation and release title.
- Use lowercase `xmlrows` for package names, executable names, CLI examples,
  bundle identifiers, storage keys and other technical identifiers.

## Version changes

Treat an application version bump as one coordinated change. Keep the version
identical in all of these files:

- `package.json`
- the root package entries in `package-lock.json`
- `src-tauri/Cargo.toml`
- the `xmlrows` package entry in `src-tauri/Cargo.lock`
- `src-tauri/tauri.conf.json`

Do not bump `crates/xmlcore` merely because the desktop application version
changes. It is a separate library and should be bumped only when it is being
versioned or released independently.

For every application version bump:

1. Move the relevant user-facing notes from `Unreleased` into a dated section
   in `CHANGELOG.md`, using `Added`, `Changed` and `Fixed` where appropriate.
2. Keep release notes concise and describe observable behavior rather than
   implementation details.
3. Update version-specific documentation or download instructions when they no
   longer remain correct through the `/releases/latest` link.
4. Do not distribute materially different builds under the same version.

## Release verification

Before calling a version ready:

1. Run `npm run build`.
2. Run the relevant Rust and GUI regression tests. The GUI suite is
   `npm run test:gui`; on macOS it may use `CHROME_PATH` when Playwright's own
   browser is unavailable.
3. Build the installer with `npm run tauri build`.
4. Verify the generated DMG with `hdiutil verify`.
5. Confirm `CFBundleDisplayName`, `CFBundleShortVersionString` and
   `CFBundleVersion` in the generated app's `Info.plist`.
6. Compute the final DMG's SHA-256 after the last build. Never reuse a checksum
   from an earlier build of the same filename.

Generated applications, DMGs and other build output must not be committed.

## Publishing a release

Publishing changes external state. Only push, tag or create a GitHub Release
when the user explicitly requests publication.

When publication is requested:

1. Commit the final version, changelog and documentation before tagging.
2. Create an annotated tag named `v<version>` on that exact release commit.
3. Never amend or rewrite a commit after its release tag has been published.
4. Push the release commit and tag.
5. Create a GitHub Release titled `XmlRows <version>` from the tag.
6. Attach the final `XmlRows_<version>_aarch64.dmg` and include installation
   guidance, the final SHA-256 and a link to `CHANGELOG.md` in the release notes.
7. Verify that the release is public, not a draft or prerelease unless that was
   intended, and that the downloadable asset has the expected name and size.

