use futures::stream::BoxStream;
use futures::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Generation(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TaskTarget {
    Stories,
    Search,
    CommentRoots(u64),
    CommentChildren(u64),
    Summary,
    SettingsSave,
    StoryStateSave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TaskId {
    target: TaskTarget,
    generation: Generation,
}

impl TaskId {
    pub(crate) fn target(self) -> TaskTarget {
        self.target
    }
}

struct InFlight {
    id: TaskId,
    handle: JoinHandle<()>,
}

pub(crate) struct TaskLifecycle<Event> {
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    failure_event: fn(TaskId, String) -> Event,
    completion_event: fn(TaskId) -> Event,
    next_generation: u64,
    in_flight: HashMap<TaskTarget, InFlight>,
}

impl<Event: Send + 'static> TaskLifecycle<Event> {
    pub(crate) fn new(
        tx: tokio::sync::mpsc::UnboundedSender<Event>,
        failure_event: fn(TaskId, String) -> Event,
        completion_event: fn(TaskId) -> Event,
    ) -> Self {
        Self {
            tx,
            failure_event,
            completion_event,
            next_generation: 0,
            in_flight: HashMap::new(),
        }
    }

    pub(crate) fn spawn<T, Fut, Map>(
        &mut self,
        target: TaskTarget,
        future: Fut,
        map_success: Map,
    ) -> TaskId
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        Map: FnOnce(TaskId, T) -> Event + Send + 'static,
    {
        let task = self.start(target);
        let tx = self.tx.clone();
        let failure_event = self.failure_event;
        let handle = spawn_tracked(async move {
            let event = match future.await {
                Ok(value) => map_success(task, value),
                Err(error) => failure_event(task, format!("{error:#}")),
            };
            let _ = tx.send(event);
        });
        self.in_flight.insert(target, InFlight { id: task, handle });
        task
    }

    pub(crate) fn spawn_stream<T, Map>(
        &mut self,
        target: TaskTarget,
        mut stream: BoxStream<'static, anyhow::Result<T>>,
        map_item: Map,
    ) -> TaskId
    where
        T: Send + 'static,
        Map: Fn(TaskId, T) -> Event + Send + 'static,
    {
        let task = self.start(target);
        let tx = self.tx.clone();
        let failure_event = self.failure_event;
        let completion_event = self.completion_event;
        let handle = spawn_tracked(async move {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(item) => {
                        let _ = tx.send(map_item(task, item));
                    }
                    Err(error) => {
                        let _ = tx.send(failure_event(task, format!("{error:#}")));
                        return;
                    }
                }
            }
            let _ = tx.send(completion_event(task));
        });
        self.in_flight.insert(target, InFlight { id: task, handle });
        task
    }

    pub(crate) fn is_current(&self, task: TaskId) -> bool {
        self.in_flight
            .get(&task.target)
            .is_some_and(|in_flight| in_flight.id == task)
    }

    pub(crate) fn is_running(&self, target: TaskTarget) -> bool {
        self.in_flight.contains_key(&target)
    }

    pub(crate) fn finish(&mut self, task: TaskId) -> bool {
        if !self.is_current(task) {
            return false;
        }
        self.in_flight.remove(&task.target);
        true
    }

    pub(crate) fn cancel(&mut self, target: TaskTarget) -> bool {
        let Some(in_flight) = self.in_flight.remove(&target) else {
            return false;
        };
        in_flight.handle.abort();
        true
    }

    pub(crate) async fn cancel_and_wait(&mut self, target: TaskTarget) -> bool {
        let Some(in_flight) = self.in_flight.remove(&target) else {
            return false;
        };
        in_flight.handle.abort();
        let _ = in_flight.handle.await;
        true
    }

    pub(crate) fn cancel_where(&mut self, predicate: impl Fn(TaskTarget) -> bool) {
        for target in self.targets_where(predicate) {
            self.cancel(target);
        }
    }

    pub(crate) fn targets_where(&self, predicate: impl Fn(TaskTarget) -> bool) -> Vec<TaskTarget> {
        self.in_flight
            .keys()
            .copied()
            .filter(|target| predicate(*target))
            .collect()
    }

    pub(crate) fn count_where(&self, predicate: impl Fn(TaskTarget) -> bool) -> usize {
        self.in_flight
            .keys()
            .filter(|target| predicate(**target))
            .count()
    }

    fn start(&mut self, target: TaskTarget) -> TaskId {
        self.cancel(target);
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("task generation overflow");
        TaskId {
            target,
            generation: Generation(self.next_generation),
        }
    }
}

