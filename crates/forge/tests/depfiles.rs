use std::path::PathBuf;

use forge_engine::Engine;

#[test]
fn planning_depfile_inputs_uses_engine_workspace_not_cwd() {
	let dir = std::env::temp_dir().join(format!("forge-depfile-plan-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("tools/bin")).unwrap();
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(dir.join("tools/bin/gcc"), "not an executable compiler").unwrap();
	std::fs::write(dir.join("src/main.c"), "int main(void) { return 0; }\n").unwrap();
	std::fs::write(dir.join("FORGE.toml"), "[binary.app]\nsrcs = [\"src/main.c\"]\n").unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!(
			"[project]\nname = \"depfile\"\n[discovery]\ninclude = [\".\"]\n[toolchains.gcc]\nfrom = \"path\"\npath = {:?}\n",
			dir.join("tools").to_str().unwrap()
		),
	)
	.unwrap();
	assert_ne!(std::env::current_dir().unwrap(), dir);
	let engine = Engine::open(&dir);
	let (_, first) = engine.plan_dag("debug").unwrap();
	let compile = first.specs.iter().find(|s| s.name == "compile src/main.c").unwrap();
	assert_eq!(compile.inputs, [PathBuf::from("src/main.c")]);
	let depfile = &compile
		.outputs
		.iter()
		.find(|o| o.path.extension().is_some_and(|e| e == "d"))
		.unwrap()
		.path;
	std::fs::create_dir_all(dir.join(depfile).parent().unwrap()).unwrap();
	std::fs::write(dir.join(depfile), format!(
		"out.o out.d: src/main.c ./src/../unusual.schema extensionless space\\ name hash\\#name dollar$$name back\\slash \\\n FORGE_EXEC_ROOT/token {} {} {}\nunusual.schema:\nextensionless:\nother: missing.data\n",
		dir.join("absolute").display(), dir.join("forge-out/exec/namespaced").display(),
		dir.join("forge-out/sandbox/123456789abc/plain").display()
	)).unwrap();
	let (_, planned) = engine.plan_dag("debug").unwrap();
	let compile = planned.specs.iter().find(|s| s.name == "compile src/main.c").unwrap();
	let mut expected: Vec<PathBuf> = [
		"src/main.c",
		"unusual.schema",
		"extensionless",
		"space name",
		"hash#name",
		"dollar$name",
		r"back\slash",
		"token",
		"absolute",
		"namespaced",
		"plain",
		"missing.data",
	]
	.into_iter()
	.map(PathBuf::from)
	.collect();
	expected.sort();
	assert_eq!(compile.inputs, expected);
	assert!(!dir.join("unusual.schema").exists());
	assert!(!dir.join("forge-out/bin/debug/app").exists());
	assert!(forge_engine::hasher::hash_inputs(&dir, &compile.inputs, &forge_engine::hasher::HashCache::default()).is_err());
	std::fs::remove_file(dir.join(depfile)).unwrap();
	std::fs::create_dir(dir.join(depfile)).unwrap();
	let error = engine.plan_dag("debug").err().expect("directory depfile must fail planning");
	assert!(error.to_string().contains("cannot read depfile"), "{error}");
	std::fs::remove_dir_all(dir).unwrap();
}
