# 数据结构与存储布局 · v0.2

本版状态 schema version=2。Rust 定义见 src/domain.rs、src/ledger.rs、src/workspace.rs。JSON 是可读的主存储格式；没有数据库和外部服务。

## 便携状态 State

| 字段 | Rust 类型 | 含义 |
|---|---|---|
| version | u32 | 数据格式版本，当前 2 |
| next_id | u64 | 共享单调 ID 分配器 |
| tasks | Vec<Task> | 任务当前状态 |
| models | Vec<Model> | 模型配置，不含任何价格 |
| pools | Vec<Pool> | 一模型一个同名订阅额度容器，多窗口共同约束 |
| accounts | BTreeMap<String, ModelAccount> | 每模型独立订阅信息和 API 上限 |
| budgets | Vec<Budget> | 任务/模型/来源的预留历史 |
| credits | Vec<Credit> | reset 卡和手动机会库存 |
| events | Vec<Event> | 用量、校准、刷新流水 |
| model_preferences | Vec<[String; 2]> | 全局模型偏序边 |
| funding_policy | FundingPolicy | subscription-first / model-first / subscription-only |
| sessions | Vec<WorkSession> | 工作复盘 |
| projects | Vec<Project> | 便携项目注册表、上下文和长期记忆 |

tasks、budgets、credits、events、sessions 共用 ID 序列。模型名、项目名是稳定键，不用本机路径当标识。当前 IDs 不是多设备全局唯一：双端离线修改时拒绝自动合并，不能手工把 JSON 数组拼起来。

## 业务结构

| 结构 | 主要字段和不变量 |
|---|---|
| Task | id、title、importance/urgency 1..5、due、minutes、input/output、capability、status、project、tags、depends、allowed_models、preferences、progress 0..100、factors、splittable、note、created。minutes 为剩余时间；input/output 是全任务基准估计。depends 和偏序分别无环。 |
| Model | name、provider、capability、enabled、bindings。bindings 必须恰有一个且绑定本模型同名 Pool，不能共享。 |
| Binding | pool、input_weight、output_weight。自定义订阅额度单位的折算；API 不使用此权重。 |
| Pool / Window | Pool{name, unit, windows}；Window{name, kind, limit, seconds, anchor}。kind=fixed/rolling/manual；固定窗口依赖锚点和秒数。 |
| ModelAccount | subscription: Option<Subscription>、api_token_limit: u64。上限独立且累计；不含单价、金额或货币。 |
| Subscription | plan、renewal_day: Option<u32>、active、estimate_note。关联 Pool 的固定 7 天 week 窗口，其 limit 是周 token 容量估计。renewal_day 仅提示日历月续期。 |
| Budget | id、task、model、funding、input/output、created、released。同任务/模型/来源最多一条未释放预算；初始值减之后的同来源用量，得到剩余预算。 |
| Credit | id、kind、count、pools、windows、expires、note。pools 字段保存适用模型名；空列表为不限。Tibo 不属于 Credit。 |
| WorkSession | id、task、at、minutes、usage_event: Option<u64>、completed、learned、next_steps、voided。usage_event 关联唯一实际使用流水；时间/成果不从 token 推导。 |
| Project | name、mode、remote: Option<String>、description、context、memory。reference 必须有可携带远程 URL；snapshot 可无远程。本结构没有本机路径。 |
| LocalSettings | hub: Option<PathBuf>、paths: BTreeMap<String, PathBuf>、base_state_hash: Option<String>。只存本机。 |
| Snapshot | commit、branch、bundle（受约束的相对路径）、bytes、sha256。用于校验已提交代码包。 |

没有窗口且没有账户配置的旧模型保留“无限额”语义并在规划中警告；配置 API-only 账户后，不会凭空获得订阅容量。已有旧窗口模型增加 API 时，旧订阅窗口仍有效。

Funding 是 subscription 或 api；它代表额度来源，与金额无关。缓存 token 是 input 子集。一个输入 token 既不与 output 重复，也不因为 cached 再算一遍。

## Event 联合类型

公共字段：id、at（RFC3339 UTC）、note、voided、funding、observed_percent。事件类型通过扁平的 type 字段区分：

| type | 专属字段 | 用途 |
|---|---|---|
| usage | task（可空）、model、input、output、cached、units: BTreeMap<String, f64> | 原始实际 token；units 只允许对应模型，用于订阅校准扣量 |
| snapshot | pool、window、used | 面板校准；observed_percent 保存原始 0..100 百分比 |
| reset | targets: BTreeMap<String, Vec<String>>、source、credit（可空） | 已发生的刷新及卡库存关联 |

