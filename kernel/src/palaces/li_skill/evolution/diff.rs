//! LCS-based diff algorithm for skill revision comparison.

/// Compute a unified-diff-style string showing changes between old and new content.
pub(crate) fn compute_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Fast path: identical content
    if old == new {
        return "(no changes)".into();
    }

    // Simple LCS-based diff: find minimal edit script and format as unified diff
    let lcs = lcs_table(&old_lines, &new_lines);
    let mut edits: Vec<(usize, usize)> = Vec::new();
    backtrack(
        &lcs,
        &old_lines,
        &new_lines,
        old_lines.len(),
        new_lines.len(),
        &mut edits,
    );

    let mut diff = String::new();
    let mut oi = 0;
    let mut ni = 0;
    for (del, add) in edits {
        // Emit deletions
        for line in old_lines.get(oi..del).into_iter().flatten() {
            diff.push_str(&format!("-{}\n", line));
        }
        // Emit additions
        for line in new_lines.get(ni..add).into_iter().flatten() {
            diff.push_str(&format!("+{}\n", line));
        }
        // Emit context (unchanged line)
        if del < old_lines.len() {
            diff.push_str(&format!(" {}\n", old_lines[del]));
        }
        oi = del + 1;
        ni = add + 1;
    }
    // Remaining deletions
    for line in old_lines.get(oi..).into_iter().flatten() {
        diff.push_str(&format!("-{}\n", line));
    }
    // Remaining additions
    for line in new_lines.get(ni..).into_iter().flatten() {
        diff.push_str(&format!("+{}\n", line));
    }
    diff
}

/// Build the LCS dynamic programming table.
pub(crate) fn lcs_table(old: &[&str], new: &[&str]) -> Vec<Vec<usize>> {
    let m = old.len();
    let n = new.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    dp
}

/// Backtrack through the LCS table to find the edit script.
pub(crate) fn backtrack(
    dp: &[Vec<usize>],
    old: &[&str],
    new: &[&str],
    i: usize,
    j: usize,
    edits: &mut Vec<(usize, usize)>,
) {
    if i == 0 && j == 0 {
        return;
    }
    if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
        backtrack(dp, old, new, i - 1, j - 1, edits);
        edits.push((i - 1, j - 1));
    } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
        backtrack(dp, old, new, i, j - 1, edits);
    } else {
        backtrack(dp, old, new, i - 1, j, edits);
    }
}
