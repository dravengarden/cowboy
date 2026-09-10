# Cowboy：跨 Service / Machine 的时空可组合架构

状态：整体重设计，2026-09-09；替代本文先前逐项补充的草案。接口、类型和目录拆分均为目标，不是已发布
SDK。设计审计基线为 `05756e67bdf6d80e8a790a160954158e1f9f109c`；第一批
[只读组合检查器](plugin-composition-checker.md)
已实现类型生成、边界解码、scope/placement
与图结构校验。第二批 [运行时绑定](plugin-runtime-bindings.md) 已将 Core RPC
绑定到认证连接代次，并让现有遥测调用核对签名 Catalog 与当前 Machine 库存。
第三批 [组件契约](plugin-components.md) 已实现显式 codec、state-store 实例/订阅资源
释放，以及保留历史的依赖闭包发布检查；不涉及后台任务取消或 durable effect revert。
第四批 [卸载操作账本](plugin-operation-journal.md) 为现有卸载流程加入 Service 耐久 intent、
事务内完成记录、重启隔离与明确的不确定结果。第五批
[Machine 回执](machine-plugin-operation-receipts.md) 加入协议 10 的耐久卸载步骤、去重与只读查询；
Machine 默认 reader-only，实际启用需要独立维护切换。第六批
[安装代次与 CAS](plugin-installation-incarnations.md) 加入协议 11 的独立 incarnation、卸载
前置条件和耐久 tombstone；兼容 reader 与 Hawk 冷启动恢复版本均已验收，写入仅在明确准入的
Machine 启用。耐久自动补偿仍未实现。
第七批 [恢复检查](plugin-recovery-assessment.md) 在同一 Machine 生命周期锁下核对历史回执与当前
tombstone，Service 再检查自身记录是否变化；只读结果不授予恢复执行权，也不清除 fence。
第八批 [执行租约](plugin-execution-leases.md) 将现有耐久卸载绑定到原认证连接和接收时开始的
单调时间预算；排队、核验或写入 intent 后失去执行资格，不再继续开始 Plugin 效果。它不授予恢复权。
第九批 [Service 持续授权](plugin-service-authorization.md) 绑定确认所用的实际凭据，并在各效果边界
重新核对当前 Operator、登录时效、精确信任与连接；观察到失效后不再接纳下一效果，仍不开放耐久补偿。
第十批 [无效果中断处置](plugin-no-effect-resolution.md) 加入独立预览/确认与本地原子审计；
仅能终止可证明尚未开始效果的 `Prepared` 中断并解除对应 Service fence，不复活原授权、安装或会话。
第十一批 [核心安全客户端](core-security-client-boundary.md) 将本地认证 UI 从 Plugin slot 分离，
并为现有 native Passkey ABI 加入闭集类型、结果校验与不确定效果禁止自动重试；不迁移存储或退役旧 SDK/原生 ABI。
通用图授权、跨端激活和耐久恢复仍是后续目标，不能把结构检查通过当成可执行计划。

本文统一定义核心、组件库、Plugin、跨端组合、状态与 effect。现行
[requirements](requirements.md)、[package contract](plugin-packages.md) 和
[components contract](plugin-components.md)
仍约束已部署系统；目标与现状冲突必须经过明确迁移，不能通过删包、删 authority
marker、改历史迁移或重启会话强行切换。首批检查器不切换现有运行路径、Plugin
版本、Catalog 或生产配置。

## 1. 总体决策：固定平台，用同一组合模型描述扩展

**Core 负责机制和最终裁决；Component 是构造单元；Plugin
是不可变交付单元；Composition 是实例的连接计划；Operation
才是一次实际效果的执行与恢复。**

Service side 与 Machine side 不是两套插件系统：一个逻辑 Composition
可以跨两端，每端拥有实际执行的局部图。统一契约、身份和恢复语义，不统一成一个进程、一个权限池或一个全局事务。

这次对前稿的关键收敛：

- 组件实例可以进入内部依赖图，但不再增加独立安装/授权身份；facet 只是 Plugin
  的类型化对外贡献。
- 逻辑 scope、进程生命周期和网络连接生命周期分开；Controller 重启不是销毁
  Machine Session。
- 能做什么、效果如何恢复分开；不能用一个 `reversible`
  标签代替资源授权与恢复前提。
- 运行实例 generation、持久数据 schema、权限 epoch
  分开；退回制品不等于退回数据库或安全状态。
- 先完成有限操作的跨端闭环，再扩大接口；不先建设通用工作流引擎或自定义语言平台。

### 1.1 固定核心，但不做一个巨大核心包

| 核心模块          | 不可交给 Plugin 的职责                                            | 扩展边界                                                  |
| ----------------- | ----------------------------------------------------------------- | --------------------------------------------------------- |
| 通信              | 认证连接、协议协商、RPC/stream framing、路由、背压、取消、去重    | Plugin 暴露业务端口；Provider 私有 ACP/网关仍在其制品内部 |
| 安装与运行管理    | Catalog、签名/摘要、staging、绑定提交、generation、租约、卸载、GC | Plugin 声明精确内容与需求，没有安装脚本或自更新器         |
| 安全              | 产品身份、授权、信任根、隔离、凭据控制、撤销、审计                | 可配置外部身份源或附加扫描器；不能替换最终授权器          |
| 状态与恢复        | 核心数据库、命名空间隔离、迁移执行、操作账本、fence               | Plugin 使用受限业务数据接口和受控迁移                     |
| Web / native 基座 | 应用壳、renderer、确认、交互、OS bridge、原生发布                 | Plugin 提供有界业务 UI 数据，不注入宿主代码               |
| 基础遥测          | OTel 接入、校验/隐私、限流、本地轮转、故障诊断                    | Victoria 等可安装 sink；不能持有恢复权威                  |

核心内部可以复用组件库、以 crate/package 模块化，按
Web、Controller、Machine、native
分别发布。组合内核只处理类型、绑定、scope、授权、操作与资源原语，不依赖
React、Provider ID、Victoria endpoint 或 Zed 实现。

安全必须留在核心：被约束者不能卸载约束机制，安装器不能靠候选包验证自己，恢复不能依赖失败插件才能开始。额外扫描器只提供证据；必需检查缺失时拒绝操作。

Native 指 Cowboy 原生客户端及 OS bridge。Machine 上 Agent/Zed
的本地可执行文件仍可随 Plugin 私有制品交付，两者不能混淆。Tauri 中名字叫 plugin
的固定编译依赖，也不因此成为 Cowboy 可安装 Plugin。

