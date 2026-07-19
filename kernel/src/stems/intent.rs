/// 操作意图 — Operation Intents
///
/// 六仪（Ceremonies）是工具调用与底层操作的天干分类。
/// 甲（LLM）通过六仪间接行动：每个工具调用、每个决策都归属六仪之一。
/// 变体名、语义与天干映射是遁甲 Axiom 2 核心，禁止改动。
#[derive(Debug, Clone)]
pub enum CeremoniesIntent {
    Wu,   // 戊仪: 读取
    Ji,   // 己仪: 写入
    Geng, // 庚仪: 执行
    Xin,  // 辛仪: 转换
    Ren,  // 壬仪: 通信
    Gui,  // 癸仪: 存储
}
