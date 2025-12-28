#![no_std]

//! The main struct of this crate owns the same data twice. One for reading and one for writing. It is inspired by the [left-right](https://crates.io/crates/left-right). This crate works in a `no_std` environment by using the lock mechanisms of [spin](https://crates.io/crates/spin).
//!
//! This struct is meant to be used as Single Producer Multiple Consumer (SPMC). Think of it as a communication agent from a lower priority task to a higher priority task. As this is a design without a queue, the last writer wins.
//!
//! # Assumptions
//! - The implementaion assumes a single core environment.
//! - The writer thread shall never interrupt a reader thread.
//! - There is only 1 writer at the same time.
//!
//! # Guarantees
//! - Simultaneous readers can coexist safely
//! - Potential deadlock situations (which can only occur if the assumptions were violated) directly implement a panic!
//!
use core::sync::atomic::{AtomicBool, Ordering};
use spin::{RwLock, RwLockReadGuard, RwLockWriteGuard};

const READ_LEFT: bool = false;
const READ_RIGHT: bool = true;
const WRITE_LEFT: bool = READ_RIGHT;
const WRITE_RIGHT: bool = READ_LEFT;

/// The main struct of this crate.
pub struct LeftRightBuffer<T> {
    left: RwLock<T>,
    right: RwLock<T>,

    // true means reading happens on right and writing on the left
    // false means reading happens on left and writing on the right
    direction: AtomicBool,
    has_been_published: AtomicBool,
}

impl<T: Copy> LeftRightBuffer<T> {
    pub const fn new(default: T) -> LeftRightBuffer<T> {
        LeftRightBuffer {
            left: RwLock::new(default),
            right: RwLock::new(default),
            direction: AtomicBool::new(false),
            has_been_published: AtomicBool::new(false),
        }
    }

