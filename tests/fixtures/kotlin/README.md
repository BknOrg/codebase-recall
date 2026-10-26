# Kotlin grammar regression

`uniffi-transaction.kt` is an 85-line syntax reduction of the MIT-licensed BknDb
UniFFI binding at commit `509f9414adc0e312354965b584cd3e165d02a08e`.
It deliberately omits semantic definitions; this tests grammar, not compilation.
Source: `bindings/uniffi/bkndb_ffi/bkndb_ffi.kt` in BknOrg/bkn-db.
The original license is preserved in `../retrieval_bkndb/LICENSE`.

The raw tree-sitter-kotlin-ng grammar reports errors; masking complete block
comments while preserving bytes/newlines parses the same declarations cleanly.
Tests verify original symbol byte/line positions and retain syntax-error checks.
