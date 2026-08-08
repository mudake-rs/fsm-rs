//! Static view over the state hierarchy: parent links, leaf enumeration,
//! initial-leaf resolution, and LCA-based entry/exit path computation.

use std::collections::HashMap;

use crate::model::StateDef;

pub struct Tree<'a> {
    /// Leaf states in pre-order — these form the runtime `State` enum.
    pub leaves: Vec<&'a StateDef>,
    /// All states (leaves and composites) in pre-order.
    pub all: Vec<&'a StateDef>,
    by_name: HashMap<String, &'a StateDef>,
    parent: HashMap<String, Option<String>>,
}

impl<'a> Tree<'a> {
    pub fn new(states: &'a [StateDef]) -> Self {
        let mut tree = Tree {
            leaves: Vec::new(),
            all: Vec::new(),
            by_name: HashMap::new(),
            parent: HashMap::new(),
        };
        fn walk<'a>(states: &'a [StateDef], parent: Option<String>, tree: &mut Tree<'a>) {
            for s in states {
                tree.all.push(s);
                tree.by_name.insert(s.name.to_string(), s);
                tree.parent.insert(s.name.to_string(), parent.clone());
                if s.children.is_empty() {
                    tree.leaves.push(s);
                } else {
                    walk(&s.children, Some(s.name.to_string()), tree);
                }
            }
        }
        walk(states, None, &mut tree);
        tree
    }

    pub fn get(&self, name: &str) -> Option<&'a StateDef> {
        self.by_name.get(name).copied()
    }

    pub fn parent_of(&self, name: &str) -> Option<&str> {
        self.parent.get(name)?.as_deref()
    }

    /// Number of ancestors; top-level states have depth 0.
    pub fn depth(&self, name: &str) -> i32 {
        let mut d = 0;
        let mut cur = self.parent_of(name);
        while let Some(p) = cur {
            d += 1;
            cur = self.parent_of(p);
        }
        d
    }

    /// All descendant leaves of a state (the state itself if it is a leaf).
    pub fn leaves_under(&self, name: &str) -> Vec<&'a StateDef> {
        match self.get(name) {
            Some(s) if s.children.is_empty() => vec![s],
            _ => self
                .leaves
                .iter()
                .copied()
                .filter(|l| self.is_ancestor(name, &l.name.to_string()))
                .collect(),
        }
    }

    pub fn is_ancestor(&self, ancestor: &str, name: &str) -> bool {
        let mut cur = self.parent_of(name);
        while let Some(p) = cur {
            if p == ancestor {
                return true;
            }
            cur = self.parent_of(p);
        }
        false
    }

    /// Follows initial-child chains down to a leaf. Returns the state itself
    /// when it is a leaf. Assumes initial markers are already validated.
    pub fn resolve_initial(&self, name: &str) -> &'a StateDef {
        let mut cur = self.get(name).expect("state exists");
        while !cur.children.is_empty() {
            cur = cur
                .children
                .iter()
                .find(|c| c.initial)
                .expect("validated: exactly one initial child");
        }
        cur
    }

    /// The machine's initial leaf: the top-level starred state (or the first
    /// top-level state) resolved down to a leaf.
    pub fn initial_leaf(&self, top_level: &'a [StateDef]) -> &'a StateDef {
        let start = top_level
            .iter()
            .find(|s| s.initial)
            .unwrap_or(&top_level[0]);
        self.resolve_initial(&start.name.to_string())
    }

    /// Ancestor chain including the state itself, innermost first.
    fn chain<'b>(&'b self, name: &'b str) -> Vec<&'b str> {
        let mut out = Vec::new();
        let mut cur = Some(name);
        while let Some(n) = cur {
            out.push(n);
            cur = self.parent_of(n);
        }
        out
    }

    /// Least common ancestor of two leaves. `None` means the virtual root
    /// (no shared composite). For `a == b` the parent is returned: a
    /// self-transition exits and re-enters the leaf only.
    pub fn lca(&self, a: &str, b: &str) -> Option<String> {
        if a == b {
            return self.parent_of(a).map(str::to_string);
        }
        let chain_a = self.chain(a);
        let chain_b: Vec<&str> = self.chain(b);
        chain_a
            .into_iter()
            .find(|n| chain_b.contains(n))
            .map(str::to_string)
    }

    /// Exit hooks from `from` up to (excluding) the LCA, innermost first.
    pub fn exit_path(&self, from: &str, to: &str) -> Vec<&'a StateDef> {
        let lca = self.lca(from, to);
        self.chain(from)
            .into_iter()
            .take_while(|n| Some(n.to_string()) != lca)
            .filter_map(|n| self.get(n))
            .collect()
    }

    /// Entry hooks from (excluding) the LCA down to `to`, outermost first.
    pub fn entry_path(&self, from: &str, to: &str) -> Vec<&'a StateDef> {
        let lca = self.lca(from, to);
        let mut path: Vec<&'a StateDef> = self
            .chain(to)
            .into_iter()
            .take_while(|n| Some(n.to_string()) != lca)
            .filter_map(|n| self.get(n))
            .collect();
        path.reverse();
        path
    }
}

