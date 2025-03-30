//!Implementation of [`Processor`] and Intersection of control flow
//!
//! Here, the continuous operation of user apps in CPU is maintained,
//! the current running state of CPU is recorded,
//! and the replacement and transfer of control flow of different applications are executed.

use super::__switch;
use super::{fetch_task, TaskStatus};
use super::{TaskContext, TaskControlBlock};
use crate::mm::{MapPermission, VirtAddr, VirtPageNum};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::sync::Arc;
use lazy_static::*;

const BIGSTRIDE: usize = 1145141919;

/// Processor management structure
pub struct Processor {
    ///The task currently executing on the current processor
    current: Option<Arc<TaskControlBlock>>,

    ///The basic control flow of each core, helping to select and switch process
    idle_task_cx: TaskContext,
}

impl Processor {
    ///Create an empty Processor
    pub fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::zero_init(),
        }
    }

    ///Get mutable reference to `idle_task_cx`
    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }

    ///Get current task in moving semanteme
    pub fn take_current(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.current.take()
    }

    ///Get current task in cloning semanteme
    pub fn current(&self) -> Option<Arc<TaskControlBlock>> {
        self.current.as_ref().map(Arc::clone)
    }

    /// mmap
    fn mmap(&self, start_va: VirtAddr, end_va: VirtAddr, prot: usize) -> isize {
        // check prot valid
        if !start_va.aligned() || prot & !0x7 != 0 || prot & 0x7 == 0 {
            return -1;
        }
        let mut perm = MapPermission::U;
        if (prot & 0b001) != 0 {
            perm |= MapPermission::R;
        }
        if (prot & 0b010) != 0 {
            perm |= MapPermission::W;
        }
        if (prot & 0b100) != 0 {
            perm |= MapPermission::X;
        }

        let mut current = self.current.as_ref().unwrap().inner_exclusive_access();
        let memory_set = &mut current.memory_set;

        // check mem valid
        let start_vpn: VirtPageNum = start_va.into();
        let end_vpn: VirtPageNum = end_va.ceil();
        for vpn in start_vpn.0..end_vpn.0 {
            if let Some(pte) = memory_set.translate(VirtPageNum::from(vpn)) {
                if pte.is_valid() {
                    return -1;
                }
            }
        }

        memory_set.insert_framed_area(start_va, end_va, perm);
        drop(current);
        0
    }

    fn munap(&self, start_va: VirtAddr, end_va: VirtAddr) -> isize {
        let mut current = self.current.as_ref().unwrap().inner_exclusive_access();
        let memory_set = &mut current.memory_set;

        // check mem valid
        if !start_va.aligned() {
            return -1;
        }

        let start_vpn: VirtPageNum = start_va.into();
        let end_vpn: VirtPageNum = end_va.ceil();
        for vpn in start_vpn.0..end_vpn.0 {
            let vpn = VirtPageNum::from(vpn);
            if let Some(pte) = memory_set.translate(vpn) {
                if !pte.is_valid() {
                    return -1;
                }
                //memory_set.unmap_vpn(vpn);
                memory_set.shrink_to(start_va, start_va);
            } else {
                return -1;
            }
        }

        drop(current);
        0
    }
}

lazy_static! {
    pub static ref PROCESSOR: UPSafeCell<Processor> = unsafe { UPSafeCell::new(Processor::new()) };
}

///The main part of process execution and scheduling
///Loop `fetch_task` to get the process that needs to run, and switch the process through `__switch`
pub fn run_tasks() {
    loop {
        let mut processor = PROCESSOR.exclusive_access();
        if let Some(task) = fetch_task() {
            let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
            // access coming task TCB exclusively
            let mut task_inner = task.inner_exclusive_access();
            let next_task_cx_ptr = &task_inner.task_cx as *const TaskContext;
            task_inner.task_status = TaskStatus::Running;
            task_inner.stride += BIGSTRIDE / task_inner.priority;
            // release coming task_inner manually
            drop(task_inner);
            // release coming task TCB manually
            processor.current = Some(task);
            // release processor manually
            drop(processor);
            unsafe {
                __switch(idle_task_cx_ptr, next_task_cx_ptr);
            }
        } else {
            warn!("no tasks available in run_tasks");
        }
    }
}

/// Get current task through take, leaving a None in its place
pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().take_current()
}

/// Get a copy of the current task
pub fn current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().current()
}

/// Get the current user token(addr of page table)
pub fn current_user_token() -> usize {
    let task = current_task().unwrap();
    task.get_user_token()
}

///Get the mutable reference to trap context of current task
pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_trap_cx()
}

///Return to idle control flow for new scheduling
pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSOR.exclusive_access();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        __switch(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}

/// mmap
/*
 * start 没有按页大小对齐
 * prot & !0x7 != 0 (prot 其余位必须为0)
 * prot & 0x7 = 0 (这样的内存无意义)
 * [start, start + len) 中存在已经被映射的页
 * 物理内存不足
 * */
pub fn task_mmap(start_va: VirtAddr, end_va: VirtAddr, prot: usize) -> isize {
    PROCESSOR.exclusive_access().mmap(start_va, end_va, prot)
}

pub fn task_munmap(start_va: VirtAddr, end_va: VirtAddr) -> isize {
    PROCESSOR.exclusive_access().munap(start_va, end_va)
}