### 1.2 实际部署拓扑

```text
Web / Native Core
  类型化视图与用户 intent；没有 Plugin JS/native loader
             │ 核心客户端协议
Service Core
  身份/策略/期望组合/协调账本/产品持久状态
             │ 核心 Machine 协议：Plan、Receipt、Lease、Replay
     ┌───────┴────────┐
Machine A Core    Machine B Core
  本地执行图        本地执行图
  制品/资源/账本    制品/资源/账本
     │                 │
隔离 Plugin runtime  隔离 Plugin runtime
```

一个逻辑 Cowboy Service 可由进程重启后的 Controller 继续承载；同机部署 Service
和 Machine 也不消除这个边界。Machine 继续拥有 detached worker，复用现有
enrollment、fencing、dedup、adopt 与 replay，不另造一套远程 agent
supervisor。[现有拓扑](architecture/00-overview.md)

## 2. 最小对象模型与组件库

### 2.1 五个对象，不用一个“插件”代指所有事情

| 对象          | 含义                                              | 不能承担的职责                   |
| ------------- | ------------------------------------------------- | -------------------------------- |
| Contract      | 版本化的数据、端口、消息与行为约束                | 本身不提供权限或运行实例         |
| Component     | 可复用定义/实现；有状态时创建 owned 实例          | 没有独立 Cowboy 安装槽或凭据权威 |
| PluginRelease | 一个 ID/version/digest/publisher 下的完整制品     | 发布不代表安装、启用或授权       |
| Composition   | 精确实例、端口、scope、placement 的不可变绑定快照 | 不是所有 Agent 行为的静态工作流  |
| Operation     | 一次有授权、预算、执行回执与恢复状态的动作        | 不用遥测日志冒充耐久账本         |

```text
ReleaseRef       = PluginId + Version + CompositeDigest + Publisher
Site             = Service(ServiceId) | Machine(MachineId)
InstallationSlot = Site + PluginId
InstallationRef  = Site + exact ReleaseRef + installation generation
InstanceRef      = InstallationRef + composition/node identity + incarnation
ComponentRef     = package + exact version/digest + export/contract identity
OwnedUnitRef     = exact owner + local key + incarnation
BindingRef       = exact endpoint + ContractRef + ScopeId + binding revision
```

这些字段是语义定义，不是现在可以提交的 JSON。CoreAnchor 与 PluginInstance
是不同身份类型；安装/卸载 API 不接受 CoreAnchor。Component 的源码 export
不是运行时端口注册，只有核心可以把合规贡献链接为 binding。

### 2.2 一个 Plugin 可以跨端贡献，仍只有一个 release

同一 PluginRelease 可声明 Service 数据 facet、Machine runtime facet 和 UI
facet。Facet 没有第二份签名、版本或安装器；完整依赖与平台矩阵绑定进同一个
composite digest。

部署意图指定哪些 facet 在哪些 Site
被消费，由同一安装/激活流程生成必要的目标步骤。Service
目标首版只解释经过验证的数据：身份源、业务配置、组合与 UI
声明；第三方代码仍不进入 Controller 地址空间。可执行实现放在 Machine 的精确隔离
runtime。

Service 所需的包选择或存储激活不能因“安装到
Machine”而隐式获得授权。若一个操作需要同时改变两端，预览和确认必须显示两端影响；无需让用户安装
adapter、网关或单独的 facet 产品。

各 Machine 可以保留不同 Plugin 版本；跨端端口按精确契约匹配，不要求整个 Service
同一 Plugin ID 永远只有一个版本。Service 数据 generation 也可因在途流程或
Machine 绑定被保留。共享业务状态是否允许这些 reader/writer
共存，另由状态契约判定。

当前 Machine-only 安装协议不会自动获得 `Site::Service` 支持：必须版本化迁移
Controller host activation 和读者库存，不能建立第二个 Catalog 或直接往旧 reader
发送新联合类型。

### 2.3 组件库是整个体系的构造层

| 库层                     | 当前落点与目标                                            | 谁运行、谁发布                                          |
| ------------------------ | --------------------------------------------------------- | ------------------------------------------------------- |
| 通用基础组件             | `state-store`、纯转换、可复用算法                         | 可复用代码不携带宿主身份；消费方按自己的权限运行        |
| 核心实现组件             | `app-shell`、宿主 state-sync/IDB、renderer、native bridge | 编译进相应核心制品，不作为 Plugin 安装                  |
| Contract / authoring SDK | plugin/provider SDK、UI IR、Code/Telemetry 契约           | 生成类型、codec、builder 与测试向量；不导出核心权限实现 |
| Plugin 私有实现库        | runtime 工具、adapter、collector、codec                   | 精确锁定；执行字节绑定到消费它的 Plugin 制品            |
| 组合模板库               | UI fragment、纯 reducer、业务子图模板                     | 构建为有界 IR 或 Plugin 内部实现，没有新运行权限        |

`plugin-api` 要拆分契约/authoring、可信 host registry、native core
bridge。拆的是依赖与权威，不只是包名。核心模块依赖 Contract，Plugin SDK 也依赖
Contract；Plugin 不反向 import 核心实现或其他 Plugin 的源码。

无状态纯函数不需要调度节点。有资源/生命周期的组件由核心或精确 Plugin 实例持有子
scope；相同组件可多次实例化，不能共享隐式可变单例。内部节点可在诊断图折叠显示，但它们的依赖、权限与
effect 仍参与校验。

跨 Plugin 共享业务服务需要明确 owner 和租约；如果还需要独立安装/升级，就成为独立
Plugin。核心服务即使内部组件化，也不变成
Plugin。包内组件不能独立替换运行字节，必须随所属制品换代。

## 3. 类型系统：一个可验证的契约源

### 3.1 Contract Bundle，而不是新造一门通用语言

首版沿用仓库已有 JSON Schema 2020-12 体系，限定一个可生成 Rust/TS/native codec
的闭集 profile；配套描述端口、消息、effect 和状态规则的版本化
descriptor。Descriptor 引用类型定义，不再复制字段。已有 JSON Schema
不能原样宣称覆盖了所有语义验证。

允许有界 scalar/refinement、record、tagged union、显式 optional/Result、bounded
collection 和 opaque
reference。拒绝未知关键字、无界递归/字典、任意表达式和运行时远端 schema
加载；引用必须解析到 bundle 内或精确锁定的契约依赖。

