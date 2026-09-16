//! 判断助手的最后一段是否在等待用户回答。两个 Adapter 共用这一套规则，
//! 它的误判会同时影响两条链路；背景见 docs/adr/0002。规则与原来的
//! tools/hook_filter.py 逐条对应。

use std::sync::OnceLock;

use regex::Regex;

struct Rules {
    question_at_end: Regex,
    question_then_reply: Regex,
    direct_reply_request: Regex,
    choice_list: Regex,
    optional_offer: Regex,
    paragraph_break: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        question_at_end: Regex::new(r#"[?？][\s*_`'"”’。.!！]*$"#).expect("regex"),
        question_then_reply: Regex::new(
            r"(?is)[?？].{0,100}(?:回复|回答|确认|选择|告诉|reply|respond|confirm|choose)",
        )
        .expect("regex"),
        direct_reply_request: Regex::new(
            r"(?i)(?:^|[。.!！]\s*)(?:(?:请(?:你)?(?:直接)?)|直接)(?:回复|回答|确认|选择|告诉)|(?:^|[.!]\s*)(?:please\s+)?(?:reply|respond|confirm|choose)\b|\blet me know\b",
        )
        .expect("regex"),
        choice_list: Regex::new(r"(?m)^\s*(?:[-*]|\d+[.)])\s+").expect("regex"),
        optional_offer: Regex::new(
            r"(?is)(?:^|[。.!！]\s*)(?:如果|若|如需|if\b).{0,120}(?:回复|回答|告诉|reply|respond|let me know)",
        )
        .expect("regex"),
        paragraph_break: Regex::new(r"\n\s*\n").expect("regex"),
    })
}

pub fn requires_user_input(message: Option<&str>) -> bool {
    let Some(message) = message else {
        return false;
    };
    let message = message.trim();
    if message.is_empty() {
        return false;
    }
    let rules = rules();
    let paragraphs: Vec<&str> = rules
        .paragraph_break
        .split(message)
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .collect();
    let Some(final_paragraph) = paragraphs.last() else {
        return false;
    };
    if rules.question_at_end.is_match(final_paragraph)
        || rules.question_then_reply.is_match(final_paragraph)
    {
        return true;
    }
    if rules.direct_reply_request.is_match(final_paragraph) {
        return !rules.optional_offer.is_match(final_paragraph);
    }
    if paragraphs.len() >= 2 && rules.choice_list.is_match(final_paragraph) {
        return rules.question_at_end.is_match(paragraphs[paragraphs.len() - 2]);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_question_waits_for_the_user() {
        assert!(requires_user_input(Some("我该用方案 A 还是方案 B？")));
        assert!(requires_user_input(Some("是否采用 GitHub Issues？直接回复“是”即可。")));
        assert!(requires_user_input(Some("Should I proceed? Please confirm.")));
    }

    #[test]
    fn a_finished_report_does_not() {
        assert!(!requires_user_input(Some("修复已完成，全量测试通过。")));
        assert!(!requires_user_input(None));
        assert!(!requires_user_input(Some("   ")));
    }

    #[test]
    fn an_optional_offer_is_not_a_request() {
        assert!(!requires_user_input(Some("如果你还需要调整配色，可以告诉我。")));
        assert!(!requires_user_input(Some("修复完成。If you need anything else, let me know.")));
    }

    #[test]
    fn a_choice_list_after_a_question_waits() {
        assert!(requires_user_input(Some("下一步选哪个？\n\n1. 先做 A\n2. 先做 B")));
        assert!(!requires_user_input(Some("做了两件事。\n\n- 修了 A\n- 修了 B")));
    }
}
