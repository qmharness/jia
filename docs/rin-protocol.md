# Rin 协议(jia Unix Socket)

`rin` 是 jia daemon 的 Unix domain socket 监听器,服务于 jia-tui(终端 UI)与
jia-rin(macOS 常驻 agent)。实现见 `kernel/src/palaces/dui_gateway/rin.rs`。

## 传输与帧格式

- 套接字路径:`~/.jia/rin.sock`,权限 `0600`;仅同 UID(或 root)的对端可连接
  (peer credential 校验)。
- 帧格式:JSON Lines——每行一个完整 JSON 对象,含 `"type"` 字段,`\n` 结尾。
- 单行上限 1 MiB(`MAX_RIN_LINE_BYTES`);超限即断连。
- 事件序列与 HTTP SSE 流共用同一 `StreamEvent` 类型(`kernel/src/types.rs`)。

## client → jia(请求)

| type | 字段 | 说明 | 响应 |
|---|---|---|---|
| `hello` | `cwd` | 解析项目:已有 `.jia/config.toml` 直接读取;否则(家目录、`/`、`/tmp`、`/usr*` 除外)先向客户端发 `confirm_request`(tool=`jia_init`)询问是否建项 | `project_resolved` |
| `agent` | `messages`(`[{role, content}]`)、`session_id`(可空,空则新建)、`cwd`、`project_id` | 发起一次 agent 运行 | 先回 `session`,随后流式事件,以 `done` 结束 |
| `cancel` | `session_id` | 取消该会话的 agent 运行 | 无直接响应(流终止) |
| `set_mode` | `session_id`、`planning`(bool) | 设置该会话下一次运行的交互模式(谋划态开关,/plan) | `interaction_mode_changed` |
| `confirm` | `id`、`token`、`approved`(bool) | 应答一次工具确认请求 | `confirm_resolved` |
| `answer` | `id`、`token`、`answer` | 应答一次 ask_user 提问 | `answer_resolved` |
| `model_info` | — | 查询当前默认主模型 | `model_info` |
| `sessions` | — | 列出全部会话 | `sessions` |
| `load_session` | `session_id` | 加载会话历史(空 sid 返回空 entries) | `session_history` |

未知 type 仅记 debug 日志,不报错。

## jia → client(事件)

### 连接级响应(请求-应答)

```json
{"type":"project_resolved","cwd":"...","project_id":"...","approved":true}
{"type":"interaction_mode_changed","planning":true}
{"type":"confirm_resolved","id":"...","resolved":true}
{"type":"answer_resolved","id":"...","resolved":true}
{"type":"model_info","provider":"...","model":"..."}
{"type":"sessions","sessions":[...]}
{"type":"session_history","session_id":"...","entries":[...]}
```

### agent 运行流(`StreamEvent`,serde tag = `type`)

```json
{"type":"session","session_id":"..."}                  // agent 请求受理后首个事件
{"type":"delta","content":"..."}                       // LLM 增量文本
{"type":"tool_call","tool":"...","input":{...}}
{"type":"tool_result","tool":"...","output":"...","error":null,"geju":null,"execution_mode":"direct"}
{"type":"tool_batch_start"}
{"type":"confirm_request","id":"...","tool":"...","reason":"...","timeout_secs":120,"token":"..."}
{"type":"user_question","id":"...","question":"...","timeout_secs":0,"token":"...","options":["..."]}
{"type":"stream_end"}
{"type":"context_pressure","tokens":123,"threshold":456}
{"type":"compacting"}
{"type":"interaction_mode_changed","planning":false}
{"type":"error","message":"..."}
{"type":"done"}
```

注:`tool_result.execution_mode` 与 `user_question.options` 为可选字段,缺省时不出现在 JSON 中。

### 广播事件(所有连接均收到)

```json
{"type":"cron_notification","job_name":"...","prompt":"...","response":"...","timestamp":1718000000}
```

由神盘 EventBus 的 `RuntimeEvent::CronCompleted` 转发,每 daemon 一个 forwarder,
每连接一个订阅任务(写失败即退出)。

## 断连语义

连接结束(EOF / 读失败 / 超限行)时,daemon 按本连接启动过的 session_id 清扫
`pending_questions` / `pending_confirmations` / `session_modes`:被清扫的
oneshot sender 随条目 drop,等待中的 ask_user / 确认立即以 Err 醒来
(视为"用户断连"/拒绝),agent 得以继续并释放 session_lock。断连**不** cancel
session token——TUI 断开后长任务继续运行(有意语义,2026-07-19 裁决)。
