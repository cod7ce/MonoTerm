# MonoTerm macOS releases

MonoTerm's local release scripts build an Apple Silicon (`arm64`) application for macOS 14 or later, sign an update archive, and write release assets under `dist/`. They do not launch the app, upload assets, create Git tags, or publish releases. Run these commands from the repository on an Apple Silicon Mac with Xcode command-line tools, Rust, Python 3.11 or later, and `curl`. Prepare the pinned native dependencies with `bash scripts/bootstrap.sh` first.

`Cargo.toml` is the version source of truth, currently `0.1.0`. Both `CFBundleShortVersionString` and `CFBundleVersion` in `scripts/Info.plist` must match its stable `0.x.x` version. `python3 -I scripts/version.py` validates the metadata and prints that version. Keep the existing bundle identifier **`local.monoterm.app`** so updates preserve application identity.

## Sparkle SDK

```sh
bash scripts/fetch-sparkle.sh
```

The default fetch pins Sparkle **2.10.0** to its official release archive:

```text
https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-2.10.0.tar.xz
SHA256: c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
```

The script verifies the archive digest before extraction and rejects unsafe archive paths and links. Each download and each extraction uses its own new, empty directory. Scripts and interpreters run outside those directories; Python reads use isolated mode (`-I`). No downloaded SDK executable is run. A successful fetch records the extracted SDK path in `target/sparkle/sdk-path`; subsequent calls reuse that cache. Remove that pathfile to request a fresh download and verification.

An existing trusted SDK can be selected explicitly:

```sh
SPARKLE_SDK_DIR='/absolute/path/to/sparkle-sdk' bash scripts/build-app.sh debug
```

This override requires a Sparkle 2.10.0 framework and its `LICENSE`. The input is read only; the build copies the framework before thinning or signing it. Its archive digest cannot be reverified from an extracted directory, so use the default fetch for reproducible release preparation. The build exports the selected path as `SPARKLE_SDK` for native build integration.

## Update signing key

The app trusts update archives signed by the private key corresponding to `SUPublicEDKey`. The repository's `scripts/update-key.swift` implements Ed25519 using Apple's CryptoKit and stores the private seed in Keychain under service **`local.monoterm.app.sparkle`**, account **`ed25519`**. This is MonoTerm's own record; do not copy another application's key.

This one-time setup creates the record if absent, prints only its public key, and writes that public key into the plist:

```sh
PUBLIC_KEY="$(xcrun swift scripts/update-key.swift public)"
/usr/libexec/PlistBuddy -c "Set :SUPublicEDKey $PUBLIC_KEY" scripts/Info.plist
unset PUBLIC_KEY
python3 -I scripts/version.py --release
```

Keychain may ask for access. The helper never prints or writes a private seed to a file. `public --existing` reads the existing record without creating one. Keep the same private key for future releases: installed versions already trust its public key. Keep it available through a secure recovery process outside this repository; this helper deliberately has no export command. The Keychain item is restricted to this device and is available only while unlocked.

A missing public key is allowed for a local build; runtime update checks remain unavailable until configured. Release packaging fails before building if the public key is missing, malformed, or does not match the available signing key. It also verifies the final archive signature against the public key inside the actual app bundle.

For unattended signing, `SPARKLE_PRIVATE_KEY` may supply a base64-encoded **32-byte Ed25519 seed** from a secret manager. The package script disables tracing, removes this variable from child build environments, passes its value to the helper over standard input, and clears its shell copy after signing. `public --private-key-stdin` can derive its public key without using Keychain. Provision the secret outside source control; never paste it into command lines, logs, or repository files. An unrelated Sparkle tool's Keychain format is not interchangeable with this helper's record.

## Build the application

```sh
bash scripts/build-app.sh
bash scripts/build-app.sh release
```

The default `debug` output is `target/MonoTerm.app`, preserving compatibility with `scripts/run.sh`. The `release` output is `dist/MonoTerm.app`. Neither command launches the app. Both build Cargo with `--locked`, set `MACOSX_DEPLOYMENT_TARGET=14.0`, retain the application icon and vendor license resources, and embed `Sparkle.framework` with its license. The framework's symlinks are preserved. The generated plist records `MonoTermBuildProfile` so release packaging can reject a debug bundle.

