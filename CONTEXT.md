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

### Interaction

**Action**:
The semantic vocabulary of user intent. Every keypress or mouse gesture resolves to an Action before any state changes.
_Avoid_: command, keybinding (a keybinding maps to an Action; it isn't one)

**AppEvent**:
The single seam through which every async result re-enters the app loop.

**Generation**:
The staleness stamp on async work; a result whose generation is no longer current is discarded.
_Avoid_: version, epoch

### Modules

**CommentLayout**:
The module that owns comment line geometry — which lines exist, which comment a line belongs to, and what is visible. The heights-match-lines invariant lives here and nowhere else.

**Summarizer**:
The core that turns a story plus its comments into a stream of summary events via an LLM.
_Avoid_: plugin (there is no plugin system; one adapter does not make a seam)

**SummaryOverlay**:
The view that presents the Summarizer's output — scrolling, copying, streaming display.
_Avoid_: plugin overlay
