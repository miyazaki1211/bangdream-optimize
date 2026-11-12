# Micro-Optimize(微操理论最高分)设计与实现

> 状态:实现完成(内核 + 数据解析 + 服务 + HTTP 端点)。
> 语义基准:Bestdori 歌曲最佳化前端(`https://bestdori.com/tool/songoptimizer`),
> 金样回归 `crates/data/tests/micro_golden.rs` 与前端实测逐位一致。
> 语义版本常量:`MICRO_SEMANTICS_VERSION = "bestdori-compatible-v1"`。

## 1. 目标与范围

`micro_opt` 计算**固定队伍**打某首歌在某活动配置下的理论最高分,包含微操:
把个别 note 提前/延后点击(帧级偏移,保持在至少 GREAT 判定内——PERFECT
或 GREAT 均能触发技能,技能键不要求打成 GREAT),使技能时间段覆盖尽可
能多的高价值 note。

- 输入:`playerId + eventId + songId + difficulty + team[5 卡 ID]`;卡详情、
  地区道具、潜能全部来自玩家档案;活动加成、卡面三维、技能效果、谱面全部
  来自 `var/game-data-cache` 静态镜像。
- 输出:基线分、理论最高分、技能发动次序、微操偏移表、技能窗口明细。
- 计算语义与 Bestdori 前端逐位兼容(含其全部怪癖,见 §3)。
- 不做队伍搜索、不做 PT、不影响 `maximize`/`score_range` 现有路径。

## 2. 分层与模块

```
crates/core/src/micro_opt/     纯计算内核(零 IO)
├── mod.rs      公共 API:MicroOptimizeInput / MicroSearchConfig /
│               micro_optimize() / MicroOptimizeResult
├── events.rs   MicroHitKind(10 种)/ MicroHitEvent / sort_events
├── judge.rs    判定窗口表、combo 表、accuracy 表、生命罚分表
├── skill.rs    技能效果族(12 类)/ MicroSkill
├── simulate.rs 单次确定性模拟(k() 移植,含技能排程与窗口统计)
└── search.rs   贪心搜索(S() 移植,120 排列 + 边界平移候选)

crates/data/src/micro.rs       解析与组装
├── micro_events_from_bestdori()   谱面 JSON → 保留 note 类型的事件表
├── micro_skill_from_bestdori()    技能 JSON → 完整效果族(按等级取值)
├── micro_card_total()             全量道具模型的每卡综合力
└── MicroOptimizeOutcome           序列化响应 DTO

crates/data/src/filesystem.rs  micro_optimize_sync(输入组装入口)
crates/data/src/cache.rs       缓存层 trait 实现(先同步谱面再计算)
crates/service/src/micro_optimize.rs  MicroOptimizeService(档案加载)
apps/server/src/main.rs        POST /v1/micro-optimize(+ /from-config)
```

现有 `Chart`/`ChartNode` 丢弃 note 类型(`Node|Skill`),微操需要 10 种
note 类型各自的判定窗,因此事件模型独立实现,不复用也不修改 `Chart`。
现有 `SkillDefinition` 把技能降维成 `{duration, score_up, rateup}`,
微操需要完整效果族,因此新增富解析,不动旧解析。

## 3. 计算语义(Bestdori 兼容,逐条编号)

偏移单位 = 帧(1/60 秒);时间 = 谱面 BPM 换算的秒。

1. **事件展开**:Single→tap/flick;Directional→directionalFlick(永不带技能位);
   Long→connections[0]/[1](头/尾,尾可 flick);Slide→全部可见 connection
   (按含 hidden 的原始下标判定头/中/尾);同时刻**普通 note 排在技能 note 前**。
2. **判定窗口**(PERFECT/GREAT/GOOD/BAD,闭区间,帧):
   tap/flick/directionalFlick/longStart ±3.5/±6.5/±7.5/±8.5;
   longEnd/longFlick ±4.5/±7.5/±8.5/±9.5;
   slideStart/slideEnd -3.5~12.5/-6.5~12.5/-7.5~12.5/-8.5~12.5;
   slideTick 仅 [0,12.5];slideFlick 仅 [0,6.5]。落入任一窗口取最高档,
   否则 MISS。滑条类各档共享上界,边界值(如 +12.5)直接 PERFECT。
