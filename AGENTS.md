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

Before calling a version ready, verify locally:

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

## Attested builds

`.github/workflows/release.yml` repeats all of the above on a GitHub-hosted
Apple Silicon runner and signs a build provenance attestation for the disk
image. A local build is for development and pre-release checking; a published
disk image must come from that workflow, because only it can be attested.

The attestation's value rests on the published source being the only input to
the build. Preserve that:

- Never attach a locally built disk image to a release.
- Do not add dependency or build caching to the release workflow. The Actions
  cache is writable from any branch, so a poisoned entry could place code in a
  release that is not in the published source.
- Keep installs locked to the committed lockfiles (`npm ci`,
  `cargo fetch --locked`) and keep the step that re-checks the lockfiles after
  the build.
- Pin every action to a commit SHA with the version in a trailing comment, and
  update the pin and the comment together.
- Keep the workflow's own permissions minimal: `contents: write` to attach
  assets, plus `id-token: write` and `attestations: write` to sign.
- Use `workflow_dispatch` to test workflow changes. It builds, verifies and
  attests without touching a release.

When the workflow's filename or path changes, update the `--signer-workflow`
value in `README.md`, because verification with the old path will fail.

## Publishing a release

Publishing changes external state. Only push, tag or create a GitHub Release
when the user explicitly requests publication.

`docs/RELEASING.md` is the step-by-step runbook, including the commands. The
numbered points below are the constraints that must hold regardless:

1. Commit the final version, changelog and documentation before tagging.
2. Create an annotated tag named `v<version>` on that exact release commit.
3. Never amend or rewrite a commit after its release tag has been published.
4. Push the release commit and tag.
5. Create a GitHub Release titled `XmlRows <version>` from the tag.
6. Publishing the release starts the `Release build` workflow, which builds,
   verifies and attests the disk image and then attaches
   `XmlRows_<version>_aarch64.dmg`, its `.sha256` checksum and its
   `.sigstore.json` attestation bundle. Wait for that run to succeed.
7. Take the final SHA-256 from that run's job summary, and include it in the
   release notes with installation guidance, the `gh attestation verify`
   command and a link to `CHANGELOG.md`.
8. Verify that the release is public, not a draft or prerelease unless that was
   intended, that the attached asset has the expected name and size, and that
   `gh attestation verify` succeeds against the published repository.

