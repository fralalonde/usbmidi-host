use alloc::boxed::Box;
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::task::{Context, Poll};

use sync_thumbv6m::alloc::Arc;
use sync_thumbv6m::array_queue::ArrayQueue;
use woke::{waker_ref, Woke};

use sync_thumbv6m::spin::SpinMutex;

static mut REACTOR: MaybeUninit<Arc<Reactor>> = MaybeUninit::uninit();

pub fn init() {
    unsafe { REACTOR = MaybeUninit::new(Arc::new(Reactor::new())) };
}

struct Task {
    pub future: SpinMutex<Option<Pin<Box<dyn Future<Output=()> + Send + 'static>>>>,
    pub reactor: Arc<Reactor>,
}

pub struct Reactor {
    exec_queue: Arc<ArrayQueue<Arc<Task>, 16>>,
}

impl Woke for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let cloned = arc_self.clone();
        arc_self.reactor.enqueue(&cloned);
    }
}

pub fn spawn(future: impl Future<Output=()> + 'static + Send) {
    let reactor = unsafe { REACTOR.assume_init_ref().clone() };
    let future = Box::pin(future);
    let task = Arc::new(Task {
        future: SpinMutex::new(Some(future)),
        reactor: reactor.clone(),
    });
    reactor.enqueue(&task)
}

pub fn process_queue() {
    unsafe { REACTOR.assume_init_ref().clone() }.process();
}


impl Reactor {
    pub fn new() -> Self {
        Self {
            exec_queue: Arc::new(ArrayQueue::new()),
        }
    }

    fn enqueue(&self, task: &Arc<Task>) {
        if self.exec_queue.push(task).is_err() { crate::warn!("Reactor queue full - is a task blocking?") }
    }

    pub fn process(&self) {
        if let Some(task) = self.exec_queue.pop() {
            let mut task_future = task.future.lock();
            if let Some(mut future) = task_future.take() {
                let waker = waker_ref(&task);
                let context = &mut Context::from_waker(&*waker);
                if let Poll::Pending = future.as_mut().poll(context) {
                    *task_future = Some(future);
                }
            } else {
                crate::warn!("NO FUTURE")
            }
        }
    }
}