非 usage 事件的 funding 必须为 subscription。Snapshot 的 used 保存登记当时换算值；有 observed_percent 时，当前额度查询用当前窗口上限重新换算基线。因此估计修正不会篡改原始百分比和事件。事件追加、撤销保留历史，任务/配置字段编辑不是完整事件溯源。

账户与事件示意（这是节选，不是可直接 import 的完整状态）：

~~~json
{
  "accounts": {
    "strong": {
      "subscription": {
        "plan": "monthly",
        "renewal_day": 15,
        "active": true,
        "estimate_note": "按最近一周手工估计"
      },
      "api_token_limit": 50000
    },
    "fast": {
      "subscription": null,
      "api_token_limit": 0
    }
  },
  "events": [{
    "id": 8,
    "at": "2026-09-21T02:00:00Z",
    "note": "面板观测",
    "voided": false,
    "funding": "subscription",
    "observed_percent": 70.0,
    "type": "snapshot",
    "pool": "strong",
    "window": "week",
    "used": 700000.0
  }]
}
~~~

## 本地目录

~~~text
<data>/
  state.json            # 权威业务状态
  state.json.bak        # 上一个有效版本
  state.json.corrupt    # 恢复时保留的损坏原件（如有）
  local.json            # 当前机器的 hub 路径、项目路径、上次同步哈希
  .lock                 # 命令级互斥锁
~~~

每次 CLI 命令持有数据目录文件锁。业务修改先在内存执行、统一验证，然后原子替换 state.json。错误不写入业务状态。写入前保留上一有效版本；损坏文件不会覆盖最后有效备份。local.json、Git 提交、网络操作与 state.json **不是跨文件分布式事务**；网络失败后数据还在本地，可以检查状态再重试。

默认数据目录及旧路径回退规则见 README。export 只导出便携 State，不导出 LocalSettings 或认证材料。实体名字与 URL 不应包含秘密；CLI 不采集 API key，但用户自由填写的 note/memory 仍属于同步内容。

## Git 工作空间目录

~~~text
schedule-hub/
  .git/
  .schedule-hub.json       # 工作空间格式标记（version=1；与 State schema 版本独立）
  .gitignore
  README.md               # 项目/任务导航索引
  workspace/state.json    # 唯一导入权威；经 State 校验
  tasks/<task-id>/
    task.json
    sessions.json
    CONTEXT.md
  projects/<project-name>/
    project.json          # 模式、原代码仓库 URL、说明、上下文、记忆
    CONTEXT.md
    MEMORY.md
    snapshot.json         # 仅 snapshot 模式且已制作快照
    project.bundle        # 同上；HEAD 可达提交历史
~~~

除 workspace/state.json 外的 task/project JSON、Markdown 是便于阅读的生成视图。请通过 CLI 修改上下文；直接在 GitHub 编辑这些派生文件不会回写 State，下次 push 会重新生成。Snapshot 元数据与 bundle 是恢复项目需要的文件，不是从 State 重建的视图。

reference 项目不复制源码；源代码仍在原仓库独立提交/推送。路径映射留在本机，换设备需 bind 或 checkout。hub push 仅暂存受管路径；已有未知已暂存文件会阻止操作，任意未跟踪文件不会一起上传。受管路径拒绝符号链接和路径穿越；快照校验 SHA-256。

## 同步与冲突

以序列化 State 的 SHA-256 比较本机当前、上次同步基线、远端三者。远端无变更时保留本机修改；本机无变更时接受远端；两端都修改且不同则失败。只快进合并、不 force push。镜像存在未提交更改时 pull 拒绝覆盖；未推送快照也会触发此保护。

--replace 允许显式选择远端状态，保存前保留本地前一版为 .bak，但不丢弃镜像里的未提交文件，也不强制解决 Git 分叉。建议先 export 独立备份。当前面向交接式单写端同步，不自动合并记录，不同步未提交的源码。

## v1 → v2

decode_state 读取 version=1 后升级到 2：旧预算与事件默认 subscription，新 accounts/preferences/sessions/projects 为空，funding_policy 默认 subscription-first。旧价格、cost、currency 字段不进入新结构；使用数据和任务进度保留。只读加载不改原文件，下一次保存先备份旧 JSON。

预测器未来需要另外记录估计版本、观测来源、置信区间和训练窗口。当前保留百分比、时间戳、原始 token、任务特征、复盘，但尚未提供完整估计变更事件日志；不得据此宣称已经具备自动学习。
