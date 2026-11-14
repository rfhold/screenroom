# Index

| Path | Info |
| --- | --- |
| [ui/](ui/) | Shared Compose catalog UI and state. |
| [mobile/](mobile/) | Mobile application shell. |
| [tv/](tv/) | TV application shell and focus behavior. |
| [scripts/build-core.sh](scripts/build-core.sh) | Kotlin/JNI core preparation. |
| [gradle/wrapper/gradle-wrapper.properties](gradle/wrapper/gradle-wrapper.properties) | Pinned Gradle distribution and checksum. |
| [README.md](README.md) | SDK, build, and device-test prerequisites. |

# Contracts

- Keep generated Kotlin and JNI libraries from the same core source and supported ABI set.
- Preserve coroutine cancellation and explicit request ownership.
- Reject stale UI results after profile, scope, or client changes.
- Keep SharedPreferences validation and commit failures visible.
- Preserve TV focus and remote navigation independently of mobile touch behavior.
- Keep local SDK paths and device state outside tracked configuration.

# Hints

- Use `changing-client-contracts` for core models or bindings.
- Treat emulator and instrumentation scripts as device operations, not ordinary unit checks.
- Select checks from [platform review inputs](../../.agents/skills/reviewing-changes/references/checks.md).
