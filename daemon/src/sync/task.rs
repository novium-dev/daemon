use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, Ordering},
};
use std::{collections::BTreeMap, sync::Arc};

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct TaskRwLock<Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: RwLock<Data>,
    tasks: std::sync::RwLock<BTreeMap<u64, Arc<std::sync::RwLock<Task>>>>,
    counter: AtomicU64,
}

pub struct TaskRwLockReadGuard<'a, Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: RwLockReadGuard<'a, Data>,
    tasks: &'a std::sync::RwLock<BTreeMap<u64, Arc<std::sync::RwLock<Task>>>>,
    task: Arc<std::sync::RwLock<Task>>,
    id: u64,
}

pub struct TaskRwLockWriteGuard<'a, Task: Send + Sync + Clone + 'static, Data: Send + Sync> {
    inner: RwLockWriteGuard<'a, Data>,
    tasks: &'a std::sync::RwLock<BTreeMap<u64, Arc<std::sync::RwLock<Task>>>>,
    task: Arc<std::sync::RwLock<Task>>,
    id: u64,
}

impl<Task: Send + Sync + Clone + 'static, Data: Send + Sync> TaskRwLock<Task, Data> {
    pub fn tasks(&self) -> BTreeMap<u64, Task> {
        self.tasks
            .read()
            .expect("Unable to lock tasks")
            .iter()
            .map(|(id, task)| (*id, task.read().expect("Unable to lock task").clone()))
            .collect()
    }

    pub async fn read(&self, task: Task) -> TaskRwLockReadGuard<'_, Task, Data> {
        let inner = self.inner.read().await;
        let id = self.counter.fetch_add(1, Ordering::SeqCst);

        let mut tasks = self.tasks.write().expect("Unable to lock tasks");
        let task = Arc::new(std::sync::RwLock::new(task));
        tasks.insert(id, Arc::clone(&task));

        TaskRwLockReadGuard {
            inner,
            tasks: &self.tasks,
            task,
            id,
        }
    }

    pub async fn write(&self, task: Task) -> TaskRwLockWriteGuard<'_, Task, Data> {
        let inner = self.inner.write().await;
        let id = self.counter.fetch_add(1, Ordering::SeqCst);

        let mut tasks = self.tasks.write().expect("Unable to lock tasks");
        let task = Arc::new(std::sync::RwLock::new(task));
        tasks.insert(id, Arc::clone(&task));

        TaskRwLockWriteGuard {
            inner,
            tasks: &self.tasks,
            task,
            id,
        }
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
