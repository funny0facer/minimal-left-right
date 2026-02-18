#![no_std]

//! The main struct of this crate owns the same data twice. One for reading and one for writing. 
//! It is loosely inspired by the [left-right](https://crates.io/crates/left-right) crate. 
//! This crate works in a `no_std` environment by using the lock mechanisms of the [spin](https://crates.io/crates/spin) crate.
//!
//! This struct is meant to be used as Single Producer Multiple Consumer (SPMC). Think of it as a communication agent from a lower priority task to a higher priority task. 
//!
//! # Assumptions
//! - The implementation assumes a single core environment.
//! - The writer thread shall never interrupt a reader thread.
//! - There is only 1 writer at the same time.
//!
//! # Guarantees
//! - Simultaneous readers can coexist safely
//! - Potential deadlock situations (which can only occur if the assumptions were violated) directly implement a panic! This is intentional to fail fast instead of failing in production.
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

    // True means reading happens on right and writing on the left
    // False means reading happens on left and writing on the right
    direction: AtomicBool,
    has_been_published: AtomicBool,
}

impl<T: Copy> LeftRightBuffer<T> {
    /// Generates a new [`LeftRightBuffer`] and takes the data.
    pub const fn new(data: T) -> LeftRightBuffer<T> {
        LeftRightBuffer {
            left: RwLock::new(data),
            right: RwLock::new(data),
            direction: AtomicBool::new(false),
            has_been_published: AtomicBool::new(false),
        }
    }

    /// Returns a read guard.
    ///
    /// Under the circumstance that read gets called between [`publish()`][LeftRightBuffer::publish] and the drop of the write mutex, it shall return the old value.
    /// The "risk" of this circumstance gets minimized by the fact that [`publish()`][LeftRightBuffer::publish] will drop the write mutex itself if used correctly.
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

    /// Returns a write guard
    ///
    /// The first call of this function after a publish syncs the 'last written data' to the 'to be written' data.
    /// This is only true if [`write_without_sync()`][LeftRightBuffer::write_without_sync] was not used in between.
    /// This function enables easy modification of partial data of T.
    ///
    /// # Panics
    /// This function shall only be called from the lower priority task, otherwise it might panic as this could violate the assumptions.
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

    /// Returns a write guard
    /// 
    /// Use this function instead of [`write()`][LeftRightBuffer::write], when you want to write T independent of the prior state of T.
    ///
    /// # Panics
    /// This function shall only be called from the lower priority task, otherwise it might panic as this could violate the assumptions.
    pub fn write_without_sync(&self) -> RwLockWriteGuard<'_, T> {
        self.has_been_published.store(false, Ordering::Relaxed);
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

    /// Syncs the data between left & right
    fn sync(&self) {
        match self.direction.load(Ordering::Relaxed) {
            WRITE_LEFT => {
                let Some(old_data) = self.right.try_read() else {
                    panic!("LRBuffer sync1")
                };
                let Some(mut new_data) = self.left.try_write() else {
                    panic!("LRBuffer sync2")
                };
                *new_data = *old_data;
            }
            WRITE_RIGHT => {
                let Some(old_data) = self.left.try_read() else {
                    panic!("LRBuffer sync3")
                };
                let Some(mut new_data) = self.right.try_write() else {
                    panic!("LRBuffer sync4")
                };
                *new_data = *old_data;
            }
        }
    }

