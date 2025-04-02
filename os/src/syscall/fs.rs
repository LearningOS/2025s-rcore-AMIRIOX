//! File and filesystem-related syscalls
use crate::fs::{open_file, OpenFlags, Stat, ROOT_INODE};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

// fn fstat(fd: i32, st: *mut Stat) -> i32
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    trace!("kernel:pid[{}] sys_fstat", current_task().unwrap().pid.0);

    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd <= 2 || fd as usize >= inner.fd_table.len() {
        println!("invalid fd {} >= {}", fd, inner.fd_table.len());
        return -1;
    }

    if let Some(inode) = inner.fd_table[fd as usize].clone() {
        drop(inner);

        let sts = Stat {
            dev: 0,
            ino: inode.inode_id(),
            mode: inode.mode(),
            nlink: inode.nlink(),
            pad: [0; 7],
        };
        let token = current_user_token();
        let mut buffer =
            translated_byte_buffer(token, st as *const u8, core::mem::size_of::<Stat>());
        if buffer.len() == 1 {
            let slice = &mut buffer[0];
            let ts_ptr = slice.as_mut_ptr() as *mut Stat;
            unsafe {
                *ts_ptr = sts;
            }
        } else if buffer.len() == 2 {
            unsafe {
                let tv_bytes = core::slice::from_raw_parts(
                    &sts as *const _ as *const u8,
                    core::mem::size_of::<Stat>(),
                );

                let first = &mut buffer[0];
                let first_len = first.len();
                first.copy_from_slice(&tv_bytes[..first_len]);

                let second = &mut buffer[1];
                second.copy_from_slice(&tv_bytes[first_len..]);
            }
        } else {
            panic!("syscall sys_fstat: *st takes more than two pages.");
        }
        return 0;
    }
    -1
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    // 两个目录项指向同一个 inode
    trace!("kernel:pid[{}] sys_linkat", current_task().unwrap().pid.0);

    let token = current_user_token();

    let old_name_str = translated_str(token, old_name);
    let new_name_str = translated_str(token, new_name);

    if old_name_str == new_name_str {
        return -1;
    }

    println!("before ROOTINODE.linkat(...)");
    ROOT_INODE.linkat(&old_name_str, &new_name_str)
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_unlinkat",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let name = translated_str(token, name);

    ROOT_INODE.unlinkat(&name)
}
