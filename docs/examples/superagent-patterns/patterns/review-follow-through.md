+++
schema = "cowboy.pattern/v1"
id = "review.follow-through"
revision = 1
role = "supervisor"
summary = "Read completed review results and close valid findings."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Read completed review results and close valid findings.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

已有托管审查进入终态，原 worker 尚未读取完整结果、处理有效发现或继续已授权的后续步骤。

## 最小证据

读取原 call 的完整结果及实际覆盖范围、被审查 revision；完成通知本身不是通过证据。结果分页读取，遇到 running、failed、partial 或快照缺失不能报 clean。

## 监督动作

请原 worker 逐项判断发现，修复有效问题，对不采纳项给出可核查理由，然后跑受影响检查并继续原任务。沿用已有 review；本 pattern 不授权新增昂贵审查。

## 停止条件

审查仍运行时等待现有完成事件，不轮询或催促；缺失源文件、审查失败或外部模型容量问题应明确标为未完成，不能伪造通过。

## 反例

源代码审查没有截图时不能证明渲染 UX 正确；修复后的新 SHA 不自动继承旧 SHA 的完整通过结论。

## 完成证据

有效发现得到处理，验证与当前变更相符，审查限制如实呈现；后续交付仍由项目规则决定。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
