# hntui

A terminal client for reading Hacker News: browse feeds, read comment threads, search, and summarize discussions with an LLM.

## Language

### Content

**Story**:
A Hacker News item that appears in a feed listing (story, job, or poll), normalized to one shape regardless of which backend produced it.
_Avoid_: item, post, hit

**Comment**:
A single reply in a story's discussion tree.

**Feed**:
One of the HN listings a user can browse (top, new, ask, show, …).
_Avoid_: list, tab

**Source**:
Where the app obtains stories, comments, or search results. Adapters at this seam: the HN client, Algolia search, and an in-memory fixture for tests.
_Avoid_: client (when meaning the seam), backend (reserved for the HN API flavor)

**StoryThread**:
What a Source returns for one story's discussion: the root Comments plus the self-post body, when the backend reports one. Exists because hackerweb only reveals the body alongside the comments (`/item/:id`), never in the feed listing.
_Avoid_: comment roots (that is only half of it), discussion

**Article**:
The original content a Story points to — the extracted text of the linked page, or the story's own body for a self-post.
_Avoid_: page, webpage (an Article is extracted text, not the rendered page), original text

**Article Link**:
A navigable HTTP(S) destination embedded in an Article, distinct from the Story's original URL.
_Avoid_: source link, hyperlink

### Interaction

**Action**:
The semantic vocabulary of user intent. Every keypress or mouse gesture resolves to an Action before any state changes.
_Avoid_: command, keybinding (a keybinding maps to an Action; it isn't one)

**AppEvent**:
The single seam through which every async result re-enters the app loop.

**Generation**:
The staleness stamp on async work; a result whose generation is no longer current is discarded.
_Avoid_: version, epoch

**ResolvedEndpoint**:
The exact URL the Summarizer's request will be sent to, derived from the Model's provider prefix and the Base URL.
_Avoid_: final URL, parsed URL

**ConnectionTest**:
The settings action that verifies the draft LLM configuration by sending a minimal real request along the Summarizer's exact path.
_Avoid_: ping, health check (it verifies the full chain, not host liveness)

### Modules

**CommentLayout**:
The module that owns comment line geometry — which lines exist, which comment a line belongs to, and what is visible. The heights-match-lines invariant lives here and nowhere else.

**Summarizer**:
The core that turns a story plus its comments into a stream of summary events via an LLM.
_Avoid_: plugin (there is no plugin system; one adapter does not make a seam)

**SummaryOverlay**:
The view that presents the Summarizer's output — scrolling, copying, streaming display.
_Avoid_: plugin overlay

**ArticleFetcher**:
The seam that obtains an Article for a Story. Adapters at this seam: the localwebrs subprocess for linked pages; the story's own body for a self-post — read straight off the Story when the backend supplied it, otherwise from the StoryThread the Source returns.
_Avoid_: visitor, scraper (localwebrs vocabulary; the seam is hntui's)

**ArticleOverlay**:
The view that presents an Article — scrolling, copying, selecting and opening Article Links, and opening the Story's original URL.
