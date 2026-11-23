# Build and release

The root `justfile` runs the native tools. Install `just`, then run `just` to list commands.
The current pnpm, Cargo, Gradle, and Xcode commands still work.

| Command | Outcome |
| --- | --- |
| `just build <target>` | Builds one target. It does not deploy a service, sign a release, or use a device. |
| `just check <target>` | Runs a local check for one target. See [Testing](testing.md) for separate E2E and database procedures. |
| `just proto-tools` | Installs the pinned Buf plugins under `target/buf-tools`. |
| `just proto` | Lints the protobuf schema and regenerates the Rust files. |
| `just check release` | Tests package inputs and the publisher with a local transport stub. |

Build and check targets: `backend`, `core`, `web`, `website`, `linux`, `macos`, `ios`, `tvos`, `android`, and `android-tv`.
Build also accepts `core-bindings`, `core-wasm`, `core-apple`, and `core-android`.
Apple build and check use simulator SDKs for iOS and tvOS. Android build and check use debug variants.

## Create native packages

Run `just package <target> <version> <build>` from a clean Git revision.
Use the same commit, numeric `MAJOR.MINOR.PATCH` version, and build number on each host.
Use a minor and patch from 0 through 99, and a build number from 1 through 9999.
The build number sets the Linux package revision and the Apple and Android build version.

| Target | Host and tools | Artifact |
| --- | --- | --- |
| `linux-deb-amd64` | Native Debian 13 `amd64`, `dpkg-deb`, `dpkg-shlibdeps` | `.deb` |
| `linux-deb-arm64` | Native Debian 13 `arm64`, `dpkg-deb`, `dpkg-shlibdeps` | `.deb` |
| `linux-arch-amd64` | Native Arch Linux `x86_64`, unprivileged `makepkg`, `bsdtar` | `.pkg.tar.zst` |
| `macos` | macOS, Xcode, XcodeGen, Rust Apple targets | Universal ad hoc-signed `.app` ZIP |
| `ios` | Same Apple host with the iOS device SDK | Unsigned arm64 `.xcarchive` ZIP for iPhone and iPad |
| `tvos` | Same Apple host with the tvOS device SDK | Unsigned arm64 `.xcarchive` ZIP |
| `android-tv` | Android SDK and NDK on macOS or x86_64 Linux | Unsigned TV APK |
| `android` | Same Android host | Unsigned mobile APK |

All package targets require `jq` and `shasum`. Linux also requires `readelf`, `desktop-file-validate`, and the [native build dependencies](../apps/linux/README.md#native-run).
See the [Apple](../apps/apple/README.md) and [Android](../apps/android/README.md) guides for SDK and Rust prerequisites.
Android package inspection uses SDK Build-Tools 36.0.0 by default. Set `ANDROID_BUILD_TOOLS_VERSION` to use another installed version.
Run only one Apple or Android build at a time in a checkout because the core artifact paths are shared.

Each target writes one package and `manifest.json` to `target/packages/v<version>-<build>/<target>/`.
The manifest records the source commit, version, target, architecture, signature status, size, and SHA-256.
For a local check from a dirty tree, set `SCREENROOM_PACKAGE_LOCAL=1`.
That mode writes under `target/packages/local/`, and the publisher rejects its manifest.

Unsigned Apple device archives and Android APKs do not install as delivered.
The macOS ZIP does not include Developer ID signing or notarization.
These packages do not grant access to a private Screenroom instance.

## Publish to private storage

Copy all eight target directories into one release directory. Keep each package with its manifest.
Set `SCREENROOM_PACKAGE_URL` to the approved Forgejo generic package URL, without a version suffix.
Confirm that the destination is private and that Forgejo rejects writes to a file that already exists.
Use the shared `FORGEJO_ACCESS_TOKEN` convention for upload access. The dry run uses no token or network.

```sh
export SCREENROOM_PACKAGE_URL=https://forgejo.example/api/packages/owner/generic/screenroom
just publish target/packages/v0.1.0-2 --dry-run
just publish target/packages/v0.1.0-2
```

The publisher checks the eight manifests and package hashes. It reads each uploaded file back and publishes `release.json` last.
If an upload fails, check the reported file before a retry. A retry skips files that match and stops on conflicts.
The publisher never rebuilds packages. It does not update apt or pacman feeds, submit to app stores, or trigger [Pantheon deployment](deployment.md).
