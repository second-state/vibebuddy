//! Decides whether the assistant's last message is waiting for the user to answer. Both adapters share these rules,
//! so a misjudgment affects both pipelines at once; background in docs/adr/0002. The rules map one-to-one to the old
//! tools/hook_filter.py.

use std::sync::OnceLock;

use regex::Regex;

struct Rules {
    question_at_end: Regex,
    question_then_reply: Regex,
    direct_reply_request: Regex,
    english_ask: Regex,
    english_sign_off: Regex,
    choice_list: Regex,
    optional_offer: Regex,
    paragraph_break: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        // Trailing markdown, quotes, punctuation, an emoji or a "(yes/no)" hint may follow the question mark.
        question_at_end: Regex::new(
            r#"[?？](?:[\s*_`'"”’。.!！\p{So}\p{Sk}\u{FE0F}\u{200D}]|\((?i:y(?:es)?\s*/\s*no?)\))*$"#,
        )
        .expect("regex"),
        question_then_reply: Regex::new(
            r"(?is)[?？].{0,100}(?:回复|回答|确认|选择|告诉|reply|respond|confirm|choose)|[?？]\s*(?:just\s+)?(?:let me know|tell me)[.!]*\s*$",
        )
        .expect("regex"),
        direct_reply_request: Regex::new(
            r"(?i)(?:^|[。.!！]\s*)(?:(?:请(?:你)?(?:直接)?)|直接)(?:回复|回答|确认|选择|告诉)|(?:^|[.!]\s*)(?:please\s+)?(?:reply|respond|confirm|choose)\b",
        )
        .expect("regex"),
        // English "let me know" / "tell me" and the rest of its sentence. It asks for an answer
        // ("let me know the database name") unless the rest is a sign-off, see english_sign_off.
        english_ask: Regex::new(
            r"(?i)(?:\blet me know|(?:^|[.!]\s*)(?:please\s+|just\s+)?tell me)\b([^.!?\n]*)",
        )
        .expect("regex"),
        // What follows "let me know" in an offer: nothing ("just let me know!"), "if…", "when…",
        // "how it goes", "what you think". Chinese offers are covered by optional_offer instead.
        english_sign_off: Regex::new(
            r"(?i)^[\s\p{So}\p{Sk}\u{FE0F}\u{200D})]*$|^\s*(?:if|when|whenever|anytime|any\s+time|how\s+(?:it|that|this|things)\s+go(?:es)?|what\s+you\s+think|your\s+thoughts)\b",
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
    if rules.direct_reply_request.is_match(final_paragraph) || asks_in_english(rules, final_paragraph) {
        return !rules.optional_offer.is_match(final_paragraph);
    }
    if paragraphs.len() >= 2 && rules.choice_list.is_match(final_paragraph) {
        return rules.question_at_end.is_match(paragraphs[paragraphs.len() - 2]);
    }
    false
}

/// A "let me know" / "tell me" that names what it needs, as opposed to a sign-off.
fn asks_in_english(rules: &Rules, paragraph: &str) -> bool {
    rules
        .english_ask
        .captures_iter(paragraph)
        .any(|ask| !rules.english_sign_off.is_match(&ask[1]))
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
    fn english_questions_wait() {
        for message in [
            "Should I go with A or B?",
            "Want me to push and open the PR?",
            "Before I continue, can you confirm the database name is `prod`?",
            "Should I proceed?**",
            "Do you want me to proceed? (yes/no)",
            "Want me to go ahead? 🙂",
            "Should I continue? Let me know.",
            "Let me know which option you'd like.",
            "Let me know the database name so I can continue.",
            "Let me know who should receive the report.",
            "Let me know how to configure the credentials before I continue.",
            "Let me know once you've pasted the key.",
            "Tell me the path to the config file.",
            "Tell me which file to start with.",
            "Please confirm before I delete these files.",
            "I've drafted two options:\n\n- A: faster\n- B: simpler\n\nWhich do you prefer?",
        ] {
            assert!(requires_user_input(Some(message)), "{message:?}");
        }
    }

    #[test]
    fn english_sign_offs_are_not_requests() {
        for message in [
            "Done. All tests pass.",
            "Fixed the bug in `parser.rs`. Let me know if you need anything else.",
            "The PR is open. Let me know if you'd like any changes.",
            "Let me know if you have any questions!",
            "Happy to help further — just let me know!",
            "Let me know how it goes.",
            "Let me know what you think.",
            "Let me know when you're ready.",
            "If you want, I can also add tests. Just let me know.",
            "I'll let you know when it's done.",
            "Why did it fail? The config was missing a key. I fixed it and the build passes now.",
        ] {
            assert!(!requires_user_input(Some(message)), "{message:?}");
        }
    }

    #[test]
    fn a_choice_list_after_a_question_waits() {
        assert!(requires_user_input(Some("下一步选哪个？\n\n1. 先做 A\n2. 先做 B")));
        assert!(!requires_user_input(Some("做了两件事。\n\n- 修了 A\n- 修了 B")));
    }
}
