# 领域文档

各工程技能在探索代码库时应如何消费本仓库的领域文档。

## 探索之前先读这些

- 根目录的 **`CONTEXT.md`**；或者
- 根目录的 **`CONTEXT-MAP.md`**（若存在）：它指向每个上下文各自的 `CONTEXT.md`，把与主题相关的都读一遍。
- **`docs/adr/`**：读与你将要动的区域相关的 ADR。多上下文仓库里还要看 `src/<context>/docs/adr/` 下限定在该上下文的决定。

这些文件若不存在，**静默继续**。不要指出它们缺失，也不要一上来就建议创建。`/domain-modeling` 技能（经 `/grill-with-docs` 与 `/improve-codebase-architecture` 进入）会在术语或决定真正定下来时按需创建。

## 文件结构

单一上下文仓库（绝大多数仓库，本仓库也是）：

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-claude-adapter-does-not-parse-transcript.md
│   └── 0002-both-adapters-share-the-waiting-heuristic.md
└── daemon/  firmware/  protocol/
```

多上下文仓库（根目录存在 `CONTEXT-MAP.md`）：

```
/
├── CONTEXT-MAP.md
├── docs/adr/                          ← 全系统的决定
└── src/
    ├── ordering/
    │   ├── CONTEXT.md
    │   └── docs/adr/                  ← 限定在该上下文的决定
    └── billing/
        ├── CONTEXT.md
        └── docs/adr/
```

## 用术语表的词汇

输出里提到领域概念时（issue 标题、重构提案、假设、测试名），用 `CONTEXT.md` 定义的那个词，不要滑向术语表明确回避的同义词。

需要的概念还不在术语表里，这本身是个信号：要么你在发明项目不用的语言（重新考虑），要么确实有空缺（记下来交给 `/domain-modeling`）。

## 标出与 ADR 的冲突

输出与已有 ADR 相悖时，明确指出来，不要悄悄覆盖：

> _与 ADR-0007（事件溯源的订单）相悖，但值得重开，因为……_