3. **分数公式**:
   `每note分 = floor(floor(bp × (3+0.03×(难度-5))/note数 × combo系数 × 判定系数) × 技能系数)`;
   combo 系数 20/50/100/150/200/250/300/400/500/600/700 分档 1.00→1.11;
   判定系数 PERFECT=1.1、GREAT=0.8、GOOD=0.5、BAD/MISS=0。
4. **技能触发**:技能 note 判定 ≤ GREAT 才发动;按 6 槽次序消费
   (5 卡排列 + 固定循环第 0 位);窗口 = `(触发时刻, 触发时刻+时长]`;
   触发时若有技能生效 → 排队到 `上窗口 end + 0.75s`;技能 note > 6 → 显式报错。
5. **技能效果族**:score、judge(判定升格)、damage(免罚分)、never_die
   (保命 5)、onceEffect:life(回复上限 2000)、score_over_life、
   score_under_life、score_continued_note_judge(断连失效)、
   score_rate_up_with_perfect(每 PERFECT +0.5%,封顶 100 次)、
   score_under_great_half、统一加成(同团/同属性,`o || value` 语义)。
   未知效果键忽略(与前端一致);升格后的判定参与 combo/排程/罚分。
6. **生命**:初始 1000;BAD -50、MISS -100(slideTick MISS -20);仅影响
   生命条件技能,不判失败。
7. **搜索**(确定性,迭代顺序全部固定):枚举前端同款 120 个五卡排列
   (或固定次序),按 `(skillId@等级)` 序列去重;每排列对 6 个技能 note
   依序贪心:每个候选偏移做一次完整模拟,并附加"边界平移"子候选
   (把窗口起点前/终点后最近的普通 note 拉入窗口);更优 = 分数严格高,
   同分取 |偏移| 累计严格小。**边界子候选的终点侧为闭区间**
   (2026-10-18 变更:与模拟器的 `hit ≤ end` 覆盖对齐,`≤` 取代前端的
   严格 `<`;压端偏移(如 785 Expert 的 306@−3.5、506@−2.5)现在会被
   提案,前端无法提出此类偏移,属有意偏离。最小 |偏移| 的压端候选在
   判定档与 tie-break 上严格占优,单个子候选即充分;  起点侧维持严格
   `>`,与排他起点一致)。另支持 `skill_window_end_grace`(秒,默认 0)
   在覆盖判定上延长窗末(如 1/60 = 帧边界含 1 帧的引擎口径),排队
   起点与上报窗界不受影响。
8. **连接对与同轨互作用约束(2026-10-18 新增,有意偏离前端)**:谱面展开时
   记录两类有序物理依赖对(`micro_events_with_pairs_from_bestdori`):
   ①每条长条/绿条的 `(头索引, 尾索引)`;②**同轨道相邻 note 对**——游戏按
   「最早的未命中 note」分配同轨输入,后键不可能先于前键命中,自然间隔
   ≤ 24 帧(= 最大延后 12.5 + 最大提前 8.5 + 余量,交叉可能范围)的同轨
   相邻 note 逐对登记(相邻对经 ≥1 帧门槛传递封闭)。搜索拒绝任何使
   **后键命中早于(或距)前键命中不足 1 帧**的偏移组合。Bestdori 前端
   无此类约束:738 Expert 旧结果中技能键绿条头 +10.5f 把头尾间隔压到
   0.23 帧(物理不可完成);约束后该曲最优 3,908,899(无约束 3,914,399,
   代价 −0.14%),绿条头尾间隔恢复 3.73 帧。缺 lane 字段的合成谱面用
   唯一伪 lane(不生成互作用对)。
8. **默认搜索参数**:范围 = 各类型 BAD 窗边界(等价前端自动最佳化默认),
   步长 0.5 帧,无限制;非技能 note 的 0 偏移不入候选,技能 note 含 0。
9. **浮点**:与 JS 相同的 IEEE754 运算顺序与 floor 语义,Rust f64 逐位复现。

### 前端怪癖(有意保留/折算)

- **同时刻排序**:前端 tie-break 为 `!!t.skill - !!e.skill`,普通 note 在前。
- **活动加成分流**:仅 `challenge`/`versus` 的活动加成计入综合力
  (前端 `liveSpecial` 门控,默认开);story 等类型活动加成不进打歌分。
  分流判定用原始 `eventType` 字符串,绕过核心枚举(核心无 story 变体)。
