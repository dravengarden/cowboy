# 2026-11-06 使用特征与 Super-agent 候选模式

本轮最明显的机会是把已接受任务做完并展示可靠证据：
完成一个批次后继续剩余范围、失败后修复、审查后处理发现、
核实已经授权的交付。跨任务协调有价值，但必须存在明确依赖和可核查产物，
不能把所有 session 交给一个无限自主的总调度器。

本轮只增加 Markdown 候选库，没有启用任何项目或 session，
没有修改线上配置，也没有证明自动监督效果。

## 样本与方法

有目的地抽取 12 个普通 session，覆盖 Cowboy、Columbus 生态项目和
Suger 工作场景；包含本功能的来源 session，其窗口截止本轮提取请求之前。
通过 Cowboy history 查询 109 页，取得 125 条 user-role 记录；
剔除 10 条明显的托管完成通知或系统提醒后，保留 115 条人类消息候选。
其中 3 条文本截断，只有 3 个 session 遍历到历史开头，其余是有界近期窗口。
每个 session 最多读取 12 页，达到至少 16 条 user-role 记录也会停止；
分页上限与消息截断会影响覆盖。

人类消息采用文本排除法，并未逐条核对原始消息 origin。
这不是随机抽样、全账号普查、长期故障率，也不是 115 个独立任务。
另参考带用户原话的历史记忆支持跨时段重复性；历史事实和授权仍须重新核实。
对关键提醒补读前后助手消息，避免仅按“继续”等词做结论。
为保护业务上下文，共享文档不保留客户、SQL、PR 标识或原始 transcript。

| 字面线索 | 条数 / 涉及 session | 解读限制 |
| --- | --- | --- |
| 简单继续提示 | 12 / 5 | 可能是正常推进，不能全部算遗漏 |
| 完成确认或要求直到做完 | 7 / 4 | 要对照最初范围判断是否过早结束 |
| push / deploy 提醒 | 2 / 2 | 需要核实当前项目的授权条件 |
| 测试或可审阅性确认 | 4 / 2 | 可能已有证据，只是呈现不清楚 |
| 新决策、批准或输入 | 4 / 2 | 不能作为自动绕过审批的依据 |

这些是人工归类的线索计数，可能有语义重叠，不应相加计算可自动化比例。

## 主要发现

### 1. 经常需要把“本批完成”纠正为“整个任务尚未完成”

两个项目的上下文出现：助手报告一批改动与检查完成，
用户追问后，助手才明确仍有较多原范围工作，随后用户再要求继续。
本功能来源 session 也出现审查发现后修复尚未完成、用户再次提醒的情形。
优先模式是范围续接和审查收尾，不是自动增添新功能。

### 2. 发布提醒反复出现，但项目授权不能混用

本样本有 push/deploy 提醒，历史来源也多次出现 Cowboy 修完就发布的明确偏好。
监督者应检查当前任务和项目规则，完成已经授权的收尾。
Suger 的一个上下文中，助手先准备成果，用户随后才批准 push 和更新 review；
不能仅因出现这句话就认定前一次等待错误。

### 3. “测试了吗”部分是交付证据呈现问题

一个 Suger 上下文已报告真实 E2E 场景和截图，用户随后再次询问测试是否完成。
适合做一次证据整理，明确版本、真实覆盖和未覆盖项；不能把这种询问
一律转成重跑整套测试。发布受理不等于成功、旧日志不等于新检查，
则需要补查权威证据。

### 4. 失败修复和跨 task 续接适合有边界的监督

历史用户明确要求失败后继续修复验证，并要求从指定来源 session 恢复剩余工作。
跨 task 需要的是验证生产者产物后提醒原消费者继续，不能只读本 worktree，
也不能根据另一 agent 的口头完成结论直接合并或部署。
本样本不足以估算跨 task 自动化成功率。

### 5. 审批与资源归属是明确的停止边界

Suger 有生产数据修改需交团队审批的上下文，应自动准备可审阅方案，
不能自动批准或执行。历史还明确希望用完的临时 Cowboy session 关闭；
这适合任务资源收尾，但不能推导出删除历史、关闭其他人的 session
或清理所有闲置 worker 的权限。

## 已提炼的 Markdown

入口是 [observed-global.md](examples/superagent-patterns/observed-global.md)，
配合 [共同约束](examples/superagent-patterns/observed-routing.md)。
全部是 revision 1 候选；原有 global/workspace/project 示例保持不变。

| Pattern | 自动监督的具体事情 |
| --- | --- |
| [delivery.follow-through](examples/superagent-patterns/patterns/follow-through.md) | 沿用已有规则，补完当前已授权的交付步骤 |
| [work.scope-continuation](examples/superagent-patterns/patterns/scope-continuation.md) | 完成一批后继续最初接受范围内的剩余工作 |
| [work.failure-recovery](examples/superagent-patterns/patterns/failure-recovery.md) | 明确失败后找根因、修复、验证；未知效果先查询 |
| [review.follow-through](examples/superagent-patterns/patterns/review-follow-through.md) | 读取审查真实结果、处理有效发现、继续原任务 |
| [delivery.evidence](examples/superagent-patterns/patterns/delivery-evidence.md) | 对齐代码、检查、远端与部署证据，改进完成说明 |
| [coordination.handoff](examples/superagent-patterns/patterns/dependency-handoff.md) | 显式依赖产物就绪后，让原消费者继续 |
| [review.ready-evidence](examples/superagent-patterns/patterns/review-ready.md) | 准备清楚可审阅的改动、测试与限制说明 |
| [work.resource-closeout](examples/superagent-patterns/patterns/resource-closeout.md) | 按所有权与当前政策关闭已完成的临时执行资源 |

每个新增规则包含匹配条件、最小证据、监督动作、停止条件、反例和完成证据。
连续提炼的过程放在共同 guidance，避免再增加一个每次催促都触发的 supervisor。

## 分层与采用方式

Global 放通用识别和停止条件。Columbus workspace 负责项目归属、
机器维护及跨项目边界；Suger workspace 负责公司交付和团队审批流程；
Cowboy project 负责组件发布证据。具体发布命令和权力来源仍取自
当前项目 guidance，不复制到各个 pattern，避免规则过时或互相矛盾。

采用候选根 profile 时，将现有 workspace 示例的 extends 改为
global.observed，并从实际项目 source root 提供自己的 guidance。
Cowboy 项目仍可显式替换 delivery.procedure。不要把 Suger profile
放在 Cowboy root 下打包，再误认为读到了 Suger 的真实规则。
按 [配置契约](superagent-patterns.md) 导入完整来源闭包后，才配置项目、
模型与 session 参与方式；本次没有执行这些操作。

首轮优先观察范围续接、失败恢复、审查收尾、交付证据。
确认误触发边界后，再按 session 选择 coordinate。
同一次原因共享一次跟进和原有上限，不为每个 pattern 单独发消息。
跨 Machine 监督、自动唤醒、定时扫描和直接控制其他 session
不属于这些 Markdown 所赋予的能力。

## 下一轮应验证的效果

用现有受控原生记录统计正确干预、误触发、重复催促、真实阻塞、
人类提醒是否减少，以及每次有效干预的 input/cache token 成本。
稳定 guidance 前缀和按需证据可以减少重复上下文，但本轮没有运行
缓存实验，也不能保证压缩后成本不增加。新案例先提案，
带反例和证据引用，经源文件审核与重新导入后才成为生效规则。
