# grpcping

A `grpcurl` equivalent for Rust: lists a gRPC server's services, describes
their methods, and makes real unary calls with a JSON payload — all via
server reflection, with **no generated client stub**. Go has `grpcurl`;
Rust's ecosystem has excellent gRPC *libraries* (`tonic`) but nothing
filling `grpcurl`'s specific job of poking an arbitrary running service
from a terminal without writing a client first.

## Installation

```sh
cargo install --path .
```

## Usage

```bash
grpcping list localhost:50051
grpcping describe localhost:50051 greeter.Greeter
grpcping call localhost:50051 greeter.Greeter/SayHello -d '{"name": "World"}'
```

```
$ grpcping call 127.0.0.1:50077 greeter.Greeter/SayHello -d '{"name": "World"}'
{
  "message": "Hello, World!",
  "createdAt": "2023-11-14T22:13:20Z"
}
```

Try it against a real (tiny) server bundled in this repo:

```bash
cargo run --example greeter_server &
grpcping list 127.0.0.1:50077
grpcping call 127.0.0.1:50077 greeter.Greeter/SayHello -d '{"name": "World"}'
```

The target server must have gRPC [server reflection](https://github.com/grpc/grpc/blob/master/doc/server-reflection.md)
enabled (in Go: `reflection.Register(s)`; in `tonic`:
`tonic_reflection::server::Builder`). Plaintext only in this v1 — no TLS.

## How it works

There's no `.proto` file, no `protoc`, no generated Rust types for
*your* service anywhere in this tool. Instead:

1. **Reflection** (`src/reflection.rs`) asks the server for the raw
   `FileDescriptorProto` bytes that declare the requested service, then
   walks that file's `dependency` list and fetches anything not yet
   known — breadth-first, over more reflection calls — until every
   transitively-imported type is resolved. This step is load-bearing:
   the reflection protocol only ever answers with the *one* file that
   directly declares whatever was asked for, never the transitive
   closure (confirmed by reading `tonic-reflection`'s own server
   implementation), so a service using any import at all — even just
   `google.protobuf.Timestamp` — requires this walk to work at all.
2. Those files build a [`prost-reflect`](https://crates.io/crates/prost-reflect)
   `DescriptorPool` — a runtime, no-codegen model of the service's
   real message shapes.
3. A JSON request body is deserialized directly into a
   [`DynamicMessage`](https://docs.rs/prost-reflect/latest/prost_reflect/struct.DynamicMessage.html)
   matching the method's real input type — a typo'd field name or
   wrong JSON type fails here, before anything reaches the network.
4. The call itself goes out through `tonic`'s low-level
   `tonic::client::Grpc` API (the same primitive generated clients are
   built on top of) paired with a hand-written `Codec`
   (`src/codec.rs`) for `DynamicMessage` in place of the generated
   `ProstCodec` — this is the one piece with no off-the-shelf crate to
   reach for, since `DynamicMessage` has no `Default` impl (there's no
   such thing as a message with no descriptor), so the response
   descriptor is baked into the decoder instead of relying on the
   `T::default()` trick tonic's own codec uses.
5. The response comes back as another `DynamicMessage`, which
   `prost-reflect`'s `serde` feature turns directly into canonical
   proto3 JSON (camelCase field names, RFC 3339 timestamps, etc.) —
   the `"createdAt": "2023-11-14T22:13:20Z"` in the example output
   above is that canonical mapping working correctly on a real
   `google.protobuf.Timestamp` field, unprompted.

## Status: built and verified against a real running gRPC server, with a real parallel-test bug caught and fixed

- **5 integration tests**, all against a real `tonic` server (with real
  reflection enabled) spawned in-process — no mocks anywhere, since
  the entire point of this tool is behavior a mock couldn't stand in
  for:
  - `list` finds the real `greeter.Greeter` service (and the
    reflection service itself) via reflection.
  - `describe`-equivalent resolution finds both of the service's real
    methods (`SayHello`, `SayGoodbye`).
  - **The transitive-dependency walk is specifically exercised**: the
    test fixture's `HelloReply.created_at` field is a
    `google.protobuf.Timestamp`, declared in a file the server never
    includes in its first reflection response (see "How it works"
    above) — successfully resolving that field's type at all is direct
    proof the second reflection round trip for
    `google/protobuf/timestamp.proto` actually happened, not just that
    the happy path with zero imports works.
  - A full dynamic unary call end-to-end: real JSON in, real gRPC call
    over the wire, real JSON out, checked against the real string the
    server actually computed (`"Hello, World!"`), not a fixture.
  - A malformed request payload (a JSON number where the schema
    expects a string) fails cleanly during JSON→message conversion,
    before any network call — not a panic.
- **The compiled CLI binary itself was separately run against a real
  standalone server** (`examples/greeter_server.rs`, the same fixture
  minus the test harness) for all three subcommands — `list`,
  `describe`, and `call` (both methods) — plus two real error paths: a
  malformed `service/method` argument and a call to a method that
  doesn't exist on the resolved service. All matched expected output
  exactly, including the canonical-JSON `createdAt` timestamp
  formatting shown above.
- **A real bug this caught**: the first version of the test suite
  bound the fixture server to a fixed port (`127.0.0.1:50077`). Under
  `cargo test`'s default parallel execution, every test after the
  first failed with `AddrInUse` — confirmed live, not hypothetical.
  Fixed by binding an OS-assigned ephemeral port (`:0`) per test and
  reading back the real assigned address, rather than documenting
  "run with `--test-threads=1`" as a workaround. Re-verified stable
  across 3 repeated full-suite runs under default threading.

**Deliberate scope limits, stated plainly**: unary calls only — no
client-streaming, server-streaming, or bidi support in this v1; no TLS
(plaintext `http://` only); no support for servers that only implement
the older `grpc.reflection.v1alpha` reflection protocol (this targets
`v1`, which every modern gRPC server implementation supports
alongside or instead of `v1alpha`); extension fields
(`FileContainingExtension`) aren't resolved, matching the fact that
`tonic-reflection`'s own server doesn't support that request type
either. CI needs `protoc` installed (`apt-get install
protobuf-compiler`) since the test fixture's `.proto` is compiled at
build time — this is the one tool in this collection whose CI workflow
isn't the standard template for exactly that reason.
