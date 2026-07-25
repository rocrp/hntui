use super::{App, AppEvent, TaskTarget};
use crate::api::Story;
use crate::article::{body_article, self_post_article, Article};
use crate::input::ArticleAction;
use crate::logging;
use std::collections::HashMap;

/// What asking for a Story's Article turned up. Callers (the `v` overlay and
/// the summarizer) decide how to surface each outcome.
#[derive(Debug)]
pub(crate) enum ArticleRequest {
    /// Already in hand — a memory hit or a self-post body.
    Ready(Article),
    /// A subprocess is running; the result arrives as an AppEvent.
    Fetching,
    /// Nothing to fetch: no link and no body.
    Unavailable,
}

impl App {
    /// Resolve a Story's Article, starting a fetch only when one is needed.
    /// Repeat requests for the same story join the in-flight fetch rather than
    /// restarting it, so `v` and `s` never fight over the subprocess.
    pub(crate) fn request_article(&mut self, story: &Story) -> ArticleRequest {
        if let Some(article) = self.articles.get(story.id) {
            return ArticleRequest::Ready(article.clone());
        }

        let story_id = story.id;
        if let Some(article) = self.local_article(story) {
            self.articles.insert(story_id, article.clone());
            return ArticleRequest::Ready(article);
        }

        // Nothing local to show. A job posting has neither a link nor a body
        // and no discussion to hide one in, so there is nothing to ask for.
        if story.url.is_none() && story.comment_count == 0 && story.kids.is_empty() {
            return ArticleRequest::Unavailable;
        }

        if self.tasks.is_running(TaskTarget::Article(story_id)) {
            return ArticleRequest::Fetching;
        }
        match story.url.clone() {
            Some(url) => self.spawn_linked_page_fetch(story_id, url),
            None => self.spawn_self_post_body_fetch(story.clone()),
        }
        ArticleRequest::Fetching
    }

    /// The Article we can produce without asking anyone: a self-post body,
    /// from the Story itself or from a discussion we already prefetched.
    fn local_article(&self, story: &Story) -> Option<Article> {
        if story.url.is_some() {
            return None;
        }
        self_post_article(story).or_else(|| {
            let thread = self.prefetched_comments_cache.peek(story.id)?;
            body_article(&story.title, thread.text.as_deref())
        })
    }

    fn spawn_linked_page_fetch(&mut self, story_id: u64, url: String) {
        let fetcher = self.article_fetcher.clone();
        self.tasks.spawn(
            TaskTarget::Article(story_id),
            async move { fetcher.fetch(url).await.map_err(anyhow::Error::from) },
            move |task, article| AppEvent::ArticleLoaded {
                task,
                story_id,
                article,
            },
        );
    }

    /// hackerweb reveals a self-post's body only with its discussion, never in
    /// the feed listing, so resolving one is a Source request — not a
    /// subprocess. Firebase stories already carry the body and never get here.
    fn spawn_self_post_body_fetch(&mut self, story: Story) {
        let story_id = story.id;
        let title = story.title.clone();
        let source = self.sources.stories.clone();
        self.tasks.spawn(
            TaskTarget::Article(story_id),
            async move {
                let thread = source.comment_roots(story).await?;
                body_article(&title, thread.text.as_deref())
                    .ok_or_else(|| anyhow::anyhow!("story has no article"))
            },
            move |task, article| AppEvent::ArticleLoaded {
                task,
                story_id,
                article,
            },
        );
    }

    pub(crate) fn cancel_article_fetch(&mut self, story_id: u64) {
        self.tasks.cancel(TaskTarget::Article(story_id));
    }

    /// `v`: show the Article for a story, fetching it only if we must.
    pub(super) fn open_article_overlay(&mut self, story: &Story) {
        match self.request_article(story) {
            ArticleRequest::Ready(article) => self.article_overlay.show(story, article),
            ArticleRequest::Fetching => self.article_overlay.begin(story),
            ArticleRequest::Unavailable => self
                .article_overlay
                .show_error(story, "story has no article".to_string()),
        }
    }

