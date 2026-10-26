//! Host-owned cooperative task scheduler (Runtime §1.3, D-154).
//!
//! The compiler projects every task with its interval, priority, `SINGLE`
//! flag, and guard identity and emits task-bound instances in scheduler rank
//! order. The host owns the run state keyed by task `GotId`: before every scan
//! it decides which tasks are due, writes each task's `TaskGuard` root, and
//! after a successful scan advances interval deadlines and marks completed
//! `SINGLE` tasks.
//!
//! Rules:
//!
//! - a task without an interval is due every scan;
//! - an interval task first runs after one interval, then at `now + interval`
//!   (no catch-up);
//! - a `SINGLE` task runs on its first due scan and completes after that cycle
//!   returns normally;
//! - state survives a compatible online swap by `GotId`; a cold install starts
//!   fresh.
//!
//! Priority ordering is a compiler concern: the generated cycle already runs
//! instances in scheduler rank order, so the host only decides due/skip.

use stc_got::error::{Error, Result, codes};
use stc_got::{GotId, Manifest, TaskRecord};

/// Runtime status of one scheduled task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskStatus {
    /// Task identity.
    pub got_id: GotId,
    /// Stable task key.
    pub key: String,
    /// `SINGLE` attribute.
    pub single: bool,
    /// Interval in nanoseconds.
    pub interval_ns: Option<i64>,
    /// Priority (lower is higher).
    pub priority: Option<i64>,
    /// `SINGLE` task completed.
    pub completed: bool,
    /// Next interval deadline, when one is scheduled.
    pub next_deadline_ns: Option<i64>,
    /// Whether the task ran in the most recent scan.
    pub due: bool,
}

#[derive(Clone, Debug)]
struct TaskRunState {
    got_id: GotId,
    key: String,
    guard_got_id: GotId,
    single: bool,
    interval_ns: Option<i64>,
    priority: Option<i64>,
    completed: bool,
    next_deadline_ns: Option<i64>,
    /// Due in the scan currently being prepared or last executed.
    due: bool,
}

impl TaskRunState {
    fn new(task: &TaskRecord) -> Result<Self> {
        let Some(guard_got_id) = task.guard_got_id else {
            return Err(Error::new(
                codes::RUN_OPTION,
                format!("task `{}` has no task-guard root", task.key),
            ));
        };
        Ok(Self {
            got_id: task.got_id,
            key: task.key.clone(),
            guard_got_id,
            single: task.single,
            interval_ns: task.interval_ns,
            priority: task.priority,
            completed: false,
            next_deadline_ns: None,
            due: false,
        })
    }

    fn same_schedule(&self, other: &Self) -> bool {
        self.single == other.single
            && self.interval_ns == other.interval_ns
            && self.priority == other.priority
    }
}

/// Host-owned task run state.
#[derive(Clone, Debug, Default)]
pub struct TaskScheduler {
    tasks: Vec<TaskRunState>,
    last_scan_ns: i64,
}

impl TaskScheduler {
    /// Builds a fresh scheduler from a manifest.
    ///
    /// # Errors
    ///
    /// Rejects a task without a guard root (`STC-E-RUN-003`).
    pub fn from_manifest(manifest: &Manifest) -> Result<Self> {
        Self::from_tasks(&manifest.tasks)
    }

    fn from_tasks(tasks: &[TaskRecord]) -> Result<Self> {
        let mut states = Vec::with_capacity(tasks.len());
        for task in tasks {
            states.push(TaskRunState::new(task)?);
        }
        Ok(Self {
            tasks: states,
            last_scan_ns: 0,
        })
    }

    /// Builds the scheduler for a candidate while preserving matching state.
    ///
    /// State is preserved by task `GotId`; a task whose schedule changed starts
    /// fresh. The migration check rejects a changed schedule as a topology
    /// change before activation, so this reset only applies to direct callers.
    ///
    /// # Errors
    ///
    /// Rejects a candidate task without a guard root (`STC-E-RUN-003`).
    pub fn reconcile(&self, manifest: &Manifest) -> Result<Self> {
        self.reconcile_tasks(&manifest.tasks)
    }