- **道具等级 0 = 1 级(前端)**:Bestdori 前端对等级 0 的道具读
  `performance[level+1]`,即"未选道具"仍计入 1 级加成(默认 PoPiCo
  乐器 +22%、powerful +4%)。金样用"9 件道具各 1 级"的档案复现前端
  默认数值。
- **综合力与 `/v1/calc-result` 完全同口径**(2026-10-17 修正):活动数据
  按同一条解析链(玩家活动预设 → `api/events/{id}.json` 详情 → 汇总
  文件,叠加 eventCharacterParameterBonusFix),道具按 maximize 的
  选择式模型——对固定队伍枚举 `area_item_combinations`,取
  `PreparedCard::add_up_stat`(选定 1 乐队键 + 1 属性键 + 1 杂志)下
  综合力最大的一组,五卡求和后单次 floor。先前版本曾按"档案全量持有"
  (所有属性排、全部杂志同时生效)计算,会高于 calc-result,已废弃。
- **技能数值的 null 槽位(有意偏离前端)**:Bestdori 线上数据在部分技能的
  `activateEffectValue`/`onceEffectValue` 第 5 级槽位存 `null`
  (如技能 66/69/72,效果文案与等级无关,例如"得分提升150%")。前端
  歌曲最佳化此处会算出 `null/100 = 0`(满级技能零加成,属前端数据缺陷)。
  本实现改为**向下回退到最近一级的非空数值**(平数组即真实值),
  与游戏实际一致;数组全空时取 0。

## 4. 接口

### HTTP

```
POST /v1/micro-optimize            # 档案驱动(需 mongo 档案库)
{ "playerId": 123456789, "eventId": 1, "songId": 101, "difficulty": 3,
  "team": [1, 5, 9, 13, 17],
  "judgementWindows": { "tap": [[-2.5,2.5],[-6.5,6.5],[-7.5,7.5],[-8.5,8.5]] } }

POST /v1/micro-optimize/from-config # 直传 PlayerConfig(无需档案库)
{ "player": {…PlayerConfig…}, "eventId": 1, "songId": 101, "difficulty": 3,
  "team": [1, 5, 9, 13, 17],
  "judgementWindows": { …同上… } }
```

`judgementWindows` 可选:按 note 类型自定义判定区间(帧,闭区间,按
perfect→great→good→bad 排序;slideTick/slideFlick 仅 1 个)。未列出的
类型回退默认 Bestdori 表(全部边界含值);默认搜索范围跟随有效窗表的
最外层窗边。校验失败 400
`judgementWindows has invalid value: <类型>: <原因>`;使用自定义表时
`meta.customJudgementWindows = true`。

`difficulty` 0-4(easy/normal/hard/expert/special)。错误统一
`{"status":"error","message":…}` 400:档案不存在、卡不在档案、活动/谱面
缺失、技能等级越界、技能 note > 6 等。

### Rust

```rust
// 纯计算入口(零 IO)
micro_optimize(&MicroOptimizeInput, &MicroSearchConfig) -> Result<MicroOptimizeResult, MicroOptError>

// 数据层组装
BestdoriFilesystemCalculationInputBuilder::micro_optimize_sync(
    &player, event_id, &song, &team, judgement_windows: Option<JudgementWindowSet>)

// 服务层
MicroOptimizeService::optimize_for_player(
    player_id, event_id, song, team, judgement_windows: Option<JudgementWindowSet>).await
```

### 响应形状

```jsonc
{
  "status": "ok",
  "data": {
    "eventId": 326, "eventType": "versus",
    "song": { "songId": 785, "difficulty": 3, "playLevel": 26, "noteCount": 676 },
    "team": { "cards": [ { "cardId": 2178, …, "skillId": 72 } ×5 ],
              "bandPower": 559522,
              "unification": { "bandId": 21, "attribute": "pure" } },
    "baselineScore": 3860878,
    "maxScore": 3888941,
    "scoreGain": 28063,
    "skillOrder": [2178, 2275, 1609, 2361, 1958, 2178],
    "optimizations": [ { "index": 34, "kind": "tap", "time": 8.02,
                         "offset": 4.5, "judgement": "great" } …15 项 ],
    "skillWindows": [ { "slot": 0, "cardIndex": 0, "cardId": 2178,
                        "triggerIndex": 35, "triggerTime": 8.07,
                        "start": 8.07, "end": 15.07,
                        "queued": false, "coveredNotes": 55 } ×6 ],
    "meta": { "simulations": 35520, "totalOffset": 73,
              "semantics": "bestdori-compatible-v1",
              "customJudgementWindows": false }
  }
}
```

