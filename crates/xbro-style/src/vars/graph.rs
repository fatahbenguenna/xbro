//! Strongly connected components of the custom property dependency graph.
//!
//! CSS Variables §2.3: an element's custom properties form a graph with an
//! edge from a property to every property its value references (fallbacks
//! included); every property that is part of a cycle is invalid at
//! computed-value time. Cycles are found with Tarjan's algorithm, written
//! iteratively so long chains of variables cannot overflow the stack.

/// The strongly connected components of the graph `adj` (node `i` has an
/// edge to each node of `adj[i]`; every index must be `< adj.len()`), in
/// reverse topological order: a component comes after every component it
/// has edges to, so dependencies are listed first.
pub(crate) fn components(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;
    let n = adj.len();
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut components = Vec::new();
    let mut next_index = 0usize;
    // Explicit call stack: (node, position of the next edge to explore).
    let mut calls: Vec<(usize, usize)> = Vec::new();
    for root in 0..n {
        if index.get(root) != Some(&UNVISITED) {
            continue;
        }
        visit(
            root,
            &mut next_index,
            &mut index,
            &mut low,
            &mut stack,
            &mut on_stack,
        );
        calls.push((root, 0));
        while let Some(top) = calls.last_mut() {
            let v = top.0;
            let edge = adj.get(v).and_then(|edges| edges.get(top.1)).copied();
            match edge {
                Some(w) if w < n => {
                    top.1 += 1;
                    if index.get(w) == Some(&UNVISITED) {
                        visit(
                            w,
                            &mut next_index,
                            &mut index,
                            &mut low,
                            &mut stack,
                            &mut on_stack,
                        );
                        calls.push((w, 0));
                    } else if on_stack.get(w) == Some(&true) {
                        let wi = index.get(w).copied().unwrap_or(UNVISITED);
                        if let Some(lv) = low.get_mut(v) {
                            *lv = (*lv).min(wi);
                        }
                    }
                }
                // Out-of-range edges (never produced by callers) are ignored.
                Some(_) => top.1 += 1,
                None => {
                    calls.pop();
                    let lv = low.get(v).copied().unwrap_or(0);
                    if let Some(&(u, _)) = calls.last() {
                        if let Some(lu) = low.get_mut(u) {
                            *lu = (*lu).min(lv);
                        }
                    }
                    if Some(&lv) == index.get(v) {
                        let mut component = Vec::new();
                        while let Some(w) = stack.pop() {
                            if let Some(flag) = on_stack.get_mut(w) {
                                *flag = false;
                            }
                            component.push(w);
                            if w == v {
                                break;
                            }
                        }
                        components.push(component);
                    }
                }
            }
        }
    }
    components
}

/// Numbers `v` and pushes it on the component stack.
fn visit(
    v: usize,
    next_index: &mut usize,
    index: &mut [usize],
    low: &mut [usize],
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
) {
    if let (Some(i), Some(l), Some(o)) = (index.get_mut(v), low.get_mut(v), on_stack.get_mut(v)) {
        *i = *next_index;
        *l = *next_index;
        *o = true;
        *next_index += 1;
        stack.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut c: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
        for comp in &mut c {
            comp.sort_unstable();
        }
        c
    }

    #[test]
    fn finds_cycles_and_orders_dependencies_first() {
        // 0 → 1 → 2 → 0 (cycle), 3 → 0, 4 alone, 5 → 5 (self loop), 6 → 3.
        let adj = vec![vec![1], vec![2], vec![0], vec![0], vec![], vec![5], vec![3]];
        let comps = sorted(components(&adj));
        assert!(comps.contains(&vec![0, 1, 2]));
        assert!(comps.contains(&vec![3]));
        assert!(comps.contains(&vec![4]));
        assert!(comps.contains(&vec![5]));
        let pos = |n: usize| comps.iter().position(|c| c.contains(&n)).unwrap();
        // Dependencies come first.
        assert!(pos(0) < pos(3));
        assert!(pos(3) < pos(6));
        assert_eq!(comps.iter().map(Vec::len).sum::<usize>(), adj.len());
    }

    #[test]
    fn result_does_not_depend_on_node_order() {
        // The cycle 0 → 1 → 2 → 3 → 0 with a chord 1 → 0: one component,
        // whatever node the search starts from (relabelled variants).
        for shift in 0..4 {
            let relabel = |i: usize| (i + shift) % 4;
            let mut adj = vec![Vec::new(); 4];
            for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (1, 0)] {
                adj[relabel(a)].push(relabel(b));
            }
            assert_eq!(sorted(components(&adj)), vec![vec![0, 1, 2, 3]]);
        }
    }

    #[test]
    fn long_chains_do_not_recurse() {
        let n = 200_000;
        let adj: Vec<Vec<usize>> = (0..n)
            .map(|i| if i + 1 < n { vec![i + 1] } else { vec![] })
            .collect();
        let comps = components(&adj);
        assert_eq!(comps.len(), n);
        // The end of the chain (the deepest dependency) comes first.
        assert_eq!(comps.first(), Some(&vec![n - 1]));
        let mut cyclic = adj.clone();
        cyclic[n - 1].push(0);
        assert_eq!(components(&cyclic).len(), 1);
        assert_eq!(
            components(&[vec![7]]).len(),
            1,
            "out-of-range edges are ignored"
        );
    }
}
