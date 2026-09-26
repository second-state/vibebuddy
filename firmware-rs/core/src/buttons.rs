//! 三个键各管一件事，与模式无关。短按都在松开时触发；按住超过阈值会
//! 触发长按，且松开时不再算一次短按。
//!
//! 板子上（ATK-DNESP32S3-BOX V1.1）K0 是 BOOT 键，直连 GPIO0；K1、K2 在
//! XL9555 扩展口的 P0.4、P0.3。三个都低电平有效。读引脚是设备层的事，
//! 这里只管去抖与长短按。

/// 一次翻转生效后，这么久之内不再接受第二次翻转。
const DEBOUNCE_MS: u32 = 40;
const LONG_PRESS_MS: u32 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonEvent {
    /// K0：番茄钟的开始 / 暂停 / 继续。
    K0Short,
    /// K0 长按：放弃当前阶段。
    K0Long,
    /// K1：在值班与番茄钟之间切换。
    K1Short,
    /// K1 长按：让小灯灵现在就去休闲。
    K1Long,
    /// K2：打开当前来源，上报 Mac。
    K2Short,
    /// K2 长按：静音开关。
    K2Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    None,
    Short,
    Long,
}

#[derive(Clone, Copy)]
struct Button {
    pressed: bool,
    changed_at: u32,
    pressed_at: u32,
    /// 长按已经触发过，松开时就不再算一次短按。
    long_fired: bool,
}

impl Button {
    fn new(pressed: bool, now: u32) -> Self {
        Self { pressed, changed_at: now, pressed_at: now, long_fired: false }
    }

    /// 翻转立即生效，只在翻转后 DEBOUNCE_MS 内忽略再次翻转。机械抖动只有几毫秒，
    /// 而主循环每 20 ms 才采样一次；要求两次采样一致并不能多滤掉什么抖动，
    /// 却会把一次短促的轻点整个丢掉——第一次实机验收时 K1 有一半按了没反应。
    ///
    /// 短按在松开时才算数：只有等到松开，才知道它不是一次长按的开头。
    fn update(&mut self, pressed: bool, now: u32) -> Press {
        if pressed != self.pressed {
            if (now.wrapping_sub(self.changed_at) as i32) < DEBOUNCE_MS as i32 {
                return Press::None;
            }
            self.pressed = pressed;
            self.changed_at = now;
            if pressed {
                self.pressed_at = now;
                self.long_fired = false;
                return Press::None;
            }
            return if self.long_fired { Press::None } else { Press::Short };
        }
        if pressed && !self.long_fired && now.wrapping_sub(self.pressed_at) as i32 >= LONG_PRESS_MS as i32 {
            self.long_fired = true;
            return Press::Long;
        }
        Press::None
    }
}

pub struct Buttons {
    k0: Button,
    k1: Button,
    k2: Button,
}

/// 一次采样里三个键各自的按下状态（true 为按下）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Levels {
    pub k0: bool,
    pub k1: bool,
    pub k2: bool,
}

impl Buttons {
    pub fn new(levels: Levels, now: u32) -> Self {
        Self { k0: Button::new(levels.k0, now), k1: Button::new(levels.k1, now), k2: Button::new(levels.k2, now) }
    }

    /// K0 与扩展口是分开读的：扩展口读失败时只更新 K0，和 C 固件一样。
    pub fn update(&mut self, k0: bool, expander: Option<(bool, bool)>, now: u32) -> [Option<ButtonEvent>; 3] {
        fn emit(press: Press, short: ButtonEvent, long: ButtonEvent) -> Option<ButtonEvent> {
            match press {
                Press::None => None,
                Press::Short => Some(short),
                Press::Long => Some(long),
            }
        }
        let mut events = [None; 3];
        events[0] = emit(self.k0.update(k0, now), ButtonEvent::K0Short, ButtonEvent::K0Long);
        if let Some((k1, k2)) = expander {
            events[1] = emit(self.k1.update(k1, now), ButtonEvent::K1Short, ButtonEvent::K1Long);
            events[2] = emit(self.k2.update(k2, now), ButtonEvent::K2Short, ButtonEvent::K2Long);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k0(buttons: &mut Buttons, pressed: bool, now: u32) -> Option<ButtonEvent> {
        buttons.update(pressed, Some((false, false)), now)[0]
    }

    #[test]
    fn a_short_tap_fires_on_release() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        assert_eq!(k0(&mut buttons, true, 100), None);
        assert_eq!(k0(&mut buttons, false, 160), Some(ButtonEvent::K0Short));
    }

    #[test]
    fn bounces_inside_the_window_are_ignored() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        assert_eq!(k0(&mut buttons, true, 100), None);
        assert_eq!(k0(&mut buttons, false, 120), None);
        assert_eq!(k0(&mut buttons, true, 130), None);
        assert_eq!(k0(&mut buttons, false, 200), Some(ButtonEvent::K0Short));
    }

    #[test]
    fn holding_fires_long_once_and_release_is_silent() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        k0(&mut buttons, true, 100);
        assert_eq!(k0(&mut buttons, true, 1099), None);
        assert_eq!(k0(&mut buttons, true, 1100), Some(ButtonEvent::K0Long));
        assert_eq!(k0(&mut buttons, true, 2500), None);
        assert_eq!(k0(&mut buttons, false, 2600), None);
    }

    #[test]
    fn expander_buttons_map_to_k1_and_k2() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        buttons.update(false, Some((true, false)), 100);
        assert_eq!(buttons.update(false, Some((false, false)), 200), [None, Some(ButtonEvent::K1Short), None]);
        buttons.update(false, Some((false, true)), 300);
        assert_eq!(buttons.update(false, Some((false, true)), 1300), [None, None, Some(ButtonEvent::K2Long)]);
    }
}
