//! Mutex (spin-like and blocking(sleep))

use super::UPSafeCell;
use crate::task::TaskControlBlock;
use crate::task::{block_current_and_run_next, suspend_current_and_run_next};
use crate::task::{current_task, wakeup_task};
use alloc::{collections::VecDeque, sync::Arc};

/// Mutex trait
pub trait Mutex: Sync + Send {
    /// Lock the mutex
    fn lock(&self, mutex_id: usize);
    /// Unlock the mutex
    fn unlock(&self, mutex_id: usize);
}

/// Spinlock Mutex struct
pub struct MutexSpin {
    locked: UPSafeCell<bool>,
}

impl MutexSpin {
    /// Create a new spinlock mutex
    pub fn new() -> Self {
        Self {
            locked: unsafe { UPSafeCell::new(false) },
        }
    }
}

impl Mutex for MutexSpin {
    /// Lock the spinlock mutex
    fn lock(&self, _mutex_id: usize) {
        trace!("kernel: MutexSpin::lock");
        loop {
            let mut locked = self.locked.exclusive_access();
            if *locked {
                drop(locked);
                suspend_current_and_run_next();
                continue;
            } else {
                *locked = true;
                return;
            }
        }
    }

    fn unlock(&self, _mutex_id: usize) {
        trace!("kernel: MutexSpin::unlock");
        let mut locked = self.locked.exclusive_access();
        *locked = false;
    }
}

/// Blocking Mutex struct
pub struct MutexBlocking {
    inner: UPSafeCell<MutexBlockingInner>,
    _id: Option<usize>,
}

pub struct MutexBlockingInner {
    locked: bool,
    wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl MutexBlocking {
    /// Create a new blocking mutex
    pub fn new() -> Self {
        trace!("kernel: MutexBlocking::new");
        Self {
            _id: None,
            inner: unsafe {
                UPSafeCell::new(MutexBlockingInner {
                    locked: false,
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }
}

impl Mutex for MutexBlocking {
    /// lock the blocking mutex
    fn lock(&self, _mutex_id: usize) {
        trace!("kernel: MutexBlocking::lock");

        let mut mutex_inner = self.inner.exclusive_access();
        let process = current_task().unwrap().process.upgrade().unwrap();
        let process_inner = process.inner_exclusive_access();

        if mutex_inner.locked {
            // has been locked, push it into wait queue and be ready to take it
            mutex_inner.wait_queue.push_back(current_task().unwrap());
            drop(mutex_inner);
            drop(process_inner);

            block_current_and_run_next();
            // get back from unlock, available does not change(really?)
        } else {
            mutex_inner.locked = true;
        }
    }

    /// unlock the blocking mutex
    fn unlock(&self, _mutex_id: usize) {
        trace!("kernel: MutexBlocking::unlock");
        let mut mutex_inner = self.inner.exclusive_access();
        assert!(mutex_inner.locked);
        let process = current_task().unwrap().process.upgrade().unwrap();
        let process_inner = process.inner_exclusive_access();
        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
            drop(process_inner);
            drop(process);
            wakeup_task(waking_task);
            // only control flow tanslation, no new allocation or create need
            // really ?
        } else {
            mutex_inner.locked = false;
            // no waiting task but unlocked mutex
            // max available of mutex: only 1
        }
    }
}
