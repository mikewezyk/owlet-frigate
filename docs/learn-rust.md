# Learn Rust with this project

Each lesson reads real code here, then changes it. Work on a branch: `git switch -c learning`.
Say "lesson 1" to Claude and work through them in order.

| # | Idea | Read | Try |
|---|---|---|---|
| 1 | Cargo, functions, `&` borrowing, tests | `src/sha256.rs` (`sha256`) | Add a test for the digest of `"abc"` |
| 2 | `enum`, `match`, exhaustiveness | `src/iotc/header.rs` (`Opcode`) | Delete a match arm in the CLI's `Command`; read the error |
| 3 | `Option`, `?`, safe parsing | `Packet::decode` | Test that a truncated packet returns `None` |
| 4 | `Result` and errors | CLI `seconds`, `auth_key` | Add a test for `--timeout 0` |
| 5 | Ownership and lifetimes | `Fragment<'a>` in `src/av/packet.rs` | Explain why `bytes` is borrowed, not copied |
| 6 | Structs and state | `Reassembly` in `src/av/frame.rs` | Test frames that arrive out of order |
| 7 | Traits | `Datagram` in `src/lib.rs` | Write a `Datagram` that drops every third packet |
| 8 | Threads, `Arc`, channels | `serve` in `src/device.rs` | Trace who owns each `Arc` |
| 9 | Security by design | `src/dtls.rs` and its tests | Remove one defence; see which test fails |

## Three ideas to hold on to

- **Ownership:** each value has one owner. When the owner goes away, the value is freed.
- **Borrowing:** `&x` lends a value for reading, `&mut x` for changing. Never both at once.
- **No null, no exceptions:** "maybe missing" is `Option`, "may fail" is `Result`, and the
  compiler makes you handle both. `?` means "on failure, return it to my caller".

## Loop

```sh
cargo check     # fast: does it compile?
cargo test      # does it work?
cargo clippy    # is there a better way to write it?
cargo fmt       # standard layout
```

Read compiler errors from the top. `rustc --explain E0382` explains any error code.
