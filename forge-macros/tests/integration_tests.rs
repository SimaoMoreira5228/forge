use forge_macros::lua_api;
use mlua::{UserData, UserDataMethods};

struct StringUtils;

#[lua_api(name = "string_utils")]
impl StringUtils {
	fn split(input: String, delimiter: String) -> Vec<String> {
		input.split(&delimiter).map(|s| s.to_string()).collect()
	}

	fn join(parts: Vec<String>, delimiter: String) -> String {
		parts.join(&delimiter)
	}

	fn to_upper(input: String) -> String {
		input.to_uppercase()
	}
}

#[derive(Clone)]
struct Calculator {
	base_value: f64,
}

impl UserData for Calculator {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {
		// This is just to satisfy the trait requirement for testing
	}
}

#[lua_api(name = "calculator")]
impl Calculator {
	fn add(&self, value: f64) -> f64 {
		self.base_value + value
	}

	fn multiply(&self, value: f64) -> f64 {
		self.base_value * value
	}

	fn get_base(&self) -> f64 {
		self.base_value
	}

	fn new(base: f64) -> Calculator {
		Calculator { base_value: base }
	}
}

#[test]
fn test_static_methods_compile() {
	let type_defs = StringUtils::string_utils_lua_type_definitions();
	assert!(type_defs.contains("string_utils"));
	assert!(type_defs.contains("split"));
	assert!(type_defs.contains("join"));
	assert!(type_defs.contains("to_upper"));
}

#[test]
fn test_instance_methods_compile() {
	let type_defs = Calculator::calculator_lua_type_definitions();
	assert!(type_defs.contains("calculator"));
	assert!(type_defs.contains("add"));
	assert!(type_defs.contains("multiply"));
	assert!(type_defs.contains("get_base"));
	assert!(type_defs.contains("new"));
}

#[test]
fn test_type_definitions_content() {
	let type_defs = StringUtils::string_utils_lua_type_definitions();

	assert!(type_defs.contains("split fun(input: string, delimiter: string): string[]"));
	assert!(type_defs.contains("join fun(parts: string[], delimiter: string): string"));

	let calc_type_defs = Calculator::calculator_lua_type_definitions();

	assert!(calc_type_defs.contains("add fun(self: Calculator, value: number): number"));
	assert!(calc_type_defs.contains("multiply fun(self: Calculator, value: number): number"));
	assert!(calc_type_defs.contains("new fun(base: number): any"));
}

#[test]
fn test_generated_functions_exist() {
	let _static_fn: fn(&mlua::Lua) -> mlua::Result<mlua::Table> = StringUtils::create_string_utils_table;

	let _instance_fn: fn(&Calculator, &mlua::Lua) -> mlua::Result<mlua::Table> = Calculator::create_calculator_table;

	let _calc_static_fn: fn(&mlua::Lua) -> mlua::Result<mlua::Table> = Calculator::create_static_table;
}

#[test]
fn test_type_definitions_format() {
	let type_defs = StringUtils::string_utils_lua_type_definitions();

	assert!(type_defs.contains("---@class String_utils"));

	assert!(type_defs.contains("---@type String_utils"));
	assert!(type_defs.contains("string_utils = nil"));

	let calc_type_defs = Calculator::calculator_lua_type_definitions();

	assert!(type_defs.contains("---@class Calculator") || calc_type_defs.contains("---@class Calculator"));

	assert!(calc_type_defs.contains("---@type Calculator"));
	assert!(calc_type_defs.contains("calculator = nil"));
}
