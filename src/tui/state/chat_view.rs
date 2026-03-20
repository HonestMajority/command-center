use ratatui::text::Line;

use crate::tui::chat::AssistantChat;

/// Cached rendered lines for the chat panel, invalidated on content or width change.
pub struct ChatRenderCache {
    pub lines: Vec<Line<'static>>,
    /// Wrapped line count (from `Paragraph::line_count(width)`).
    pub wrapped_line_count: usize,
    /// Generation counter of the `AssistantChat` when cache was built.
    generation: u64,
    /// Terminal width when cache was built — must invalidate on resize.
    width: u16,
}

pub struct ChatViewState {
    pub assistant: AssistantChat,
    chat_scroll: u16,
    chat_viewport_height: u16,
    /// Render cache for assistant chat lines.
    pub render_cache: Option<ChatRenderCache>,
}

impl ChatViewState {
    pub(super) fn new(assistant: AssistantChat) -> Self {
        Self {
            assistant,
            chat_scroll: 0,
            chat_viewport_height: 0,
            render_cache: None,
        }
    }

    /// Returns true if the cached lines are still valid for the current state.
    pub fn cache_valid(&self, width: u16) -> bool {
        self.render_cache
            .as_ref()
            .is_some_and(|c| c.generation == self.assistant.generation && c.width == width)
    }

    /// Store a freshly built set of lines into the cache.
    pub fn set_cache(&mut self, lines: Vec<Line<'static>>, wrapped_line_count: usize, width: u16) {
        self.render_cache = Some(ChatRenderCache {
            lines,
            wrapped_line_count,
            generation: self.assistant.generation,
            width,
        });
    }

    pub fn chat_scroll(&self) -> u16 {
        self.chat_scroll
    }

    pub fn reset_scroll(&mut self) {
        self.chat_scroll = 0;
    }

    pub fn update_chat_viewport_height(&mut self, area_height: u16) {
        self.chat_viewport_height = area_height.saturating_sub(2);
    }

    pub fn scroll_chat_up(&mut self) {
        let half = (self.chat_viewport_height / 2).max(1);
        self.chat_scroll = self.chat_scroll.saturating_add(half);
    }

    pub fn scroll_chat_down(&mut self) {
        let half = (self.chat_viewport_height / 2).max(1);
        self.chat_scroll = self.chat_scroll.saturating_sub(half);
    }
}
