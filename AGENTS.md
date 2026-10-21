# SDK release rules

- Apply these rules to this SDK and coordinated releases across the 11 Solana SDK repositories.
- Prefer one release version across language implementations of the same SDK family (for example, all sol-parser-sdk packages use 0.9.0). Different SDK families need not share a version.
- Before choosing versions, inspect published registries and Git tags. Never downgrade, overwrite a published version, or reuse a release tag.
- Respect semantic versioning and each package manager's rules. A coordinated major release may align version numbers across languages. For Go v2+, migrate go.mod, all SDK/example imports, installation commands and package documentation to the required /vN module path, validate an external consumer, and document the user migration. The user has authorized the trade SDK's /v9 migration for the 9.0.0 family release. If alignment would require other compatibility changes outside the requested scope, document the exception instead of silently breaking consumers.
- Update package manifests, lockfiles, exported version constants where present, exact cross-SDK dependency pins, English/Chinese README installation examples, and all 11 SDK navigation versions together. Preserve historical changelog versions.
- A shared version does not prove feature parity. Check SDK code, examples, and native-language validation for each language; document unsupported features explicitly.
- Publish in dependency order. Verify package contents, registry versions/checksums, GitHub commits, annotated tags, and Releases agree. Record partial failures accurately.
- Version alignment does not by itself authorize publishing: publish packages, push commits/tags, and create Releases only when requested by the user.
