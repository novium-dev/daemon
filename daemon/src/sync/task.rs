use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, Ordering},
};
use std::{collections::BTreeMap, sync::Arc};

use tokio::{
    sync::{
        OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
        TryLockError,
    },
    time::Instant,
};

struct TaskDesc<Task: Send + Sync + Clone + 'static> {
    instant: Instant,
    write: bool,
    data: Arc<std::sync::RwLock<Task>>,
}

pub struct TaskDescriptor<Task: Send + Sync + Clone + 'static> {
    pub id: u64,
    pub instant: Instant,
    pub write: bool,
    pub data: Task,
}

pub struct TaskRwLock<Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: Arc<RwLock<Data>>,
    tasks: std::sync::RwLock<BTreeMap<u64, TaskDesc<Task>>>,
    counter: AtomicU64,
}

pub struct TaskRwLockReadGuard<'a, Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: RwLockReadGuard<'a, Data>,
    tasks: &'a std::sync::RwLock<BTreeMap<u64, TaskDesc<Task>>>,
    task: Arc<std::sync::RwLock<Task>>,
    id: u64,
}

pub struct TaskRwLockWriteGuard<'a, Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: RwLockWriteGuard<'a, Data>,
    tasks: &'a std::sync::RwLock<BTreeMap<u64, TaskDesc<Task>>>,
    task: Arc<std::sync::RwLock<Task>>,
    id: u64,
}

pub struct TaskOwnedRwLockReadGuard<Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: OwnedRwLockReadGuard<Data>,
    lock: Arc<TaskRwLock<Task, Data>>,
    task: Arc<std::sync::RwLock<Task>>,
    id: u64,
}