/// Finds a state by name anywhere in the tree, mutably.
pub fn find_mut<'a>(states: &'a mut [StateDef], name: &str) -> Option<&'a mut StateDef> {
    for s in states.iter_mut() {
        if s.name == name {
            return Some(s);
        }
        if let Some(found) = find_mut(&mut s.children, name) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::Ident;

    fn state(name: &str, initial: bool, children: Vec<StateDef>) -> StateDef {
        StateDef {
            name: Ident::new(name, proc_macro2::Span::call_site()),
            initial,
            entry: None,
            exit: None,
            children,
        }
    }

    // Idle, Active { *Charging, Discharging { *Slow, Fast } }, Done
    fn fixture() -> Vec<StateDef> {
        vec![
            state("Idle", true, vec![]),
            state(
                "Active",
                false,
                vec![
                    state("Charging", true, vec![]),
                    state(
                        "Discharging",
                        false,
                        vec![state("Slow", true, vec![]), state("Fast", false, vec![])],
                    ),
                ],
            ),
            state("Done", false, vec![]),
        ]
    }

    fn names(path: Vec<&StateDef>) -> Vec<String> {
        path.iter().map(|s| s.name.to_string()).collect()
    }

    #[test]
    fn leaves_and_depth() {
        let states = fixture();
        let tree = Tree::new(&states);
        let leaves: Vec<String> = tree.leaves.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(leaves, ["Idle", "Charging", "Slow", "Fast", "Done"]);
        assert_eq!(tree.depth("Idle"), 0);
        assert_eq!(tree.depth("Charging"), 1);
        assert_eq!(tree.depth("Slow"), 2);
    }

    #[test]
    fn resolve_initial_drills_down() {
        let states = fixture();
        let tree = Tree::new(&states);
        assert_eq!(tree.resolve_initial("Active").name, "Charging");
        assert_eq!(tree.resolve_initial("Discharging").name, "Slow");
        assert_eq!(tree.resolve_initial("Done").name, "Done");
        assert_eq!(tree.initial_leaf(&states).name, "Idle");
    }

    #[test]
    fn lca_cases() {
        let states = fixture();
        let tree = Tree::new(&states);
        // siblings in one composite
        assert_eq!(tree.lca("Slow", "Fast").as_deref(), Some("Discharging"));
        // cross-boundary
        assert_eq!(tree.lca("Charging", "Done"), None);
        // nested vs. other branch of same composite
        assert_eq!(tree.lca("Charging", "Slow").as_deref(), Some("Active"));
        // self -> parent
        assert_eq!(tree.lca("Slow", "Slow").as_deref(), Some("Discharging"));
        // top-level self -> root
        assert_eq!(tree.lca("Idle", "Idle"), None);
    }

    #[test]
    fn exit_entry_paths() {
        let states = fixture();
        let tree = Tree::new(&states);

        // sibling move inside Discharging: only the two leaves
        assert_eq!(names(tree.exit_path("Slow", "Fast")), ["Slow"]);
        assert_eq!(names(tree.entry_path("Slow", "Fast")), ["Fast"]);

        // cross-boundary: Slow -> Done
        assert_eq!(
            names(tree.exit_path("Slow", "Done")),
            ["Slow", "Discharging", "Active"]
        );
        assert_eq!(names(tree.entry_path("Slow", "Done")), ["Done"]);

        // entering a composite from outside: Idle -> Slow (target resolved)
        assert_eq!(names(tree.exit_path("Idle", "Slow")), ["Idle"]);
        assert_eq!(
            names(tree.entry_path("Idle", "Slow")),
            ["Active", "Discharging", "Slow"]
        );

        // self transition: exit and re-enter the leaf only
        assert_eq!(names(tree.exit_path("Slow", "Slow")), ["Slow"]);
        assert_eq!(names(tree.entry_path("Slow", "Slow")), ["Slow"]);
    }

    #[test]
    fn find_mut_recurses() {
        let mut states = fixture();
        assert!(!states[1].children[1].children[1].initial); // Fast
        let found = find_mut(&mut states, "Fast").expect("found");
        found.initial = true;
        assert!(states[1].children[1].children[1].initial);
    }
}