impl<Event> Drop for TaskLifecycle<Event> {
    fn drop(&mut self) {
        for (_, in_flight) in self.in_flight.drain() {
            in_flight.handle.abort();
        }
    }
}

fn spawn_tracked(future: impl Future<Output = ()> + Send + 'static) -> JoinHandle<()> {
    tokio::spawn(future)
}

pub(crate) fn spawn_detached(future: impl Future<Output = ()> + Send + 'static) {
    drop(spawn_tracked(future));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    enum Event {
        Completed,
        Failed(TaskId, String),
    }

    fn completed_event(_task: TaskId) -> Event {
        Event::Completed
    }

    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    fn lifecycle() -> (
        TaskLifecycle<Event>,
        tokio::sync::mpsc::UnboundedReceiver<Event>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (TaskLifecycle::new(tx, Event::Failed, completed_event), rx)
    }

    async fn pending_task(
        started: tokio::sync::oneshot::Sender<()>,
        dropped: tokio::sync::oneshot::Sender<()>,
    ) -> anyhow::Result<()> {
        let _drop_signal = DropSignal(Some(dropped));
        let _ = started.send(());
        futures::future::pending::<()>().await;
        Ok(())
    }

    #[tokio::test]
    async fn starting_the_same_target_cancels_the_previous_task() {
        let (mut lifecycle, mut rx) = lifecycle();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();

        let first = lifecycle.spawn(
            TaskTarget::Stories,
            pending_task(started_tx, dropped_tx),
            |_task, ()| Event::Completed,
        );
        started_rx.await.expect("first task started");

        let second = lifecycle.spawn(
            TaskTarget::Stories,
            futures::future::pending::<anyhow::Result<()>>(),
            |_task, ()| Event::Completed,
        );

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("first task was not cancelled")
            .expect("drop signal closed");
        assert_ne!(first, second);
        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
        assert_eq!(
            lifecycle.count_where(|target| target == TaskTarget::Stories),
            1
        );
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn differently_keyed_targets_run_concurrently() {
        let (mut lifecycle, _rx) = lifecycle();
        let mut dropped = Vec::new();

        for parent_id in [11, 12] {
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
            lifecycle.spawn(
                TaskTarget::CommentChildren(parent_id),
                pending_task(started_tx, dropped_tx),
                |_task, ()| Event::Completed,
            );
            started_rx.await.expect("keyed task started");
            dropped.push(dropped_rx);
        }

        assert_eq!(
            lifecycle.count_where(|target| matches!(target, TaskTarget::CommentChildren(_))),
            2
        );
        lifecycle.cancel_where(|target| matches!(target, TaskTarget::CommentChildren(_)));
        for dropped_rx in dropped {
            tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
                .await
                .expect("keyed task was not cancelled")
                .expect("drop signal closed");
        }
    }

    #[tokio::test]
    async fn failures_use_the_single_failure_event() {
        let (mut lifecycle, mut rx) = lifecycle();
        let task = lifecycle.spawn(
            TaskTarget::SettingsSave,
            async { Err::<(), _>(anyhow::anyhow!("save failed")) },
            |_task, ()| Event::Completed,
        );

        let event = rx.recv().await.expect("failure event");
        match event {
            Event::Failed(event_task, message) => {
                assert_eq!(event_task, task);
                assert_eq!(message, "save failed");
            }
            Event::Completed => panic!("expected failure event"),
        }
    }
}
