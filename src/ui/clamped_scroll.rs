#[derive(Default)]
pub(super) struct ClampedScroll {
    offset: usize,
    viewport_height: usize,
    content_height: usize,
}

impl ClampedScroll {
    pub(super) fn set_extents(&mut self, content_height: usize, viewport_height: usize) {
        self.content_height = content_height;
        self.viewport_height = viewport_height;
        self.clamp();
    }

    pub(super) fn set_content_height(&mut self, content_height: usize) {
        self.content_height = content_height;
        self.clamp();
    }

    pub(super) fn set_viewport_height(&mut self, viewport_height: usize) {
        self.viewport_height = viewport_height;
        self.clamp();
    }

    pub(super) fn scroll_down(&mut self, amount: usize) {
        self.offset = self.offset.saturating_add(amount);
        self.clamp();
    }

    pub(super) fn scroll_up(&mut self, amount: usize) {
        self.offset = self.offset.saturating_sub(amount);
    }

    pub(super) fn go_top(&mut self) {
        self.offset = 0;
    }

    pub(super) fn go_bottom(&mut self) {
        self.offset = self.max_offset();
    }

    pub(super) fn page_amount(&self) -> usize {
        self.viewport_height.saturating_sub(2).max(1)
    }

    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    pub(super) fn content_height(&self) -> usize {
        self.content_height
    }

    pub(super) fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    pub(super) fn max_offset(&self) -> usize {
        self.content_height.saturating_sub(self.viewport_height)
    }

    pub(super) fn render_offset(&self) -> u16 {
        self.offset
            .try_into()
            .expect("clamped scroll offset exceeds ratatui's u16 limit")
    }

    fn clamp(&mut self) {
        self.offset = self.offset.min(self.max_offset());
    }
}
