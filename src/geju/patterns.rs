use super::evaluate::{ApprovalGate, ExecutionMode, GeJuResult};
use crate::stems::Stem;

/// Layer 1: Named specific patterns (~20 key patterns)
pub fn named_pattern(heaven: Stem, earth: Stem) -> Option<GeJuResult> {
    use Stem::*;
    let (name, mode, requires_audit, max_retries, approval_chain) = match (heaven, earth) {
        // ── 大吉格 ──────────────────────────────
        (Bing, Wu) => ("飞鸟跌穴", ExecutionMode::Direct, false, 0, vec![]),
        (Wu, Bing) => ("青龙返首", ExecutionMode::Direct, false, 0, vec![]),
        (Yi, Wu) => (
            "乙加戊（奇克六仪）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Ding, Bing) => ("星随月转", ExecutionMode::Direct, false, 0, vec![]),

        // ── 凶格 ────────────────────────────────
        (Geng, Bing) => (
            "太白入荧",
            ExecutionMode::Sandbox,
            true,
            3,
            vec![ApprovalGate::SandboxIsolation],
        ),
        (Bing, Geng) => (
            "荧入太白",
            ExecutionMode::Guarded,
            true,
            1,
            vec![ApprovalGate::Permission("exec_in_transform".into())],
        ),
        (Xin, Ding) => ("狱神得奇", ExecutionMode::Direct, false, 0, vec![]),
        (Ren, Geng) => (
            "壬加庚",
            ExecutionMode::Denied,
            true,
            0,
            vec![ApprovalGate::Permission("comm_in_exec".into())],
        ),
        (Ding, Gui) => (
            "朱雀投江",
            ExecutionMode::Sandbox,
            true,
            2,
            vec![ApprovalGate::SandboxIsolation],
        ),
        (Gui, Ding) => (
            "腾蛇夭矫",
            ExecutionMode::Guarded,
            true,
            2,
            vec![ApprovalGate::UserConfirmation("存储操作涉及钩子域".into())],
        ),
        (Ji, Bing) => ("太阴入庙", ExecutionMode::Direct, true, 0, vec![]),
        (Ding, Ji) => ("六合逢春", ExecutionMode::Direct, false, 0, vec![]),
        (Wu, Ji) => ("勾陈得位", ExecutionMode::Direct, false, 0, vec![]),

        // ── 伏吟格（同干叠加）────────────────────
        (Wu, Wu) => ("戊加戊（伏吟·读）", ExecutionMode::Direct, false, 0, vec![]),
        (Ji, Ji) => ("己加己（伏吟·写）", ExecutionMode::Direct, false, 0, vec![]),
        (Geng, Geng) => (
            "庚加庚（伏吟·执行）",
            ExecutionMode::Guarded,
            false,
            3,
            vec![],
        ),
        (Xin, Xin) => (
            "辛加辛（伏吟·转换）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Ren, Ren) => (
            "壬加壬（伏吟·通信）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Gui, Gui) => ("癸加癸（伏吟·藏）", ExecutionMode::Direct, false, 0, vec![]),

        _ => return None,
    };

    Some(GeJuResult {
        name: name.into(),
        execution_mode: mode,
        requires_audit,
        max_retries,
        approval_chain,
        layer: 0, // layer set by caller
    })
}

/// Layer 2: Capability semantic matching
///
/// For each capability stem (Wu..Gui) as heaven stem against each capability
/// earth stem, derive a deterministic execution strategy based on what the
/// operation type + target domain means.
///
/// Also handles 三奇 (Yi/Bing/Ding) as heaven stems for meta-operations
/// (skill injection, context compaction, hook dispatch). These are
/// system-level semantic labels — not classical Qimen patterns.
pub fn capability_semantic(heaven: Stem, earth: Stem) -> Option<GeJuResult> {
    // Try 三奇 heaven stems first (meta-operations)
    if let Some(result) = marvels_semantic(heaven, earth) {
        return Some(result);
    }

    // Then 六仪 heaven stems
    use Stem::*;
    let heaven = heaven.as_ceremony()?;

    let (name, mode, requires_audit, max_retries, approval_chain) = match (heaven, earth) {
        // ── Wu (Read) + any earth ──────────────────
        (Wu, _) => ("戊加X（读取）", ExecutionMode::Direct, false, 0, vec![]),

        // ── Ji (Write) + earth ─────────────────────
        (Ji, Ji) => (
            "己加己（写入本位）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Ji, Geng) => (
            "己加庚（写入执行域）",
            ExecutionMode::Guarded,
            false,
            1,
            vec![ApprovalGate::Permission("write_in_exec".into())],
        ),
        (Ji, Ren) => (
            "己加壬（写入通信域）",
            ExecutionMode::Guarded,
            true,
            1,
            vec![ApprovalGate::Permission("write_in_comm".into())],
        ),
        (Ji, Bing) => (
            "己加丙（写入存储域）",
            ExecutionMode::Guarded,
            true,
            1,
            vec![],
        ),
        (Ji, _) => ("己加X（写入）", ExecutionMode::Guarded, false, 1, vec![]),

        // ── Geng (Exec) + earth ────────────────────
        (Geng, Geng) => (
            "庚加庚（执行本位）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Geng, Ji) => (
            "庚加己（执行写入域）",
            ExecutionMode::Sandbox,
            true,
            3,
            vec![ApprovalGate::SandboxIsolation],
        ),
        (Geng, Wu) => (
            "庚加戊（执行读取域）",
            ExecutionMode::Guarded,
            false,
            1,
            vec![],
        ),
        (Geng, Gui) => (
            "庚加癸（执行存储域）",
            ExecutionMode::Guarded,
            true,
            2,
            vec![],
        ),
        (Geng, _) => ("庚加X（执行）", ExecutionMode::Guarded, false, 1, vec![]),

        // ── Xin (Transform) + earth ────────────────
        (Xin, Xin) => (
            "辛加辛（转换本位）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Xin, Wu) => (
            "辛加戊（转换读取）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Xin, _) => ("辛加X（转换）", ExecutionMode::Direct, false, 0, vec![]),

        // ── Ren (Communicate) + earth ──────────────
        (Ren, Ren) => (
            "壬加壬（通信本位）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Ren, Ji) => (
            "壬加己（通信写入）",
            ExecutionMode::Guarded,
            true,
            1,
            vec![
                ApprovalGate::Permission("external_write".into()),
                ApprovalGate::UserConfirmation("外部通信写入确认".into()),
            ],
        ),
        (Ren, _) => ("壬加X（通信）", ExecutionMode::Direct, false, 0, vec![]),

        // ── Gui (Store) + earth ────────────────────
        (Gui, Gui) => (
            "癸加癸（存储本位）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Gui, Wu) => (
            "癸加戊（存储读取）",
            ExecutionMode::Direct,
            false,
            0,
            vec![],
        ),
        (Gui, _) => ("癸加X（存储）", ExecutionMode::Guarded, false, 1, vec![]),
        _ => ("未分类语义", ExecutionMode::Guarded, false, 1, vec![]),
    };

    Some(GeJuResult {
        name: name.into(),
        execution_mode: mode,
        requires_audit,
        max_retries,
        approval_chain,
        layer: 0, // layer set by caller
    })
}

/// 三奇 as heaven stems: system-level semantic labels for meta-operations
/// (skill injection, context compaction, hook dispatch).
fn marvels_semantic(heaven: Stem, earth: Stem) -> Option<GeJuResult> {
    use Stem::*;
    let (name, mode, requires_audit, max_retries, approval_chain) = match (heaven, earth) {
        // Yi (乙·技能注入) — default earth: Ding (Dui, skill's home palace)
        (Yi, Yi) => ("乙奇自临", ExecutionMode::Direct, false, 0, vec![]),
        (Yi, Ding) => ("奇入兑宫", ExecutionMode::Direct, false, 0, vec![]),
        (Yi, Geng) => (
            "技能入刑",
            ExecutionMode::Guarded,
            true,
            1,
            vec![ApprovalGate::Permission("skill_in_exec".into())],
        ),
        (Yi, _) => ("技能注入", ExecutionMode::Direct, false, 0, vec![]),

        // Bing (丙·上下文压缩) — default earth: Xin (Xun, transform domain)
        (Bing, Bing) => ("丙奇自明", ExecutionMode::Direct, false, 0, vec![]),
        (Bing, Xin) => ("火入金宫", ExecutionMode::Direct, false, 0, vec![]),
        (Bing, Gui) => ("火入水宫", ExecutionMode::Guarded, true, 1, vec![]),
        (Bing, _) => ("上下文压缩", ExecutionMode::Direct, false, 0, vec![]),

        // Ding (丁·钩子触发)
        (Ding, Ding) => (
            "丁奇自星",
            ExecutionMode::Guarded,
            true,
            2,
            vec![ApprovalGate::UserConfirmation("钩子自触发确认".into())],
        ),
        (Ding, Geng) => (
            "星入金宫",
            ExecutionMode::Sandbox,
            true,
            2,
            vec![ApprovalGate::SandboxIsolation],
        ),
        (Ding, _) => ("钩子触发", ExecutionMode::Guarded, false, 1, vec![]),

        // Non-三奇 stems fall through to 六仪 handling
        _ => return None,
    };

    Some(GeJuResult {
        name: name.into(),
        execution_mode: mode,
        requires_audit,
        max_retries,
        approval_chain,
        layer: 0, // layer set by caller
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_patterns_are_unique() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for &h in &Stem::ALL {
            for &e in &Stem::ALL {
                if let Some(r) = named_pattern(h, e) {
                    let key = (h as u8, e as u8);
                    assert!(
                        seen.insert(key),
                        "duplicate pattern for {h:?}+{e:?} = {}",
                        r.name
                    );
                }
            }
        }
    }

    #[test]
    fn capability_matrix_covers_all_ceremony_stems() {
        for &h in &Stem::CEREMONY_STEMS {
            for &e in &Stem::ALL {
                let result = capability_semantic(h, e);
                assert!(result.is_some(), "missing semantic for {h:?}+{e:?}");
            }
        }
    }

    #[test]
    fn marvels_semantic_covers_all_marvels_stems() {
        // Yi, Bing, Ding as heaven stems should all produce results
        for &h in &[Stem::Yi, Stem::Bing, Stem::Ding] {
            for &e in &Stem::ALL {
                let result = capability_semantic(h, e);
                assert!(result.is_some(), "missing marvels semantic for {h:?}+{e:?}");
            }
        }
    }
}
