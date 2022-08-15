pub mod cli;
pub mod sim_drivers;
pub mod sim_tracing;

use std::{collections::HashMap, sync::RwLock, thread};

use maitake::task::TaskId;
use mnemos_kernel::Kernel;
use once_cell::sync::Lazy;
use tracing_modality::{TimelineInfo, TimelineId};

pub struct Timelines {
    thread_info: TimelineInfo,
    pub kernel: Option<&'static Kernel>,
    task_map: HashMap<TaskId, TimelineInfo>,
}

pub(crate) fn get_timeline() -> TimelineInfo {
    TIMELINES.with(|tl| {
        // First, see if we can resolve without mut access
        let tl_immut = tl.read().unwrap();

        // Has the kernel been registered in this thread?
        let kernel = match tl_immut.kernel.as_ref() {
            None => return tl_immut.thread_info.clone(),
            Some(k) => k,
        };

        // Are we in a maitake task?
        let task_id = match kernel.task_id() {
            None => return tl_immut.thread_info.clone(),
            Some(tid) => tid,
        };

        // Has this task already been assigned a UUID?
        if let Some(tl_id) = tl_immut.task_map.get(&task_id) {
            return tl_id.clone();
        }

        // Nope, we need to add one, meaning we need to get mut access to the Timelines
        // structure to add the new timeline.
        drop(tl_immut);

        let new_name = format!("kerneltask-{}", task_id);
        let new_info = TimelineInfo::new(
            new_name,
            TimelineId::allocate(),
        );
        let mut tl_mut = tl.write().unwrap();
        tl_mut.task_map.insert(task_id, new_info.clone());
        new_info
    })
}

thread_local! {
    pub static TIMELINES: Lazy<RwLock<Timelines>>  = Lazy::new(|| {
        let cur = thread::current();
        let name = cur
            .name()
            .map(Into::into)
            .unwrap_or_else(|| format!("thread-{:?}", cur.id()));

        let id = TimelineId::allocate();

        let thread_info = TimelineInfo::new(
            name,
            id,
        );

        RwLock::new(Timelines {
            thread_info,
            kernel: None,
            task_map: HashMap::new(),
        })
    });
}
