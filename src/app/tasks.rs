use super::{App, AppEvent};
use std::future::Future;
use tokio::task::JoinHandle;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Generation(u64);

impl Generation {
    pub(crate) fn advance(&mut self) -> Self {
        self.0 = self.0.wrapping_add(1);
        *self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadTarget {
    Stories,
    Comments,
    Search,
}

impl App {
    pub(crate) fn is_current_generation(&self, target: LoadTarget, generation: Generation) -> bool {
        match target {
            LoadTarget::Stories => self.stories_generation == generation,
            LoadTarget::Comments => self.comments_generation == generation,
            LoadTarget::Search => self.search_generation == generation,
        }
    }

    pub(crate) fn spawn_load<T, Fut, OkEvent>(
        &self,
        target: LoadTarget,
        generation: Generation,
        future: Fut,
        ok_event: OkEvent,
    ) -> JoinHandle<()>
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        OkEvent: FnOnce(T) -> AppEvent + Send + 'static,
    {
        self.spawn_fetch(future, ok_event, move |message| AppEvent::Error {
            target,
            generation,
            message,
        })
    }

    pub(crate) fn spawn_load_detached<T, Fut, OkEvent>(
        &self,
        target: LoadTarget,
        generation: Generation,
        future: Fut,
        ok_event: OkEvent,
    ) where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        OkEvent: FnOnce(T) -> AppEvent + Send + 'static,
    {
        std::mem::drop(self.spawn_load(target, generation, future, ok_event));
    }

    pub(crate) fn spawn_fetch<T, Fut, OkEvent, ErrEvent>(
        &self,
        future: Fut,
        ok_event: OkEvent,
        err_event: ErrEvent,
    ) -> JoinHandle<()>
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        OkEvent: FnOnce(T) -> AppEvent + Send + 'static,
        ErrEvent: FnOnce(String) -> AppEvent + Send + 'static,
    {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match future.await {
                Ok(value) => {
                    let _ = tx.send(ok_event(value));
                }
                Err(err) => {
                    let _ = tx.send(err_event(format!("{err:#}")));
                }
            }
        })
    }

    pub(crate) fn spawn_fetch_detached<T, Fut, OkEvent, ErrEvent>(
        &self,
        future: Fut,
        ok_event: OkEvent,
        err_event: ErrEvent,
    ) where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        OkEvent: FnOnce(T) -> AppEvent + Send + 'static,
        ErrEvent: FnOnce(String) -> AppEvent + Send + 'static,
    {
        std::mem::drop(self.spawn_fetch(future, ok_event, err_event));
    }
}
