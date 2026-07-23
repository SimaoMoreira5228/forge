use std::collections::{BTreeMap, BTreeSet};

use rhai::{Array, Dynamic, Map};

pub type Adjacency = BTreeMap<String, Vec<String>>;

pub fn adjacency_from_rhai(map: &Map) -> Adjacency {
	map.iter()
		.map(|(key, value)| {
			let deps = value
				.clone()
				.try_cast::<Array>()
				.unwrap_or_default()
				.into_iter()
				.filter_map(|dep| dep.into_string().ok())
				.collect::<Vec<_>>();
			(key.to_string(), deps)
		})
		.collect()
}

pub fn keys_to_rhai(keys: impl IntoIterator<Item = String>) -> Array {
	keys.into_iter().map(Dynamic::from).collect()
}

pub fn roots(adjacency: &Adjacency) -> Vec<String> {
	let referenced: BTreeSet<&String> = adjacency.values().flatten().collect();
	adjacency.keys().filter(|key| !referenced.contains(*key)).cloned().collect()
}

pub fn reachable(adjacency: &Adjacency, roots: &[String]) -> Vec<String> {
	let mut seen: BTreeSet<String> = BTreeSet::new();
	let mut stack: Vec<String> = roots.to_vec();
	while let Some(key) = stack.pop() {
		if !seen.insert(key.clone()) {
			continue;
		}
		if let Some(deps) = adjacency.get(&key) {
			for dep in deps {
				if !seen.contains(dep) {
					stack.push(dep.clone());
				}
			}
		}
	}
	seen.into_iter().collect()
}

pub fn transitive(adjacency: &Adjacency, start: &str) -> Vec<String> {
	let mut seen: BTreeSet<String> = BTreeSet::new();
	let mut stack: Vec<String> = adjacency.get(start).cloned().unwrap_or_default();
	while let Some(key) = stack.pop() {
		if !seen.insert(key.clone()) {
			continue;
		}
		if let Some(deps) = adjacency.get(&key) {
			for dep in deps {
				if !seen.contains(dep) {
					stack.push(dep.clone());
				}
			}
		}
	}
	seen.into_iter().collect()
}

pub fn reverse(adjacency: &Adjacency) -> Adjacency {
	let mut parents: BTreeMap<String, Vec<String>> = BTreeMap::new();
	for key in adjacency.keys() {
		parents.entry(key.clone()).or_default();
	}
	for (key, deps) in adjacency {
		for dep in deps {
			parents.entry(dep.clone()).or_default().push(key.clone());
		}
	}
	for list in parents.values_mut() {
		list.sort();
		list.dedup();
	}
	parents
}

pub fn toposort(adjacency: &Adjacency) -> Vec<String> {
	let mut indegree: BTreeMap<String, usize> = adjacency.keys().map(|key| (key.clone(), 0)).collect();
	for deps in adjacency.values() {
		for dep in deps {
			if let Some(count) = indegree.get_mut(dep) {
				*count += 1;
			}
		}
	}
	let mut ready: Vec<String> = indegree
		.iter()
		.filter(|(_, count)| **count == 0)
		.map(|(key, _)| key.clone())
		.collect();
	ready.sort();
	let mut order = Vec::new();
	while let Some(key) = ready.pop() {
		order.push(key.clone());
		if let Some(deps) = adjacency.get(&key) {
			for dep in deps {
				if let Some(count) = indegree.get_mut(dep) {
					*count -= 1;
					if *count == 0 {
						ready.push(dep.clone());
					}
				}
			}
		}
	}
	for key in adjacency.keys() {
		if !order.contains(key) {
			order.push(key.clone());
		}
	}
	order
}

#[cfg(test)]
mod tests {
	use super::*;

	fn adjacency(rows: &[(&str, &[&str])]) -> Adjacency {
		rows.iter()
			.map(|(key, deps)| (key.to_string(), deps.iter().map(|d| d.to_string()).collect()))
			.collect()
	}

	#[test]
	fn roots_reachable_and_transitive() {
		let graph = adjacency(&[("app", &["mid"]), ("mid", &["base"]), ("base", &[]), ("orphan", &["base"])]);
		let roots = roots(&graph);
		assert_eq!(roots, vec!["app".to_string(), "orphan".to_string()]);
		let reachable = reachable(&graph, &roots);
		assert_eq!(reachable, vec!["app", "base", "mid", "orphan"]);
		assert_eq!(transitive(&graph, "app"), vec!["base".to_string(), "mid".to_string()]);
	}

	#[test]
	fn topo_orders_parents_before_children() {
		let graph = adjacency(&[("app", &["mid"]), ("mid", &["base"]), ("base", &[])]);
		let order = toposort(&graph);
		let position = |key: &str| order.iter().position(|k| k == key).unwrap();
		assert!(position("app") < position("mid"));
		assert!(position("mid") < position("base"));
	}
}
