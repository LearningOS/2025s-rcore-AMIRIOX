//! Process management syscalls

#[allow(unused_imports)]
use crate::mm::{translated_byte_buffer, translated_byte_u8_read, translated_byte_u8_write};
use crate::mm::VirtAddr;
use crate::mm::StepByOne;
use crate::task::current_user_token;
use crate::task::get_syscall_count;
use crate::task::{task_mmap, task_munmap, change_program_brk, exit_current_and_run_next, suspend_current_and_run_next};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let tv = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };

    let token = current_user_token();
    let mut buffer =
        translated_byte_buffer(token, ts as *const u8, core::mem::size_of::<TimeVal>());
    if buffer.len() == 1 {
        let slice = &mut buffer[0];
        let ts_ptr = slice.as_mut_ptr() as *mut TimeVal;
        unsafe {
            *ts_ptr = tv;
        }
    } else if buffer.len() == 2 {
        unsafe {
            /*
            let mut last = 0;
            for part in buffer.iter_mut() {
                let len = part.len();
                let tv_bytes = core::slice::from_raw_parts(
                    &tv as *const _ as *const u8,
                    core::mem::size_of::<TimeVal>(),
                );
                part.copy_from_slice(&tv_bytes[last..last + len]);
                last += len;
            }
            */
            let tv_bytes = core::slice::from_raw_parts(
                &tv as *const _ as *const u8,
                core::mem::size_of::<TimeVal>(),
            );

            let first = &mut buffer[0];
            let first_len = first.len();
            first.copy_from_slice(&tv_bytes[..first_len]);

            let second = &mut buffer[1];
            second.copy_from_slice(&tv_bytes[first_len..]);
        }
    } else {
        panic!("syscall get_time: *ts takes more than two pages.");
    }
    0
}

/// Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    match trace_request {
        0 => {
            // read
            let ptr = translated_byte_u8_read(token, id as *const u8);
            match ptr {
                None => -1,
                Some(ptr) => *ptr as isize,
            }
        }
        1 => {
            // write
            let ptr = translated_byte_u8_write(token, id as *mut u8);
            match ptr {
                None => -1,
                Some(ptr) => {
                    *ptr = data.try_into().unwrap();
                    0
                }
            }
        }
        // 2 => get_syscall_count(id) as isize,
        2 => {
            let res = get_syscall_count(id) as isize;
            println!("syscall count for id: {} = {}", id, res);
            res
        }
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    task_mmap(VirtAddr::from(start), VirtAddr::from(start + len), prot)
    /*
    let page_table = crate::mm::PageTable::from_token(current_user_token());
    let mut start = start;
    let end = start + len;
    while start < end {
        let start_va = VirtAddr(start);
        let mut vpn = start_va.floor();
        
        task_mmap(vpn);
        
        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        start = end_va.into();
    }
    */
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    task_munmap(VirtAddr(start), VirtAddr(start + len))
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
