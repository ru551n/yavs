//! Element sets, and the element-level dependence graph of the processes that would join the
//! region: the proof that their combinational logic has no loop, the loop groups of processes
//! (a process reading, through others or itself, what it drives without an element loop) and the
//! static number of passes each group needs.
//!
//! Nodes are atoms: runs of elements of one signal leaf, split at every boundary a read or a
//! write names. A process's dependence of a driven run on a set of elements is a hub node (one
//! per distinct set and process): set -> hub -> driven atoms. A driven run that copies a run of
//! the same length element by element (`a <= b`) is direct edges between the matching atoms.
//! Every edge over-approximates the true dependence, so an acyclic graph proves the true one
//! acyclic.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

/// The key bit of a variable leaf (process-local numbering); signal leaves are their `LeafId`.
pub(crate) const VAR: u64 = 1 << 32;

/// Elements as sorted, disjoint, non-adjacent half-open runs per key (a signal leaf, or a
/// variable leaf with [`VAR`] set). Never one entry per element: a 16 K-element RAM is one run.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct ESet(BTreeMap<u64, Vec<(u32, u32)>>);

impl ESet {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn add(&mut self, k: u64, lo: u32, hi: u32) {
        if lo >= hi {
            return;
        }
        let v = self.0.entry(k).or_default();
        let i = v.partition_point(|&(_, h)| h < lo);
        let (mut a, mut z, mut j) = (lo, hi, i);
        while j < v.len() && v[j].0 <= z {
            a = a.min(v[j].0);
            z = z.max(v[j].1);
            j += 1;
        }
        v.splice(i..j, [(a, z)]);
    }

    pub fn union(&mut self, o: &ESet) {
        for (k, a, z) in o.iter() {
            self.add(k, a, z);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, u32, u32)> + '_ {
        (self.0.iter()).flat_map(|(&k, v)| v.iter().map(move |&(a, z)| (k, a, z)))
    }

    /// The first element of `lo .. hi` of `k` not in the set.
    pub fn first_missing(&self, k: u64, lo: u32, hi: u32) -> Option<u32> {
        let mut at = lo;
        for &(a, z) in self.0.get(&k).into_iter().flatten() {
            if at >= hi {
                break;
            }
            if a > at {
                return Some(at);
            }
            at = at.max(z);
        }
        (at < hi).then_some(at)
    }