必须明确整数表示、字符串/字节上限、非法 Unicode、重复 JSON key、null/缺省和
union 分支。超出 JS 安全整数范围的
generation/序号使用经过范围验证的规范十进制字符串，不做静默 number 转换。身份
URL/路径等语义由核心明确校验，不能只依赖 `format`
注解。[JSON Schema 的验证与 format 边界](https://json-schema.org/draft/2020-12/json-schema-validation#section-7.2)

生成物包括 Rust newtype/enum、TS branded/discriminated types、codec、typed
client、authoring builder
与正反测试向量。契约指纹使用版本化的确定性规范化规则，覆盖类型/refinement、端口、effect
和状态兼容要求；实际源码/制品另有字节摘要。不能因指纹相同就免除行为验收。现有手写验证器按领域差分迁移；最终每个类型只有一个可编辑事实源。调用图、权限、SQL隔离、资产安全、平台可用性等由核心语义检查完成，schema
无法证明它们。

Native 使用独立 CoreBridge 契约命名空间和产物，可复用生成器，但不依赖 Plugin
SDK/Catalog。OTLP 保留标准 protobuf，不为图统一改成自创遥测格式；契约明确引用其
wire adapter 和有界验证规则。

### 3.2 数据类型、身份类型、授权状态都区分

```text
DownloadedBytes -> VerifiedRelease -> StagedGeneration
UntrustedIntent -> ResolvedPlan -> AuthorizedPlan -> PreparedPlan

LocalRef<T, Scope>        不能序列化为远程对象
RemotePort<Contract>      异步调用，显式超时/撤销/断线错误
SecretRef<Owner, Purpose> 不等于可读取的明文
MachinePath<MachineId>   不等于 Service 上的文件路径
```

只有核心私有构造器可生成 Verified/Authorized
类型；重启时重新验证当前证据，不能通过反序列化复活永久授权。PluginId、MachineId、SessionId、ArtifactDigest、ContractFingerprint、OperationId、AuthGeneration、PolicyEpoch
不能混用字符串。

下例只示意生成 SDK 的关联关系，不是已实现 API：

```typescript
interface Contracts {
  "cowboy.code.query.v1": {
    request: CodeQuery;
    response: CodeReply;
    error: CodeFailure;
  };
  "cowboy.telemetry.export.v1": {
    request: OtlpBatch;
    response: ExportReceipt;
    error: ExportFailure;
  };
}
interface Port<
  C extends keyof Contracts,
  S extends ScopeKind,
  P extends SiteKind,
> {
  readonly binding: OpaqueBinding<C, S, P>;
  call(
    request: Contracts[C]["request"],
  ): Promise<
    Result<Contracts[C]["response"], CallFailure<Contracts[C]["error"]>>
  >;
}
```

Rust 使用关联类型与受限构造器，不跨进程传递 trait-object ABI；跨端都用明确 wire
codec。TS brand 和 Rust ScopeKind 都不能区分同类型的两个具体
Session：调用时仍需检查真实实例、Site、scope、grant、generation、deadline
与撤销状态。一个 executor 自报 success，不等于核心验证了业务效果。

### 3.3 端口元数据与六道检查

每个端口/方法声明
request/response/domain-error、单个/多提供者、placement、可见性、权限需求、资源预算、超时、重试/幂等和
effect 上限。事件/stream
额外规定顺序、背压、丢失与消费者集合；不能把同名事件同时解释为广播和授权决策。

作者编译 → 打包链接 → Catalog 信任 → 目标安装校验 → 组合解析/授权 →
每次调用解码/撤销检查，六道检查都保留。SDK 不公开任意
`invoke(string, JSON)`、`getService(string)` 或宽泛 unknown context。

接口按 namespace、version、schema fingerprint、行为契约匹配；SemVer
只是预筛选。首版精确接口匹配，兼容转换必须显式且受测。一个新 Provider
实现已知接口不改核心 Provider-ID 分支；新权限原语、native
方法或新领域接口需升级相应核心契约。自定义业务接口、Wasm
执行器均不作为首版前置条件。

## 4. 空间结构：统一寻址，分离四种关系

| 关系       | 问题             | 规则                                                  |
| ---------- | ---------------- | ----------------------------------------------------- |
| Placement  | 在哪一端执行     | Service 或指定 Machine；Web/native 只是可信核心客户端 |
| Visibility | 谁能解析这个端口 | 精确 Service/Machine/Workspace/Session/Request 视图   |
| Authority  | 谁准许哪些操作   | 按真实主体、用途、资源、Site、期限签发的 grant        |
| Ownership  | 谁结束时释放资源 | 唯一 owner；其他消费者只借用 lease                    |

Service/Machine/Workspace/Session
的导航层次用于组织与寻址，不代表默认继承父权限，也不代表父进程退出就级联销毁子资源。Workspace
identity 包含 Machine
与规范化工作区身份；同名路径不等于同一资源。逻辑用户/安全域也必须匹配，不能仅用
scope 层级替代账号隔离。

节点/边数量、owner 深度、每端总队列、并发、重试与恢复记录保留都有核心上限；一个
Plugin 拆成多个组件实例不能重复领取全局预算。

只维护三种职责明确的图投影：制品依赖图、运行时能力 DAG、实际 Operation/effect
因果图。所有权树是单独关系，授权不是依赖边。解析时校验 owner
与硬依赖形成的联合等待图，不能只检查能力图无环。

同一视图的 One<C> 不允许“最后注册者获胜”；多个提供者须显式
Many<C>，规定顺序和失败策略。可选依赖使用独立 binding scope 或 Option
快照，临时离线显示 Waiting，歧义/错版本/错误权限立即失败，不以永远 Pending 伪装
ready。

组合成立的条件是显式依赖、独立授权、唯一 owner
和受控资源冲突。只有相同前提、无共享冲突且无顺序敏感外发的独立操作，才要求交错执行结果等价。共享状态必须排序、单写者或
CAS；卸载 A 不覆盖 B 或用户之后的合法修改。布局树、双向业务调用、Agent
循环不强行变成激活 DAG。

## 5. Service × Machine：同一个图，不是同一个故障域

### 5.1 权威归属

| 状态/决定                    | Service side                                   | Machine side                                             |
| ---------------------------- | ---------------------------------------------- | -------------------------------------------------------- |
| 产品身份与 Provider 账户     | 唯一登录、会话与 Provider 凭据权威             | 仅使用精确授权投影；不能产生独立 Provider 账户           |
| 期望 Composition             | 持久化部署意图、逻辑 revision、批准的目标/影响 | 验证和应用分配到本机的局部计划                           |
| 本机安装/运行事实            | 持有经过认证的观察与回执，不猜测已完成         | 安装槽、实际 generation、进程/资源、租约和本地提交的权威 |
| 策略                         | 主体/用途/资源/目标授权                        | 本地安全策略可收窄，不可扩大 Service 委托                |
| 操作恢复                     | 协调账本、跨端前提、目标状态汇总               | 执行前 intent、去重、效果核验、本地恢复回执              |
| Machine 私有 endpoint/secret | 不因协调权获得读取权限                         | 本地连接器策略持有；不是 Provider 账户凭据副本           |
| UI 状态                      | 产品事实与允许动作投影                         | 提供本机诊断事实；不能向 UI 声称已完成 Service 登录      |

Service 是 Provider 账户凭据的唯一权威，不是所有机器秘密的所有者。例如 Victoria
endpoint/token 继续由 Machine 私有策略拥有；Service 只能选择精确导出
binding。跨端凭据 projection 按目标加密、按用途与 generation 限定，不能通过
Context 继承整个 vault。

Service→Machine 是现有核心认证协议，不是通信 Plugin。跨 Machine
业务调用首版通过核心路由协调，不建立插件自己的发现/mesh/公开监听器。路径、裸指针、进程对象、数据库连接不跨端传递；使用目标可验证的
opaque reference 和 typed RPC。

Service-scoped 业务组件也可以显式放在指定 Machine 执行，包括与 Controller
同机的已登记 Machine：逻辑 scope 不决定进程位置。其计算通过 RemotePort
调用，资源仍由执行端管理，秘密与权限只按用途投影。首版不另建 Service
可执行插件宿主；若未来确有需要，新增经过验证的核心 executor backend，而不是开放
Controller 内插件代码或第二套通信系统。

### 5.2 逻辑 scope 与连接 scope 必须分开

```text
逻辑 Composition（耐久意图，revision R）
├── Service 上的 exact bindings
├── Machine A 上的 Workspace / Session bindings
└── Machine B 上的 Workspace / Session bindings

Controller process / WebSocket connection
└── 只拥有路由、在途 request、观察订阅等短期资源
```

这不是跨机器的递归 destructor。持久 Session 的执行归 Machine-owned session
scope；Controller process scope 与 connection scope
只借用它。客户端关闭、Service 重启或 WS 断开，不自动安装/卸载
Plugin、释放会话工作区或终止 Agent。

每次重连重新验证 Machine 身份、协议库存、进程/boot incarnation、已应用 revision
和未完成 operation。Site/逻辑实例身份不因换一条连接而改变；旧连接、旧 epoch
的晚到命令或回执不能绑定新实例。

### 5.3 计划、局部提交与协调回执

1. Service 解析请求为不可变 Plan：精确制品/端口、各端预期
   revision、影响集合、状态前提、权限、预算与失败策略。逻辑 revision 属于该
   Composition，不建立全系统每次调用共用的一把 revision 锁。
2. 安全模块批准这个 Plan；每个 Site
   重新验证实际制品、本地策略、资源冲突、契约和前态。准备不是获得无限网络/文件权限。
3. 凡涉及 durable mutation，协调端和执行端先记录自己的 intent。需要多个 Site
   准备时，准备租约有期限，不能无限持锁等待失联端。
4. 每个 Site 只对自身拥有的状态做
   CAS/本地事务提交，持久化结果后回传带身份和版本的 Receipt。
5. Service 核对各端回执与现实状态，记录 Completed、Partial、Reconciling 或
   NeedsAttention。失败后按预先批准的有界策略暂停或补偿已完成步骤，而不是私自重新选择方案。
6. 回执丢失先查询同一 Operation/StepId；仍无法判定外部结果时记录 Unknown，不用新
   ID 重试来掩盖歧义。

OperationId/StepId 的去重记录绑定批准的 Plan digest、真实主体、目标 Site
与请求内容摘要；相同 ID
配不同参数必须拒绝，不能复用旧成功回执。每个提交也重新核对期限与当前 policy/auth
epoch，旧确认不能覆盖后来扩大的影响集合。

一个跨端计划可以有统一确认与追踪，但没有全局瞬时原子切换保证。只有位于同一实际事务域内的状态更新才承诺原子；保留跨端部分结果供恢复。核心需同时检查混合
scope 等待环和运行调用超时，激活 DAG 不能证明任意异步程序不会死锁。

### 5.4 离线、租约与故障语义

| 情况                        | 必须发生                                                        | 不得发生                                     |
| --------------------------- | --------------------------------------------------------------- | -------------------------------------------- |
| Controller 重启             | Machine 继续已有执行；重连恢复观察和待处理协调步骤              | 以 root scope dispose 杀掉全部 Session       |
| Machine 断线                | 按已授予的本地执行/离线期限处理现有任务；跨端新调用明确不可用   | 自动找另一 Machine、另一 Plugin 或新账号接管 |
| Machine host 重启           | 优先按已验证身份 adopt 存活 worker；重建本地路由和恢复状态      | 重复启动 worker，或把恢复失败变成空白新会话  |
| Machine 不支持所需隔离/协议 | 计划在执行前判不兼容                                            | 用 full access 执行却保留原保证标签          |
| 撤销/unenroll               | 拒绝新授权；在线端执行撤销，离线端报告 pending 并受原有效期限制 | 在未收到回执时宣布全局即时撤销               |
| 部分 Site 已提交            | 保留真实进度、前态/制品引用，按批准策略 reconcile               | 把一个 Site 的成功当整个计划成功             |

共享资源由提供端持有 owner；跨端消费者只持有被提供端确认的有界
lease。断线不等于主动释放，租约过期也不直接证明业务效果已撤销。长任务的执行租约与短
RPC 超时分开，保留已有明确的离线运行政策；不通过缩短连接超时变相终止 Agent。

授权有效期与本地超时使用可验证时间规则；进程内预算用单调时钟，不能因宿主时钟回拨无限延长离线授权。恢复状态无法证明原授权仍有效时拒绝新敏感操作；核心仍可回收它自己拥有的临时资源。

## 6. 时间模型：实例换代、数据兼容、权限撤销各司其职

### 6.1 三种版本不互相回滚

| 版本轴                                | 控制什么                       | 退回旧 Plugin 时                       |
| ------------------------------------- | ------------------------------ | -------------------------------------- |
| Artifact / instance generation        | 哪些字节、端口和实例被使用     | 只能选择仍被信任、保留、受测的精确代次 |
| State schema / reader-writer contract | 数据形状、写入语义、可用读者   | 必须单独证明兼容或执行明确数据恢复     |
| Policy / auth epoch                   | 当前允许什么、哪个凭据权威有效 | 单调不降级，不复活已撤销授权           |

一个实例有明确状态：

```text
Declared -> WaitingDependencies -> Preparing -> Ready -> Active
                                    |                    |
                                  Failed              Draining
                                                         |
                                           Disposed / NeedsReconcile
```

安装状态与实例状态分开。失败重新激活创建新 incarnation；disposed handle
永远失效。健康/连接状态是独立观察，不能用一次心跳让旧实例身份复活。

### 6.2 激活和热替换只改受影响闭包

候选在私有 staging 验证精确字节、平台、受限 readiness
与状态兼容性。不能抢占共享端口、覆盖 active
数据，也不能把真实模型消费或数据库迁移隐藏进 probe。

局部绑定以预期 revision 的 CAS 切换；新消费者拿新快照，已有 Session/Workspace
保留其精确租约。安全撤销优先于 generation pin。可选服务变化使用独立子
binding，不能把 telemetry 升级变成全部 Agent 重启。

硬依赖失效时拒绝新的依赖调用；消费者按契约隔离/排空，并在条件恢复后重新解析，不能自动换到“任意可用”提供者。需要同一时刻看到整组新接口的消费者按共同
activation group 切换；跨端 group 仍只提供协调状态，不伪装分布式原子事务。

无法支持新旧共存、状态读写不兼容或占用独占资源时，使用明确的维护计划与影响确认。不要为“热更新”允许两个不兼容
writer 同时写同一 namespace。

### 6.3 关闭、卸载与 GC

先撤新入口，再排空已接纳调用；消费者先于提供者释放，只有无顺序依赖的清理组可以并行。listener、stream、callback
的在途任务同样持有短租约，晚到结果不能写进另一个 incarnation。

取消只是请求。清理失败或超时进入 NeedsReconcile，不返回 Disposed/Success；按实际
cgroup/process/目录 handle 身份回收，不能只凭重启后可复用的 PID 或路径。

scope 结束时撤销它的临时 grant、注册与借用；持久授权政策、Service
登录、业务数据、会话历史与用户 worktree 不属于普通组件 finalizer
可销毁的对象。卸载预览必须列出影响对象、排空/取消策略、保留和明确清理期限；永久清理是独立授权动作。重新安装不偷偷恢复已删除会话。

GC 同时考虑 active/retained generation、Session/Workspace lease、未完成
Operation、前态和补偿
executor。历史保留有期限；到期但仍有恢复引用时不能假装可安全删除，需进入明确的恢复/放弃恢复处置流程。

## 7. Effect：操作能力与恢复承诺分开建模

### 7.1 每个操作描述两件不同的事

`EffectSpec` 记录一个闭集操作及其资源范围，例如
AcquireResource、MutateManagedState、EmitExternal、OpaqueExecute，并关联
`RecoverySpec`。同一个业务调用可产生多个效果，不把它们压成一个等级。

| RecoverySpec       | 承诺                     | 约束                                              |
| ------------------ | ------------------------ | ------------------------------------------------- |
| 无效果 / Pure      | 有界纯变换，无需恢复     | 核心可验证的无环境计算；不包含远端 read           |
| ReleaseOwned       | 释放自己获得的资源       | owner、唯一 handle、清理与释放验证                |
| RestoreIfUnchanged | 前提成立时恢复受管理状态 | 前态/版本、独占或事务条件、CAS 冲突与恢复核验     |
| Compensate         | 按业务规则抵消部分结果   | 有界幂等命令、剩余效果、失败进度                  |
| NoRestore          | 不承诺撤销               | 外发、模型消费或无法控制的内部 I/O 明确告知并授权 |

一个进程租约可以
ReleaseOwned，但该进程之前的文件修改或网络消费不会因此恢复。外部 read
也可能计费或产生日志。补偿不是复原，回滚配置也不能撤销已发送消息。

`EffectSet`
取自身、生命周期、可达调用与事件消费者的效果并集；相同集合不等于相同资源权限。计划后新增
binding/消费者要重新解析和授权，不能借事件扩大原效果边界。

### 7.2 强保证来自实际控制，不来自 Plugin 声明

首版执行形态只需两类：

- 核心解释经过验证的有界数据/命令，执行已知资源与状态原语。
- Machine 运行隔离的精确 Plugin executable；其不受中介控制的内部行为记为
  OpaqueExecute。

第三方空 `undo()`、签名、Rust trait 或 TypeScript effect
泛型，都不能证明可逆。严格模式首版只接受核心实际控制的
Pure、ReleaseOwned、满足恢复前提的 RestoreIfUnchanged 组合；不能经过 opaque
子调用、启动 hook
或事件绕过。以后若引入新的受控执行器，单独证明隔离与语义，不先假定 Wasm
或进程隔离足够。

严格模式定义要恢复的受管理资源集合与可观察状态，不承诺抹去审计、恢复物理时间或撤回外部观察。核心诊断/审计的保留是明确边界；若启用远端
OTel，它按独立外发策略处理，不能向用户声称“完全没有外部痕迹”。

普通模式可在明确授权下使用模型、外发和现有 Agent 原生工具；安装授权不是 I/O
授权，重复调用仍受用途与预算限制。Cowboy 不代理执行现有 Agent
自带的全部文件/终端工具，也不重写它们的记忆、账号或 native session 数据。

### 7.3 持久恢复协议

受核心管理、会改变 durable 状态的操作在执行前记录 intent、精确 executor、版本化
RecoverySpec、幂等键、前态引用、授权证据与保留要求；纯读取/高频 best-effort
遥测不因此变成每条 fsync 的工作流。遥测是否重试继续服从其领域契约。

Service 有协调记录，实际执行 Site 有自己的耐久
step/去重记录。记录优先与本地状态更新同事务提交；跨存储/外部系统的歧义窗口必须
reconcile。不能拿 OTel、`/tmp` 轮转文件或内存 map 充当这个账本。

```text
Planned -> Prepared -> Applying -> Verifying -> Completed
                         |             |
                         +-------------+-> Reconciling
                                            ├── Recovered(outcomes)
                                            └── NeedsAttention(outcomes)
```

恢复结果是 tagged
union：Restored、Released、Compensated、Conflict、Unknown、Blocked、Failed，各带核验/剩余效果/失败原因。父操作保留逐端逐步结果；不把
Compensated 报为 Restored，也不把缺回执报为尚未发生。

保存版本化命令数据，不保存 JS/Rust 闭包。插件专有补偿需保留精确 executor
和最小恢复授权；权限已撤销或证据缺失时阻塞，不能自动登录或扩大范围。恢复顺序来自实际因果与业务前提，不只是简单倒转
Plugin
DAG。补偿本身也可能失败。[补偿事务的并发与恢复边界](https://learn.microsoft.com/en-us/azure/architecture/patterns/compensating-transaction)

## 8. 状态、配置和凭据是组合模型的一部分

### 8.1 所有权与生命周期

| 数据                                             | Owner / 生命周期                                       | 图变更时                                |
| ------------------------------------------------ | ------------------------------------------------------ | --------------------------------------- |
| 产品用户、Passkey、Provider 凭据、历史、恢复账本 | 对应核心服务的耐久权威                                 | Plugin/组件卸载不能删除                 |
| Plugin 业务数据                                  | 核心管理的稳定业务 namespace + scope/security identity | 与实例 generation 分离，按契约保留/迁移 |
| 实例临时数据与进程资源                           | 精确 owned scope                                       | 排空、验证、释放                        |
| UI 局部状态/观察                                 | 可信 view scope                                        | 关闭只清观察与临时状态                  |
| 诊断 OTel 文件                                   | 核心受限 `/tmp` 目录                                   | 可轮转，不参与业务恢复                  |
| 原生应用/Provider native memory                  | 各自已有明确所有者                                     | 不因图统一转移或清理                    |

稳定业务 namespace 不能简单包含新 generation 就创建一份空数据，也不能只用全局
Plugin ID 忽略用户/工作区隔离。其身份绑定所属 Service、安全域、业务 dataset
与逻辑 scope，实例只持有已授权 reader/writer lease。

迁移声明 reader/writer 兼容范围、前置 schema、写入
fencing、验证与恢复策略。先判断旧实例能否继续访问；不能共存时维护切换，不能在候选
probe 中偷偷迁移并让旧 active 失效。PostgreSQL/SQLite 原有迁移精确字节保持不变。

核心数据库驱动和 SQL 隔离检查保留。新增业务访问优先用 schema-typed
操作或受限查询声明，不向 Plugin 交出 core pool、连接字符串或任意 SQL
执行权；迁移同样不能访问安全/账本 namespace。当前 SQL guard 不是通用可逆性证明。

### 8.2 配置引用不等于秘密或授权

公开默认值随签名包，实际绑定/启用选择属于对应 Site
的私有配置。动态配置更新作为有前提的绑定变更，显式列出影响；秘密仅用
owner/purpose 受限引用，不放在 IR、graph dump、日志、回执或用户可见诊断中。

Provider Service 登录/刷新/退出继续维护单调 auth generation、完整候选 CAS、按
Machine 加密投影与明确 ACK。Machine 上兼容代次的可刷新凭据不能复制成多个旋转
token 分支；保留现有单一刷新 lineage，不能因组件实例化创建一份新账号。

Machine 本地连接器的 endpoint/token 使用单独本地策略；不是 Service Provider
账户，不通过通用组件 props 透传。Policy/auth epoch 永不因 graph rollback 降低。

## 9. UI 组件库：通用创作层与可信呈现层分开

### 9.1 组件复用不能依赖 Provider 身份

Button、Stack、Sheet、主题、焦点、移动手势、动画、无障碍与 renderer
实现属于核心组件库。Plugin 使用通用布局/状态/消息类型，通过领域 facet
获得只读业务投影与允许的 intent；Code/Telemetry/Identity UI 不需要伪装成 Agent
Provider。

UI builder、fragment
与业务子图模板可独立发布为组件包，编译为有界数据。模板展开必须纯且有总量/深度预算，子状态、资源与消息
ID 有确定的私有命名空间。React 组件树不是运行时能力
DAG，不给每个按钮建立分布式节点。

Slot 决定 context/props/model/message 联合；消息 tag 决定
payload；引用字段必须存在且类型匹配。不能保留任意 context
或通用事件字符串，再依赖调用处断言。输入、输出和持久状态均经过同源 codec。

Plugin 不携带浏览器可执行 JS、任意 HTML/CSS、原生 bridge handler
或可执行模板；资产/URL/矢量数据经核心按类型、内容与用途检查。新增 renderer
要更新核心契约与实现，不能由一个签名模板自行注册。

### 9.2 ViewScope 不等于 OperationScope

每个有生命周期的组件在明确 owner 下登记监听、observer、timer、订阅、stream
与临时状态。核心全局监听允许存在，但由显式 CoreRootScope 持有；消费者借用，不在
import 时创建无归属 effect。

关闭一个 UI 只结束它的观察与临时资源。用户提交安装/模型任务后，核心
OperationScope 或 Machine SessionScope 持有执行；React unmount
不取消操作、不删除偏好、不撤销用户已经确认的效果。显式取消走新的类型化、授权请求。

同一个定义多次挂载不共享隐式单例。晚到 callback 不能写入新
incarnation。销毁要验证资源计数，异步 cleanup 失败保持可见；不能用框架 cleanup
的存在证明 durable recovery。

精确运行绑定和显示投影分开：Machine 卡片的业务动作绑定实际安装 release。历史
Session
可使用经过兼容性验证的较新签名展示，但仅限已验证的纯呈现契约，不能借显示刷新更换执行器、权限或操作
payload。核心 renderer/主题更新也不改变 Plugin 的实际 generation。

### 9.3 Native 与本地登录的核心边界

CoreNativeBridge 使用闭集
request/response/error，验证核心协议版本、实际平台、可信
origin/frame、用户手势和 OS 权限。feature inventory 是核心兼容信息，不是 Plugin
capability grant。Plugin SDK 移除通用 native invoke、native_capabilities 和原生
handler 注册。

Password、Passkey、device authorization、产品会话、安全存储和敏感确认归
CoreSecurity；空 Catalog 或非必需 Plugin
损坏不能移除已配置的本地登录。若运营策略只允许外部
IdP，失效时报告不可用，不能擅自开启本地密码或免认证恢复。

Google/Apple/Cardea 等连接器提供核心 OIDC driver
可接受的数据；协议验证、subject/account
映射、会话签发由核心做。停用阻止新登录；在途流程只在政策未撤销且有界期限内使用原精确配置。Machine
上临时 Provider auth helper 返回候选，不获得 Service 授权裁决权。

Web/native 更新分别走组件发布边界，Plugin 生命周期不能更新 App 二进制。现有
iPhone caret/IME 问题不属于本设计已解决的内容。

## 10. 发布模型：构建依赖、接口依赖和运行依赖分开

组件包可独立版本化和分发，但 Cowboy 安装产品仍只有 Plugin。发布、Site
安装、facet 激活、业务授权是四个不同状态；任何一个成功都不能冒充另外三个成功。

依赖图的边要有明确角色：

- Build/embedded：精确工具、源码与库输入；实际消费者重新构建验证。
- Private runtime：精确平台字节、entrypoint 与 probe；没有 ambient PATH、浮动
  npm、运行时下载或自更新器。
- Contract requirement：消费哪个接口与行为版本；核心实现变动不自动改变接口。
- Tested matrix：验证过的组合证据；不能变成不相关 Plugin 的隐式运行依赖。

签名字节或绑定的依赖声明发生变化，必须发布新的消费者版本。只改变核心 Sheet
或内部 renderer、Plugin 契约和字节未变时，目标是发布 Web
并运行兼容性测试，不要求重发全部 Agent。

`components/registry.json` 保留不可变历史。新设计把整组 release
作为可验证测试索引，逐步转向真实依赖闭包；必须先证明独立构建、完整依赖、影响集与读者兼容，再替换
all-plugins-bump gate，不能提前删检查或重写旧版本。

当前首批迁移：registry schema 3 先追加不改组件的 3.0.0 基线，再在 3.1.0 仅更新
state-store/state-sync/state-sync-idb。保存全部内部 package 边及 Plugin 源码/绑定摘要，
验证直接/间接影响、精确 pins、独立源码副本构建和双 schema Controller 读取。
旧协调规则仍校验历史条目。所有声明的 package/contract 边暂按会导致发布处理；
接口行为兼容豁免、通用运行 DAG 和 durable recovery 尚未实现。

发布继续采用同一个 generic Plugin package/composite signature/平台矩阵。Service
数据与 Machine runtime 绑定到同一
release，任何一个缺失不能宣称相应部署目标完整。Catalog
发布不启用主机存储、不登录、不产生外发授权。

兼容性同时校验 Web/Controller/Machine/CoreBridge
的适用库存、接口、平台和状态读者。未知协议/类型在发送前阻断，失败明确显示需要更新哪个端；不能将新格式推给旧
reader 后再依赖反序列化报错。

## 11. 用完整跨端场景检查抽象

### 11.1 同一 Service，两台 Machine

```text
可信 UI view（只观察/提交 intent）
                 │
Service Core：Session / Code / Telemetry 领域入口
  ├── exact Agent port ───> Machine A / Session S1 / Agent release A
  ├── exact Code port ────> Machine A / Workspace W / Zed release Z
  ├── exact Agent port ───> Machine B / Session S2 / Agent release B
  └── optional export ────> Machine A / Victoria release V ───> 外部 Victoria
```

图中箭头是不同领域的显式 binding，不表示 S1、S2 硬依赖 Victoria。S1/S2
可在不同代次上继续执行。Service 重启后恢复观察；Machine A 断线不改变 B；撤销 A
的外发策略不影响本地遥测和 Agent 的租约。

Victoria 的 Service 选择和 Machine endpoint
策略都满足后才接受远端导出。核心本地文件通路持续工作，exporter
不可用只影响独立有界队列。释放 exporter 可验证，已发出的 logs/metrics/traces
不回收。队列接纳、HTTP
receipt、远端存储可查询是不同证据，不互相替代。[遥测契约](telemetry-plugins.md)

### 11.2 升级 Zed：揭示 state 与 generation 的区别

1. Plan 选择 Machine A 的新 Zed 制品并列出相关 Workspace；不把其他 Machine 或
   Agent 加入维护边界。
2. 准备新 runtime 与其状态，验证旧、新 reader/writer
   是否能共存；不兼容则改成需确认的维护计划。
3. 局部提交只改变新 Workspace 的 binding，既有 worktree/buffer lease
   仍指向原精确代次。
4. 切换失败保持旧入口；如果已经跨边界提交，则按本地 intent
   和协调回执恢复，不声称整个系统未发生变化。
5. 关闭一个 Workspace 只释放自己的借用；制品 GC 等待全部引用，用户源代码不由
   finalizer 删除。

### 11.3 从复用库到独立 Plugin 的判断

一个 context parser 是普通纯组件。组合 parser、Code port 和受控缓存可以形成
scoped 子图。如果这个能力需要独立安装、升级和跨多个 Plugin
持续提供服务，才包装成独立 Context Plugin。

包装改变发布与故障边界，不会凭空使副作用可逆。若其调用模型或直接执行外部
CLI，普通模式授权
Opaque/NoRestore；只有核心可控制的部分进入严格恢复保证。首版不为了图统一改造所有
Agent 内部工具。

## 12. 从现状迁移：按可验证的纵向闭环推进

### 12.1 当前实现可复用什么、缺什么

| 现有位置                                              | 已有基础 / 需要补足                                                                  |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------ |
| [plugin_runtime](../src/plugin_runtime.rs)            | 精确 host 快照与 namespace；不是通用跨端 typed resolver                              |
| [Machine Plugins](../src/machine_plugins.rs)          | 签名制品、generation、安装/回收；耐久卸载回执与独立 installation CAS，写入受 reader-floor 切换约束 |
| [server uninstall](../src/server/plugin_uninstall.rs) | Service 卸载账本、事务完成、重启 fence 与只读 Machine 核对；仍缺恢复授权与补偿步骤 |
| [Plugin storage](../src/plugin_storage.rs)            | namespace/迁移执行；补 reader-writer 共存、稳定数据身份与恢复契约                    |
| [plugin-api](../components/plugin-api/types.ts)       | slot/host/native 混合且 context 宽泛；拆出核心 bridge/host 与 typed authoring        |
| [state-store](../components/state-store/store.ts)     | 已有强类型 codec、owned 订阅与 dispose；仍需其他组件的资源作用域模型                 |
| [Provider UI](../components/provider-ui/src/index.ts) | 有闭集 IR 与验证；生成更强的字段/消息关联，抽出通用 UI 与领域投影                    |

这些是设计差距，不是对全部现有代码的安全审计，也不是本轮已经修复的事项。

### 12.2 分阶段及退出条件

| 阶段                   | 最小可交付                                                                        | 退出条件                                                                    |
| ---------------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| P0：契约与只读模型     | 固定对象/权限模型，两个领域的 Contract profile，纯 resolver，scope/type 负例      | Rust/TS 编解码及 link 验证一致，Service/Machine placement 模型可重放        |
| P1：核心权威与组件分层 | 安全/native 退出 Plugin 身份；拆 SDK/host/native，修组件 codec/释放边界           | 零非必需 Plugin 时核心可用；真实登录/native 与已有业务 gate 通过            |
| P2：首个跨端执行闭环   | Victoria binding 选择/撤销，Service+Machine 局部计划、lease 与耐久 intent/receipt | 双侧授权、断线、ack 丢失、重启恢复、独立本地遥测全部验收                    |
| P3：多实例与状态       | Zed 多 Workspace/代次、Agent 现有流程适配、身份源选择，状态兼容维护计划           | 不改变会话/memory 所有权，跨端 partial/recovery 与旧新 reader/writer 被验证 |
| P4：统一生命周期与发布 | 安装/升级/卸载接入同一有限操作执行器，组件真实依赖闭包，完整诊断投影              | 全部当前 Plugin 的独立 release/lifecycle gate 不退化，旧入口按验收退出      |

顺序有明确依赖，但每阶段仍按 Web、Controller、Machine、native
分别发布，不把正在运行的 Session 作为统一重启的代价。P2 起把最小
resolver、执行与恢复一起做完整，不先发布一个只会画图、还没有恢复语义的通用调度器。

Context/Tool
新领域、自定义业务接口、Wasm、可视化编辑器是后续扩展，不是当前迁移完成的必要条件。没有必要引入
Node/Cordis/Temporal 作为 Cowboy 核心运行依赖；现有精确打包的 Provider
私有运行时不因此被禁止。

### 12.3 安全/native 迁移的硬条件

1. CoreSecurity 先具备不依赖 Catalog 的 Password/Passkey/device-auth/UI
   路径，并保持已配置登录政策。
2. 优先接管现有安全数据的逻辑 ownership，不为改名先搬表。原 SQLx/Plugin
   已应用迁移的精确字节、凭据 ID、公钥、会话和单调安全状态保留；新增迁移与
   authority 记录完成单写者切换。
3. 若后续搬表，单独定义写入
   fence、核验与恢复；不能长期双写出两个凭据权威，也不能让已接管表被 Plugin
   uninstall 清理。
4. CoreNativeBridge 与 Web 客户端先兼容上线。仅在有实际存量需要时保留有限迁移
   shim；它只映射明确核心流程，不接受 Plugin 授权或任意 native 名称。
5. 真实验证密码、设备授权、Passkey、会话保持、iOS
   origin/手势与已支持客户端；通过后退出旧本地 Authentication 包与 native Plugin
   API。
6. 接受新的 Controller/配置/数据库 reader floor 后再退出旧 pin/marker
   路径。回滚仅使用兼容新 ownership 的恢复版本，不能删 authority marker
   去启动旧核心。

旧格式兼容只服务安全迁移与真实在用对象，不保留无限历史或无用
fallback。生产清理、永久数据删除和机器维护仍需要各自明确范围，本文不授权这些操作。

## 13. 验收标准：证明边界，不只证明 happy path

下表是待实现 gate；本次只校验设计文档，不声称运行了这些测试。

| 验收组       | 必须证明                                                                                                                |
| ------------ | ----------------------------------------------------------------------------------------------------------------------- |
| 类型/codec   | 错请求、slot/context、字段/消息关联、未验证 release/plan 被拒绝；重复 key、超限、错 union、64-bit/Unicode/null 边界一致 |
| 组合/作用域  | 混合 owner/依赖环、歧义、无权限、错误 Site、安全域、scope/instance 被拒绝；独立操作交错满足受管理状态等价               |
| 资源与组件   | 双实例隔离、重复/失败初始化、重复 dispose、异步 cleanup、observer/timer/stream 计数正确；晚到响应不能串代               |
| Service 重启 | 连接 scope 消失不销毁 Machine Session；恢复相同 operation/worker 身份，不重复执行不可去重效果                           |
| Machine 故障 | 断线/host 重启/旧 epoch/乱序 ACK 正确处理；adopt 验证、有限离线授权与 partial revocation 状态真实                       |
| 跨端事务     | 在 intent/执行/ack/commit/补偿每个窗口注入崩溃，协调记录与执行记录可核对；不把局部成功报为全局原子成功                  |
| 状态版本     | 旧新 reader/writer 兼容，不兼容必须维护切换；CAS 保留用户后续修改，恢复配置不降低 auth/policy epoch                     |
| 可逆性       | 严格计划拒绝 opaque/emission 绕过；Compensated、Unknown、Conflict、Blocked 与 Restored 不混用                           |
| UI / native  | 关闭 UI 不取消已提交操作；Plugin 不能注册 native/可信 renderer 或窃取确认界面；既有移动交互 gate 不退化                 |
| GC / 删除    | active lease、恢复步骤、前态和 executor 未释放前不删除；用户 worktree/核心数据不随组件卸载清理                          |
| 实际业务     | 六个 Agent、Zed、Victoria、外部身份源均通过对应生命周期；OTel 丢失和 /tmp 清空不影响恢复权威                            |
| 发布/迁移    | 核心 UI、公共契约、私有依赖三类变化有正确影响闭包；真实认证/native验收、精确平台制品、当前/回滚 reader gate 通过        |

## 14. 取舍与依据

这不是 Everything Is a Plugin，也不是把所有模块都变成分布式
RPC。固定核心可以用相同 ownership 原语；纯函数和普通 UI
树保持轻量；只给实际资源、依赖和跨端操作建立必要记录。高频观察不强制经过通用持久工作流。

借鉴 Cordis 的 context-mediated ownership
和生命周期组合，不复制其运行时，也不继承未经 Cowboy 验证的安全/恢复保证。异步
disposer 可以并行，Cowboy
必须显式表达有序清理。[Cordis 生命周期](https://github.com/deepseek-ai/deepseek-harness/blob/c389f96bf3a9b6807cb71ed6bdad5849be0df6d8/docs/cordis-tutorial/02-lifecycle-and-effects.md)

可逆保证受系统边界限制，内部资源释放不能撤回外部
emission。[时空可组合论文，§6.1](https://arxiv.org/pdf/2608.25512#page=70)
本文的 Site 划分、固定核心、数据版本与分布式恢复是 Cowboy
的设计选择，不声称来自论文的直接证明。

首版优先选择：固定领域接口而非任意动态服务注册；局部事务与明确恢复而非全局原子承诺；数据型
UI
而非宿主代码注入；精确制品与按影响发布而非运行时共享可变库。代价是新权力/renderer
需核心升级、故障状态更明确、跨端部分完成需要恢复界面；这些代价换取可解释的权限、可验证的生命周期和可维护的类型边界。
