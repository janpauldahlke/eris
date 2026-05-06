#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterState {
    Content,
    MatchThink(usize), // Tracks matched bytes of "<think>"
    Thought,
    MatchEndThink(usize), // Tracks matched bytes of "</think>"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsmEvent {
    Content(String),
    Thought(String),
}

pub struct ReasoningRouter {
    pub state: RouterState,
    pub is_enabled: bool,
    pub content_buffer: Vec<u8>,
}

impl ReasoningRouter {
    pub fn new(is_enabled: bool) -> Self {
        Self {
            state: RouterState::Content,
            is_enabled,
            content_buffer: Vec::new(),
        }
    }

    pub fn process_chunk(&mut self, chunk: &str) -> Vec<FsmEvent> {
        if !self.is_enabled {
            return vec![FsmEvent::Content(chunk.to_string())];
        }

        let mut events = Vec::new();
        let think_tag = b"<think>";
        let end_think_tag = b"</think>";

        for &byte in chunk.as_bytes() {
            match self.state {
                RouterState::Content => {
                    if byte == think_tag[0] {
                        self.state = RouterState::MatchThink(1);
                        self.content_buffer.push(byte);
                    } else {
                        self.content_buffer.push(byte);
                    }
                }
                RouterState::MatchThink(matched) => {
                    if byte == think_tag[matched] {
                        self.state = RouterState::MatchThink(matched + 1);
                        self.content_buffer.push(byte);
                        if matched + 1 == think_tag.len() {
                            self.state = RouterState::Thought;
                            self.content_buffer
                                .truncate(self.content_buffer.len() - think_tag.len());
                            if !self.content_buffer.is_empty() {
                                let content =
                                    String::from_utf8_lossy(&self.content_buffer).into_owned();
                                events.push(FsmEvent::Content(content));
                                self.content_buffer.clear();
                            }
                        }
                    } else {
                        self.state = RouterState::Content;
                        self.content_buffer.push(byte);
                    }
                }
                RouterState::Thought => {
                    if byte == end_think_tag[0] {
                        self.state = RouterState::MatchEndThink(1);
                        self.content_buffer.push(byte);
                    } else {
                        self.content_buffer.push(byte);
                    }
                }
                RouterState::MatchEndThink(matched) => {
                    if byte == end_think_tag[matched] {
                        self.state = RouterState::MatchEndThink(matched + 1);
                        self.content_buffer.push(byte);
                        if matched + 1 == end_think_tag.len() {
                            self.state = RouterState::Content;
                            self.content_buffer
                                .truncate(self.content_buffer.len() - end_think_tag.len());
                            if !self.content_buffer.is_empty() {
                                let thought =
                                    String::from_utf8_lossy(&self.content_buffer).into_owned();
                                events.push(FsmEvent::Thought(thought));
                                self.content_buffer.clear();
                            }
                        }
                    } else {
                        self.state = RouterState::Thought;
                        self.content_buffer.push(byte);
                    }
                }
            }
        }

        match self.state {
            RouterState::Content => {
                if !self.content_buffer.is_empty() {
                    let content = String::from_utf8_lossy(&self.content_buffer).into_owned();
                    events.push(FsmEvent::Content(content));
                    self.content_buffer.clear();
                }
            }
            RouterState::Thought => {
                if !self.content_buffer.is_empty() {
                    let thought = String::from_utf8_lossy(&self.content_buffer).into_owned();
                    events.push(FsmEvent::Thought(thought));
                    self.content_buffer.clear();
                }
            }
            _ => {}
        }

        events
    }

    pub fn finish(&mut self) -> Vec<FsmEvent> {
        if !self.is_enabled {
            return Vec::new();
        }

        let mut events = Vec::new();
        if !self.content_buffer.is_empty() {
            let content = String::from_utf8_lossy(&self.content_buffer).into_owned();
            match self.state {
                RouterState::Content | RouterState::MatchThink(_) => {
                    events.push(FsmEvent::Content(content));
                }
                RouterState::Thought | RouterState::MatchEndThink(_) => {
                    events.push(FsmEvent::Thought(content));
                }
            }
            self.content_buffer.clear();
        }
        self.state = RouterState::Content;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_disabled_bypasses_fsm() {
        let mut router = ReasoningRouter::new(false);
        let events = router.process_chunk("Hello <think>world</think>!");

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            FsmEvent::Content("Hello <think>world</think>!".to_string())
        );
    }

    #[test]
    fn test_router_single_chunk() {
        let mut router = ReasoningRouter::new(true);
        let events = router.process_chunk("Hello <think>world</think>!");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], FsmEvent::Content("Hello ".to_string()));
        assert_eq!(events[1], FsmEvent::Thought("world".to_string()));
        assert_eq!(events[2], FsmEvent::Content("!".to_string()));
    }

    #[test]
    fn test_router_split_tags() {
        let mut router = ReasoningRouter::new(true);
        let mut events = router.process_chunk("Hello <th");
        assert!(events.is_empty());

        events.extend(router.process_chunk("ink>wor"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], FsmEvent::Content("Hello ".to_string()));
        assert_eq!(events[1], FsmEvent::Thought("wor".to_string()));

        events.extend(router.process_chunk("ld</th"));
        // At the end of "ld</th", the router is in MatchEndThink.
        // It holds "ld</th" in its buffer and does NOT emit anything for this chunk.
        assert_eq!(events.len(), 2);

        events.extend(router.process_chunk("ink>!"));
        // The chunk "ink>!" completes the "</think>" tag.
        // The buffer contains "ld</think>!".
        // The tag matching logic removes "</think>", leaving "ld".
        // It emits Thought("ld").
        // Then it processes "!", which goes into Content state, and emits Content("!").
        assert_eq!(events.len(), 4);
        assert_eq!(events[2], FsmEvent::Thought("ld".to_string()));
        assert_eq!(events[3], FsmEvent::Content("!".to_string()));
    }

    #[test]
    fn test_router_broken_tag() {
        let mut router = ReasoningRouter::new(true);
        let events = router.process_chunk("Hello <thiXworld");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], FsmEvent::Content("Hello <thiXworld".to_string()));
    }
}