All copied Mach-O code is thinned to `arm64`. The script signs executable leaves, then nested `.app`, `.xpc`, and `.framework` bundles from the inside out. It retains existing entitlements and explicitly gives Sparkle's downloader its sandbox and outgoing-network entitlements. The completed bundle must pass `codesign --verify --deep --strict` before publication. The previous bundle is moved aside and the new one moved into place on the same filesystem; existing executable and loaded-framework inodes are never overwritten. Launch Services registration is refreshed for the resulting bundle without restarting a running instance.

Code signing defaults to **ad hoc** (`-`). To select an installed Developer ID identity:

```sh
CODE_SIGN_IDENTITY='Developer ID Application: Your Name (TEAMID)' \
  bash scripts/build-app.sh release
```

A real identity adds hardened runtime and a signing timestamp to all embedded code. These scripts do not notarize or staple. An Ed25519 update signature authenticates the archive to Sparkle; it does not establish Developer ID trust or bypass Gatekeeper. An ad hoc build can pass local signature checks and still be blocked when downloaded. If notarizing separately, notarize and staple the completed app before using `--skip-build` below, so the final archived bytes receive the update signature.

## Package version 0.1.0

The plist's stable update feed is:

```text
https://github.com/cod7ce/MonoTerm/releases/latest/download/appcast.xml
```

`BASE_URL` is the future HTTPS directory containing the versioned archive. It is encoded in the appcast enclosure and is not contacted by the script:

```sh
bash scripts/package-release.sh \
  'https://github.com/cod7ce/MonoTerm/releases/download/v0.1.0'
```

To package the already built and optionally notarized release app:

```sh
bash scripts/package-release.sh \
  'https://github.com/cod7ce/MonoTerm/releases/download/v0.1.0' --skip-build
```

`--skip-build` still validates source and bundled versions, key agreement, exact feed URL, bundle identifier, macOS minimum, release profile, embedded Sparkle version and components, all Mach-O architectures, and the complete bundle signature. It does not prove that a prebuilt app contains the current source revision; use it only with the intended release build.

The script creates exactly these files in `dist/releases/0.1.0/`:

- `MonoTerm-0.1.0-arm64.zip`, a `ditto` ZIP containing `MonoTerm.app`.
- `appcast.xml`, describing the signed archive, byte length, version, macOS 14 minimum, and `arm64` hardware requirement.
- `SHA256SUMS`, containing the ZIP's SHA256 digest.

The repository's CryptoKit helper signs the exact ZIP bytes and verifies the result against the bundled public key. The enclosure is signed; the XML feed itself is not separately signed. The feed contains a single release and no delta updates. A failed attempt leaves no published version directory. Existing output is never overwritten, and a per-version lock prevents concurrent packaging. If a process was forcibly killed, remove its `dist/releases/.0.1.0.lock` directory only after confirming no packaging process still owns it.

## Publish the assets

The local scripts stop after packaging. Publish the ZIP, `appcast.xml`, and `SHA256SUMS` together under the matching `v0.1.0` GitHub release in `cod7ce/MonoTerm`. A published stable release selected as GitHub's latest release serves the fixed feed URL; a draft or prerelease does not. Keep the appcast's versioned enclosure URL and the uploaded ZIP filename unchanged. Do not modify or replace the ZIP after signing it.

For later releases, update `Cargo.toml`, the root `monoterm` package version in `Cargo.lock`, and both version fields in `scripts/Info.plist` together. Keep the bundle identifier, feed URL, and signing key. Use an increasing `0.x.x` version and the matching `v0.x.x` archive URL. Retain older release assets so installed clients following an older appcast can still retrieve their archive. The first installed build must already contain the correct public key to trust later updates.

## Targeted checks

```sh
bash -n scripts/build-app.sh scripts/fetch-sparkle.sh scripts/package-release.sh
plutil -lint scripts/Info.plist
python3 -I scripts/version.py --release
xcrun swift scripts/update-key.swift self-test
codesign --verify --deep --strict dist/MonoTerm.app
(cd dist/releases/0.1.0 && shasum -a 256 -c SHA256SUMS)
```

The helper self-test uses the published RFC 8032 vector and rejects a tampered message; it never accesses Keychain. SDKs, bundles, appcasts, checksums, module caches, and release archives remain under ignored `target/` or `dist/` directories. For an updater installation test, use disposable signed application copies and an isolated macOS user/home. `MONOTERM_CONFIG_DIR` deliberately disables the updater, so it is appropriate for Settings UI tests but cannot exercise an actual installation. Packaging success alone does not validate the complete download/relaunch flow or notarization.