pub struct TaskOwnedRwLockWriteGuard<Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: OwnedRwLockWriteGuard<Data>,
    lock: Arc<TaskRwLock<Task, Data>>,
    task: Arc<std::sync::RwLock<Task>>,
    id: u64,
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> TaskRwLock<Task, Data> {
    pub fn new(data: Data) -> Self {
        Self {
            inner: Arc::new(RwLock::new(data)),
            counter: AtomicU64::default(),
            tasks: std::sync::RwLock::default(),
        }
    }

    pub fn tasks(&self) -> Vec<TaskDescriptor<Task>> {
        self.tasks
            .read()
            .expect("Unable to lock tasks")
            .iter()
            .map(|(id, task)| TaskDescriptor {
                id: *id,
                instant: task.instant,
                write: task.write,
                data: task.data.read().expect("Unable to lock task").clone(),
            })
            .collect()
    }

    fn insert_task(&self, task: Task, write: bool) -> (u64, Arc<std::sync::RwLock<Task>>) {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let task = Arc::new(std::sync::RwLock::new(task));
        self.tasks.write().expect("Unable to lock tasks").insert(
            id,
            TaskDesc {
                instant: Instant::now(),
                write,
                data: Arc::clone(&task),
            },
        );
        (id, task)
    }

    pub async fn read(&self, task: Task) -> TaskRwLockReadGuard<'_, Task, Data> {
        let inner = self.inner.read().await;
        let (id, task) = self.insert_task(task, false);

        TaskRwLockReadGuard {
            inner,
            tasks: &self.tasks,
            task,
            id,
        }
    }

    pub async fn write(&self, task: Task) -> TaskRwLockWriteGuard<'_, Task, Data> {
        let inner = self.inner.write().await;
        let (id, task) = self.insert_task(task, true);

        TaskRwLockWriteGuard {
            inner,
            tasks: &self.tasks,
            task,
            id,
        }
    }

    pub fn try_read(
        &self,
        task: Task,
    ) -> Result<TaskRwLockReadGuard<'_, Task, Data>, TryLockError> {
        let inner = self.inner.try_read()?;
        let (id, task) = self.insert_task(task, false);

        Ok(TaskRwLockReadGuard {
            inner,
            tasks: &self.tasks,
            task,
            id,
        })
    }

    pub fn try_write(
        &self,
        task: Task,
    ) -> Result<TaskRwLockWriteGuard<'_, Task, Data>, TryLockError> {
        let inner = self.inner.try_write()?;
        let (id, task) = self.insert_task(task, true);

        Ok(TaskRwLockWriteGuard {
            inner,
            tasks: &self.tasks,
            task,
            id,
        })
    }

    pub async fn read_owned(self: Arc<Self>, task: Task) -> TaskOwnedRwLockReadGuard<Task, Data> {
        let inner = Arc::clone(&self.inner).read_owned().await;
        let (id, task) = self.insert_task(task, false);

        TaskOwnedRwLockReadGuard {
            inner,
            lock: self,
            task,
            id,
        }
    }

    pub async fn write_owned(self: Arc<Self>, task: Task) -> TaskOwnedRwLockWriteGuard<Task, Data> {
        let inner = Arc::clone(&self.inner).write_owned().await;
        let (id, task) = self.insert_task(task, true);

        TaskOwnedRwLockWriteGuard {
            inner,
            lock: self,
            task,
            id,
        }
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync + Default> Default
    for TaskRwLock<Task, Data>
{
    fn default() -> Self {
        Self::new(Data::default())
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> TaskRwLockReadGuard<'_, Task, Data> {
    pub fn task<F: for<'a> FnOnce(&'a Task) -> R, R>(&self, func: F) -> R {
        let task = self.task.read().expect("Unable to lock task");
        (func)(&task)
    }

    pub fn task_mut<F: for<'a> FnOnce(&'a mut Task) -> R, R>(&self, func: F) -> R {
        let mut task = self.task.write().expect("Unable to lock task");
        (func)(&mut task)
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Deref
    for TaskRwLockReadGuard<'_, Task, Data>
{
    type Target = Data;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Drop
    for TaskRwLockReadGuard<'_, Task, Data>
{
    fn drop(&mut self) {
        self.tasks
            .write()
            .expect("Unable to lock tasks")
            .remove(&self.id);
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> TaskRwLockWriteGuard<'_, Task, Data> {
    pub fn task<F: for<'a> FnOnce(&'a Task) -> R, R>(&self, func: F) -> R {
        let task = self.task.read().expect("Unable to lock task");
        (func)(&task)
    }

    pub fn task_mut<F: for<'a> FnOnce(&'a mut Task) -> R, R>(&self, func: F) -> R {
        let mut task = self.task.write().expect("Unable to lock task");
        (func)(&mut task)
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Deref
    for TaskRwLockWriteGuard<'_, Task, Data>
{
    type Target = Data;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> DerefMut
    for TaskRwLockWriteGuard<'_, Task, Data>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Drop
    for TaskRwLockWriteGuard<'_, Task, Data>
{
    fn drop(&mut self) {
        self.tasks
            .write()
            .expect("Unable to lock tasks")
            .remove(&self.id);
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> TaskOwnedRwLockReadGuard<Task, Data> {
    pub fn task<F: for<'a> FnOnce(&'a Task) -> R, R>(&self, func: F) -> R {
        let task = self.task.read().expect("Unable to lock task");
        (func)(&task)
    }

    pub fn task_mut<F: for<'a> FnOnce(&'a mut Task) -> R, R>(&self, func: F) -> R {
        let mut task = self.task.write().expect("Unable to lock task");
        (func)(&mut task)
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Deref
    for TaskOwnedRwLockReadGuard<Task, Data>
{
    type Target = Data;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Drop
    for TaskOwnedRwLockReadGuard<Task, Data>
{
    fn drop(&mut self) {
        self.lock
            .tasks
            .write()
            .expect("Unable to lock tasks")
            .remove(&self.id);
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> TaskOwnedRwLockWriteGuard<Task, Data> {
    pub fn task<F: for<'a> FnOnce(&'a Task) -> R, R>(&self, func: F) -> R {
        let task = self.task.read().expect("Unable to lock task");
        (func)(&task)
    }

    pub fn task_mut<F: for<'a> FnOnce(&'a mut Task) -> R, R>(&self, func: F) -> R {
        let mut task = self.task.write().expect("Unable to lock task");
        (func)(&mut task)
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Deref
    for TaskOwnedRwLockWriteGuard<Task, Data>
{
    type Target = Data;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> DerefMut
    for TaskOwnedRwLockWriteGuard<Task, Data>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> Drop
    for TaskOwnedRwLockWriteGuard<Task, Data>
{
    fn drop(&mut self) {
        self.lock
            .tasks
            .write()
            .expect("Unable to lock tasks")
            .remove(&self.id);
    }
}
