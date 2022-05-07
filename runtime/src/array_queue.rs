//! A constant size, no_std, lock free, multi-producer, multi-consumer (mpmc) queue.
//! A.K.A ring buffer, circular thingie.
//! Because I couldn't find a crate that checked all the boxes.
//!
//! Adapted from https://www.codeproject.com/articles/153898/yet-another-implementation-of-a-lock-free-circul
//!
//! Stored values <T> must impl Clone
//! Actual capacity is N - 1 e.g. an ArrayQueue<5> can only hold 4 elements

#[allow(dead_code)]

use core::sync::atomic::Ordering::Relaxed;
use atomic_polyfill::AtomicUsize;

#[derive(Debug)]
pub struct ArrayQueue<T, const N: usize> {
    /// FIXME maybe Rust allows type param arithmetic?
    ///   we could internally allocate a buffer of N + 1 elements for clearer API capacity expectations
    ///   #![feature(generic_const_exprs)] does not seem to allow [T; N + 1] (yet? else how?)
    buffer: [Option<T>; N],
    write_idx: AtomicUsize,
    read_idx: AtomicUsize,
    max_read_idx: AtomicUsize,
}

impl<T: Clone, const N: usize> ArrayQueue<T, N> {
    pub const fn new(blank: [Option<T>; N]) -> Self {
        Self {
            write_idx: AtomicUsize::new(0),
            read_idx: AtomicUsize::new(0),
            max_read_idx: AtomicUsize::new(0),
            buffer: blank,
        }
    }

    fn count_to(&self, a_count: usize) -> usize {
        a_count % N
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        N - 1
    }

    #[cfg(test)]
    pub fn size(&self) -> usize {
        let max_idx = self.max_read_idx.load(Relaxed);
        let read_idx = self.read_idx.load(Relaxed);
        max_idx - read_idx
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        let max_idx = self.max_read_idx.load(Relaxed);
        let read_idx = self.read_idx.load(Relaxed);
        read_idx == max_idx
    }

    #[inline]
    fn is_full(&self, write_idx: usize, read_idx: usize) -> bool {
        self.count_to(write_idx + 1) == self.count_to(read_idx)
    }

    #[must_use]
    pub fn push(&self, a_data: T) -> Result<(), ()> {
        let mut write_idx;
        let mut read_idx;

        loop {
            read_idx = self.read_idx.load(Relaxed);
            write_idx = self.write_idx.load(Relaxed);

            if self.is_full(write_idx, read_idx) {
                return Err(());
            }
            if let Ok(_) = self.write_idx.compare_exchange(write_idx, write_idx + 1, Relaxed, Relaxed) { break; }
        }

        // this is safe because element at write_idx is now reserved for us
        let as_mut = unsafe { &mut *(self.buffer.as_ptr() as *mut [Option<T>; N]) };
        assert!(as_mut[self.count_to(write_idx)].is_none());
        as_mut[self.count_to(write_idx)].replace(a_data);

        while self.max_read_idx.compare_exchange(write_idx, write_idx + 1, Relaxed, Relaxed).is_err() {
            // async version of queue would yield here
        }
        Ok(())
    }


    pub fn pop(&self) -> Option<T> {
        loop {
            // to ensure thread-safety when there is more than one producer thread
            let max_idx = self.max_read_idx.load(Relaxed);
            let read_idx = self.read_idx.load(Relaxed);

            if self.count_to(read_idx) == self.count_to(max_idx) {
                // buffer empty or any pending writes not committed yet
                return None;
            }

            // try reserving read index
            if self.read_idx.compare_exchange(read_idx, read_idx + 1, Relaxed, Relaxed).is_ok() {
                let as_mut: &mut [Option<T>; N] = unsafe { &mut *(self.buffer.as_ptr() as *mut [Option<T>; N]) };
                // assert!(as_mut[self.count_to(read_idx)].is_some());
                return as_mut[self.count_to(read_idx)].take();
            }
            // failed reserving read index, try again
        }
    }
}

/// TODO concurrency testing (yeeaaah...)
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn overfill() {
        let queue: ArrayQueue<i32, 3> = ArrayQueue::new();
        assert_eq!(2, queue.capacity());
        assert_eq!(0, queue.size());
        assert_eq!(Ok(()), queue.push(1));
        assert_eq!(1, queue.size());
        assert_eq!(Ok(()), queue.push(2));
        assert_eq!(2, queue.size());

        let z = queue.push(3);
        assert!(z.is_err());
        assert_eq!(Err(()), queue.push(3));
        assert_eq!(2, queue.size());
    }

    #[test]
    fn push_pop() {
        let queue: ArrayQueue<i32, 5> = ArrayQueue::new();
        assert_eq!(0, queue.size());
        assert!(queue.is_empty());
        for _ in 0..8 {
            assert_eq!(Ok(()), queue.push(1));
            assert!(!queue.is_empty());
            assert_eq!(Ok(()), queue.push(2));
            assert_eq!(Ok(()), queue.push(3));

            assert_eq!(3, queue.size());
            assert_eq!(Some(1), queue.pop());
            assert_eq!(Some(2), queue.pop());
            assert!(!queue.is_empty());
            assert_eq!(Some(3), queue.pop());
            assert_eq!(0, queue.size());
            assert!(queue.is_empty());
        }
    }

    #[test]
    fn push_pop2() {
        let queue: ArrayQueue<i32, 5> = ArrayQueue::new();
        assert_eq!(0, queue.size());
        assert!(queue.is_empty());
        for _ in 0..8 {
            assert_eq!(Ok(()), queue.push(1));
            assert_eq!(Ok(()), queue.push(2));
            assert_eq!(Ok(()), queue.push(3));

            assert_eq!(Some(1), queue.pop());
            assert_eq!(Some(2), queue.pop());

            assert_eq!(Ok(()), queue.push(1));
            assert_eq!(Ok(()), queue.push(2));
            assert_eq!(Ok(()), queue.push(3));

            assert_eq!(Some(1), queue.pop());
            assert_eq!(Some(2), queue.pop());
            assert_eq!(Some(1), queue.pop());
            assert_eq!(Some(2), queue.pop());

            assert_eq!(0, queue.size());
            assert!(queue.is_empty());
        }
    }
}