    fn reconcile_tasks(&self, tasks: &[TaskRecord]) -> Result<Self> {
        let mut next = Self::from_tasks(tasks)?;
        for task in &mut next.tasks {
            let Some(previous) = self.tasks.iter().find(|state| state.got_id == task.got_id) else {
                continue;
            };
            if previous.same_schedule(task) {
                task.completed = previous.completed;
                task.next_deadline_ns = previous.next_deadline_ns;
            }
            // `due` describes a scan; the next `begin_scan` recomputes it.
            task.due = false;
        }
        next.last_scan_ns = self.last_scan_ns;
        Ok(next)
    }

    /// Computes due state for a scan at `now_ns`.
    ///
    /// Deadlines are advanced by [`Self::finish_scan`] after the scan returns
    /// normally, so a failed scan does not consume an interval.
    pub fn begin_scan(&mut self, now_ns: i64) {
        self.last_scan_ns = now_ns;
        for task in &mut self.tasks {
            task.due = if task.single && task.completed {
                false
            } else if let Some(interval) = task.interval_ns {
                match task.next_deadline_ns {
                    Some(deadline) => now_ns >= deadline,
                    None => {
                        let deadline = now_ns.saturating_add(interval);
                        task.next_deadline_ns = Some(deadline);
                        now_ns >= deadline
                    }
                }
            } else {
                true
            };
        }
    }

    /// Records the outcome of the scan prepared by [`Self::begin_scan`].
    pub fn finish_scan(&mut self, success: bool) {
        if !success {
            return;
        }
        let now = self.last_scan_ns;
        for task in &mut self.tasks {
            if !task.due {
                continue;
            }
            if task.single {
                task.completed = true;
            }
            if let Some(interval) = task.interval_ns {
                task.next_deadline_ns = Some(now.saturating_add(interval));
            }
        }
    }

