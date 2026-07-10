use crate::api::{CommentNode, FeedKind, Story, StorySource};
use futures::future::BoxFuture;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

pub(super) struct RootRequest {
    started: tokio::sync::oneshot::Sender<u64>,
    result: tokio::sync::oneshot::Receiver<anyhow::Result<Vec<CommentNode>>>,
    dropped: tokio::sync::oneshot::Sender<()>,
}

pub(super) struct RootControl {
    pub(super) started: tokio::sync::oneshot::Receiver<u64>,
    pub(super) result: tokio::sync::oneshot::Sender<anyhow::Result<Vec<CommentNode>>>,
    pub(super) dropped: tokio::sync::oneshot::Receiver<()>,
}

pub(super) fn controlled_root_request() -> (RootRequest, RootControl) {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    (
        RootRequest {
            started: started_tx,
            result: result_rx,
            dropped: dropped_tx,
        },
        RootControl {
            started: started_rx,
            result: result_tx,
            dropped: dropped_rx,
        },
    )
}

#[derive(Clone)]
pub(super) struct ControlledStorySource {
    stories: Vec<Story>,
    root_requests: Arc<Mutex<VecDeque<RootRequest>>>,
}

impl ControlledStorySource {
    pub(super) fn new(stories: Vec<Story>, root_requests: Vec<RootRequest>) -> Self {
        Self {
            stories,
            root_requests: Arc::new(Mutex::new(root_requests.into())),
        }
    }
}

impl StorySource for ControlledStorySource {
    fn initial_stories(
        &self,
        _feed: FeedKind,
        count: usize,
    ) -> BoxFuture<'static, anyhow::Result<(Vec<u64>, Vec<Story>)>> {
        let stories = self.stories.clone();
        Box::pin(async move {
            let story_ids = stories.iter().map(|story| story.id).collect();
            Ok((story_ids, stories.into_iter().take(count).collect()))
        })
    }

    fn more_stories(
        &self,
        _feed: FeedKind,
        _story_ids: Vec<u64>,
        _loaded_count: usize,
        _page_size: usize,
    ) -> BoxFuture<'static, anyhow::Result<Vec<Story>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn comment_roots(&self, story: Story) -> BoxFuture<'static, anyhow::Result<Vec<CommentNode>>> {
        let request = self
            .root_requests
            .lock()
            .expect("controlled source lock poisoned")
            .pop_front()
            .expect("missing controlled comment-root response");
        Box::pin(async move {
            let _drop_signal = DropSignal(Some(request.dropped));
            if request.started.send(story.id).is_err() {
                anyhow::bail!("controlled comment-root observer dropped");
            }
            request
                .result
                .await
                .map_err(|_| anyhow::anyhow!("controlled comment-root response dropped"))?
        })
    }

    fn comment_children(
        &self,
        _ids: Vec<u64>,
        _depth: usize,
    ) -> BoxFuture<'static, anyhow::Result<Vec<CommentNode>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
