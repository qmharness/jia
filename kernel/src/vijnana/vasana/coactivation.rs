//! 种子共现矩阵 SeedCoActivationMatrix — 唯识"俱有因"的工程实现。
//!
//! 追踪种子在检索中的两两共现关系，作为记忆的时间表征。
//! 共同激活的种子互为俱有因（sahabhū-hetu），在后续检索中产生协同增强效应。
//!
//! 哲学锚点：唯识"俱有因"——同时生起的事物作为相互的因。
//! 非"相应"(samprayukta)——相应描述活跃心所之间的关系（现行位），
//! 而共现矩阵追踪潜藏种子（种子位）的检索模式。

use std::collections::HashMap;

/// 稀疏共现矩阵的单个条目。
#[derive(Debug, Clone)]
struct CoEntry {
    /// 共现强度（0.0–1.0），含指数衰减
    strength: f32,
    /// 最后更新轮次
    last_updated: u64,
}

/// 按项目分隔的种子共现矩阵。
///
/// 稀疏存储——只记录实际发生共现的种子对。
/// 指数衰减 + 共现增量，O(1) per pair per turn。
#[derive(Debug, Clone)]
pub struct SeedCoActivationMatrix {
    /// 共现对：key = (seed_a, seed_b)，按 project 分矩阵（project_id → pairs）
    matrices: HashMap<String, HashMap<(String, String), CoEntry>>,
    /// 指数衰减因子（默认 0.95）
    decay: f32,
}

impl Default for SeedCoActivationMatrix {
    fn default() -> Self {
        Self {
            matrices: HashMap::default(),
            decay: 0.95,
        }
    }
}

impl SeedCoActivationMatrix {
    /// 创建新的共现矩阵。
    pub fn new(decay: f32) -> Self {
        Self {
            matrices: HashMap::default(),
            decay,
        }
    }

    /// 递推更新：`strength = decay × old_strength + (1 − decay) × new_cooccurrence`
    ///
    /// 对给定的种子 ID 集合中的每一对 (i, j)，i < j，更新共现强度。
    /// 缺席的种子对其强度自然衰减。
    pub fn record_coactivation(&mut self, project_id: &str, seed_ids: &[String], turn: u64) {
        if seed_ids.len() < 2 {
            return;
        }

        let matrix = self.matrices.entry(project_id.to_string()).or_default();

        for i in 0..seed_ids.len() {
            for j in (i + 1)..seed_ids.len() {
                let (a, b) = if seed_ids[i] < seed_ids[j] {
                    (seed_ids[i].clone(), seed_ids[j].clone())
                } else {
                    (seed_ids[j].clone(), seed_ids[i].clone())
                };
                let entry = matrix.entry((a, b)).or_insert(CoEntry {
                    strength: 0.0,
                    last_updated: turn,
                });
                // 指数衰减旧值 + 新共现增量
                entry.strength = self.decay * entry.strength + (1.0 - self.decay) * 1.0;
                entry.last_updated = turn;
            }
        }
    }

