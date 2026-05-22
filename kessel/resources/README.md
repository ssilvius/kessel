# kessel/resources

Static dictionaries baked into the kessel binary via `include_str!`.

| File | Source | Records | Size |
|---|---|---:|---:|
| `gom_enums.json` | `/resources/systemgenerated/client.gom` enum records (`first_str_off=0x1E`) | 748 | ~289 KB |
| `gom_classes.json` | `client.gom` class records (`first_str_off=0x34`) | 2,220 | ~409 KB |
| `gom_properties.json` | `client.gom` property records (`first_str_off=0x20`) | 10,006 | ~678 KB |

## Regenerating

After a SWTOR patch (new client.gom):

```bash
# 1. Extract + decode client.gom via the discovery-crate binary
cargo run --release -p kessel-discovery --bin extract_client_gom_final -- \
    -i ~/swtor/Assets -H ~/swtor/data/hashes_filename.txt -o /tmp/

# 2. Minify into kessel/resources/ -- see scripts/regen_gom_schema.py
#    (or rerun the inline python from issue #124 / PR #N50)
```

Loaded lazily by `kessel::gom_schema` via `OnceLock`. Production code uses
the module-level helpers (`property_for_cf40`, `class_for_type_hi32`,
`enum_for_hash`) rather than `schema()` directly.
