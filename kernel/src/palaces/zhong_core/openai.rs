// ── OpenAI-compatible Provider ──────────────────────────────

use std::pin::Pin;

use futures::Stream;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::error::ProviderError;
use crate::stems::action::ToolSchema;
use crate::types::Message;

use super::{LlmProvider, StreamChunk, build_openai_content, classify_http_error, run_or_cancel};

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    api_base: String,
    model: String,
    max_tokens: u32,
}

impl OpenAIProvider {
    pub fn new(api_key: String, api_base: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: Client::new(),
            api_key,
            api_base,
            model,
            max_tokens,
        }
    }
}

/// #7 · OpenAI 流式 tool_calls 增量装配器。
///
/// OpenAI 按 `index` 分片下发 tool call:id 与 function.name 在首片,
/// arguments 逐片增量追加。此前缓冲到 finish_reason 才一次性发出,早派发
/// (EarlyDispatch)对 OpenAI 无延迟收益。此处增量判定"完成"并立即发射:
///   ① 出现更大 index 的分片 → 所有更小 index 的 tool call 完成(OpenAI
///      按 index 升序串行下发,不回写旧 index);仅当其 arguments JSON
///      完整可解析才发射,不完整则继续等待(容错与缓冲路径一致);
///   ② finish_reason 到达 → `finish()` 取走全部剩余(按 index 升序,
///      不再校验 JSON,与旧缓冲路径容错一致)。
/// 单 tool call / 非流式等无更大 index 的场景,行为与旧缓冲路径完全一致。
#[derive(Default)]
struct ToolCallAssembler {
    /// index → (id, name, args_json);BTreeMap 保证发射顺序按 index 升序。
    pending: std::collections::BTreeMap<usize, (String, String, String)>,
}

impl ToolCallAssembler {
    /// 吞入一个 tool_calls 分片,返回因此完成、可立即发射的 tool call
    /// (按 index 升序)。
    fn push_fragment(&mut self, tc: &Value) -> Vec<StreamChunk> {
        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
        let entry = self.pending.entry(idx).or_default();
        if let Some(id) = tc["id"].as_str() {
            entry.0 = id.to_string();
        }
        if let Some(n) = tc["function"]["name"].as_str() {
            entry.1 = n.to_string();
        }
        if let Some(a) = tc["function"]["arguments"].as_str() {
            entry.2.push_str(a);
        }
        // 更大 index 出现 → 更小 index 的 call 完成;仅发射 JSON 完整的。
        let sealed: Vec<usize> = self
            .pending
            .range(..idx)
            .filter(|(_, (_, _, args))| {
                args.is_empty() || serde_json::from_str::<Value>(args).is_ok()
            })
            .map(|(k, _)| *k)
            .collect();
        sealed
            .into_iter()
            .filter_map(|k| self.pending.remove(&k))
            .map(|(id, name, arguments)| StreamChunk::NativeToolCall {
                id,
                name,
                arguments,
            })
            .collect()
    }

    /// finish_reason 到达:取走全部剩余 tool call(按 index 升序,容错同旧
    /// 缓冲路径 —— arguments 不完整也原样发出,由消费侧兜底)。
    fn finish(&mut self) -> Vec<StreamChunk> {
        std::mem::take(&mut self.pending)
            .into_values()
            .map(|(id, name, arguments)| StreamChunk::NativeToolCall {
                id,
                name,
                arguments,
            })
            .collect()
    }
}

