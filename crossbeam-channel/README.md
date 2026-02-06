# Crossbeam Channel

[![Build Status](https://github.com/crossbeam-rs/crossbeam/workflows/CI/badge.svg)](
https://github.com/crossbeam-rs/crossbeam/actions)
[![License](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg)](
https://github.com/crossbeam-rs/crossbeam/tree/master/crossbeam-channel#license)
[![Cargo](https://img.shields.io/crates/v/crossbeam-channel.svg)](
https://crates.io/crates/crossbeam-channel)
[![Documentation](https://docs.rs/crossbeam-channel/badge.svg)](
https://docs.rs/crossbeam-channel)
[![Rust 1.60+](https://img.shields.io/badge/rust-1.60+-lightgray.svg)](
https://www.rust-lang.org)
[![chat](https://img.shields.io/discord/569610676205781012.svg?logo=discord)](https://discord.com/invite/JXYwgWZ)

This crate provides multi-producer multi-consumer channels for message passing.
It is an alternative to [`std::sync::mpsc`] with more features and better performance.

Some highlights:

* [`Sender`]s and [`Receiver`]s can be cloned and shared among threads.
* Two main kinds of channels are [`bounded`] and [`unbounded`].
* Convenient extra channels like [`after`], [`never`], and [`tick`].
* The [`select!`] macro can block on multiple channel operations.
* [`Select`] can select over a dynamically built list of channel operations.
* Channels use locks very sparingly for maximum [performance](benchmarks).
* Experimental [`wait_free_bounded`] supports fixed-topology non-blocking messaging.

[`std::sync::mpsc`]: https://doc.rust-lang.org/std/sync/mpsc/index.html
[`Sender`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/struct.Sender.html
[`Receiver`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/struct.Receiver.html
[`bounded`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/fn.bounded.html
[`unbounded`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/fn.unbounded.html
[`after`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/fn.after.html
[`never`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/fn.never.html
[`tick`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/fn.tick.html
[`wait_free_bounded`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/fn.wait_free_bounded.html
[`select!`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/macro.select.html
[`Select`]: https://docs.rs/crossbeam-channel/latest/crossbeam_channel/struct.Select.html

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
crossbeam-channel = "0.5"
```

## Experimental wait-free API

For fixed-topology systems that only need non-blocking operations, the crate provides
[`wait_free_bounded`].

This API has explicit trade-offs:

* Sender count is fixed up front and each sender must register a slot.
* Capacity is preallocated per sender slot.
* Only `try_send` and `try_recv` are supported.
* Blocking operations and `select!` are intentionally unsupported.

```rust
use std::thread;
use crossbeam_channel::{wait_free_bounded, TryRecvError, TrySendError};

let (registry, mut receiver) = wait_free_bounded::<u64>(2, 32);
let sender0 = registry.register().unwrap();
let sender1 = registry.register().unwrap();
registry.close();

thread::spawn(move || {
    for value in 0..100 {
        let mut pending = value;
        loop {
            match sender0.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(v)) => {
                    pending = v;
                    thread::yield_now();
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    }
});

let mut received = 0;
while received < 100 {
    match receiver.try_recv() {
        Ok(_) => received += 1,
        Err(TryRecvError::Empty) => thread::yield_now(),
        Err(TryRecvError::Disconnected) => break,
    }
}
```

See [`examples/wait_free.rs`](examples/wait_free.rs) for a complete runnable example.

## Compatibility

Crossbeam Channel supports stable Rust releases going back at least six months,
and every time the minimum supported Rust version is increased, a new minor
version is released. Currently, the minimum supported Rust version is 1.60.

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

#### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

#### Third party software

This product includes copies and modifications of software developed by third parties:

* [examples/matching.rs](examples/matching.rs) includes
  [matching.go](http://www.nada.kth.se/~snilsson/concurrency/src/matching.go) by Stefan Nilsson,
  licensed under Creative Commons Attribution 3.0 Unported License.

* [tests/mpsc.rs](tests/mpsc.rs) includes modifications of code from The Rust Programming Language,
  licensed under the MIT License and the Apache License, Version 2.0.

* [tests/golang.rs](tests/golang.rs) is based on code from The Go Programming Language, licensed
  under the 3-Clause BSD License.

See the source code files for more details.

Copies of third party licenses can be found in [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY).