    /// This method guarantees that the old writer is dropped before the new readers get active.
    /// For this to work correctly, the caller must transfer the correct guard.
    pub fn publish(&self, writer: RwLockWriteGuard<'_, T>) {
        drop(writer);
        if self.direction.load(Ordering::Acquire) {
            self.direction.store(false, Ordering::Release);
        } else {
            self.direction.store(true, Ordering::Release);
        }

        self.has_been_published.store(true, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn direction(&self) -> &AtomicBool {
        &self.direction
    }

    #[cfg(test)]
    fn has_been_published(&self) -> &AtomicBool {
        &self.has_been_published
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spin::Mutex;

    #[derive(Clone, Copy)]
    struct VeryComplexData{
        pub a: u32,
    }

    // The Mutex in this test is only used to simulate the assumption of a single core.
    static LR_BUFFER: Mutex<LeftRightBuffer<VeryComplexData>> = Mutex::new(LeftRightBuffer::new(VeryComplexData{a:0}));

    fn assert_leftright_eq(global: &spin::MutexGuard<'_, LeftRightBuffer<VeryComplexData>>, cmp: u32) {
        let foo = global.read();
        assert_eq!(foo.a, cmp);
    }

    #[test]
    fn test_autosync() {
        let global = LR_BUFFER.lock();
        {
            // Low Priority Task 1
            let mut foo = global.write();
            foo.a = 5;
            global.publish(foo);
        }
        {
            // Low Priority Task 2
            let foo = global.write();
            assert_eq!(foo.a, 5);
        }
    }

    #[test]
    #[should_panic(expected = "LRBuffer write")] //depending on the circumstances, it could be "LRBuffer write1" or "LRBuffer write2"
    fn a_second_writer_panics() {
        let global = LR_BUFFER.lock();
        {
            // Low Priority Task
            let mut foo = global.write();
            {
                // Interruption from High Priority Task. High Priority Task tries to write as well. This will panic as this violates the assumptions.
                let _ = global.write();
            }
            foo.a = 1;
            global.publish(foo);
        }
    }

    #[test]
    fn test_interruption_before_and_after_publish() {
        let global = LR_BUFFER.lock();
        {
            // Low Priority Task
            let mut foo = global.write();
            foo.a = 10;
            global.publish(foo);
        }
        {
            // Low Priority Task
            let mut foo = global.write();
            foo.a = 20;
            {
                // High Priority Task
                // before publish
                assert_leftright_eq(&global, 10);
            }
            global.publish(foo);
            {
                // High Priority Task
                // after publish
                assert_leftright_eq(&global, 20);
            }
        }
        {
            // High Priority Task
            // After Low Priority Task
            assert_leftright_eq(&global, 20);
        }
    }

    #[test]
    fn test_interruption_during_publish() {
        let global = LR_BUFFER.lock();
        {
            // Low Priority Task
            let mut foo = global.write();
            foo.a = 30;
            global.publish(foo);
        }
        {
            // Low Priority Task
            let mut foo = global.write();
            foo.a = 40;

            // simulate the interruption within "publish()"
            drop(foo);
            {
                // High Priority Task
                assert_leftright_eq(&global, 30);
            }

            if global.direction().load(Ordering::Acquire) {
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 30);
                }
                global.direction().store(false, Ordering::Release);
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 40);
                }
            } else {
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 30);
                }
                global.direction().store(true, Ordering::Release);
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 40);
                }
            }
            global.has_been_published().store(true, Ordering::Relaxed);
            {
                // High Priority Task
                assert_leftright_eq(&global, 40);
            }
        }
        assert_leftright_eq(&global, 40);
    }

    #[test]
    fn simulate_publish_without_drop() {
        let global = LR_BUFFER.lock();
        {
            // Low Priority Task
            let mut foo = global.write();
            foo.a = 50;
            global.publish(foo);
        }
        {
            // Low Priority Task
            let mut foo = global.write();
            foo.a = 60;

            if global.direction().load(Ordering::Acquire) {
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 50);
                }
                global.direction().store(false, Ordering::Release);
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 50);
                }
            } else {
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 50);
                }
                global.direction().store(true, Ordering::Release);
                {
                    // High Priority Task
                    assert_leftright_eq(&global, 50);
                }
            }
            global.has_been_published().store(true, Ordering::Relaxed);
            {
                // High Priority Task
                // still the old value!
                assert_leftright_eq(&global, 50);
            }
        }
        assert_leftright_eq(&global, 60);
    }
}