    /// Fan a settled fetch out to whoever is waiting on it. Both the overlay
    /// and a pending summarize can be waiting on the same fetch.
    pub(super) fn deliver_article(&mut self, story_id: u64, result: Result<Article, String>) {
        if let Err(message) = &result {
            logging::log_error(format!(
                "article fetch failed story_id={story_id}: {message}"
            ));
        }

        if self.article_overlay.is_visible() && self.article_overlay.story_id() == story_id {
            match result.clone() {
                Ok(article) => self.article_overlay.finish(article),
                Err(message) => self.article_overlay.fail(message),
            }
        }

        self.settle_pending_summary_article(story_id, result);
    }

    pub(super) fn handle_article_action(&mut self, action: ArticleAction) {
        match action {
            ArticleAction::Dismiss => {
                self.cancel_article_fetch(self.article_overlay.story_id());
                self.article_overlay.dismiss();
            }
            ArticleAction::ScrollDown(amount) => self.article_overlay.scroll_down(amount),
            ArticleAction::ScrollUp(amount) => self.article_overlay.scroll_up(amount),
            ArticleAction::PageDown => {
                let amount = self.article_overlay.page_scroll_amount();
                self.article_overlay.scroll_down(amount);
            }
            ArticleAction::PageUp => {
                let amount = self.article_overlay.page_scroll_amount();
                self.article_overlay.scroll_up(amount);
            }
            ArticleAction::GoTop => self.article_overlay.go_top(),
            ArticleAction::GoBottom => self.article_overlay.go_bottom(),
            ArticleAction::Copy => {
                if let Err(error) = self.article_overlay.copy_article() {
                    self.last_error = Some(format!("clipboard: {error:#}"));
                }
            }
            ArticleAction::OpenBrowser => self.open_article_source_in_browser(),
            ArticleAction::OpenHelp => self.help_overlay.open(),
        }
    }

    fn open_article_source_in_browser(&mut self) {
        let url = self
            .article_overlay
            .story_url()
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "https://news.ycombinator.com/item?id={}",
                    self.article_overlay.story_id()
                )
            });
        match crate::browser::open_url(&url) {
            Ok(crate::browser::OpenOutcome::CopiedToClipboard) => {
                self.copied_flash = Some(std::time::Instant::now());
            }
            Ok(crate::browser::OpenOutcome::Launched) => {}
            Err(error) => self.last_error = Some(format!("{error:#}")),
        }
    }
}

/// Articles kept for the session, so a second `v` (or a follow-up summarize)
/// is instant. Bounded and oldest-first — articles are large and unranked, so
/// there is no proximity signal worth the bookkeeping.
pub(crate) struct ArticleStore {
    entries: HashMap<u64, Article>,
    order: Vec<u64>,
    capacity: usize,
}

impl ArticleStore {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "article store capacity must be > 0");
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    pub(crate) fn get(&self, story_id: u64) -> Option<&Article> {
        self.entries.get(&story_id)
    }

    pub(crate) fn insert(&mut self, story_id: u64, article: Article) {
        if self.entries.insert(story_id, article).is_none() {
            self.order.push(story_id);
        }
        while self.order.len() > self.capacity {
            let evicted = self.order.remove(0);
            self.entries.remove(&evicted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ARTICLE_CACHE_CAP;

    fn article(marker: &str) -> Article {
        Article {
            title: None,
            content: marker.to_string(),
        }
    }

    #[test]
    fn the_store_evicts_the_oldest_article_once_full() {
        let mut store = ArticleStore::new(2);

        store.insert(1, article("one"));
        store.insert(2, article("two"));
        store.insert(3, article("three"));

        assert!(store.get(1).is_none());
        assert_eq!(store.get(2).map(|a| a.content.as_str()), Some("two"));
        assert_eq!(store.get(3).map(|a| a.content.as_str()), Some("three"));
    }

    #[test]
    fn reinserting_a_story_replaces_it_without_growing_the_order() {
        let mut store = ArticleStore::new(2);

        store.insert(1, article("one"));
        store.insert(1, article("one-again"));
        store.insert(2, article("two"));

        assert_eq!(store.order, vec![1, 2]);
        assert_eq!(store.get(1).map(|a| a.content.as_str()), Some("one-again"));
    }

    #[test]
    fn the_capacity_is_the_configured_one() {
        let mut store = ArticleStore::new(ARTICLE_CACHE_CAP);
        for id in 0..(ARTICLE_CACHE_CAP as u64 + 5) {
            store.insert(id, article("x"));
        }

        assert_eq!(store.order.len(), ARTICLE_CACHE_CAP);
        assert!(store.get(0).is_none());
    }
}
