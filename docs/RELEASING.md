# MonoTerm · ONES macOS 发布

MonoTerm's local release scripts build an Apple Silicon (`arm64`) application for macOS 14 or later, sign an update archive, and write release assets under `dist/`. They do not launch the app, upload assets, create Git tags, or publish releases. Run these commands from the repository on an Apple Silicon Mac with Xcode command-line tools, Rust, Python 3.11 or later, and `curl`. Prepare the pinned native dependencies with `bash scripts/bootstrap.sh` first.

版本号以 `Cargo.toml` 为准，当前为 `0.3.5`。`scripts/Info.plist` 中的 `CFBundleShortVersionString` 和 `CFBundleVersion` 必须与其稳定版 `0.x.x` 版本号一致。运行 `python3 -I scripts/version.py` 可校验元数据并输出版本号。保留应用标识 **`local.monoterm.app`**，确保更新前后使用相同的应用身份。

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

默认 `debug` 输出为 `target/MonoTerm · ONES.app`，`scripts/run.sh` 会启动该应用；`release` 输出为 `dist/MonoTerm · ONES.app`。构建命令本身不启动应用。两种构建均使用 Cargo `--locked`、`MACOSX_DEPLOYMENT_TARGET=14.0`，保留图标和依赖许可证，并嵌入 `Sparkle.framework` 及其许可证和符号链接。生成的 plist 通过 `MonoTermBuildProfile` 记录构建类型，防止将开发版用于发布。

应用包目录、显示名称和包内可执行文件均使用 `MonoTerm · ONES`；安装包文件名使用 `MonoTerm-ONES-<版本>-arm64.zip`。`local.monoterm.app`、更新源、签名密钥和原有配置目录保持兼容。历史 Release 的已发布文件不重命名或替换，后续发布使用新的安装包名称。

All copied Mach-O code is thinned to `arm64`. The script signs executable leaves, then nested `.app`, `.xpc`, and `.framework` bundles from the inside out. It retains existing entitlements and explicitly gives Sparkle's downloader its sandbox and outgoing-network entitlements. The completed bundle must pass `codesign --verify --deep --strict` before publication. The previous bundle is moved aside and the new one moved into place on the same filesystem; existing executable and loaded-framework inodes are never overwritten. Launch Services registration is refreshed for the resulting bundle without restarting a running instance.

Code signing defaults to **ad hoc** (`-`). To select an installed Developer ID identity:

```sh
CODE_SIGN_IDENTITY='Developer ID Application: Your Name (TEAMID)' \
  bash scripts/build-app.sh release
```

A real identity adds hardened runtime and a signing timestamp to all embedded code. These scripts do not notarize or staple. An Ed25519 update signature authenticates the archive to Sparkle; it does not establish Developer ID trust or bypass Gatekeeper. An ad hoc build can pass local signature checks and still be blocked when downloaded. If notarizing separately, notarize and staple the completed app before using `--skip-build` below, so the final archived bytes receive the update signature.

## 打包 0.3.5

The plist's stable update feed is:

```text
https://github.com/cod7ce/MonoTerm/releases/latest/download/appcast.xml
```

`BASE_URL` is the future HTTPS directory containing the versioned archive. It is encoded in the appcast enclosure and is not contacted by the script:

```sh
bash scripts/package-release.sh \
  'https://github.com/cod7ce/MonoTerm/releases/download/v0.3.5'
```

To package the already built and optionally notarized release app:

```sh
bash scripts/package-release.sh \
  'https://github.com/cod7ce/MonoTerm/releases/download/v0.3.5' --skip-build
```

`--skip-build` still validates source and bundled versions, key agreement, exact feed URL, bundle identifier, macOS minimum, release profile, embedded Sparkle version and components, all Mach-O architectures, and the complete bundle signature. It does not prove that a prebuilt app contains the current source revision; use it only with the intended release build.

脚本会在 `dist/releases/0.3.5/` 中生成以下三个文件：

- `MonoTerm-ONES-0.3.5-arm64.zip`：使用 `ditto` 打包，包含 `MonoTerm · ONES.app`。
- `appcast.xml`：记录安装包签名、字节长度、版本号、macOS 14 最低版本和 `arm64` 硬件要求。
- `SHA256SUMS`：记录 ZIP 安装包的 SHA256 摘要。

仓库中的 CryptoKit 工具对最终 ZIP 字节签名，并使用应用内的公钥验证签名。签名保护安装包，XML 更新源本身没有单独签名。更新源只包含一个版本，不提供增量更新。打包失败不会留下已发布版本目录，也不会覆盖已有产物；每个版本的锁目录可防止并发打包。若进程被强制终止，必须先确认没有打包进程仍在使用锁，才能移除 `dist/releases/.0.3.5.lock`。

## Publish the assets

本地脚本仅完成打包。将 ZIP、`appcast.xml` 和 `SHA256SUMS` 一起上传到 `cod7ce/MonoTerm` 对应的 `v0.3.5` GitHub Release，并将其发布为最新稳定版，固定更新源地址才会指向该版本；草稿或预发布版不会生效。保持 appcast 中包含版本号的下载地址和上传的 ZIP 文件名一致。签名后不要修改或替换 ZIP。

For later releases, update `Cargo.toml`, the root `monoterm` package version in `Cargo.lock`, and both version fields in `scripts/Info.plist` together. Keep the bundle identifier, feed URL, and signing key. Use an increasing `0.x.x` version and the matching `v0.x.x` archive URL. Retain older release assets so installed clients following an older appcast can still retrieve their archive. The first installed build must already contain the correct public key to trust later updates.

## Targeted checks

```sh
bash -n scripts/build-app.sh scripts/fetch-sparkle.sh scripts/package-release.sh
plutil -lint scripts/Info.plist
python3 -I scripts/version.py --release
xcrun swift scripts/update-key.swift self-test
codesign --verify --deep --strict 'dist/MonoTerm · ONES.app'
(cd dist/releases/0.3.5 && shasum -a 256 -c SHA256SUMS)
```

The helper self-test uses the published RFC 8032 vector and rejects a tampered message; it never accesses Keychain. SDKs, bundles, appcasts, checksums, module caches, and release archives remain under ignored `target/` or `dist/` directories. For an updater installation test, use disposable signed application copies and an isolated macOS user/home. `MONOTERM_CONFIG_DIR` deliberately disables the updater, so it is appropriate for Settings UI tests but cannot exercise an actual installation. Packaging success alone does not validate the complete download/relaunch flow or notarization.