`offset` 单位帧;`optimizations` 只含非 0 偏移;`judgement` 为微操后的
最终判定(升格后);`coveredNotes` = 窗口覆盖 note 数;`queued` = 是否排队。
`meta.customJudgementWindows` = 本轮是否使用了自定义判定窗表。
`skillOrder` 为 6 个发动槽的**卡牌 ID**;第 6 槽(循环位)由搜索在全部
卡位中枚举(2026-10-17 修正:先前硬编码为队伍 0 号位,系 Bestdori 工具
UI 限制的移植,会漏掉最优绑定,与 calc-result 的队长位枚举不一致)。
`baselineScore` 为**枚举全部技能次序(含第 6 槽绑定)后**的自然时序
(全 0 偏移)最高分;`maxScore` 为最优次序叠加微操后的理论最高分,
`scoreGain` 为两者之差。注意 Bestdori 工具的概要分数在点击「计算!」
后会显示优化分,不要与自然基线混读。

## 5. 验证

1. **金样回归**(`crates/data/tests/micro_golden.rs`):SAKURAスキップ
   Expert(101/3)、活动 1、初始五卡(等级 20/双后篇/技能 1/未特训)、
   9 件 1 级道具、零潜能 →
   `bandPower 49783 / baseline 215014 / max 215399 / 18 项偏移逐项相等 /
   次序 [0,1,2,3,4,0] / 6 窗口时长 5.0`。
   期望值取自 Bestdori 前端实测(2026-10-17,两次独立运行一致)。
2. 单测:判定边界(含滑条共享上界)、双 floor 计分、排队 0.75s、
   BAD 技能 note 占槽不发动、MISS 断连、候选去重、确定性、
   谱面展开(kind/skill 标记/hidden)、技能效果族解析、生命回复条件。
3. **端到端**:`POST /v1/micro-optimize/from-config` 实测返回与金样一致
   (见 §4 响应示例,590 次模拟)。

## 6. 已知边界

- 技能 note > 6 的谱面显式报错(前端此处崩溃,安全子集)。
- 自定义卡(custom cards)不在 Bestdori 卡目录中,暂不支持(显式
  MissingEntity);接档案后可后续扩展。
- 微操搜索为贪心近似(与前端同款),不保证全局最优;前端自述
  "无法正确计算重叠的技能"——本实现同样以 0.75s 串行排队建模重叠。
- `server` 字段未进入入参(与定稿一致);数据解析固定 `Server::Jp` 视图,
  如需分服道具可见性再评估。
