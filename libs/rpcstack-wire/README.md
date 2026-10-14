# rpcstack-wire

The layout of the header that carries a message's module wire data, and the
primitives to read and write it. A leaf crate: it depends only on `serde`,
`bincode` and `base64`, so a crate that cannot depend on tonic or http can
still read one section.

```text
ctx: <name> : <base64 bincode> [ . <name> : <base64 bincode> ]*
```

Each section is one module's wire data. Neither `.` nor `:` is in the base64
alphabet and section names may not contain them, so the value is split with
plain string searches and one section is found without decoding any other.

Public surface:

- `HEADER_NAME`: the header's name, `ctx`.
- `SECTION_SEPARATOR`, `NAME_SEPARATOR`.
- `sections(header)`: the undecoded `(name, payload)` pairs.
- `find_section(header, name)`: one undecoded payload.
- `decode_payload::<W>(payload)` and `encode_payload(&wire)`: `bincode` (little
  endian, variable-length integers, trailing bytes rejected) then base64, with
  an error that says which step failed. Because `bincode` does not describe
  itself, a wire type writes every field every time: an `Option` for "may be
  absent", never `skip_serializing_if` or `default`.
- `encode_payload_with(&wire, |payload| ..)`: the same, handing the ASCII
  payload to a closure so the caller chooses where it goes (payloads of up to
  192 serialized bytes are encoded on the stack).
- `describe(header)`: each section's name and bytes in hex, or why it is not
  base64, for reading a header by eye.
- `decode_payload_into(payload, &mut buf)`: the same bytes as the base64 half of
  `decode_payload`, without allocating; `None` if it does not handle the payload
  (invalid, or too long for `buf`), which `decode_payload` then explains.
- `push_section(&mut header, name, payload)`.

`rpcstack` builds the typed module API (`WireIn`, `WireOut`) on these.

`SCHEDULING_HEADER` (`x-rpcstack-scheduling`) carries an optional decimal `u64`
scheduling key. The Masa-enabled Hyper executor maps this opaque key to task
metadata before the first task enqueue, without decoding a policy module's
private schema. The run queue defines its meaning. Missing/invalid keys retain
the existing Masa context fallback. Modules set the header on outbound requests;
this mechanism does not select a queue or supply preemption.
