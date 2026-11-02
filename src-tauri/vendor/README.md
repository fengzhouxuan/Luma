# Everything SDKs

Luma statically builds the official C IPC wrappers. These are interfaces to a
running Everything client, not the Everything indexing engine itself.

- SDK2: https://www.voidtools.com/Everything-SDK.zip (retrieved 2026-11-01),
  SHA-256 `f5716d9513cce6b462b5170a0a2e7e081e191d9bcac6774f5decc061497df443`.
  The MIT-style license notices remain in each original source/header file.
- SDK3 3.0.1.12: https://github.com/voidtools/everything_sdk3,
  pinned commit `4b4393519f79249e781cf40d1306d20519330c58`.
  Original files and MIT LICENSE are retained unmodified.

SDK3 is preferred for Everything 1.5; SDK2 supports 1.4 and is the fallback.
No source is fetched at build time. `cc` builds for the target architecture.

The SDK3 3.0.1.12 viewport setters incorrectly return FALSE on success. Luma
checks the requested offset/count using the official getters; vendor files are
not patched. A native regression test verifies first/subsequent page bounds.