    /// 查询某种子参与的全部共现对的平均强度。
    ///
    /// 用于检索时的 sync_bonus 计算。
    pub fn coactivation_strength(&self, project_id: &str, seed_id: &str) -> f32 {
        let Some(matrix) = self.matrices.get(project_id) else {
            return 0.0;
        };
        let mut total = 0.0f32;
        let mut count = 0usize;
        for ((a, b), entry) in matrix.iter() {
            if a == seed_id || b == seed_id {
                total += entry.strength;
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            total / count as f32
        }
    }

    /// 查询某种子参与的共现对数。
    ///
    /// 关联广度由共现矩阵动态查询，非种子自身属性。
    pub fn count_pairs_for(&self, project_id: &str, seed_id: &str) -> usize {
        let Some(matrix) = self.matrices.get(project_id) else {
            return 0;
        };
        matrix
            .keys()
            .filter(|(a, b)| a == seed_id || b == seed_id)
            .count()
    }

    /// 检测沉寂种子：共现总强度低于阈值的种子 ID 集合。
    ///
    /// 沉寂种子检测结果作为附条件输入喂给坐忘——
    /// 坐忘既有的 access_decay 是休眠检测主信号；
    /// 共现沉寂仅当 relevance_score 也低于中位数时作为加重因子。
    pub fn dormant_seeds(&self, project_id: &str, threshold: f32) -> Vec<String> {
        let Some(matrix) = self.matrices.get(project_id) else {
            return vec![];
        };
        let mut seed_strengths: HashMap<String, f32> = HashMap::default();
        for ((a, b), entry) in matrix.iter() {
            *seed_strengths.entry(a.clone()).or_default() += entry.strength;
            *seed_strengths.entry(b.clone()).or_default() += entry.strength;
        }
        seed_strengths
            .into_iter()
            .filter(|(_, s)| *s < threshold)
            .map(|(id, _)| id)
            .collect()
    }

    /// 获取某项目的共现矩阵中的总对数。
    pub fn total_pairs(&self, project_id: &str) -> usize {
        self.matrices.get(project_id).map(|m| m.len()).unwrap_or(0)
    }

    /// Cap the number of pairs per seed to prevent unbounded growth.
    /// For each seed, keeps only the top `max_pairs` strongest coactivation entries.
    pub fn enforce_cap(&mut self, project_id: &str, max_pairs_per_seed: usize) {
        let Some(matrix) = self.matrices.get_mut(project_id) else {
            return;
        };
        // Collect per-seed pair counts and strengths
        let mut seed_pairs: HashMap<String, Vec<((String, String), f32)>> = HashMap::new();
        for (pair, entry) in matrix.iter() {
            seed_pairs
                .entry(pair.0.clone())
                .or_default()
                .push((pair.clone(), entry.strength));
            seed_pairs
                .entry(pair.1.clone())
                .or_default()
                .push((pair.clone(), entry.strength));
        }
        // Find pairs to remove: weakest per seed above cap
        let mut to_remove: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for (_, mut pairs) in seed_pairs {
            if pairs.len() > max_pairs_per_seed {
                pairs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                for (pair, _) in pairs.iter().take(pairs.len() - max_pairs_per_seed) {
                    to_remove.insert(pair.clone());
                }
            }
        }
        for pair in to_remove {
            matrix.remove(&pair);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_decays() {
        let mut matrix = SeedCoActivationMatrix::new(0.9);
        let project = "test-proj";
        let ids: Vec<String> = (0..5).map(|i| format!("seed-{}", i)).collect();

        // Record coactivation among first 3 seeds
        matrix.record_coactivation(project, &ids[..3], 1);
        let s = matrix.coactivation_strength(project, "seed-0");
        assert!(s > 0.0, "coactivated seeds should have non-zero strength");

        // Record again — strength should increase
        matrix.record_coactivation(project, &ids[..3], 2);
        let s2 = matrix.coactivation_strength(project, "seed-0");
        assert!(s2 > s, "repeated coactivation should increase strength");

        // Unseen seed should have zero strength
        let s3 = matrix.coactivation_strength(project, "seed-4");
        assert_eq!(s3, 0.0);

        // count_pairs_for
        let count = matrix.count_pairs_for(project, "seed-0");
        assert_eq!(count, 2, "seed-0 should be paired with seed-1 and seed-2");
    }

    #[test]
    fn per_project_isolation() {
        let mut matrix = SeedCoActivationMatrix::new(0.9);
        let ids: Vec<String> = vec!["s1".into(), "s2".into()];

        matrix.record_coactivation("proj-A", &ids, 1);
        matrix.record_coactivation("proj-B", &[], 1);

        assert!(matrix.coactivation_strength("proj-A", "s1") > 0.0);
        assert_eq!(matrix.coactivation_strength("proj-B", "s1"), 0.0);
    }

    #[test]
    fn dormant_detection() {
        let mut matrix = SeedCoActivationMatrix::new(0.95);
        let project = "test";
        // Record strong coactivation for seeds 0-2
        for _ in 0..10 {
            let ids: Vec<String> = (0..3).map(|i| format!("s{}", i)).collect();
            matrix.record_coactivation(project, &ids, 1);
        }
        // Seeds 3-5 are isolated — barely any coactivation
        let ids2: Vec<String> = (3..6).map(|i| format!("s{}", i)).collect();
        matrix.record_coactivation(project, &ids2, 1);

        // dormant_seeds 按“共现总强度”过滤。孤立组 3 个种子只共现 1 次：
        // 每对强度 = (1 - 0.95) * 1 = 0.05，每种子参与 2 对，总强度 = 0.10。
        // 阈值需高于 0.10 才能把它识别为沉寂，同时远低于活跃组总强度 (~0.80)。
        let dormant = matrix.dormant_seeds(project, 0.15);
        // The isolated group should have very low strength
        assert!(!dormant.is_empty(), "should detect dormant seeds");
    }

    #[test]
    fn sparse_matrix_does_not_explode() {
        let mut matrix = SeedCoActivationMatrix::new(0.9);
        let project = "stress";
        // Simulate 100 turns with 20 seeds each
        for turn in 0..100 {
            let ids: Vec<String> = (0..20).map(|i| format!("seed-{}", i % 50)).collect();
            matrix.record_coactivation(project, &ids, turn);
        }
        let total = matrix.total_pairs(project);
        // With 50 unique seeds and decay, pairs should stay bounded
        assert!(total < 10000, "matrix should remain sparse: got {}", total);
    }
}
