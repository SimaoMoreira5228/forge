use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use forge_diagnostics::ForgeDiagnostic;
use parking_lot::{Condvar, Mutex};

use crate::planner::ActionDag;

pub fn execute_dag<T: Send + Sync>(
	dag: &ActionDag,
	shared: &T,
	run: impl Fn(&T, usize) -> Result<(), ForgeDiagnostic> + Sync,
) -> Result<(), ForgeDiagnostic> {
	validate_dag(dag)?;
	let pending = AtomicUsize::new(dag.specs.len());
	let in_flight = AtomicUsize::new(0);
	let indegree: Vec<AtomicUsize> = dag.deps.iter().map(|d| AtomicUsize::new(d.len())).collect();
	let dependents = dag.dependents();

	let ready: Mutex<VecDeque<usize>> = Mutex::new(
		indegree
			.iter()
			.enumerate()
			.filter(|(_, d)| d.load(Ordering::SeqCst) == 0)
			.map(|(i, _)| i)
			.collect(),
	);
	let progress = Condvar::new();
	let failure: Mutex<Option<ForgeDiagnostic>> = Mutex::new(None);

	rayon::scope(|scope| {
		let workers = rayon::current_num_threads().min(dag.specs.len().max(1));
		for _ in 0..workers {
			scope.spawn(|_| {
				loop {
					if failure.lock().is_some() {
						break;
					}

					let next = {
						let mut queue = ready.lock();
						loop {
							if failure.lock().is_some() {
								break None;
							}
							if let Some(index) = queue.pop_front() {
								in_flight.fetch_add(1, Ordering::AcqRel);
								break Some(index);
							}
							if pending.load(Ordering::Acquire) == 0 {
								break None;
							}
							if in_flight.load(Ordering::Acquire) == 0 {
								let mut slot = failure.lock();
								if slot.is_none() {
									*slot = Some(ForgeDiagnostic::error(
										3,
										"action scheduling stalled: no runnable action and nothing in flight",
									));
								}
								drop(slot);
								progress.notify_all();
								break None;
							}
							progress.wait(&mut queue);
						}
					};

					let Some(index) = next else { break };

					if let Err(e) = run(shared, index) {
						let mut slot = failure.lock();
						if slot.is_none() {
							*slot = Some(e);
						}
						drop(slot);
						progress.notify_all();
					}

					if failure.lock().is_none() {
						for &dependent in &dependents[index] {
							if indegree[dependent].fetch_sub(1, Ordering::AcqRel) == 1 {
								ready.lock().push_back(dependent);
								progress.notify_one();
							}
						}
					}

					in_flight.fetch_sub(1, Ordering::AcqRel);
					if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
						progress.notify_all();
					}
				}
			});
		}
	});

	match failure.into_inner() {
		Some(e) => Err(e),
		None => Ok(()),
	}
}

fn validate_dag(dag: &ActionDag) -> Result<(), ForgeDiagnostic> {
	let mut indegree: Vec<usize> = dag.deps.iter().map(Vec::len).collect();
	let mut ready: Vec<usize> = indegree
		.iter()
		.enumerate()
		.filter_map(|(index, degree)| (*degree == 0).then_some(index))
		.collect();
	let dependents = dag.dependents();
	let mut visited = 0;
	while let Some(index) = ready.pop() {
		visited += 1;
		for &dependent in &dependents[index] {
			indegree[dependent] -= 1;
			if indegree[dependent] == 0 {
				ready.push(dependent);
			}
		}
	}
	if visited == dag.specs.len() {
		Ok(())
	} else {
		Err(ForgeDiagnostic::error(3, "action dependency cycle detected"))
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use forge_core::{ActionSpec, OutputDeclaration, OutputKind};

	use super::*;

	fn action(name: &str) -> ActionSpec {
		ActionSpec {
			name: name.into(),
			component: name.into(),
			command: "true".into(),
			args: Vec::new(),
			inputs: Vec::new(),
			execution_deps: Vec::new(),
			outputs: vec![OutputDeclaration {
				path: name.into(),
				kind: OutputKind::File,
			}],
			workdir: None,
			is_test: false,
			stdout: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: BTreeMap::new(),
			toolchain_id: None,
		}
	}

	#[test]
	fn rejects_action_cycles_before_workers_start() {
		let dag = ActionDag {
			specs: vec![action("a"), action("b")],
			deps: vec![vec![1], vec![0]],
		};
		assert!(execute_dag(&dag, &(), |_, _| Ok(())).is_err());
	}
}
