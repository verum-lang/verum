# Publication type boundary controls

These fixtures require ordinary, complete project checking against the exact
production modules. They are prepared controls, with no passing type/privacy
or affine result recorded yet. Parse acceptance alone accepts none of them.

For each fixture, copy the exact `src/protocol/{authority,publication,
publication_metadata}.vr` and `src/node/{publication_auth,
publication_prepare}.vr` modules, the regular component runner's manifest,
and this one fixture as `src/main.vr`. Replace only its module header with
`module verum_registry.main;`. Invoke argument-less `verum check` with a
pinned executable/archive and deadline. Preserve input hashes and full
diagnostics; standalone file checking cannot replace project mode.

| Fixture | Required result |
| --- | --- |
| `publication_prepare_consume.vr` | Check succeeds; one public `into_parts` call is available. |
| `publication_auth_private_store.vr` | Refuse access to private `CredentialStore.credentials`. |
| `publication_auth_construct_store.vr` | Refuse external construction through private store fields. |
| `publication_prepare_construct.vr` | Refuse external construction through private preparation fields. |
| `publication_prepare_private_field.vr` | Refuse access to private `PreparedPublication.publication`. |
| `publication_prepare_affine_reuse.vr` | Refuse the second consuming use of `prepared`. |

A dependency/type-resolution error or an unrelated crash is inconclusive,
not the required refusal. Exact diagnostic identities must be recorded from
the pinned current compiler. The separate public preparation runtime fixture
constructs preparation only through real credential and schema checks; the
private-state isolation component deliberately injects a test helper and
cannot establish external visibility enforcement.