    /// returns a read guard.
    ///
    /// Under the circumstance that read gets called between publish() and the drop of the write mutex, it shall return the old value.
    /// The risk of this circumstance gets minimized by the fact that publish() will drop the write mutex itself if used correctly.
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        match self.direction.load(Ordering::Relaxed) {
            READ_RIGHT => match self.right.try_read() {
                Some(thing) => thing,
                None => self.left.read(), // the special circumstance
            },
            READ_LEFT => match self.left.try_read() {
                Some(thing) => thing,
                None => self.right.read(), // the special circumstance
            },
        }
    }

    /// returns a write guard
    ///
    /// The first call of this function after a publish syncs the 'last written data' to the 'to be written' data.
    /// This enables easy modification of partial data.
    ///
    /// # Safety
    ///  `write` shall only be called from the lower priority task, otherwise it might panic as this could violate the assumptions.

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        if self.has_been_published.load(Ordering::Relaxed) {
            self.sync();
            self.has_been_published.store(false, Ordering::Relaxed);
        }
        match self.direction.load(Ordering::Relaxed) {
            WRITE_LEFT => match self.left.try_write() {
                Some(thing) => thing,
                None => panic!("LRBuffer write1"), // wrong usage as there is already a writer.
            },
            WRITE_RIGHT => match self.right.try_write() {
                Some(thing) => thing,
                None => panic!("LRBuffer write2"), // wrong usage as there is already a writer.
            },
        }
    }

    /// returns a write guard
    ///
    /// # Safety
    ///  `write` shall only be called from the lower priority task, otherwise it might panic as this could violate the assumptions.
    pub fn write_without_sync(&self) -> RwLockWriteGuard<'_, T> {
        match self.direction.load(Ordering::Relaxed) {
            WRITE_LEFT => match self.left.try_write() {
                Some(thing) => thing,
                None => panic!("LRBuffer write1"), // wrong usage as there is already a writer.
            },
            WRITE_RIGHT => match self.right.try_write() {
                Some(thing) => thing,
                None => panic!("LRBuffer write2"), // wrong usage as there is already a writer.
            },
        }
    }

    /// syncs the data between left & right
    fn sync(&self) {
        match self.direction.load(Ordering::Relaxed) {
            WRITE_LEFT => {
                let old_data = match self.right.try_read() {
                    Some(thing) => thing,
                    None => panic!("LRBuffer sync1"),
                };
                let mut new_data = match self.left.try_write() {
                    Some(thing) => thing,
                    None => panic!("LRBuffer sync2"),
                };
                *new_data = *old_data;
            }
            WRITE_RIGHT => {
                let old_data = match self.left.try_read() {
                    Some(thing) => thing,
                    None => panic!("LRBuffer sync3"),
                };
                let mut new_data = match self.right.try_write() {
                    Some(thing) => thing,
                    None => panic!("LRBuffer sync4"),
                };
                *new_data = *old_data;
            }
        }
    }

    /// This method guarantees that the old writer is dropped before the new readers get active.
    /// For this to work correctly, the caller must transfer the correct Guard.
    pub fn publish(&self, writer: RwLockWriteGuard<'_, T>) {
        drop(writer);
        match self.direction.load(Ordering::Acquire) {
            true => self.direction.store(false, Ordering::Release),
            false => self.direction.store(true, Ordering::Release),
        }
        self.has_been_published.store(true, Ordering::Relaxed);
    }

    /// # DO NOT USE!
    /// only inside for educational purpose.
    #[deprecated(note = "please use `publish(&self, writer: RwLockWriteGuard<'_, T>)` instead")]
    fn _old_publish(&self) {
        match self.direction.load(Ordering::Acquire) {
            true => self.direction.store(false, Ordering::Release),
            false => self.direction.store(true, Ordering::Release),
        }
        self.has_been_published.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spin::Mutex;

    // the mutex is only here to simulate the assumption of a single core.
    static LR_BUFFER: Mutex<LeftRightBuffer<u32>> = Mutex::new(LeftRightBuffer::new(0));

    fn assert_leftright_eq(global: &spin::MutexGuard<'_, LeftRightBuffer<u32>>, cmp: u32) {
        let foo = global.read();
        assert_eq!(*foo, cmp);
    }

    #[test]
    fn test_autosync() {
        let global = LR_BUFFER.lock();
        {
            // Low Prio Task 1
            let mut foo = global.write();
            *foo = 5;
            global.publish(foo);
        }
        {
            // Low Prio Task 2
            let foo = global.write();
            assert_eq!(*foo, 5);
        }
    }

    #[test]
    fn test_interruption() {
        let global = LR_BUFFER.lock();
        {
            // Low Prio Task
            let mut foo = global.write();
            *foo = 1;
            global.publish(foo);
        }
        assert_leftright_eq(&global, 1);
        {
            // Low Prio Task
            let mut foo = global.write();
            *foo = 2;
            {
                // Interruption from High Prio Task. High Prio Task reads old value
                assert_leftright_eq(&global, 1);
            }
            global.publish(foo);
        }
        assert_leftright_eq(&global, 2);
    }

    #[test]
    #[should_panic(expected = "LRBuffer write")] //depending on the circumstances, it could be "LRBuffer write1" or "LRBuffer write2"
    fn test_second_writer_panics() {
        let global = LR_BUFFER.lock();
        {
            // Low Prio Task
            let mut foo = global.write();
            {
                // Interruption from High Prio Task. High Prio Task tries to write as well. This will panic as this violates the assumptions.
                let _ = global.write();
            }
            *foo = 1;
            global.publish(foo);
        }
    }

    #[test]
    fn test_interruption2() {
        let global = LR_BUFFER.lock();
        {
            // Low Prio Task
            let mut foo = global.write();
            *foo = 11;
            global.publish(foo);
        }
        {
            // Low Prio Task
            let mut foo = global.write();
            *foo = 1;
            global.publish(foo);
            assert_leftright_eq(&global, 1);
        }
    }

    #[test]
    fn test_old_publish_method() {
        let global = LR_BUFFER.lock();
        {
            // Low Prio Task
            let mut foo = global.write();
            *foo = 11;
            global.publish(foo);
        }
        {
            // Low Prio Task
            let mut foo = global.write();
            *foo = 1;
            #[allow(deprecated)]
            global._old_publish();
            assert_leftright_eq(&global, 11); // still the old value. This is described in the read function.
        }
    }
}
