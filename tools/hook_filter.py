"""判断助手的最后一段是否在等待用户回答。

两个 Adapter 共用这一套规则，因此它的误判会同时影响两条链路；
背景见 docs/adr/0002-both-adapters-share-the-waiting-heuristic.md。
"""

import re


QUESTION_AT_END = re.compile(r"[?？][\s*_`'\"”’。.!！]*$")
QUESTION_THEN_REPLY = re.compile(
    r"[?？].{0,100}(?:回复|回答|确认|选择|告诉|reply|respond|confirm|choose)",
    re.IGNORECASE | re.DOTALL,
)
DIRECT_REPLY_REQUEST = re.compile(
    r"(?:^|[。.!！]\s*)(?:(?:请(?:你)?(?:直接)?)|直接)"
    r"(?:回复|回答|确认|选择|告诉)|"
    r"(?:^|[.!]\s*)(?:please\s+)?(?:reply|respond|confirm|choose)\b|"
    r"\blet me know\b",
    re.IGNORECASE,
)
CHOICE_LIST = re.compile(r"^\s*(?:[-*]|\d+[.)])\s+", re.MULTILINE)
OPTIONAL_OFFER = re.compile(
    r"(?:^|[。.!！]\s*)(?:如果|若|如需|if\b).{0,120}"
    r"(?:回复|回答|告诉|reply|respond|let me know)",
    re.IGNORECASE | re.DOTALL,
)


def requires_user_input(message: object) -> bool:
    if not isinstance(message, str) or not message.strip():
        return False

    paragraphs = [
        paragraph.strip()
        for paragraph in re.split(r"\n\s*\n", message.strip())
        if paragraph.strip()
    ]
    final = paragraphs[-1]
    if QUESTION_AT_END.search(final) or QUESTION_THEN_REPLY.search(final):
        return True
    if DIRECT_REPLY_REQUEST.search(final):
        return OPTIONAL_OFFER.search(final) is None
    if len(paragraphs) >= 2 and CHOICE_LIST.search(final):
        return QUESTION_AT_END.search(paragraphs[-2]) is not None
    return False