    pub fn intersect(&self, o: &ESet) -> ESet {
        let mut out = ESet::default();
        for (k, v) in &self.0 {
            let Some(w) = o.0.get(k) else { continue };
            let (mut i, mut j) = (0, 0);
            while i < v.len() && j < w.len() {
                let (a, z) = (v[i].0.max(w[j].0), v[i].1.min(w[j].1));
                if a < z {
                    out.add(*k, a, z);
                }
                if v[i].1 < w[j].1 {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
        out
    }

    /// The elements of `self` not in `o`.
    pub fn minus(&self, o: &ESet) -> ESet {
        let mut out = ESet::default();
        for (k, a, z) in self.iter() {
            let mut at = a;
            for &(b, y) in o.0.get(&k).into_iter().flatten() {
                if y <= at || b >= z {
                    continue;
                }
                if b > at {
                    out.add(k, at, b);
                }
                at = at.max(y);
            }
            out.add(k, at, z);
        }
        out
    }
}

/// What a run of elements depends on: every element of `set`, and, with `ident` `(k, b)`,
/// element `b + j` of key `k` for its own element `j` (a copy element by element).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dep {
    pub ident: Option<(u64, u32)>,
    pub set: Rc<ESet>,
}

impl Dep {
    pub fn set(s: Rc<ESet>) -> Dep {
        Dep {
            ident: None,
            set: s,
        }
    }
}

/// A driven run `lo .. hi` of signal leaf `leaf` of one process and what it depends on.
#[derive(Clone, Debug)]
pub(crate) struct Out {
    pub leaf: u64,
    pub lo: u32,
    pub hi: u32,
    pub dep: Dep,
}

/// A loop group: processes that read, through each other or themselves, what they drive.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Group {
    /// In the order a pass runs them.
    pub procs: Vec<usize>,
    /// Passes in that order that reach the fixed point.
    pub k: u32,
    /// The pieces the group's processes split into, one per process and pass level, so that
    /// one ordered pass suffices.
    pub slices: usize,
    /// Each process of the group with each pass level of the elements it drives, by level and
    /// then in the order of a pass: running each process whole at those points reaches the fixed
    /// point, its last run on settled values only.
    pub runs: Vec<(usize, u32)>,
}

pub(crate) struct Analysis {
    /// Each element loop: the processes in it and one loop printed.
    pub cycles: Vec<(Vec<usize>, String)>,
    pub groups: Vec<Group>,
    /// Every process in an order that runs each after the processes it reads from, groups
    /// contiguous (empty while there are cycles).
    pub order: Vec<usize>,
}

/// Atoms: per key the sorted boundaries, and the index of its first atom.
struct Atoms {
    keys: BTreeMap<u64, (Vec<u32>, u32)>,
    owner: Vec<(u64, u32, u32)>,
}

impl Atoms {
    fn of(&self, k: u64, lo: u32, hi: u32) -> std::ops::Range<u32> {
        let Some((b, first)) = self.keys.get(&k) else {
            return 0..0;
        };
        // Atom i is b[i] .. b[i + 1]; those overlapping lo .. hi.
        let a = b.partition_point(|&x| x <= lo).saturating_sub(1);
        let z = b.partition_point(|&x| x < hi).saturating_sub(1);
        if lo >= hi || a >= b.len() - 1 {
            return 0..0;
        }
        first + a as u32..first + (z.min(b.len() - 2) + 1) as u32
    }
}

/// Analyse the dependences of the processes in `procs` (process index, its driven runs).
/// `atom` names a run of elements, `pname` a process.
pub(crate) fn analyse(
    procs: &[(usize, Vec<Out>)],
    atom: &dyn Fn(u64, u32, u32) -> String,
    pname: &dyn Fn(usize) -> String,
) -> Analysis {
    // Boundaries.
    let mut bounds: BTreeMap<u64, BTreeSet<u32>> = BTreeMap::new();
    let mut mark = |k: u64, a: u32, z: u32| {
        let s = bounds.entry(k).or_default();
        s.insert(a);
        s.insert(z);
    };
    for (_, outs) in procs {
        for o in outs {
            mark(o.leaf, o.lo, o.hi);
            if let Some((k, b)) = o.dep.ident {
                mark(k, b, b + (o.hi - o.lo));
            }
            for (k, a, z) in o.dep.set.iter() {
                mark(k, a, z);
            }
        }
    }
    // A copy element by element needs the same boundaries on both sides, or its atoms would
    // join elements a shift keeps apart (`s(7 downto 1) <= s(6 downto 0)`).
    let copies: Vec<(u64, u32, u32, u64, u32)> = (procs.iter())
        .flat_map(|(_, outs)| outs.iter())
        .filter_map(|o| o.dep.ident.map(|(k, b)| (o.leaf, o.lo, o.hi, k, b)))
        .collect();
    let mut budget = 1usize << 22;
    loop {
        let mut added = 0usize;
        for &(d, lo, hi, s, b) in &copies {
            let len = hi - lo;
            let dst: Vec<u32> = bounds[&d].range(lo + 1..hi).map(|x| x - lo).collect();
            let src: Vec<u32> = bounds[&s].range(b + 1..b + len).map(|x| x - b).collect();
            for x in src {
                added += usize::from(bounds.get_mut(&d).expect("marked").insert(lo + x));
            }
            for x in dst {
                added += usize::from(bounds.get_mut(&s).expect("marked").insert(b + x));
            }
        }
        // ponytail: past the budget atoms stay coarser (more edges, never fewer).
        budget = budget.saturating_sub(added);
        if added == 0 || budget == 0 {
            break;
        }
    }
    let mut atoms = Atoms {
        keys: BTreeMap::new(),
        owner: Vec::new(),
    };
    for (k, s) in bounds {
        let b: Vec<u32> = s.into_iter().collect();
        let first = atoms.owner.len() as u32;
        for w in b.windows(2) {
            atoms.owner.push((k, w[0], w[1]));
        }
        atoms.keys.insert(k, (b, first));
    }
    let na = atoms.owner.len();
    // Nodes: atoms, then hubs.
    let mut succ: Vec<Vec<u32>> = vec![Vec::new(); na];
    let mut hub_of: Vec<usize> = Vec::new();
    let mut writer: Vec<Option<usize>> = vec![None; na];
    let mut hubs: HashMap<(usize, *const ESet), u32> = HashMap::new();
    for (pi, outs) in procs {
        for o in outs {
            let dst = atoms.of(o.leaf, o.lo, o.hi);
            for d in dst.clone() {
                writer[d as usize].get_or_insert(*pi);
            }
            if !o.dep.set.is_empty() {
                let key = (*pi, Rc::as_ptr(&o.dep.set));
                let h = *hubs.entry(key).or_insert_with(|| {
                    let h = (na + hub_of.len()) as u32;
                    hub_of.push(*pi);
                    succ.push(Vec::new());
                    for (k, a, z) in o.dep.set.iter() {
                        for s in atoms.of(k, a, z) {
                            succ[s as usize].push(h);
                        }
                    }
                    h
                });
                succ[h as usize].extend(dst.clone());
            }
            if let Some((k, b)) = o.dep.ident {
                for d in dst {
                    let (_, x, y) = atoms.owner[d as usize];
                    let (x, y) = (x.max(o.lo), y.min(o.hi));
                    for s in atoms.of(k, b + (x - o.lo), b + (y - o.lo)) {
                        succ[s as usize].push(d);
                    }
                }
            }
        }
    }
    for s in &mut succ {
        s.sort_unstable();
        s.dedup();
    }
    let owner_of = |n: u32| -> Option<usize> {
        let n = n as usize;
        if n >= na {
            Some(hub_of[n - na])
        } else {
            writer[n]
        }
    };
    let sccs = tarjan(&succ);
    // Element loops.
    let mut cycles = Vec::new();
    for c in &sccs {
        let looped = c.len() > 1 || succ[c[0] as usize].contains(&c[0]);
        if !looped {
            continue;
        }
        let mut ps: Vec<usize> = c.iter().filter_map(|&n| owner_of(n)).collect();
        ps.sort_unstable();
        ps.dedup();
        let path = loop_path(c, &succ);
        let mut text = String::new();
        for (i, &n) in path.iter().enumerate() {
            if (n as usize) < na {
                // An atom reached from an atom: the copy of the process that writes it.
                if i > 0 && (path[i - 1] as usize) < na {
                    let w = writer[n as usize].map_or_else(|| "?".into(), pname);
                    text.push_str(&format!(" -> {w}"));
                }
                if i > 0 {
                    text.push_str(" -> ");
                }
                let (k, a, z) = atoms.owner[n as usize];
                text.push_str(&atom(k, a, z));
            } else {
                text.push_str(&format!(" -> {}", pname(hub_of[n as usize - na])));
            }
        }
        cycles.push((ps, text));
    }
    if !cycles.is_empty() {
        return Analysis {
            cycles,
            groups: Vec::new(),
            order: Vec::new(),
        };
    }
    // Topological order of the nodes: Tarjan emits a component after everything it reaches.
    let n = succ.len();
    let mut topo = vec![0u32; n];
    for (i, c) in sccs.iter().rev().enumerate() {
        topo[c[0] as usize] = i as u32;
    }
    // Process graph: q -> p when p reads what q writes.
    let pidx: BTreeMap<usize, usize> = procs.iter().enumerate().map(|(i, p)| (p.0, i)).collect();
    let mut psucc: Vec<Vec<u32>> = vec![Vec::new(); procs.len()];
    for (u, ss) in succ.iter().enumerate() {
        let Some(q) = owner_of(u as u32).filter(|_| u < na) else {
            continue;
        };
        for &v in ss {
            if let Some(p) = owner_of(v) {
                psucc[pidx[&q]].push(pidx[&p] as u32);
            }
        }
    }
    for s in &mut psucc {
        s.sort_unstable();
        s.dedup();
    }
    let pscc = tarjan(&psucc);
    // The earliest output of each process in the element order.
    let mut first = vec![u32::MAX; procs.len()];
    for (a, w) in writer.iter().enumerate() {
        if let Some(p) = w {
            let i = pidx[p];
            first[i] = first[i].min(topo[a]);
        }
    }
    let mut groups = Vec::new();
    let mut order = Vec::new();
    // Rank of each process inside its group, and its group.
    let mut rank = vec![0u32; procs.len()];
    let mut group = vec![usize::MAX; procs.len()];
    for c in pscc.iter().rev() {
        let mut c: Vec<u32> = c.clone();
        c.sort_by_key(|&i| (first[i as usize], i));
        let looped = c.len() > 1 || psucc[c[0] as usize].contains(&c[0]);
        for (r, &i) in c.iter().enumerate() {
            rank[i as usize] = r as u32;
            if looped {
                group[i as usize] = groups.len();
            }
            order.push(procs[i as usize].0);
        }
        if looped {
            groups.push(Group {
                procs: c.iter().map(|&i| procs[i as usize].0).collect(),
                k: 1,
                slices: 0,
                runs: Vec::new(),
            });
        }
    }
    // Pass levels: b(v) = max over predecessors u written in v's group of b(u), plus one when
    // u's process does not run strictly before v's.
    let mut pred: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (u, ss) in succ.iter().enumerate() {
        for &v in ss {
            pred[v as usize].push(u as u32);
        }
    }
    let mut by_topo: Vec<u32> = (0..na as u32).collect();
    by_topo.sort_by_key(|&a| topo[a as usize]);
    let mut b = vec![0u32; na];
    let mut levels: Vec<BTreeSet<(usize, u32)>> = vec![BTreeSet::new(); groups.len()];
    for &v in &by_topo {
        let Some(p) = writer[v as usize] else {
            continue;
        };
        let pi = pidx[&p];
        let g = group[pi];
        if g == usize::MAX {
            continue;
        }
        let mut best = 0;
        let mut look = |u: u32, b: &[u32]| {
            if let Some(q) = writer[u as usize]
                && group[pidx[&q]] == g
            {
                let bump = u32::from(rank[pidx[&q]] >= rank[pi]);
                best = best.max(b[u as usize] + bump);
            }
        };
        for &u in &pred[v as usize] {
            if (u as usize) < na {
                look(u, &b);
            } else {
                for &w in &pred[u as usize] {
                    look(w, &b);
                }
            }
        }
        b[v as usize] = best;
        groups[g].k = groups[g].k.max(best + 1);
        levels[g].insert((p, best));
    }
    for (g, l) in groups.iter_mut().zip(levels) {
        g.slices = l.len();
        g.runs = l.into_iter().collect();
        g.runs.sort_by_key(|&(p, b)| (b, rank[pidx[&p]]));
    }
    Analysis {
        cycles,
        groups,
        order,
    }
}

/// Strongly connected components, each emitted after every component it reaches (iterative).
fn tarjan(succ: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let n = succ.len();
    let mut index = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut on = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    let mut next = 0u32;
    for root in 0..n as u32 {
        if index[root as usize] != u32::MAX {
            continue;
        }
        let mut work: Vec<(u32, usize)> = vec![(root, 0)];
        while let Some(&mut (v, ref mut i)) = work.last_mut() {
            let vu = v as usize;
            if *i == 0 && index[vu] == u32::MAX {
                index[vu] = next;
                low[vu] = next;
                next += 1;
                stack.push(v);
                on[vu] = true;
            }
            if let Some(&w) = succ[vu].get(*i) {
                *i += 1;
                let wu = w as usize;
                if index[wu] == u32::MAX {
                    work.push((w, 0));
                } else if on[wu] {
                    low[vu] = low[vu].min(index[wu]);
                }
                continue;
            }
            work.pop();
            if let Some(&(p, _)) = work.last() {
                low[p as usize] = low[p as usize].min(low[vu]);
            }
            if low[vu] == index[vu] {
                let mut c = Vec::new();
                while let Some(w) = stack.pop() {
                    on[w as usize] = false;
                    c.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(c);
            }
        }
    }
    out
}

/// One loop inside component `c`, from its first atom back to it (inclusive at both ends).
fn loop_path(c: &[u32], succ: &[Vec<u32>]) -> Vec<u32> {
    let inside: BTreeSet<u32> = c.iter().copied().collect();
    let start = *c.iter().min().expect("a component is not empty");
    // Breadth first from the start's successors back to the start.
    let mut prev: HashMap<u32, u32> = HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    for &w in &succ[start as usize] {
        if inside.contains(&w) && !prev.contains_key(&w) {
            prev.insert(w, start);
            queue.push_back(w);
        }
    }
    while let Some(v) = queue.pop_front() {
        if v == start {
            break;
        }
        for &w in &succ[v as usize] {
            if inside.contains(&w) && !prev.contains_key(&w) {
                prev.insert(w, v);
                queue.push_back(w);
            }
        }
    }
    let mut path = vec![start];
    let mut at = start;
    while let Some(&p) = prev.get(&at) {
        path.push(p);
        if p == start {
            break;
        }
        at = p;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(rs: &[(u64, u32, u32)]) -> Rc<ESet> {
        let mut s = ESet::default();
        for &(k, a, z) in rs {
            s.add(k, a, z);
        }
        Rc::new(s)
    }

    fn out(leaf: u64, lo: u32, hi: u32, deps: &[(u64, u32, u32)]) -> Out {
        Out {
            leaf,
            lo,
            hi,
            dep: Dep::set(set(deps)),
        }
    }

    fn run(procs: &[(usize, Vec<Out>)]) -> Analysis {
        analyse(procs, &|k, a, z| format!("s{k}({a}..{z})"), &|p| {
            format!("p{p}")
        })
    }

    #[test]
    fn sets_merge_intersect_and_subtract() {
        let mut s = ESet::default();
        s.add(1, 0, 4);
        s.add(1, 8, 10);
        s.add(1, 4, 6);
        assert_eq!(s.iter().collect::<Vec<_>>(), [(1, 0, 6), (1, 8, 10)]);
        assert_eq!(s.first_missing(1, 0, 10), Some(6));
        assert_eq!(s.first_missing(1, 2, 5), None);
        let t = set(&[(1, 5, 9)]);
        assert_eq!(
            s.intersect(&t).iter().collect::<Vec<_>>(),
            [(1, 5, 6), (1, 8, 9)]
        );
        assert_eq!(
            s.minus(&t).iter().collect::<Vec<_>>(),
            [(1, 0, 5), (1, 9, 10)]
        );
    }

    /// `popcnt_a`: levels of one signal array feeding the next, in one process: no element
    /// loop, a self-reading group of four passes.
    #[test]
    fn a_process_reading_its_own_levels_needs_one_pass_per_level() {
        // Leaf 1 = pc2 (4 elements), 2 = pc4 (2), 3 = pc8 (1); leaf 0 = the input.
        let p = vec![
            out(1, 0, 2, &[(0, 0, 2)]),
            out(1, 2, 4, &[(0, 2, 4)]),
            out(2, 0, 1, &[(1, 0, 2)]),
            out(2, 1, 2, &[(1, 2, 4)]),
            out(3, 0, 1, &[(2, 0, 2)]),
        ];
        let a = run(&[(7, p)]);
        assert!(a.cycles.is_empty());
        assert_eq!(a.groups.len(), 1);
        assert_eq!(a.groups[0].k, 3);
        assert_eq!(a.groups[0].slices, 3);
        assert_eq!(a.groups[0].runs, [(7, 0), (7, 1), (7, 2)]);
    }

    #[test]
    fn a_true_loop_is_named_with_its_processes() {
        // p1: b <= a; p2: a <= b.
        let a = run(&[
            (1, vec![out(2, 0, 1, &[(1, 0, 1)])]),
            (2, vec![out(1, 0, 1, &[(2, 0, 1)])]),
        ]);
        assert_eq!(a.cycles.len(), 1);
        assert_eq!(a.cycles[0].0, [1, 2]);
        assert_eq!(
            a.cycles[0].1,
            "s1(0..1) -> p1 -> s2(0..1) -> p2 -> s1(0..1)"
        );
    }

    #[test]
    fn processes_reading_each_others_other_outputs_form_a_group() {
        // p1: x <= qa; p2: qa <= in, qb <= x. No element loop; p2 before p1 still leaves qb
        // one pass behind.
        let a = run(&[
            (1, vec![out(10, 0, 1, &[(11, 0, 1)])]),
            (
                2,
                vec![out(11, 0, 1, &[(0, 0, 1)]), out(12, 0, 1, &[(10, 0, 1)])],
            ),
        ]);
        assert!(a.cycles.is_empty());
        assert_eq!(a.groups.len(), 1);
        assert_eq!(a.groups[0].k, 2);
        // p2 (qa first in the order) runs before p1, then again for qb.
        assert_eq!(a.groups[0].runs, [(2, 0), (1, 0), (2, 1)]);
        // A chain without a loop needs no group, and orders the reader after the writer.
        let a = run(&[
            (1, vec![out(10, 0, 1, &[(11, 0, 1)])]),
            (2, vec![out(11, 0, 1, &[(0, 0, 1)])]),
        ]);
        assert!(a.groups.is_empty());
        assert_eq!(a.order, [2, 1]);
    }

    #[test]
    fn a_copy_element_by_element_is_no_loop() {
        // s(7 downto 1) <= s(6 downto 0): a shift inside one signal.
        let o = Out {
            leaf: 1,
            lo: 1,
            hi: 8,
            dep: Dep {
                ident: Some((1, 0)),
                set: Rc::new(ESet::default()),
            },
        };
        let a = run(&[(1, vec![o, out(1, 0, 1, &[(0, 0, 1)])])]);
        assert!(a.cycles.is_empty());
        assert_eq!(a.groups[0].k, 8);
    }
}
