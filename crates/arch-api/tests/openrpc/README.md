# Vendored meta-schemas

`arch_api_schema_is_an_openrpc_1_3_document` (`tests/rpc.rs`) validates `schemas/arch-api.json` against these
two files, offline.

| file | source | license |
|---|---|---|
| `open-rpc-meta-schema-1.14.9.json` | npm `@open-rpc/meta-schema` 1.14.9, the `openrpcDocument` export (OpenRPC 1.3.2) | Apache-2.0 |
| `json-schema-tools-meta-schema-1.8.0.json` | npm `@json-schema-tools/meta-schema` 1.8.0, `schema.json`; the first file refers to it as `https://meta.json-schema.tools` | Apache-2.0 |
