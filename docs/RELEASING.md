# Releasing XmlRows

The disk image people download is built by GitHub Actions, never on a developer
machine, and it is published with a signed build provenance attestation. That
attestation is what lets somebody confirm a download came from this
repository's published source without having to trust whoever uploaded it, so
the procedure below exists mainly to keep that link intact.

A local `npm run tauri -- build` is for development and for fast feedback
before a release. It cannot be attested, and it must never be attached to a
GitHub Release.

## Before you start

- The [GitHub CLI](https://cli.github.com) authenticated with push access to
  the repository.
- Node.js/npm, Rust/Cargo, Python 3 and the Xcode Command Line Tools, if you
  intend to run the local checks in step 2.
- A decision about the version number. The project uses
  [Semantic Versioning](https://semver.org/), with the usual flexibility below
  1.0.0.

## 1. Prepare the release commit

Treat the version bump as one coordinated change. The same version must appear
in all five places:

| File | Where |
| --- | --- |
| `package.json` | `version` |
| `package-lock.json` | both root `version` entries |
| `src-tauri/Cargo.toml` | `[package]` `version` |
| `src-tauri/Cargo.lock` | the `xmlrows` package entry |
| `src-tauri/tauri.conf.json` | `version` |

`crates/xmlcore` is a separate library and is not bumped just because the
application version changes.

Then move the relevant notes from `Unreleased` in `CHANGELOG.md` into a dated
section, using `Added`, `Changed` and `Fixed`. Keep them short and describe
observable behavior rather than implementation. Update any documentation that
has stopped being correct — anything that stays right through the
`/releases/latest` link needs no change.

Confirm the manifests agree before going any further:

```sh
python3 .github/scripts/check-version.py --tag v<version>
```

The same check runs first in CI, so a mismatch fails in seconds rather than
after a release is already public.

## 2. Verify locally (optional)

CI repeats all of this, so treat it as fast feedback rather than a gate:

```sh
npm run build
cargo test --manifest-path crates/xmlcore/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npx playwright install chromium   # once
npm run test:gui
npm run tauri -- build --bundles app,dmg
```

## 3. Dry-run the build in CI

Do this on the exact commit you intend to tag. The run builds, verifies and
attests, and skips the release upload, so it changes nothing:

```sh
gh workflow run "Release build" --ref main
gh run watch "$(gh run list --workflow 'Release build' --limit 1 \
  --json databaseId --jq '.[0].databaseId')" --exit-status
```

It takes roughly seven minutes: about a minute for the Rust suites, twenty
seconds for the GUI suite and four and a half minutes for the release build.
There is no dependency caching, deliberately, so that time is the real cost
every run.

A green dry run matters because publishing the release is what starts the real
build, and a failure at that point leaves a public release with no asset.

## 4. Commit, tag and push

```sh
git add -A
git commit -m "Release XmlRows <version>"
git tag -a v<version> -m "XmlRows <version>"
git push origin main
git push origin v<version>
```

The tag must be annotated, named `v<version>`, and sit on that exact release
commit. Once a release tag is pushed, never amend or rewrite the commit under
it.

## 5. Publish the GitHub Release

This step is the trigger for the attested build:

```sh
gh release create v<version> \
  --title "XmlRows <version>" \
  --notes "Installation guidance. Checksum added below once the build finishes."
```

A release saved as a draft starts no build while it stays a draft; publishing
it starts one. The notes cannot carry the checksum yet, because the image that
produces it has not been built.

## 6. Finish the release notes

The workflow builds, verifies and attests the image, then attaches
`XmlRows_<version>_aarch64.dmg`, its `.sha256` checksum and its
`.sigstore.json` attestation bundle. Wait for that run to succeed.

Its job summary prints the final SHA-256 together with a ready-to-paste
verification command. Put the checksum, installation guidance, that command
and a link to `CHANGELOG.md` into the notes. Never reuse a checksum from an
earlier build of the same filename.

## 7. Confirm the result

Check that the release is public and not a draft or prerelease unless that was
intended, that the attached assets have the expected names and sizes, and that
verification succeeds against the published repository:

```sh
gh attestation verify XmlRows_<version>_aarch64.dmg \
  --repo valland/XmlRows \
  --signer-workflow valland/XmlRows/.github/workflows/release.yml
```

## What the attestation does and does not establish

The signed statement binds the image's SHA-256 digest to this repository, to
`.github/workflows/release.yml`, to the exact source commit, and to a
GitHub-hosted runner. For a tagged release the recorded ref is
`refs/tags/v<version>`.

It proves which source commit and which workflow produced the image. It is not
a reproducible build: nobody can rebuild the image and get identical bytes. It
also says nothing about whether the source is any good — it proves origin, not
behavior. Its value comes from the source being public and auditable.

Four things in the workflow protect that link, and should not be "optimised"
away:

- **No dependency or build caching.** The Actions cache is writable from any
  branch, so a poisoned entry could place code in a release that is not in the
  published source.
- **Locked installs.** `npm ci` and `cargo fetch --locked` install exactly what
  the committed lockfiles pin, and a later step re-checks that the build
  changed no lockfile.
- **Actions pinned to commit SHAs**, with the version in a trailing comment.
  Update the pin and the comment together.
- **Minimal permissions**: `contents: write` to attach assets, plus
  `id-token: write` and `attestations: write` to sign.

If the workflow's path or filename ever changes, update the
`--signer-workflow` value in `README.md` and in step 7 above. Verification
against the old path fails.

## If the build fails after you published

For a transient failure — a network blip, a flaky step — re-run the same run.
It keeps the original release event, so the upload step runs again:

```sh
gh run rerun <run-id> --failed
```

For a real defect, do not retag. The tag is published and the commit under it
must not be rewritten. Fix the problem, bump to the next patch version and
release that. Delete the broken release only if nothing was distributed from
it.
