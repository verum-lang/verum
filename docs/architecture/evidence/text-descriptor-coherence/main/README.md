# Main-based archive and Text integration checks

The isolated main candidate at `aa7974646` passes all nine compiler archive
function-identity controls in 205.462 seconds and all 19 Text descriptor
controls in 30.198 seconds. The selected tests have no failures or ignores;
the compiler command filters out unrelated compiler tests.

The [independent review](review.json) verifies 10,600 source-at-commit
bindings, both retained executables and exact logs. The compiler command
explicitly reuses the five unchanged Stage23 standard-library artifacts;
the Text target exercises parsed source and wire imports without embedding
them. These checks do not establish ordinary CLI consumer, fresh bake,
whole-registry or AOT acceptance.
