//! A constant size, no_std, lock free, multi-producer, single-consumer (mpsc) queue.
//! Because I couldn't find one that checked all the boxes.
//!
//! Adapted from https://www.codeproject.com/articles/153898/yet-another-implementation-of-a-lock-free-circul
//!
//! Stored values <T> must impl Clone
//! Actual capacity is N - 1 e.g. an ArrayQueue<5> can only hold 4 elements

use heapless::Vec;

struct Node<P, T> {
    priority: P,
    data: T,
}

pub struct PriorityQueue<P, T, const N: usize> {
    queue: Vec<Node<P, T>, N>,
}

impl<P: Ord + Copy, T: Clone, const N: usize> PriorityQueue<P, T, N> {
    pub const fn new() -> Self {
        Self {
            queue: Vec::new(),
        }
    }

    #[must_use]
    pub fn push(&mut self, priority: P, item: &T) -> bool {
        let node = Node {
            priority,
            data: item.clone(),
        };

        if let Ok(_) = self.queue.push(node) {
            self.queue.sort_by(|i, j| i.priority.cmp(&j.priority).reverse());
            true
        } else {
            false
        }
    }

    // pub fn pop(&mut self) -> Option<T> {
    //     self.queue.pop().map(|node| node.data)
    // }

    pub fn pop_due(&mut self, now: P) -> Option<T> {
        if let Some(sched_time) = self.peek_priority() {
            if  now > sched_time {
                unsafe { return Some(self.queue.pop_unchecked().data); }
            }
        }
        None
    }

    pub fn peek_priority(&mut self) -> Option<P> {
        if self.queue.is_empty() {
            None
        } else {
            self.queue.get(self.queue.len() - 1).map(|node| node.priority)
        }
    }
}

#[cfg(test)]
mod test {
    use super::PriorityQueue;

    #[test]
    fn single_item() {
        let mut queue: PriorityQueue<u32, i32, 1> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert_eq!(Some(2), queue.peek_priority());
        assert_eq!(Some(1), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn two_items_eq() {
        let mut queue: PriorityQueue<u32, i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(2, &2));
        assert_eq!(Some(2), queue.pop());
        assert_eq!(Some(1), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn two_items_ordered() {
        let mut queue: PriorityQueue<u32, i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(4, &2));
        assert_eq!(Some(1), queue.pop());
        assert_eq!(Some(2), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn two_items_rev() {
        let mut queue: PriorityQueue<u32, i32, 2> = PriorityQueue::new();
        assert!(queue.push(4, &1));
        assert!(queue.push(2, &2));
        assert_eq!(Some(2), queue.pop());
        assert_eq!(Some(1), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn peek_items_rev() {
        let mut queue: PriorityQueue<u32, i32, 2> = PriorityQueue::new();
        assert!(queue.push(4, &1));
        assert!(queue.push(2, &2));
        assert_eq!(Some(2), queue.peek_priority());
        assert_eq!(Some(2), queue.peek_priority());
    }

    #[test]
    fn peek_items_fwd() {
        let mut queue: PriorityQueue<u32, i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(4, &2));
        assert_eq!(Some(2), queue.peek_priority());
        assert_eq!(Some(2), queue.peek_priority());
    }

    #[test]
    fn pop_due() {
        let mut queue: PriorityQueue<u32, i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(4, &2));
        assert_eq!(None, queue.pop_due(1));
        assert_eq!(Some(1), queue.pop_due(2));
        assert_eq!(None, queue.pop_due(3));
        assert_eq!(Some(2), queue.pop_due(4));
    }
}