- **默认判定窗表 = 游戏实测值(2026-10-18 变更)**:tap/flick/directionalFlick/
  直线绿条首键/**长条首键(longStart)** PERFECT ±2.5 帧(±41.667ms,含边界);直线绿条尾键
  (−3.5, +3.5) 帧**不含边界**(±58.333ms);变轨绿条首尾键
  −3.5 ~ +12.5 帧(−58.333 ~ +208.333ms,含边界)。为此谱面解析按
  「连接点 lane 是否变化」把绿条拆分为 slideStraight* / slideShift* 四类
  (隐藏连接点计入几何)。GREAT/GOOD/BAD 行与长条窗维持 Bestdori 数值。
  窗口用 `JudgementWindow{min, max, inclusive}` 表示;请求
  `judgementWindows` 覆盖时元素可为 `[min, max]`(含边界)或
  `[min, max, inclusive]`;`meta.customJudgementWindows` 回显。金样改为
  显式钉住 Bestdori 旧表(通过 overrides)以保持 215,014 → 215,399;
  785 Expert 在新默认表下搜索最优 4,302,668、次序 [1,3,2,0,4,0]——压端
  子候选(≤ 修正)使总偏移 67.0 → 66.5(419/506 压端 −2.5 等,同分更省);
  longStart PERFECT 曾为 ±3.5,收窄至 ±2.5 后中间值 4,302,991(W3 触发
  +6.5、306 以 −3.5 压端拉入)失效;旧表搜索最优 4,311,708
  (严格 < 时期 4,303,980)。若开启 `skill_window_end_grace = 1/60`
  (帧边界含端口径,默认关闭):game-truth 4,314,954 / 旧表 4,315,616。

## 7. 与 `/v1/calc-result` 计分方式的一致性对照

两条链路**公式骨架一致、时间/数值语义存在已知的刻意差异**,同一输入下
分数不相等。金样谱面(101 Expert、综合力 49783、6×(+10%/5s)技能、
自然时序)实测:`micro baseline 215014`,`Chart::get_score 215110`,
**calc 高 96 分(约 0.045%)**。

强技能场景放大了同一组差异(事件 326 / 歌曲 785 Expert / 五卡技能
135%-150%,同综合力同技能对照):`micro max 13,448,150`,
`calc 最优次序 13,473,181`,**calc 高 25,031(约 0.19%)**;其中
同次序对比是 micro 高 5.2 万——即差距全部来自 calc 的理想化时间
语义(窗口末端 +1 帧的得分最大化端点、同刻 note 免费入窗)与其
内部技能次序精确搜索的组合,而非 micro 搜索失灵。

一致的部分:

| 维度 | 两者 |
| --- | --- |
| 单 note 公式 | 同为 `floor(floor(综合力 × 基础系数 × combo × 判定) × 技能)` 双重 floor |
| 基础系数 | `3×(1+0.01×(等级-5))/note数`,代数等价(浮点结合顺序不同,存在 ulp 级差异) |
| 判定系数 | 全 PERFECT 假设下同为 1.1(calc 折入 base_multiplier;micro 逐 note 判定,理想打法等价) |
| combo 表 | 完全一致(calc 用后置计数、micro 用前置计数,阈值等价;calc 另有 medley 续表) |
| rateup 技能 | 完全一致(2.0 起步、+0.005/note、2.5 封顶) |
| 技能槽 | 同为 6 槽(5 卡 + 循环位);按次序消费 |
| 综合力取整 | 同为五卡求和后单次 floor |

差异的部分(按影响排序):

1. **同时刻排序**:calc 默认技能 note 在前,且"允许普通键延后一帧获得
   刚触发的技能加成"(项目文档明示的理想化假设);micro 按 Bestdori
   普通 note 在前,同刻覆盖需靠微操偏移实现——这是 96 分差异的主要来源。
   即 calc 的"自然时序"已隐含一部分帧级微操,micro 的 baseline 没有。
2. **技能窗口时间语义**:calc 用 Ideal60 帧级调度(触发帧下一帧起效、
   终点按 `触发帧+时长帧` 含帧边界、排队起点 = 起点+时长+48 帧 ≈ 时长
   +0.8s);micro 用 Bestdori 实数窗口 `(触发, 触发+时长]`、排队
   = `上窗 end + 0.75s`。窗口边界与排队间隔相差 1-3 帧。
3. **综合力求解模型(已对齐)**:两者都走 `calculate_area_item_percent`
   + `PreparedCard::add_up_stat` + 单次 `floor_team_stat`;maximize 在
   队伍搜索中取最优道具组合,micro 对固定队伍取综合力最大的组合,
   同档案同队伍下数值一致(2026-10-17 修正,micro 曾按全量持有计算
   导致偏高)。
4. **活动加成分流**:calc 对其支持的全部活动类型(含 medley)都把活动
   加成计入综合力;micro 仅 challenge/versus 计入(Bestdori 工具无
   medley 类型,story/mission/live_try 均不计入)。medley 活动下两者
   分流不同,需要业务确认 micro 是否跟随 calc。
5. **micro 独有建模**:判定降级(GREAT/GOOD/BAD/MISS)、生命值与
   生命条件技能、12 类技能效果族、逐 note 偏移搜索;calc 假设理想
   全 PERFECT 且只建模 score/rateup 两类技能。

结论:两者是**同一游戏公式的两种实现口径**——calc 面向队伍搜索的
理想化精确计分,micro 面向固定队伍的 Bestdori 兼容微操模拟。数值上
不互相校验属于预期;若需要口径统一,差异点 1/2/4 是候选决策项。

## 8. 性能

金样(650 note、6 技能位、去重后 1 个排列)590 次完整模拟,
release 单请求 < 100ms;最坏情形(5 张不同技能 → 120 排列全枚举)
约 7 万次模拟,Rust 下仍在秒级内,同步端点即可承载。
