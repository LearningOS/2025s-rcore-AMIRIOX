//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::mm::{MapArea, MapPermission, VirtAddr, VirtPageNum};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
    syscall_cnt: [[u32; 500]; 50],
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                    syscall_cnt: [[0; 500]; 50],
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
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

        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;

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
        drop(inner);
        0
    }

    fn munap(&self, start_va: VirtAddr, end_va: VirtAddr) -> isize {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;

        // check mem valid
        if !start_va.aligned() {
            return -1;
        }

        let start_vpn: VirtPageNum = start_va.into();
        let end_vpn: VirtPageNum = end_va.ceil();
        for vpn in start_vpn.0..end_vpn.0 {
            let vpn = VirtPageNum::from(vpn);
            if let Some(pte) = memory_set.translate(vpn) {
                if !pte.is_valid() { return -1; }
                //memory_set.unmap_vpn(vpn);
                memory_set.shrink_to(start_va, start_va);
            }else {
                return -1;
            }
        }

        drop(inner);
        0
    }

    /// Update syscall count
    fn update_syscall_count(&self, id: usize) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.syscall_cnt[current][id] += 1;
        drop(inner);
    }

    /// Get syscall count
    fn get_syscall_count(&self, id: usize) -> u32 {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let ret = inner.syscall_cnt[current][id];
        drop(inner);
        ret
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}

/// Update syscall count
pub fn update_syscall_count(id: usize) {
    TASK_MANAGER.update_syscall_count(id);
}

/// Get syscall count
pub fn get_syscall_count(id: usize) -> u32 {
    TASK_MANAGER.get_syscall_count(id)
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
    TASK_MANAGER.mmap(start_va, end_va, prot)
}

pub fn task_munmap(start_va: VirtAddr, end_va: VirtAddr) -> isize {
    TASK_MANAGER.munap(start_va, end_va)
}
