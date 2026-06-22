# DevRelay Local RPC Compatibility Policy

Last updated: 2026-06-23

DevRelay local agent RPC uses JSON-RPC 2.0 over the local IPC transport. The
transport is local-machine only; compatibility rules here cover the JSON
request, response, method, and error contract.

## Version Negotiation

- Clients must call `rpc.negotiate` before depending on method availability.
- The current protocol version is `1`.
- A server must reject unsupported client protocol versions with
  `RPC_VERSION_MISMATCH` and include both client and server versions.
- A successful negotiation response lists supported method names.

## Request IDs

- Every request must include an ID.
- IDs may be strings or non-negative integers.
- Notifications are not supported in M2.
- Servers must echo the request ID unchanged in success and method-level error
  responses.
- Envelope parse and validation failures return a JSON-RPC error response with
  `id: null`.

## Additive Changes

- New methods may be added without increasing the protocol version.
- New optional params may be added without increasing the protocol version.
- New result fields may be added without increasing the protocol version.
- Clients must ignore unknown result fields.

## Breaking Changes

A protocol version increase is required before:

- Removing or renaming a method.
- Removing a request param that existing clients may send.
- Making an optional param required.
- Removing or changing the type of an existing result field.
- Changing error code meaning.

## Error Contract

- Standard JSON-RPC error codes are used for parse, request, method, params, and
  internal errors.
- DevRelay-specific server errors use the reserved server-error range.
- Error `message` values are stable summaries, while `data.detail` is diagnostic
  text and may change.
