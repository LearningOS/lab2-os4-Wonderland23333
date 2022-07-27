//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see [`__switch`]. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use alloc::vec::Vec;
use crate::mm::{VirtpageNum,VirtAddr,MapPermission,VPNRange};
use crate::config::{MAX_APP_NUM, MAX_SYSCALL_NUM};
use crate::loader::{get_num_app, init_app_cx};
use crate::sync::UPSafeCell;
use lazy_static::*;
pub use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus,TaskInfoInner};
use crate::syscall::TaskInfo;
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
    tasks: [TaskControlBlock; MAX_APP_NUM],
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        let num_app = get_num_app();
        let mut tasks = [TaskControlBlock {
            task_cx: TaskContext::zero_init(),
            task_status: TaskStatus::UnInit,
        }; MAX_APP_NUM];
        for (i, t) in tasks.iter_mut().enumerate().take(num_app) {
            t.task_cx = TaskContext::goto_restore(init_app_cx(i));
            t.task_status = TaskStatus::Ready;
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch3, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let task0 = &mut inner.tasks[0];
        task0.task_status = TaskStatus::Running;
        next_task.start_time = timer::get_time_us();
        let next_task_cx_ptr = &task0.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut TaskContext, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Exited;
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

    // LAB1: Try to implement your function to update or get task info!

    fn set_current_task_info(&self, ti: *mut TaskInfo){
        let inner = self.inner.exclusive_access();
        let cr_id = inner.current_task;
        let sct = inner.tasks[cr_id].task_info_inner.syscall_times;
        let st = inner.tasks[cr_id].task_info_inner.start_time;
        unsafe{
            *ti = TaskInfo {
                status: TaskStatus::Running,
                syscall_times: sct,
                time: get_time_us() / 1000 - st,
            };
        }
    }
}

fn get_syscall_times(&self) -> [u32; 500] {
    let inner = self.inner.exclusive_access();
    let current = inner.current_task;
    inner.tasks[current].syscall_times
}

fn get_start_time(&self) -> usize {
    let inner = self.inner.exclusive_access();
    let current = inner.current_task;
    return timer::get_time_us() - inner.tasks[current].start_time;
}

fn update_syscall_times(&self, syscall_id: usize) {
    let mut inner = self.inner.exclusive_access();
    let index = inner.current_task;
    inner.tasks[index].syscall_times[syscall_id] += 1;
}

fn call_mmap(&self,start: usize, len: usize, port: usize) -> isize{
    if start & (PAGE - 1) != 0{
        return -1;
    }

    if port <= 0 || port > 7usize {
            return -1;
    }

    let mut inner = self.inner.exclusive_access();
    let task_id = inner.current_task;
    let current_taskb = &mut inner.tasks[task_id];
    let memory_set = &mut inner.task[task_id].memory_set;

    let v1 = VirtpageNum::from(VirtAddr(start));
    let v2 = VirtpageNum::from(VirtAddr(start + len).ceil());

    for i in v1.0..v2.0{
        let Some(m) = memory_set.translate(VirtPageNum(i));
        if !m.is_valid() {
            return -1;
        }
    }
    let permission = MapPermission::from_bits((port as u8) << 1).unwrap() | MapPermission::U;
        memory_set.insert_framed_area(VirtAddr(start), VirtAddr(start+len), permission);
        0
}

fn drop_munmap(&self,start: usize, len: usize) -> isize{
    if start % config::PAGE_SIZE != 0 {
        return -1;
    }

    let mut inner = self.inner.exclusive_access();
    let task_id = inner.current_task;
    let current_taskb = &mut inner.tasks[task_id];
    let memory_set = &mut inner.task[task_id].memory_set;

    let v1 = VirtpageNum::from(VirtAddr(start));
    let v2 = VirtpageNum::from(VirtAddr(start + len).ceil());

    for x in v1.0 .. v2.0 {
        if let Some(m) = memory_set.translate(VirtPageNum(x)) {
            if !m.is_valid() {
                println!("vpn {} is not valid before unmap", x);
                return -1;
            }
        }
    }
    for i in v1.0..v2.0{
        let Some(m) = memory_set.translate(VirtPageNum(i));
        if !m.is_valid() {
            return -1;
        }
    }
    let bound = VPNRange::new(v1, v2);
    memory_set.munmap(bound);
    0
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

// LAB1: Public functions implemented here provide interfaces.
// You may use TASK_MANAGER member functions to handle requests.

pub fn set_task_info(ti: *mut TaskInfo){
    TASK_MANAGER.set_current_task_info(ti);
}