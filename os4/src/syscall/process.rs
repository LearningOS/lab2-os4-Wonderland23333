//! Process management syscalls

use crate::config::{MAX_SYSCALL_NUM, PAGE_SIZE};
use crate::mm::{frame_alloc, PTEFlags, PageTable, PhysAddr, VirtAddr, VirtPageNum};

use crate::task::{current_user_token, exit_current_and_run_next, set_task_info, suspend_current_and_run_next,TaskStatus, call_mmap,drop_munmap};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct TaskInfo {
    pub status: TaskStatus,
    pub syscall_times: [u32; MAX_SYSCALL_NUM],
    pub time: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    info!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    suspend_current_and_run_next();
    0
}

// YOUR JOB: 引入虚地址后重写 sys_get_time
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    
    let vaddr = VirtAddr(ts as usize);
    if let Some(x) = get_phy_addr(vaddr) {
        let us = get_time_us();
        let nwts = x.0 as *mut TimeVal;
     unsafe {
         *nwts = TimeVal {
             sec: us / 1_000_000,
             usec: us % 1_000_000,
         };
     }
    0
    } else {
    -1
    }
}

pub fn get_phy_addr(vaddr: VirtAddr)->Option<PhysAddr>{
    let ofs = vaddr.page_offset();
    let vpn = vaddr.floor();
    let ppn = PageTable::from_token(current_user_token()).translate(vpn).map(|pgentry| pgentry.ppn());
    if let Some(ppn) = ppn {
        Some(PhysAddr::combine(ppn,ofs))
    } else {
        None
    }
}

// CLUE: 从 ch4 开始不再对调度算法进行测试~
pub fn sys_set_priority(_prio: isize) -> isize {
    -1
}

// YOUR JOB: 扩展内核以实现 sys_mmap 和 sys_munmap
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    call_mmap(start,len,port)
}

pub fn sys_munmap(start: usize, len: usize) -> isize {
    drop_munmap(start, len)
}

// YOUR JOB: 引入虚地址后重写 sys_task_info
pub fn sys_task_info(ti: *mut TaskInfo) -> isize {
    if let Some(x) = get_phy_addr(VirtAddr(ti as usize)) {
        set_task_info(x.0 as *mut TaskInfo);
        0
    } else {
        -1
    }
}