    /// Returns each task's guard root and run/skip value for this scan.
    pub fn guards(&self) -> impl Iterator<Item = (GotId, bool)> + '_ {
        self.tasks.iter().map(|task| (task.guard_got_id, task.due))
    }

    /// Returns the run status of every task in manifest order.
    #[must_use]
    pub fn status(&self) -> Vec<TaskStatus> {
        self.tasks
            .iter()
            .map(|task| TaskStatus {
                got_id: task.got_id,
                key: task.key.clone(),
                single: task.single,
                interval_ns: task.interval_ns,
                priority: task.priority,
                completed: task.completed,
                next_deadline_ns: task.next_deadline_ns,
                due: task.due,
            })
            .collect()
    }

    /// Returns `true` when the manifest has no scheduled tasks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u8, guard: bool) -> GotId {
        let mut bytes = [0_u8; 16];
        bytes[0] = value;
        bytes[15] = u8::from(guard);
        GotId::from_bytes(bytes)
    }

    fn task(
        value: u8,
        name: &str,
        single: bool,
        interval_ns: Option<i64>,
        priority: Option<i64>,
    ) -> TaskRecord {
        TaskRecord {
            got_id: id(value, false),
            key: format!("_ROOT::C/R#{name}"),
            previous_keys: Vec::new(),
            configuration: "C".into(),
            resource: "R".into(),
            name: name.into(),
            single,
            interval_ns,
            priority,
            guard_got_id: Some(id(value, true)),
            source: None,
        }
    }

    fn scheduler(tasks: Vec<TaskRecord>) -> TaskScheduler {
        TaskScheduler::from_tasks(&tasks).expect("scheduler")
    }

    fn by_name<'a>(status: &'a [TaskStatus], name: &str) -> &'a TaskStatus {
        status
            .iter()
            .find(|task| task.key.ends_with(&format!("#{name}")))
            .unwrap_or_else(|| panic!("task `{name}`"))
    }

    #[test]
    fn interval_tasks_first_run_after_one_interval_without_catch_up() {
        let mut scheduler = scheduler(vec![task(1, "T", false, Some(10), None)]);

        scheduler.begin_scan(0);
        assert!(
            !scheduler.status()[0].due,
            "first scan runs no interval task"
        );
        scheduler.finish_scan(true);

        scheduler.begin_scan(5);
        assert!(!scheduler.status()[0].due);
        scheduler.finish_scan(true);

        scheduler.begin_scan(10);
        assert!(scheduler.status()[0].due);
        scheduler.finish_scan(true);
        assert_eq!(scheduler.status()[0].next_deadline_ns, Some(20));

        // A late scan does not run the missed periods.
        scheduler.begin_scan(35);
        assert!(scheduler.status()[0].due);
        scheduler.finish_scan(true);
        assert_eq!(scheduler.status()[0].next_deadline_ns, Some(45));
        scheduler.begin_scan(36);
        assert!(!scheduler.status()[0].due);
    }

    #[test]
    fn single_tasks_complete_after_one_successful_scan() {
        let mut scheduler = scheduler(vec![task(1, "Once", true, None, None)]);
        scheduler.begin_scan(0);
        assert!(scheduler.status()[0].due);
        scheduler.finish_scan(false);
        assert!(
            !scheduler.status()[0].completed,
            "a failed scan does not complete"
        );
        scheduler.begin_scan(0);
        assert!(scheduler.status()[0].due);
        scheduler.finish_scan(true);
        assert!(scheduler.status()[0].completed);
        scheduler.begin_scan(0);
        assert!(!scheduler.status()[0].due);
    }

    #[test]
    fn tasks_without_intervals_run_every_scan() {
        let mut scheduler = scheduler(vec![task(1, "Always", false, None, None)]);
        for _ in 0..3 {
            scheduler.begin_scan(0);
            assert!(scheduler.status()[0].due);
            scheduler.finish_scan(true);
        }
    }

    #[test]
    fn single_tasks_with_an_interval_run_once_after_the_interval() {
        let mut scheduler = scheduler(vec![task(1, "Once", true, Some(10), None)]);
        scheduler.begin_scan(0);
        assert!(!scheduler.status()[0].due);
        scheduler.finish_scan(true);
        assert!(!scheduler.status()[0].completed);

        scheduler.begin_scan(10);
        assert!(scheduler.status()[0].due);
        scheduler.finish_scan(true);
        assert!(scheduler.status()[0].completed);

        scheduler.begin_scan(20);
        assert!(!scheduler.status()[0].due);
    }

    #[test]
    fn reconcile_preserves_state_by_got_id() {
        let mut scheduler = scheduler(vec![
            task(1, "Once", true, None, None),
            task(2, "Timer", false, Some(10), Some(1)),
        ]);
        scheduler.begin_scan(0);
        scheduler.finish_scan(true);
        scheduler.begin_scan(10);
        scheduler.finish_scan(true);

        let candidate = vec![
            task(1, "Once", true, None, None),
            task(2, "Timer", false, Some(10), Some(1)),
            task(3, "New", true, None, None),
        ];
        let reconciled = scheduler.reconcile_tasks(&candidate).expect("reconcile");
        let status = reconciled.status();
        assert!(by_name(&status, "Once").completed);
        assert_eq!(by_name(&status, "Timer").next_deadline_ns, Some(20));
        assert!(!by_name(&status, "New").completed);
        assert_eq!(by_name(&status, "New").next_deadline_ns, None);
    }

    #[test]
    fn a_task_without_a_guard_is_rejected() {
        let mut record = task(1, "T", false, None, None);
        record.guard_got_id = None;
        let error = TaskScheduler::from_tasks(&[record]).expect_err("no guard");
        assert_eq!(error.code, "STC-E-RUN-003");
    }
}
