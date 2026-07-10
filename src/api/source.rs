use super::{CommentNode, FeedKind, HnClient, SearchClient, Story};
use anyhow::Result;
use futures::future::BoxFuture;
use std::sync::Arc;

pub trait StorySource: Send + Sync {
    fn initial_stories(
        &self,
        feed: FeedKind,
        count: usize,
    ) -> BoxFuture<'static, Result<(Vec<u64>, Vec<Story>)>>;

    fn more_stories(
        &self,
        feed: FeedKind,
        story_ids: Vec<u64>,
        loaded_count: usize,
        page_size: usize,
    ) -> BoxFuture<'static, Result<Vec<Story>>>;

    fn comment_roots(&self, story: Story) -> BoxFuture<'static, Result<Vec<CommentNode>>>;

    fn comment_children(
        &self,
        ids: Vec<u64>,
        depth: usize,
    ) -> BoxFuture<'static, Result<Vec<CommentNode>>>;
}

pub trait SearchSource: Send + Sync {
    fn search(&self, query: String) -> BoxFuture<'static, Result<Vec<Story>>>;
}

#[derive(Clone)]
pub struct Sources {
    pub(crate) stories: Arc<dyn StorySource>,
    pub(crate) search: Arc<dyn SearchSource>,
}

impl Sources {
    pub fn new(stories: Arc<dyn StorySource>, search: Arc<dyn SearchSource>) -> Self {
        Self { stories, search }
    }
}

impl StorySource for HnClient {
    fn initial_stories(
        &self,
        feed: FeedKind,
        count: usize,
    ) -> BoxFuture<'static, Result<(Vec<u64>, Vec<Story>)>> {
        let source = self.clone();
        Box::pin(async move { source.fetch_initial_stories(feed, count).await })
    }

    fn more_stories(
        &self,
        feed: FeedKind,
        story_ids: Vec<u64>,
        loaded_count: usize,
        page_size: usize,
    ) -> BoxFuture<'static, Result<Vec<Story>>> {
        let source = self.clone();
        Box::pin(async move {
            source
                .fetch_more_stories(feed, &story_ids, loaded_count, page_size)
                .await
        })
    }

    fn comment_roots(&self, story: Story) -> BoxFuture<'static, Result<Vec<CommentNode>>> {
        let source = self.clone();
        Box::pin(async move { source.fetch_comment_roots(&story).await })
    }

    fn comment_children(
        &self,
        ids: Vec<u64>,
        depth: usize,
    ) -> BoxFuture<'static, Result<Vec<CommentNode>>> {
        let source = self.clone();
        Box::pin(async move { source.fetch_comment_children(&ids, depth).await })
    }
}

impl SearchSource for SearchClient {
    fn search(&self, query: String) -> BoxFuture<'static, Result<Vec<Story>>> {
        let source = self.clone();
        Box::pin(async move { source.search_stories(&query).await })
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct InMemorySource {
    stories: Vec<Story>,
    comments: std::collections::HashMap<u64, Vec<CommentNode>>,
    children: std::collections::HashMap<u64, CommentNode>,
    searches: std::collections::HashMap<String, Vec<Story>>,
    initial_error: Option<String>,
}

#[cfg(test)]
impl InMemorySource {
    pub fn new(stories: Vec<Story>) -> Self {
        Self {
            stories,
            ..Self::default()
        }
    }

    pub fn with_comments(mut self, story_id: u64, comments: Vec<CommentNode>) -> Self {
        self.comments.insert(story_id, comments);
        self
    }

    pub fn with_children(mut self, children: Vec<CommentNode>) -> Self {
        self.children
            .extend(children.into_iter().map(|node| (node.comment.id, node)));
        self
    }

    pub fn with_search(mut self, query: impl Into<String>, stories: Vec<Story>) -> Self {
        self.searches.insert(query.into(), stories);
        self
    }

    pub fn with_initial_error(mut self, message: impl Into<String>) -> Self {
        self.initial_error = Some(message.into());
        self
    }
}

#[cfg(test)]
impl StorySource for InMemorySource {
    fn initial_stories(
        &self,
        _feed: FeedKind,
        count: usize,
    ) -> BoxFuture<'static, Result<(Vec<u64>, Vec<Story>)>> {
        let stories = self.stories.clone();
        let error = self.initial_error.clone();
        Box::pin(async move {
            if let Some(message) = error {
                anyhow::bail!(message);
            }
            let story_ids = stories.iter().map(|story| story.id).collect();
            let stories = stories.into_iter().take(count).collect();
            Ok((story_ids, stories))
        })
    }

    fn more_stories(
        &self,
        _feed: FeedKind,
        _story_ids: Vec<u64>,
        loaded_count: usize,
        page_size: usize,
    ) -> BoxFuture<'static, Result<Vec<Story>>> {
        let stories = self
            .stories
            .iter()
            .skip(loaded_count)
            .take(page_size)
            .cloned()
            .collect();
        Box::pin(async move { Ok(stories) })
    }

    fn comment_roots(&self, story: Story) -> BoxFuture<'static, Result<Vec<CommentNode>>> {
        let comments = self.comments.get(&story.id).cloned().unwrap_or_default();
        Box::pin(async move { Ok(comments) })
    }

    fn comment_children(
        &self,
        ids: Vec<u64>,
        _depth: usize,
    ) -> BoxFuture<'static, Result<Vec<CommentNode>>> {
        let children = ids
            .iter()
            .filter_map(|id| self.children.get(id).cloned())
            .collect();
        Box::pin(async move { Ok(children) })
    }
}

#[cfg(test)]
impl SearchSource for InMemorySource {
    fn search(&self, query: String) -> BoxFuture<'static, Result<Vec<Story>>> {
        let stories = self.searches.get(&query).cloned().unwrap_or_default();
        Box::pin(async move { Ok(stories) })
    }
}
