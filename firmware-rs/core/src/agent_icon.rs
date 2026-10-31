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

    /// `draw` at a whole-number scale.
    pub fn draw_scaled(self, canvas: &mut Canvas, x: i32, y: i32, scale: i32) {
        for (index, &color) in self.pixels().iter().enumerate() {
            if color != 0 {
                let index = index as i32;
                canvas.fill_rect(x + index % SIZE * scale, y + index / SIZE * scale, scale, scale, color);
            }
        }
    }

    /// The icon at `size` × `size` pixels, smaller than 2×: each screen pixel mixes the colours of the icon pixels it
    /// covers, weighted by how much of it they cover, over what is already drawn.
    pub fn draw_sized(self, canvas: &mut Canvas, x: i32, y: i32, size: i32) {
        let pixels = self.pixels();
        for ty in 0..size {
            for tx in 0..size {
                let (mut red, mut green, mut blue, mut cover) = (0, 0, 0, 0);
                // Screen pixel t spans [t·16, (t+1)·16) and icon pixel s spans [s·size, (s+1)·size), in 1/(16·size) units.
                for sy in ty * SIZE / size..=((ty + 1) * SIZE - 1) / size {
                    let height = ((sy + 1) * size).min((ty + 1) * SIZE) - (sy * size).max(ty * SIZE);
                    for sx in tx * SIZE / size..=((tx + 1) * SIZE - 1) / size {
                        let color = i32::from(pixels[(sy * SIZE + sx) as usize]);
                        if color == 0 {
                            continue;
                        }
                        let weight = height * (((sx + 1) * size).min((tx + 1) * SIZE) - (sx * size).max(tx * SIZE));
                        red += (color >> 11 & 0x1F) * weight;
                        green += (color >> 5 & 0x3F) * weight;
                        blue += (color & 0x1F) * weight;
                        cover += weight;
                    }
                }
                if cover > 0 {
                    let color = ((red / cover) << 11 | (green / cover) << 5 | (blue / cover)) as u16;
                    canvas.blend(x + tx, y + ty, color, ((cover * 15 + SIZE * SIZE / 2) / (SIZE * SIZE)) as u8);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawn_at_its_own_size_the_icon_is_its_pixels() {
        use crate::canvas::FRAME_BYTES;
        let (mut sized, mut plain) = (alloc::vec![0; FRAME_BYTES], alloc::vec![0; FRAME_BYTES]);
        AgentIcon::ClaudeCode.draw_sized(&mut Canvas::new(&mut sized), 3, 4, SIZE);
        AgentIcon::ClaudeCode.draw(&mut Canvas::new(&mut plain), 3, 4);
        assert_eq!(sized, plain);
    }

    #[test]
    fn agents_map_to_their_icons_and_prefixes() {
        assert_eq!(AgentIcon::for_agent("claude-code").map(AgentIcon::prefix), Some(&b"CC:"[..]));
        assert_eq!(AgentIcon::for_agent("github-actions"), Some(AgentIcon::Ci));
        assert_eq!(AgentIcon::for_agent("unknown"), None);
    }
}
