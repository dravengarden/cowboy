+++
schema = "cowboy.pattern/v1"
id = "review.ready-evidence"
revision = 1
role = "supervisor"
summary = "Prepare a reviewable change and make its validation coverage explicit."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Prepare a reviewable change and make its validation coverage explicit.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

任务要求交给人 review，代码已形成可检查成果，但缺少清楚的 diff 范围、验证说明、限制或必要附件。

## 最小证据

读取当前需求、真实 diff、对应测试及必要 UX 证据和已有审查结论；区分本次改动与预先存在的脏工作区。

## 监督动作

请原 worker 补成可审阅成果：问题和行为变化、精确版本、测试范围及结果、未覆盖项、必要截图。对于 SQL 或其他生产变更，准备前置条件和恢复方案，提交给既定审批方。

## 停止条件

PR 创建、修改、评论、push 或生产执行各自遵守当前项目规则及授权；若缺授权，准备好具体成果后等待真正的决策，不代替审批。

## 反例

Suger 的团队审批不能继承 Cowboy 的交付偏好；测试通过也不能替人批准生产数据修改。

## 完成证据

成果可供审阅、证据可定位、遗漏与风险明确。已授权发布则按项目流程完成；否则明确停在待审批状态。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
