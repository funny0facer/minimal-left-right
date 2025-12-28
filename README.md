The main struct of this crate owns the same data twice. One for reading and one for writing. It is inspired by the [left-right](https://crates.io/crates/left-right). This crate works in a `no_std` environment by using the lock mechanisms of [spin](https://crates.io/crates/spin).
This struct is meant to be used as Single Producer Multiple Consumer (SPMC). Think of it as a communication agent from a lower priority task to a higher priority task. As this is a design without a queue, the last writer wins.

# Assumptions
- The implementaion assumes a single core environment.
- The writer thread shall never interrupt a reader thread.
- There is only 1 writer at the same time.

# Guarantees
- Simultaneous readers can coexist safely
- Potential deadlock situations (which can only occur if the assumptions were violated) directly implement a panic!
