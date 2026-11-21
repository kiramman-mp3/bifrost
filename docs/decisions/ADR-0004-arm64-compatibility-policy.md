# ADR-0004: Clean-Room x86_64 Policy and Exclusion of Proprietary ARM Translators & GApps

- **Status**: Accepted
- **Date**: 2026-11-21
- **Milestone**: Milestone 0

## Context

Many real-world Android APKs ship native JNI libraries compiled exclusively for `arm64-v8a`, and many users expect Google Play Services. Existing x86 Android projects rely on `libhoudini` (Intel) or `libndk_translation` (Google) to execute ARM binaries on x86_64 hosts. Our Milestone 0 legal and technical review confirmed that both `libhoudini` and `libndk_translation` are **proprietary, closed-source binaries** whose redistribution violates Intel's and Google's intellectual property rights, as does redistributing proprietary Google Apps (GApps) / Play Services without licensing.

## Decision

1. **Pure x86_64 AOSP Baseline**: BIFRÖST ships strictly clean, open-source **x86_64 AOSP** images supporting pure ART (Java/Kotlin/DEX) applications and APKs containing `x86_64` native libraries.
2. **Strict Prohibition on Proprietary Blobs**: BIFRÖST will **never** bundle, mirror, or automatically download proprietary Google Play Services / GApps, `libhoudini`, or `libndk_translation`.
3. **Transparent Compatibility Reporting**: `valhalla` and `mimir` (`bifrost install`) must inspect APK native library directories (`lib/`) and warn or fail explicitly with an `UnsupportedAbi` diagnostic when an APK requires `arm64-v8a` / `armeabi-v7a` native code without an `x86_64` equivalent.
4. **Independent ARM64 Research Track**: ARM64 binary translation remains explicitly classified as **`Unsupported`** until a legally clean, open-source `native_bridge` implementation (e.g., evaluating open AOSP Berberis components or clean-room alternatives) is genuinely implemented and verified by tests.

## Alternatives Considered

- **Bundling extracted `libndk_translation` or `libhoudini` scripts/binaries**: Rejected for clear legal, licensing, and security (unverified binary provenance) reasons.
- **Claiming "experimental ARM support" without implementation**: Explicitly forbidden by project engineering standards.

## Consequences

- **Positive**: 100% legally clean open-source distribution safe from DMCA takedowns and supply-chain tampering.
- **Negative / Trade-off**: ARM64-only native Android apps and games will not run until an open-source translation layer is built or users install x86_64 / universal APKs.
