# 模块架构 · v0.2

## 分层

| 文件 | 职责 |
|---|---|
| src/main.rs、src/bin/tc.rs、src/bin/tongchou.rs | 三个等价入口，命令执行与失败退出 |
| src/cli.rs | 参数定义、原命令组、业务命令分发、解析、只读/写入边界 |
| src/commands.rs | subscription/api/policy/work/project/hub 命令组 |
| src/domain.rs | 任务/模型/窗口/预留/事件/卡、校验、当前额度、重置 |
| src/ledger.rs | ModelAccount、Subscription、Funding、FundingPolicy、WorkSession；API 独立余额 |
| src/planner.rs | 依赖优先级继承、偏序极大元、来源策略、整数可行性、预留与重规划 |
| src/workspace.rs | 项目注册、本机绑定、Git 聚合、bundle、中心仓库索引与冲突检测 |
| src/store.rs | 数据目录、文件锁、原子写、备份、v1→v2 解码迁移 |
| src/ui.rs | 中文表格、可见宽度、控制符处理、四象限、额度条、排程与趋势 |
| tests/core.rs、tests/cli.rs、tests/v2.rs | 数学约束、CLI 事务、用量来源、项目与双端同步回归 |

## 本地执行路径

CLI 解析 → 打开数据目录并持锁 → 读取/迁移/验证 State → 内存执行命令 → 写操作验证并保存 → 中文或 JSON 输出。

业务错误不保存内存修改。State 保存使用同目录临时文件及原子替换，旧有效状态保留 .bak。LocalSettings 独立原子写；Git 与网络副作用不是跨文件事务，因此不能声称整次 hub/project 操作跨网络全原子。

本地文件锁防止两个终端同时修改同一 data 目录；多设备同步通过中心 Git 仓库和上次状态指纹检测冲突，不靠把同一个 state.json 放入网盘目录。

## 规划路径

1. 筛选任务范围，计算重要性/紧迫性/截止分和依赖继承。
2. 从全量有效预留扣减各模型各来源可新分配额（包括范围外任务）。
3. 保留并校验任务已有预留，换算已覆盖的基准需求。
4. 按硬约束、来源策略、偏序极大元选择可行模型/来源。
5. 对原始 token 向上取整后检查额度；只有能覆盖全部剩余需求的任务才提交试分配。
6. 生成时间表、模型来源片段、警告和暂缓原因。
7. 默认不保存；commit 新增预算。rebalance 先在副本释放范围内未用预留。

## 同步路径

hub push：检查标识 → 查询/获取远端 → 状态指纹和 Git 祖先检查 → 生成受管索引 → 仅暂存受管文件 → commit → push → 保存基线指纹。

hub pull：查询/获取远端 → 解码/校验远端 State → 三方状态冲突检查 → 拒绝镜像脏文件 → 快进 → 备份/保存本地业务 State → 保存本机基线。

reference 源码不通过 hub 传输。snapshot 从干净已提交仓库生成 HEAD bundle，再显式 push 中央仓库。checkout 仅向新目录写入，Windows 扩展路径先转换为 Git 可识别的路径。

## 扩展接口

未来预测读 Event + WorkSession + Task，并引入显式估计版本；自动采集经幂等入口写 Event。任何新功能不能绕过模型/来源独立约束，不能以已用 token 替代成果。完整数据结构、未实现功能和决策见 DATA_MODEL.md、ROADMAP.md、DECISIONS.md。
