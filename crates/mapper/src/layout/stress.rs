//! Constrained stress majorization: minimise neato's stress over graph-theoretic
//! distances, projecting onto axis separation constraints each iteration (SMACOF
//! Guttman transform + per-axis VPSC).

use super::vpsc::{self, Constraint};

/// All-pairs shortest-path hop counts over an undirected adjacency list (local
/// indices). Unreachable pairs are `f64::INFINITY` (callers pass one connected
/// component, so all finite in practice).
pub fn all_pairs_dist(n: usize, adjacency: &[Vec<usize>]) -> Vec<Vec<f64>> {
    let mut dist = vec![vec![f64::INFINITY; n]; n];
    for s in 0..n {
        let mut depth = vec![usize::MAX; n];
        let mut q = std::collections::VecDeque::new();
        depth[s] = 0;
        q.push_back(s);
        while let Some(u) = q.pop_front() {
            for &v in &adjacency[u] {
                if depth[v] == usize::MAX {
                    depth[v] = depth[u] + 1;
                    q.push_back(v);
                }
            }
        }
        for t in 0..n {
            if depth[t] != usize::MAX {
                dist[s][t] = depth[t] as f64;
            }
        }
    }
    dist
}

/// One axis of the SMACOF Guttman transform: returns `(desired, weight)` where
/// `weight_i = Σ_j w_ij` and `desired_i` is the stress-minimising target for axis
/// `a` (0 = x, 1 = y) given current positions `p`. `w_ij = 1/d_ij²`.
fn guttman_axis(p: &[(f64, f64)], dist: &[Vec<f64>], axis: usize) -> (Vec<f64>, Vec<f64>) {
    let n = p.len();
    let comp = |q: &(f64, f64)| if axis == 0 { q.0 } else { q.1 };
    let mut desired = vec![0.0; n];
    let mut weight = vec![0.0; n];
    for i in 0..n {
        let mut num = 0.0;
        let mut den = 0.0;
        for j in 0..n {
            if i == j {
                continue;
            }
            let d = dist[i][j];
            if !d.is_finite() || d == 0.0 {
                continue;
            }
            let w = 1.0 / (d * d);
            let dx = p[i].0 - p[j].0;
            let dy = p[i].1 - p[j].1;
            let norm = (dx * dx + dy * dy).sqrt();
            let target = if norm > 1e-9 {
                comp(&p[j]) + d * (comp(&p[i]) - comp(&p[j])) / norm
            } else {
                comp(&p[j])
            };
            num += w * target;
            den += w;
        }
        if den > 0.0 {
            desired[i] = num / den;
            weight[i] = den;
        } else {
            desired[i] = comp(&p[i]);
            weight[i] = 1.0;
        }
    }
    (desired, weight)
}

/// Constrained stress majorization. Seeds from `seed`, runs `iters` SMACOF
/// iterations, projecting each axis onto its separation constraints via VPSC.
/// Returns final continuous positions.
pub fn stress_layout(
    n: usize,
    dist: &[Vec<f64>],
    x_constraints: &[Constraint],
    y_constraints: &[Constraint],
    seed: &[(f64, f64)],
    iters: usize,
) -> Vec<(f64, f64)> {
    let mut p = seed.to_vec();
    if n <= 1 {
        return p;
    }
    for _ in 0..iters {
        let (dx, wx) = guttman_axis(&p, dist, 0);
        let nx = vpsc::solve_axis(&dx, &wx, x_constraints);
        for i in 0..n {
            p[i].0 = nx[i];
        }
        let (dy, wy) = guttman_axis(&p, dist, 1);
        let ny = vpsc::solve_axis(&dy, &wy, y_constraints);
        for i in 0..n {
            p[i].1 = ny[i];
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_pairs_dist_path_graph() {
        // 0 - 1 - 2 path.
        let adj = vec![vec![1], vec![0, 2], vec![1]];
        let d = all_pairs_dist(3, &adj);
        assert_eq!(d[0][2], 2.0);
        assert_eq!(d[0][1], 1.0);
        assert_eq!(d[1][2], 1.0);
        assert_eq!(d[0][0], 0.0);
    }

    #[test]
    fn east_constraint_orders_x() {
        // Two nodes, ideal distance 1, with x[1] - x[0] >= 1. Seed reversed on x.
        let dist = vec![vec![0.0, 1.0], vec![1.0, 0.0]];
        let xc = vec![Constraint { left: 0, right: 1, gap: 1.0 }];
        let yc = vec![];
        let seed = vec![(5.0, 0.0), (0.0, 0.0)]; // node 0 east of node 1 initially
        let out = stress_layout(2, &dist, &xc, &yc, &seed, 60);
        assert!(out[1].0 - out[0].0 >= 1.0 - 1e-6, "constraint x1 >= x0 + 1 must hold: {out:?}");
    }

    #[test]
    fn deterministic() {
        let dist = vec![vec![0.0, 1.0, 2.0], vec![1.0, 0.0, 1.0], vec![2.0, 1.0, 0.0]];
        let seed = vec![(0.0, 0.0), (1.0, 0.3), (2.0, -0.2)];
        let a = stress_layout(3, &dist, &[], &[], &seed, 60);
        let b = stress_layout(3, &dist, &[], &[], &seed, 60);
        assert_eq!(a, b, "same inputs must give identical output");
    }
}
