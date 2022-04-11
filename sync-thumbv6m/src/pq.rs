//! A constant size, no_std, lock free, multi-producer, single-consumer (mpsc) queue.
//! Because I couldn't find one that checked all the boxes.
//!
//! Adapted from https://www.codeproject.com/articles/153898/yet-another-implementation-of-a-lock-free-circul
//!
//! Stored values <T> must impl Clone
//! Actual capacity is N - 1 e.g. an ArrayQueue<5> can only hold 4 elements

use heapless::Vec;

pub type Priority = u32;

struct Node<T> {
    priority: Priority,
    data: T,
}

pub struct PriorityQueue<T, const N: usize> {
    queue: Vec<Node<T>, N>,
    // delegate sorting to consumer
    should_sort: bool,
}

impl<T: Clone, const N: usize> PriorityQueue<T, N> {
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            should_sort: false,
        }
    }

    #[must_use]
    pub fn push(&mut self, priority: Priority, item: &T) -> bool {
        let node = Node {
            priority,
            data: item.clone(),
        };

        if let Ok(_) = self.queue.push(node) {
            self.should_sort = true;
            true
        } else {
            false
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        self.next(|v| unsafe { v.pop_unchecked() }.data)
    }

    pub fn peek_priority(&mut self) -> Option<Priority> {
        self.next(|v| unsafe { v.get_unchecked(v.len() - 1) }.priority)
    }

    fn next<O, F: Fn(&mut Vec<Node<T>, N>) -> O>(&mut self, op: F) -> Option<O> {
        match self.queue.len() {
            0 => return None,
            2 if self.should_sort => {
                self.queue.sort_by(|i, j| i.priority.cmp(&j.priority).reverse());
                self.should_sort = false;
            }
            _ => {}
        }
        Some(op(&mut self.queue))
    }
}

#[cfg(test)]
mod test {
    use crate::pq::PriorityQueue;

    #[test]
    fn single_item() {
        let mut queue: PriorityQueue<i32, 1> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert_eq!(Some(2), queue.peek_priority());
        assert_eq!(Some(1), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn two_items_eq() {
        let mut queue: PriorityQueue<i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(2, &2));
        assert_eq!(Some(2), queue.pop());
        assert_eq!(Some(1), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn two_items_ordered() {
        let mut queue: PriorityQueue<i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(4, &2));
        assert_eq!(Some(1), queue.pop());
        assert_eq!(Some(2), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn two_items_rev() {
        let mut queue: PriorityQueue<i32, 2> = PriorityQueue::new();
        assert!(queue.push(4, &1));
        assert!(queue.push(2, &2));
        assert_eq!(Some(2), queue.pop());
        assert_eq!(Some(1), queue.pop());
        assert_eq!(None, queue.pop());
    }

    #[test]
    fn peek_items_rev() {
        let mut queue: PriorityQueue<i32, 2> = PriorityQueue::new();
        assert!(queue.push(4, &1));
        assert!(queue.push(2, &2));
        assert_eq!(Some(2), queue.peek_priority());
        assert_eq!(Some(2), queue.peek_priority());
    }

    #[test]
    fn peek_items_fwd() {
        let mut queue: PriorityQueue<i32, 2> = PriorityQueue::new();
        assert!(queue.push(2, &1));
        assert!(queue.push(4, &2));
        assert_eq!(Some(2), queue.peek_priority());
        assert_eq!(Some(2), queue.peek_priority());
    }
}