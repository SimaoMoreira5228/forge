use std::collections::VecDeque;

use forge_diagnostics::ForgeDiagnostic;
use parking_lot::{Condvar, Mutex};

use crate::build::planner::ActionDag;

pub fn execute_dag<T: Send + Sync>(
	dag: &ActionDag,
	shared: &T,
	run: impl Fn(&T, usize) -> Result<(), ForgeDiagnostic> + Sync,
) -> Result<(), ForgeDiagnostic> {
	validate_dag(dag)?;
	struct State {
		ready: VecDeque<usize>,
		indegree: Vec<usize>,
		pending: usize,
		in_flight: usize,
		failure: Option<ForgeDiagnostic>,
	}

	let indegree: Vec<usize> = dag.deps.iter().map(Vec::len).collect();
	let dependents = dag.dependents();
	let state = Mutex::new(State {
		ready: indegree
			.iter()
			.enumerate()
			.filter_map(|(i, d)| (*d == 0).then_some(i))
			.collect(),
		indegree,
		pending: dag.specs.len(),
		in_flight: 0,
		failure: None,
	});
	let progress = Condvar::new();

	rayon::scope(|scope| {
		let workers = rayon::current_num_threads().min(dag.specs.len().max(1));
		for _ in 0..workers {
			scope.spawn(|_| {
				loop {
					let next = {
						let mut state = state.lock();
						loop {
							if state.failure.is_some() || state.pending == 0 {
								break None;
							}
							if let Some(index) = state.ready.pop_front() {
								state.in_flight += 1;
								break Some(index);
							}
							if state.in_flight == 0 {
								state.failure = Some(ForgeDiagnostic::error(
									3,
									"action scheduling stalled: no runnable action and nothing in flight",
								));
								progress.notify_all();
								break None;
							}
							progress.wait(&mut state);
						}
					};

					let Some(index) = next else { break };

					let result = run(shared, index);
					let mut state = state.lock();
					if let Err(e) = result {
						state.failure.get_or_insert(e);
					}
					if state.failure.is_none() {
						for &dependent in &dependents[index] {
							state.indegree[dependent] -= 1;
							if state.indegree[dependent] == 0 {
								state.ready.push_back(dependent);
								progress.notify_one();
							}
						}
					}
					state.in_flight -= 1;
					state.pending -= 1;
					if state.pending == 0 || state.failure.is_some() {
						progress.notify_all();
					}
				}
			});
		}
	});

	match state.into_inner().failure {
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

	use forge_core::{ActionSpec, ConfigTransition, OutputDeclaration, OutputKind};

	use super::*;

	fn action(name: &str) -> ActionSpec {
		ActionSpec {
			name: name.into(),
			component: name.into(),
			configuration: ConfigTransition::Target,
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
			compile_command: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: BTreeMap::new(),
			toolchain_id: None,
			worker: None,
		}
	}

	fn stress_scheduler(failing: Option<usize>) {
		use std::sync::atomic::{AtomicUsize, Ordering};
		use std::sync::{Barrier, mpsc};
		use std::time::Duration;

		let (done, finished) = mpsc::channel();
		let thread = std::thread::spawn(move || {
			let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
			let dag = ActionDag {
				specs: (0..10).map(|i| action(&i.to_string())).collect(),
				deps: vec![
					vec![],
					vec![],
					vec![],
					vec![],
					vec![0],
					vec![1],
					vec![2],
					vec![3],
					vec![4, 5, 6, 7],
					vec![8],
				],
			};
			for iteration in 0..512 {
				let started: Vec<AtomicUsize> = (0..10).map(|_| AtomicUsize::new(0)).collect();
				let completed: Vec<AtomicUsize> = (0..10).map(|_| AtomicUsize::new(0)).collect();
				let roots = Barrier::new(4);
				let result = pool.install(|| {
					execute_dag(&dag, &(), |_, index| {
						assert_eq!(started[index].fetch_add(1, Ordering::SeqCst), 0);
						for &dependency in &dag.deps[index] {
							assert_eq!(completed[dependency].load(Ordering::SeqCst), 1);
						}
						if index < 4 {
							roots.wait();
						}
						std::thread::yield_now();
						if (index == 9 || failing == Some(index)) && iteration % 4 == 0 {
							std::thread::sleep(Duration::from_micros(100));
						}
						if failing == Some(index) {
							return Err(ForgeDiagnostic::error(3, "scheduler regression failure"));
						}
						completed[index].store(1, Ordering::SeqCst);
						Ok(())
					})
				});
				if let Some(index) = failing {
					assert_eq!(result.unwrap_err().message, "scheduler regression failure");
					assert_eq!(started[index].load(Ordering::SeqCst), 1);
					assert_eq!(started[9].load(Ordering::SeqCst), 0);
					for (action, dependencies) in dag.deps.iter().enumerate() {
						if dependencies.contains(&index) {
							assert_eq!(started[action].load(Ordering::SeqCst), 0);
						}
					}
				} else {
					result.unwrap();
					assert!(completed.iter().all(|count| count.load(Ordering::SeqCst) == 1));
				}
			}
			done.send(()).unwrap();
		});
		finished
			.recv_timeout(Duration::from_secs(15))
			.expect("scheduler did not finish");
		thread.join().unwrap();
	}

	#[test]
	fn final_completion_wakes_competing_workers() {
		stress_scheduler(None);
	}

	#[test]
	fn failure_wakes_competing_workers_without_running_dependents() {
		stress_scheduler(Some(0));
		stress_scheduler(Some(8));
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