impl LlmProvider for OpenAIProvider {
    fn infer_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        let (tx, rx) = mpsc::unbounded_channel();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role.to_api_str(),
                    "content": build_openai_content(m),
                })
            }).collect::<Vec<_>>(),
        });
        if let Some(tools) = tools {
            body["tools"] = serde_json::Value::Array(
                tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": t.parameters,
                            }
                        })
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));

        let cancel_tx = tx.clone();
        tokio::spawn(async move {
            run_or_cancel(cancel_token, cancel_tx, async {
                // #7 · 流式 tool call 增量装配(完成即发射,见 ToolCallAssembler)。
                let mut tc_assembler = ToolCallAssembler::default();
                let resp = match client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", &api_key))
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(Err(ProviderError::Network(e.to_string())));
                        return;
                    }
                };

                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    // Grab headers before `text()` consumes the response —
                    // Retry-After rides on 429/5xx responses (#1).
                    let headers = resp.headers().clone();
                    let body = resp.text().await.unwrap_or_default();
                    let err = classify_http_error(status, &body, &headers);
                    let _ = tx.send(Err(err));
                    return;
                }

                let mut byte_stream = resp.bytes_stream();
                let mut buffer = String::new();
                let mut input_tokens: u64 = 0;
                let mut output_tokens: u64 = 0;

                loop {
                    let chunk = match tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        futures::StreamExt::next(&mut byte_stream),
                    )
                    .await
                    {
                        Ok(Some(Ok(bytes))) => bytes,
                        Ok(Some(Err(e))) => {
                            let _ = tx.send(Err(ProviderError::Stream(e.to_string())));
                            return;
                        }
                        Ok(None) => break,
                        Err(_elapsed) => {
                            let _ = tx.send(Err(ProviderError::StreamStalled));
                            return;
                        }
                    };

                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].trim().to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.is_empty() || !line.starts_with("data: ") {
                            continue;
                        }
                        let data = &line[6..];
                        if data == "[DONE]" {
                            continue;
                        }

                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            if let Some(err) = event["error"].as_object() {
                                let msg = err["message"].as_str().unwrap_or("Unknown OpenAI error");
                                let _ = tx.send(Err(ProviderError::Provider(msg.to_string())));
                                return;
                            }
                            if let Some(choice) =
                                event["choices"].as_array().and_then(|c| c.first())
                            {
                                if let Some(text) = choice["delta"]["content"].as_str() {
                                    let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
                                }
                                // Parse streaming tool_calls (native tools API);
                                // completed calls are emitted incrementally (#7).
                                if let Some(tc_arr) = choice["delta"]["tool_calls"].as_array() {
                                    for tc in tc_arr {
                                        for chunk in tc_assembler.push_fragment(tc) {
                                            let _ = tx.send(Ok(chunk));
                                        }
                                    }
                                }
                                // When finish_reason appears, emit remaining tool calls
                                if let Some(reason) = choice["finish_reason"].as_str()
                                    && reason == "tool_calls"
                                {
                                    for chunk in tc_assembler.finish() {
                                        let _ = tx.send(Ok(chunk));
                                    }
                                }
                            }
                            // Parse usage from final chunk (finish_reason == "stop")
                            if let Some(usage) = event["usage"].as_object() {
                                input_tokens = usage
                                    .get("prompt_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                output_tokens = usage
                                    .get("completion_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                            }
                        }
                    }
                }
                if input_tokens > 0 || output_tokens > 0 {
                    let _ = tx.send(Ok(StreamChunk::Usage {
                        input_tokens,
                        output_tokens,
                    }));
                }
            })
            .await;
        });

        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(index: u64, id: Option<&str>, name: Option<&str>, args: Option<&str>) -> Value {
        let mut tc = serde_json::json!({ "index": index });
        if let Some(id) = id {
            tc["id"] = Value::String(id.to_string());
        }
        if name.is_some() || args.is_some() {
            let mut f = serde_json::json!({});
            if let Some(n) = name {
                f["name"] = Value::String(n.to_string());
            }
            if let Some(a) = args {
                f["arguments"] = Value::String(a.to_string());
            }
            tc["function"] = f;
        }
        tc
    }

    /// 提取 NativeToolCall 三元组,便于断言。
    fn calls(chunks: &[StreamChunk]) -> Vec<(String, String, String)> {
        chunks
            .iter()
            .map(|c| match c {
                StreamChunk::NativeToolCall {
                    id,
                    name,
                    arguments,
                } => (id.clone(), name.clone(), arguments.clone()),
                _ => panic!("expected NativeToolCall"),
            })
            .collect()
    }

    /// #7 · 两片 tool call:第一个 call 在第二个首片到达后即增量发出,
    /// 参数完整可解析;finish_reason 时剩余全部发出;顺序按 index 升序。
    #[test]
    fn emits_first_call_when_larger_index_arrives() {
        let mut asm = ToolCallAssembler::default();

        // index 0 首片(id + name + 部分 arguments)→ 不发射。
        assert!(asm
            .push_fragment(&frag(0, Some("call_0"), Some("read_file"), Some("{\"pa")))
            .is_empty());
        // index 0 续片(arguments 拼齐)→ 仍不发射(无更大 index)。
        assert!(asm
            .push_fragment(&frag(0, None, None, Some("th\":\"/tmp/a.txt\"}")))
            .is_empty());
        // index 1 首片到达 → index 0 立即完成并发射,参数完整可解析。
        let emitted = asm.push_fragment(&frag(1, Some("call_1"), Some("run_shell"), Some("{\"cm")));
        let got = calls(&emitted);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "call_0");
        assert_eq!(got[0].1, "read_file");
        assert_eq!(got[0].2, "{\"path\":\"/tmp/a.txt\"}");
        assert!(serde_json::from_str::<Value>(&got[0].2).is_ok());

        // index 1 续片拼接;finish_reason 时剩余全部发出,顺序按 index。
        assert!(asm
            .push_fragment(&frag(1, None, None, Some("d\":\"ls\"}")))
            .is_empty());
        let rest = calls(&asm.finish());
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0], (
            "call_1".to_string(),
            "run_shell".to_string(),
            "{\"cmd\":\"ls\"}".to_string()
        ));
        assert!(serde_json::from_str::<Value>(&rest[0].2).is_ok());
    }

    /// #7 · arguments 分多片到达:拼接结果与原 JSON 等价。
    #[test]
    fn concatenates_argument_fragments() {
        let mut asm = ToolCallAssembler::default();
        asm.push_fragment(&frag(0, Some("c"), Some("f"), Some("{\"a\":1,")));
        asm.push_fragment(&frag(0, None, None, Some("\"b\":[2,")));
        asm.push_fragment(&frag(0, None, None, Some("3]}")));
        // 更大 index 封口发射。
        let emitted = calls(&asm.push_fragment(&frag(1, Some("d"), Some("g"), Some("{}"))));
        let parsed: Value = serde_json::from_str(&emitted[0].2).unwrap();
        assert_eq!(parsed, serde_json::json!({"a": 1, "b": [2, 3]}));
    }

    /// #7 · 交错到达(同一事件里两片):先 index 0 后 index 1 的分片
    /// 依序吞入,index 0 在 index 1 首片处理时发射,顺序仍按 index。
    #[test]
    fn interleaved_fragments_emit_in_index_order() {
        let mut asm = ToolCallAssembler::default();
        let mut out: Vec<(String, String, String)> = Vec::new();
        for tc in [
            frag(0, Some("c0"), Some("f0"), Some("{\"x\":1}")),
            frag(1, Some("c1"), Some("f1"), Some("{\"y\":2}")),
        ] {
            out.extend(calls(&asm.push_fragment(&tc)));
        }
        out.extend(calls(&asm.finish()));
        assert_eq!(
            out,
            vec![
                ("c0".into(), "f0".into(), "{\"x\":1}".into()),
                ("c1".into(), "f1".into(), "{\"y\":2}".into()),
            ]
        );
    }

    /// #7 · 容错:封口时 arguments JSON 不完整则不发射(等后续分片);
    /// 直至 finish_reason 仍不完整的,与旧缓冲路径一致原样发出。
    #[test]
    fn incomplete_json_waits_then_emits_raw_on_finish() {
        let mut asm = ToolCallAssembler::default();
        asm.push_fragment(&frag(0, Some("c0"), Some("f0"), Some("{bad")));
        // 更大 index 出现,但 index 0 JSON 不完整 → 不发射。
        assert!(asm
            .push_fragment(&frag(1, Some("c1"), Some("f1"), Some("{}")))
            .is_empty());
        // finish:两者都发出,index 升序,残缺 args 原样保留(容错同旧路径)。
        let rest = calls(&asm.finish());
        assert_eq!(
            rest,
            vec![
                ("c0".into(), "f0".into(), "{bad".into()),
                ("c1".into(), "f1".into(), "{}".into()),
            ]
        );
    }

    /// #7 · 单 tool call:无更大 index,全程不发射,finish 时一次性发出
    /// —— 与旧缓冲路径行为一致。
    #[test]
    fn single_tool_call_behaves_like_buffered_path() {
        let mut asm = ToolCallAssembler::default();
        assert!(asm
            .push_fragment(&frag(0, Some("c"), Some("f"), Some("{\"a\":")))
            .is_empty());
        assert!(asm.push_fragment(&frag(0, None, None, Some("1}"))).is_empty());
        let rest = calls(&asm.finish());
        assert_eq!(rest, vec![("c".into(), "f".into(), "{\"a\":1}".into())]);
    }
}
