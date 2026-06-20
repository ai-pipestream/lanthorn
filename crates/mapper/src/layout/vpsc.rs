//! Variable Placement with Separation Constraints (VPSC): 1-D projection.
//!
//! Solves, for one axis: minimise `Σ weight_i·(x_i − desired_i)²`
//! subject to `x[c.right] − x[c.left] ≥ c.gap` for every constraint `c`.
//!
//! Implementation: Dwyer's block-merge "satisfy" algorithm. Variables start in
//! singleton blocks; the most-violated cross-block constraint is repeatedly made
//! active by merging its two blocks (which fixes their relative offset and moves
//! the merged block to its weight-optimal position) until no constraint is
//! violated. The optional split-for-optimality pass is omitted: the result is
//! always feasible, and the outer stress loop re-projects each iteration.

/// `x[right] − x[left] ≥ gap`. `left`/`right` are variable indices.
#[derive(Debug, Clone, Copy)]
pub struct Constraint {
    pub left: usize,
    pub right: usize,
    pub gap: f64,
}

const TOL: f64 = 1e-9;

/// Project `desired` onto the feasible region of `constraints` (closest feasible
/// point under the weighted L2 objective). Returns one position per variable.
pub fn solve_axis(desired: &[f64], weight: &[f64], constraints: &[Constraint]) -> Vec<f64> {
    let n = desired.len();
    if n == 0 {
        return Vec::new();
    }
    // Block of each variable, and the variable's fixed offset within its block.
    let mut block: Vec<usize> = (0..n).collect();
    let mut offset: Vec<f64> = vec![0.0; n];
    // Per block: member variable indices, total weight, position.
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut bweight: Vec<f64> = weight.to_vec();
    let mut bpos: Vec<f64> = desired.to_vec();

    loop {
        // Find the most-violated constraint whose endpoints are in different blocks.
        let mut worst: Option<usize> = None;
        let mut worst_v = TOL;
        for (ci, c) in constraints.iter().enumerate() {
            if block[c.left] == block[c.right] {
                continue;
            }
            let pl = bpos[block[c.left]] + offset[c.left];
            let pr = bpos[block[c.right]] + offset[c.right];
            let v = c.gap - (pr - pl);
            if v > worst_v {
                worst_v = v;
                worst = Some(ci);
            }
        }
        let Some(ci) = worst else { break };
        let c = &constraints[ci];
        let bl = block[c.left];
        let br = block[c.right];

        // Merge br into bl, keeping bl's frame and making this constraint active:
        // after the merge, offset[right] - offset[left] == gap exactly.
        let shift = (offset[c.left] + c.gap) - offset[c.right];
        let moved: Vec<usize> = std::mem::take(&mut members[br]);
        for &v in &moved {
            offset[v] += shift;
            block[v] = bl;
        }
        members[bl].extend(moved);
        bweight[bl] += bweight[br];
        bweight[br] = 0.0;

        // Re-derive the merged block's weight-optimal position.
        let mut num = 0.0;
        for &v in &members[bl] {
            num += weight[v] * (desired[v] - offset[v]);
        }
        bpos[bl] = num / bweight[bl];
    }

    (0..n).map(|i| bpos[block[i]] + offset[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-6, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn feasible_input_unchanged() {
        // Already satisfies x1 - x0 >= 1; projection must not move anything.
        let out = solve_axis(&[0.0, 5.0], &[1.0, 1.0], &[Constraint { left: 0, right: 1, gap: 1.0 }]);
        approx(&out, &[0.0, 5.0]);
    }

    #[test]
    fn single_constraint_pushes_to_gap() {
        // desired both 0, equal weight, need x1 - x0 >= 1 → symmetric split to -0.5, 0.5.
        let out = solve_axis(&[0.0, 0.0], &[1.0, 1.0], &[Constraint { left: 0, right: 1, gap: 1.0 }]);
        approx(&out, &[-0.5, 0.5]);
    }

    #[test]
    fn chain_of_three() {
        // desired all 0, gaps 1 → -1, 0, 1.
        let cs = [
            Constraint { left: 0, right: 1, gap: 1.0 },
            Constraint { left: 1, right: 2, gap: 1.0 },
        ];
        let out = solve_axis(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0], &cs);
        approx(&out, &[-1.0, 0.0, 1.0]);
    }

    #[test]
    fn weight_biases_merged_position() {
        // x0 desired 0 (weight 3), x1 desired 0 (weight 1), gap 1.
        // Merged block position = (3*(0-0) + 1*(0-1))/4 = -0.25; x0=-0.25, x1=0.75.
        let out = solve_axis(&[0.0, 0.0], &[3.0, 1.0], &[Constraint { left: 0, right: 1, gap: 1.0 }]);
        approx(&out, &[-0.25, 0.75]);
    }
}
