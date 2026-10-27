//! The small icon a task card shows for its agent, in place of the text prefix the daemon puts in its title
//! (#155). The agents' icons are their makers' marks reduced to 16 × 16 by `tools/make-agent-icons.py`, which
//! names the sources; CI isn't an agent, so its icon is our own. An agent without an icon keeps its prefix.

use crate::canvas::Canvas;

mod pixels;

/// Icons are this many pixels square.
pub const SIZE: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentIcon {
    ClaudeCode,
    Codex,
    OpenCode,
    Copilot,
    Pi,
    /// GitHub Actions runs.
    Ci,
}

impl AgentIcon {
    /// The icon for an agent as the daemon names it (`agent` in `tasks`).
    pub fn for_agent(agent: &str) -> Option<Self> {
        Some(match agent {
            "claude-code" => Self::ClaudeCode,
            "codex" => Self::Codex,
            "opencode" => Self::OpenCode,
            "copilot" => Self::Copilot,
            "pi" => Self::Pi,
            "github-actions" => Self::Ci,
            _ => return None,
        })
    }

    /// The prefix the daemon gives this agent's titles, which the icon replaces.
    pub fn prefix(self) -> &'static [u8] {
        match self {
            Self::ClaudeCode => b"CC:",
            Self::Codex => b"CX:",
            Self::OpenCode => b"OC:",
            Self::Copilot => b"CP:",
            Self::Pi => b"PI:",
            Self::Ci => b"CI:",
        }
    }

    fn pixels(self) -> &'static [u16; (SIZE * SIZE) as usize] {
        match self {
            Self::ClaudeCode => &pixels::CLAUDE,
            Self::Codex => &pixels::CODEX,
            Self::OpenCode => &pixels::OPENCODE,
            Self::Copilot => &pixels::COPILOT,
            Self::Pi => &pixels::PI,
            Self::Ci => &pixels::CI,
        }
    }

    pub fn draw(self, canvas: &mut Canvas, x: i32, y: i32) {
        self.draw_scaled(canvas, x, y, 1);
    }

    /// `draw` at a whole-number scale; the korekore screens draw the icons at 2×.
    pub fn draw_scaled(self, canvas: &mut Canvas, x: i32, y: i32, scale: i32) {
        for (index, &color) in self.pixels().iter().enumerate() {
            if color != 0 {
                let index = index as i32;
                canvas.fill_rect(x + index % SIZE * scale, y + index / SIZE * scale, scale, scale, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_map_to_their_icons_and_prefixes() {
        assert_eq!(AgentIcon::for_agent("claude-code").map(AgentIcon::prefix), Some(&b"CC:"[..]));
        assert_eq!(AgentIcon::for_agent("github-actions"), Some(AgentIcon::Ci));
        assert_eq!(AgentIcon::for_agent("unknown"), None);
    }
}
